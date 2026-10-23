// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: source-ordered register arrays and draw snapshots.

use super::{
    Action, CommandLimits, CommandPlan, CommandUsage, Draw, Error, Flip, GraphicsState,
    MemoryAction,
};
use crate::{
    Header, RegisterSpace,
    compute::{CommandRange, Memory},
    registers::{self, Program},
};

/// An action from a complete immutable flat stream. Array contents are read
/// only by resolve_array at this action's actual execution point.
// Inline sealed state avoids a fallible allocation after an earlier effect.
#[allow(clippy::large_enum_variant)]
pub enum OrderedStep {
    Array,
    IndirectDraw,
    WaitFlip(crate::WaitFlip),
    LodStats(crate::lod_stats_request::Request),
    Draw(Draw),
    Memory(usize),
    Flip(Flip),
}

pub struct OrderedCommands {
    program: Program,
    plan: CommandPlan,
    state: GraphicsState,
    packet: usize,
    action: usize,
    pending: Option<crate::IndirectRegisters>,
    pending_indirect: Option<super::IndirectDraw>,
    max_vertices: u64,
    failed: bool,
    finished: bool,
}
impl OrderedCommands {
    pub fn prepare(
        buffers: &[&[u8]],
        roots: &[CommandRange],
        initial: GraphicsState,
        limits: &CommandLimits,
    ) -> Result<Self, Error> {
        let program = Program::new(
            buffers,
            roots,
            registers::Limits {
                buffers: limits.max_buffers,
                command_bytes: limits.max_command_bytes,
                packets: limits.max_packets,
                host: limits.max_host_bytes,
                work: limits.max_work,
            },
        )
        .map_err(convert)?;
        Self::from_program(program, initial, limits)
    }

    pub(super) fn captured(
        capture: &mut super::CapturedCommands,
        initial: GraphicsState,
        limits: &CommandLimits,
    ) -> Result<Self, Error> {
        let program = Program::captured(
            capture,
            registers::Limits {
                buffers: limits.max_buffers,
                command_bytes: limits.max_command_bytes,
                packets: limits.max_packets,
                host: limits.max_host_bytes,
                work: limits.max_work,
            },
        )
        .map_err(convert)?;
        let mut ordered = Self::from_program(program, initial, limits)
            .map_err(|error| capture.source_error(error))?;
        let usage = capture.usage();
        ordered
            .plan
            .source_usage(usage.command_bytes, usage.packets);
        let provenance = capture.origins_host_bytes()?;
        ordered
            .program
            .usage
            .host
            .checked_add(provenance)
            .ok_or(Error::Limit("graphics provenance capacity"))?;
        capture.transfer_origins(&mut ordered.plan)?;
        Ok(ordered)
    }

    fn from_program(
        mut program: Program,
        initial: GraphicsState,
        limits: &CommandLimits,
    ) -> Result<Self, Error> {
        program
            .charge(
                std::mem::size_of::<Self>(),
                std::mem::size_of::<Self>() as u64,
            )
            .map_err(convert)?;
        let sanitized = program
            .sanitized(std::mem::size_of::<GraphicsState>())
            .map_err(convert)?;
        let (host, work) = program.remaining();
        let mut static_limits = *limits;
        static_limits.max_host_bytes = host;
        static_limits.max_work = work;
        let plan = super::commands::plan_ordered_commands(&[&sanitized], initial, &static_limits)?;
        if program
            .array_count()
            .checked_add(plan.actions().len())
            .is_none_or(|count| {
                limits
                    .max_draws
                    .checked_mul(2)
                    .is_none_or(|maximum| count > maximum)
            })
        {
            return Err(Error::Limit("ordered graphics actions"));
        }
        program
            .charge(plan.usage().host_bytes, plan.usage().work)
            .map_err(convert)?;
        Ok(Self {
            program,
            plan,
            state: initial,
            packet: 0,
            action: 0,
            pending: None,
            pending_indirect: None,
            max_vertices: limits.max_vertices,
            failed: false,
            finished: false,
        })
    }
    /// Existing sealed stream and actual ordered cursor state, not completion
    /// or the static plan's future final state. No copy, read or new authority.
    pub fn diagnostic_view(&self) -> (&[u8], &GraphicsState) {
        (self.program.diagnostic_bytes(), &self.state)
    }

    pub fn static_actions(&self) -> &[Action] {
        self.plan.actions()
    }
    pub fn has_arrays(&self) -> bool {
        self.program.has_arrays()
    }
    pub fn usage(&self) -> CommandUsage {
        let mut usage = self.plan.usage();
        // Provenance was moved from the already-admitted capture after static
        // planning, not allocated a second time inside the register program.
        usage.host_bytes = self.program.usage.host + self.plan.provenance_bytes();
        usage.work = self.program.usage.work;
        // Sanitization preserves packet extents; retain the complete static
        // packet count, including compound wait packets and each array slot.
        usage
    }
    pub fn command_ranges(&self) -> &[CommandRange] {
        self.program.ranges()
    }
    /// Retains the actual packet location across an array/draw late failure.
    pub fn current_source(&self) -> Option<super::CommandLocation> {
        let packet = self.program.packet(self.packet.checked_sub(1)?)?;
        self.plan.command_location(packet.at / 4)
    }
    pub fn memory_action(&self, index: usize) -> &MemoryAction {
        self.plan.memory_action(index)
    }
    pub fn final_state(&self) -> Result<GraphicsState, Error> {
        if self.failed
            || !self.finished
            || self.pending.is_some()
            || self.pending_indirect.is_some()
            || self.program.packet(self.packet).is_some()
        {
            return Err(Error::Unsupported("unfinished ordered graphics stream"));
        }
        Ok(self.state)
    }
    pub fn resolve_array(&mut self, memory: &impl Memory) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Unsupported("failed ordered graphics stream"));
        }
        self.failed = true;
        let array = self
            .pending
            .take()
            .ok_or(Error::Unsupported("no current graphics register array"))?;
        self.program
            .resolve(array, memory, |space, offset, value| {
                self.state.write(space, offset, value).map_err(reverse)
            })
            .map_err(convert)?;
        self.failed = false;
        Ok(())
    }
    pub fn resolve_indirect(&mut self, memory: &impl Memory) -> Result<Option<Draw>, Error> {
        if self.failed {
            return Err(Error::Unsupported("failed ordered graphics stream"));
        }
        self.failed = true;
        let descriptor = self
            .pending_indirect
            .take()
            .ok_or(Error::Unsupported("no current indirect draw"))?;
        let result = descriptor.resolve(&mut self.state, memory, self.max_vertices)?;
        self.failed = false;
        Ok(result)
    }
    pub fn next_step(&mut self) -> Result<Option<OrderedStep>, Error> {
        if self.failed || self.pending.is_some() || self.pending_indirect.is_some() {
            return Err(Error::Unsupported("unresolved ordered graphics stream"));
        }
        self.failed = true;
        let result = self.advance();
        if result.is_ok() {
            self.failed = false;
        }
        result
    }
    fn advance(&mut self) -> Result<Option<OrderedStep>, Error> {
        while let Some(packet) = self.program.packet(self.packet) {
            self.packet += 1;
            let header = Header::new(self.program.word(packet, 0));
            if registers::indirect_opcode(header) {
                self.pending = Some(self.program.array(packet).map_err(convert)?);
                return Ok(Some(OrderedStep::Array));
            }
            let word = |index| self.program.word(packet, index);
            if crate::context_state::packet_words(header.raw()).is_some() {
                let mut bytes = [0u8; 20];
                for index in 0..packet.words {
                    bytes[index * 4..index * 4 + 4].copy_from_slice(&word(index).to_le_bytes());
                }
                let operation = crate::context_state::decode(&bytes[..packet.words * 4])
                    .map_err(Error::Unsupported)?;
                self.state.context_operation(operation)?;
                continue;
            }
            if crate::queue_reset::is_header(header.raw()) {
                let reset = crate::queue_reset::decode(header.raw(), word(1))
                    .map_err(Error::Unsupported)?;
                self.state.reset(reset);
                continue;
            }
            if matches!(header.opcode(), 0x69 | 0x76 | 0x79) && header.raw() != 0xc002_7904 {
                let space = match header.opcode() {
                    0x69 => RegisterSpace::Context,
                    0x76 => RegisterSpace::Sh,
                    _ => RegisterSpace::Uconfig,
                };
                for index in 2..packet.words {
                    self.state
                        .write(space, word(1) + (index - 2) as u32, word(index))?;
                }
                continue;
            }
            match header.opcode() {
                0x11 => {
                    self.state.draw_indirect_base =
                        Some(u64::from(word(2) & !7) | (u64::from(word(3) & 0xffff) << 32))
                }
                0x2f => self.state.instances = Some(word(1)),
                0x7a => {
                    self.state.index_type = Some(if word(2) & 3 == 0 {
                        super::IndexType::U16
                    } else {
                        super::IndexType::U32
                    })
                }
                0x26 => {
                    self.state.index_base = Some(u64::from(word(1)) | (u64::from(word(2)) << 32))
                }
                0x13 => self.state.index_count = Some(word(1)),
                0x2a => {
                    self.state.index_type = Some(if word(1) == 0 {
                        super::IndexType::U16
                    } else {
                        super::IndexType::U32
                    })
                }
                _ => {}
            }
            if header.raw() == crate::lod_stats_request::HEADER {
                let Some(Action::LodStats(request)) = self.plan.actions().get(self.action) else {
                    return Err(Error::Unsupported(
                        "ordered LOD statistics action alignment",
                    ));
                };
                self.action += 1;
                return Ok(Some(OrderedStep::LodStats(*request)));
            }
            if header.raw() == 0xc005_1018 {
                let Some(Action::WaitFlip(wait)) = self.plan.actions().get(self.action) else {
                    return Err(Error::Unsupported("ordered wait-flip action alignment"));
                };
                self.action += 1;
                return Ok(Some(OrderedStep::WaitFlip(*wait)));
            }
            if header.raw() == 0xc003_2500 {
                let Some(Action::IndirectDraw(descriptor)) = self.plan.actions().get(self.action)
                else {
                    return Err(Error::Unsupported("ordered indirect action alignment"));
                };
                self.action += 1;
                self.pending_indirect = Some(*descriptor);
                return Ok(Some(OrderedStep::IndirectDraw));
            }
            let draw = matches!(header.opcode(), 0x27 | 0x35 | 0x2d);
            let memory = crate::memory_sync::packet_words(header.raw()).is_some()
                || crate::marker::packet_words(header.raw()).is_some()
                || matches!(
                    header.opcode(),
                    0x37 | 0x3c | 0x93 | 0x49 | 0x42 | 0x40 | 0x50
                )
                || matches!(header.raw(), 0xc006_1060 | 0xc002_7904);
            let flip = header.raw() == 0xc004_105c;
            if draw {
                let count = word(match header.opcode() {
                    0x27 => 4,
                    0x35 => 3,
                    _ => 1,
                });
                if count == 0 || self.state.instances == Some(0) {
                    if matches!(self.plan.actions().get(self.action), Some(Action::LiveDraw)) {
                        self.action += 1;
                    }
                    continue;
                }
            }
            if draw || memory || flip {
                let action = *self
                    .plan
                    .actions()
                    .get(self.action)
                    .ok_or(Error::Unsupported("ordered graphics action alignment"))?;
                self.action += 1;
                return Ok(Some(match action {
                    Action::Draw(_) | Action::LiveDraw if draw => {
                        // The immutable plan proves packet/action budgets. The
                        // execution product is materialized from the original
                        // packet and actual replayed state, never its snapshot.
                        OrderedStep::Draw(materialize(&self.program, packet, self.state)?)
                    }
                    Action::Memory(index) if memory => OrderedStep::Memory(index),
                    Action::Flip(value) if flip => OrderedStep::Flip(value),
                    _ => return Err(Error::Unsupported("ordered graphics action kind")),
                }));
            }
        }
        if self.action != self.plan.actions().len() {
            return Err(Error::Unsupported("unconsumed graphics actions"));
        }
        self.finished = true;
        Ok(None)
    }
}
fn convert(error: crate::compute::Error) -> Error {
    match error {
        crate::compute::Error::Memory(error) => Error::Memory(error),
        crate::compute::Error::Limit(reason) => Error::Limit(reason),
        crate::compute::Error::Allocation => Error::Allocation,
        crate::compute::Error::Unsupported(reason) => Error::Unsupported(reason),
        _ => Error::Unsupported("ordered graphics packet profile"),
    }
}
fn reverse(error: Error) -> crate::compute::Error {
    match error {
        Error::Unsupported(reason) => crate::compute::Error::Unsupported(reason),
        _ => crate::compute::Error::Unsupported("graphics indirect register"),
    }
}

fn materialize(
    program: &Program,
    packet: registers::Packet,
    state: GraphicsState,
) -> Result<Draw, Error> {
    let word = |index| program.word(packet, index);
    let opcode = Header::new(word(0)).opcode();
    let count = word(match opcode {
        0x27 => 4,
        0x35 => 3,
        _ => 1,
    });
    let instances = state
        .instances
        .ok_or(Error::Unsupported("unknown NUM_INSTANCES"))?;
    let indices = if opcode == 0x2d {
        None
    } else {
        let index_type = state
            .index_type
            .ok_or(Error::Unsupported("unknown INDEX_TYPE"))?;
        let address = if opcode == 0x35 {
            state
                .index_base
                .ok_or(Error::Unsupported("unknown INDEX_BASE"))?
                .checked_add(u64::from(word(2)) * index_type.byte_width() as u64)
                .ok_or(Error::Unsupported("index offset extent"))?
        } else {
            u64::from(word(2)) | (u64::from(word(3)) << 32)
        };
        let size = u64::from(count) * index_type.byte_width() as u64;
        if address == 0
            || !address.is_multiple_of(index_type.byte_width() as u64)
            || address.checked_add(size).is_none_or(|end| end > 1 << 47)
        {
            return Err(Error::Unsupported("index extent"));
        }
        Some(super::commands::IndexSource {
            address,
            index_type,
        })
    };
    Ok(Draw {
        native_offsets: None,
        indirect_arguments: None,
        state,
        vertex_count: count,
        instance_count: instances,
        indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::{MemoryError, MemoryWrite};
    use std::cell::{Cell, RefCell};
    struct Backing {
        base: u64,
        bytes: RefCell<Vec<u8>>,
        reads: Cell<usize>,
        denied: Cell<bool>,
    }
    impl Memory for Backing {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            self.reads.set(self.reads.get() + 1);
            if self.denied.get() {
                return Err(MemoryError::ReadDenied);
            }
            let start = usize::try_from(
                address
                    .checked_sub(self.base)
                    .ok_or(MemoryError::Unavailable)?,
            )
            .map_err(|_| MemoryError::Unavailable)?;
            let bytes = self.bytes.borrow();
            let end = start
                .checked_add(output.len())
                .ok_or(MemoryError::Unavailable)?;
            output.copy_from_slice(bytes.get(start..end).ok_or(MemoryError::Unavailable)?);
            Ok(())
        }
    }
    fn bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|word| word.to_le_bytes()).collect()
    }
    fn array(opcode: u32, address: u64, count: u32) -> [u32; 5] {
        [
            0xc003_0000 | opcode << 8,
            address as u32,
            (address >> 32) as u32,
            0x8000_0000,
            count,
        ]
    }
    fn prepare(words: &[u32]) -> OrderedCommands {
        let data = bytes(words);
        OrderedCommands::prepare(
            &[&data],
            &[CommandRange {
                address: 0x1000,
                size: data.len() as u64,
            }],
            GraphicsState::empty(),
            &CommandLimits::default(),
        )
        .unwrap()
    }
    fn write(backing: &Backing, write: &MemoryWrite) {
        let start = (write.address() - backing.base) as usize;
        backing.bytes.borrow_mut()[start..start + write.bytes().len()]
            .copy_from_slice(write.bytes());
    }
    #[test]
    fn ordered_register_arrays_supply_real_draw_state_after_writes_and_direct_overrides() {
        let base = 0x1_0000_2000;
        let backing = Backing {
            base,
            bytes: RefCell::new(vec![0; 64]),
            reads: Cell::new(0),
            denied: Cell::new(false),
        };
        let mut words = vec![0xc000_2f00, 2];
        // No shader/target/primitive values exist in the initial/direct state.
        words.extend([
            0xc00a_3700,
            0x100,
            base as u32,
            (base >> 32) as u32,
            0x7000_0008,
            11,
            0x09,
            12,
            0x318,
            0x55,
            0x242,
            3,
        ]);
        words.extend(array(0x63, base, 2));
        words.extend(array(0x9f, base + 16, 1));
        words.extend(array(0x64, base + 24, 1));
        words.extend([0xc001_7600, 0x08, 99]);
        words.extend([0xc001_2d00, 3, 2]);
        // Intervening WriteData targets a distinct future array. The prior
        // captured array remains a source exclusion, not a global write ban.
        words.extend([
            0xc004_3700,
            0x100,
            (base + 32) as u32,
            ((base + 32) >> 32) as u32,
            0x08,
            77,
        ]);
        words.extend(array(0x63, base + 32, 1));
        words.extend([0xc001_2d00, 5, 2]);
        let mut plan = prepare(&words);
        assert_eq!(backing.reads.get(), 0);
        assert_eq!(plan.command_ranges().len(), 1);
        let mut draws = 0;
        while let Some(step) = plan.next_step().unwrap() {
            match step {
                OrderedStep::Array => plan.resolve_array(&backing).unwrap(),
                OrderedStep::Memory(index) => {
                    let MemoryAction::Write(value) = plan.memory_action(index) else {
                        panic!("original write")
                    };
                    write(&backing, value);
                }
                OrderedStep::Draw(draw) => {
                    draws += 1;
                    assert_eq!(
                        draw.state.get(RegisterSpace::Sh, 0x08),
                        Ok(if draws == 1 { 99 } else { 77 })
                    );
                    assert_eq!(draw.state.get(RegisterSpace::Sh, 0x09), Ok(12));
                    assert_eq!(draw.state.get(RegisterSpace::Context, 0x318), Ok(0x55));
                    assert_eq!(draw.state.get(RegisterSpace::Uconfig, 0x242), Ok(3));
                    assert_eq!(draw.instance_count(), 2);
                    assert_eq!(draw.vertex_count(), if draws == 1 { 3 } else { 5 });
                }
                OrderedStep::Flip(_)
                | OrderedStep::IndirectDraw
                | OrderedStep::WaitFlip(_)
                | OrderedStep::LodStats(_) => {
                    panic!("no original flip/indirect draw")
                }
            }
        }
        assert_eq!(draws, 2);
        assert_eq!(backing.reads.get(), 4);
        assert_eq!(plan.command_ranges().len(), 5);
        assert_eq!(
            plan.final_state().unwrap().get(RegisterSpace::Sh, 8),
            Ok(77)
        );
    }
    #[test]
    fn arrays_decode_full_addresses_zero_count_and_fail_closed_without_replay() {
        let base = 0x2_0000_2000;
        let backing = Backing {
            base,
            bytes: RefCell::new(bytes(&[0x8000_0008, 1])),
            reads: Cell::new(0),
            denied: Cell::new(false),
        };
        let mut zero = prepare(&array(0x63, u64::MAX, 0));
        assert!(matches!(zero.next_step(), Ok(Some(OrderedStep::Array))));
        zero.resolve_array(&backing).unwrap();
        assert_eq!(backing.reads.get(), 0);
        assert!(zero.next_step().unwrap().is_none());
        let mut invalid = prepare(&array(0x63, base | 3, 0x4001));
        assert!(matches!(invalid.next_step(), Ok(Some(OrderedStep::Array))));
        // Only 0x70000000 control bits are masked; bit31 remains unknown.
        assert_eq!(
            invalid.resolve_array(&backing),
            Err(Error::Unsupported("graphics register"))
        );
        assert!(invalid.next_step().is_err());
        assert!(invalid.resolve_array(&backing).is_err());
        assert_eq!(backing.reads.get(), 1);
        let mut hole = prepare(&array(0x63, base + 4096, 1));
        assert!(matches!(hole.next_step(), Ok(Some(OrderedStep::Array))));
        assert_eq!(
            hole.resolve_array(&backing),
            Err(Error::Memory(MemoryError::Unavailable))
        );
        backing.denied.set(true);
        let mut denied = prepare(&array(0x63, base, 1));
        assert!(matches!(denied.next_step(), Ok(Some(OrderedStep::Array))));
        assert_eq!(
            denied.resolve_array(&backing),
            Err(Error::Memory(MemoryError::ReadDenied))
        );
    }
    #[test]
    fn complete_packet_grammar_and_all_quotas_are_admitted_before_array_access() {
        let good = bytes(&array(0x63, 0x2000, 1));
        let root = CommandRange {
            address: 0x1000,
            size: good.len() as u64,
        };
        for limits in [
            CommandLimits {
                max_host_bytes: 1,
                ..Default::default()
            },
            CommandLimits {
                max_work: 1,
                ..Default::default()
            },
            CommandLimits {
                max_packets: 0,
                ..Default::default()
            },
            CommandLimits {
                max_command_bytes: 19,
                ..Default::default()
            },
            CommandLimits {
                max_draws: 0,
                ..Default::default()
            },
        ] {
            assert!(
                OrderedCommands::prepare(&[&good], &[root], GraphicsState::empty(), &limits)
                    .is_err()
            );
        }
        let mut malformed = array(0x63, 0x2000, 1).to_vec();
        malformed.extend([0xc001_7600, 0x8000_0008, 2]);
        let malformed = bytes(&malformed);
        assert!(
            OrderedCommands::prepare(
                &[&malformed],
                &[CommandRange {
                    address: 0x1000,
                    size: malformed.len() as u64
                }],
                GraphicsState::empty(),
                &Default::default()
            )
            .is_err()
        );
        for words in [
            [0xc003_6300, 0, 0, 0, 0],
            [0xc003_6300, 0, 0, 0x8000_0000, 1],
            [0xc003_6300, 0, 0x8000, 0x8000_0000, 1],
        ] {
            let data = bytes(&words);
            assert!(
                OrderedCommands::prepare(
                    &[&data],
                    &[root],
                    GraphicsState::empty(),
                    &Default::default()
                )
                .is_err()
            );
        }
    }
}
