// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: ordered CS-bank register-array snapshots.

use super::{
    Command, CommandLimits, CommandPlan, CommandRange, CommandUsage, ComputeState, Dispatch, Error,
    Memory, MemoryError,
};
use crate::{
    Header, RegisterSpace,
    registers::{self, Program},
};

// Inline sealed state avoids a fallible allocation after an earlier effect.
#[allow(clippy::large_enum_variant)]
pub enum OrderedStep {
    Array,
    Dispatch(Dispatch),
    Memory(usize),
}
pub struct OrderedCommands {
    program: Program,
    plan: CommandPlan,
    state: ComputeState,
    packet: usize,
    action: usize,
    pending: Option<crate::IndirectRegisters>,
    failed: bool,
    finished: bool,
}
impl OrderedCommands {
    pub fn prepare(
        bytes: &[u8],
        root: CommandRange,
        initial: ComputeState,
        limits: &CommandLimits,
    ) -> Result<Self, Error> {
        let mut program = Program::new(
            &[bytes],
            &[root],
            registers::Limits {
                buffers: 1,
                command_bytes: limits.max_command_bytes,
                packets: limits.max_packets,
                host: limits.max_host_bytes,
                work: limits.max_work,
            },
        )?;
        if program
            .packets()
            .iter()
            .any(|packet| Header::new(program.word(*packet, 0)).opcode() == 0x3f)
        {
            return Err(Error::Unsupported(
                "indirect buffers in ordered register stream",
            ));
        }
        program.charge(
            std::mem::size_of::<Self>(),
            std::mem::size_of::<Self>() as u64,
        )?;
        let sanitized = program.sanitized(std::mem::size_of::<ComputeState>())?;
        struct Snapshot<'a> {
            bytes: &'a [u8],
            root: CommandRange,
        }
        impl Memory for Snapshot<'_> {
            fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
                if address != self.root.address || output.len() != self.bytes.len() {
                    return Err(MemoryError::Unavailable);
                }
                output.copy_from_slice(self.bytes);
                Ok(())
            }
        }
        let (host, work) = program.remaining();
        let mut static_limits = *limits;
        static_limits.max_host_bytes = host;
        static_limits.max_work = work;
        let plan = super::plan_commands(
            &initial,
            root,
            &Snapshot {
                bytes: &sanitized,
                root,
            },
            &static_limits,
        )?;
        if program
            .array_count()
            .checked_add(plan.actions.len())
            .is_none_or(|count| count > limits.max_actions)
        {
            return Err(Error::Limit("ordered compute actions"));
        }
        program.charge(plan.usage().host_bytes, plan.usage().work)?;
        Ok(Self {
            program,
            plan,
            state: initial,
            packet: 0,
            action: 0,
            pending: None,
            failed: false,
            finished: false,
        })
    }
    pub fn usage(&self) -> CommandUsage {
        let mut usage = self.plan.usage();
        usage.host_bytes = self.program.usage.host;
        usage.work = self.program.usage.work;
        usage
    }
    pub fn command_ranges(&self) -> &[CommandRange] {
        self.program.ranges()
    }
    pub fn memory_command(&self, index: usize) -> Result<Command<'_>, Error> {
        Ok(
            match self
                .plan
                .actions
                .get(index)
                .ok_or(Error::Unsupported("ordered compute action index"))?
            {
                super::commands::Action::Write(value) => Command::Write(value),
                super::commands::Action::Wait(value) => Command::Wait(value),
                super::commands::Action::Release(value) => Command::Release(value),
                super::commands::Action::Drain => Command::Drain,
                _ => return Err(Error::Unsupported("ordered compute memory action")),
            },
        )
    }
    pub fn final_state(&self) -> Result<ComputeState, Error> {
        if self.failed
            || !self.finished
            || self.pending.is_some()
            || self.program.packet(self.packet).is_some()
        {
            return Err(Error::Unsupported("unfinished ordered compute stream"));
        }
        Ok(self.state)
    }
    pub fn resolve_array(&mut self, memory: &impl Memory) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Unsupported("failed ordered compute stream"));
        }
        self.failed = true;
        let array = self
            .pending
            .take()
            .ok_or(Error::Unsupported("no current compute register array"))?;
        self.program
            .resolve(array, memory, |space, offset, value| {
                if space != RegisterSpace::Sh {
                    return Err(Error::Unsupported("compute indirect register space"));
                }
                self.state.write(offset, value)
            })?;
        self.failed = false;
        Ok(())
    }
    pub fn next_step(&mut self) -> Result<Option<OrderedStep>, Error> {
        if self.failed || self.pending.is_some() {
            return Err(Error::Unsupported("unresolved ordered compute stream"));
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
                self.pending = Some(self.program.array(packet)?);
                return Ok(Some(OrderedStep::Array));
            }
            let word = |index| self.program.word(packet, index);
            if crate::queue_reset::is_header(header.raw()) {
                let reset = crate::queue_reset::decode(header.raw(), word(1))
                    .map_err(Error::Unsupported)?;
                if reset == crate::queue_reset::Reset::Processor {
                    self.state.reset();
                }
                continue;
            }
            if header.opcode() == 0x76 {
                for index in 2..packet.words {
                    self.state
                        .write(word(1) + (index - 2) as u32, word(index))?;
                }
                continue;
            }
            if header.opcode() == 0x11 {
                self.state.indirect_base =
                    Some(u64::from(word(2) & !7) | (u64::from(word(3) & 0xffff) << 32));
                continue;
            }
            let dispatch = matches!(header.opcode(), 0x15 | 0x16);
            let memory = matches!(header.opcode(), 0x37 | 0x3c | 0x93 | 0x49 | 0x42)
                || matches!(header.raw(), 0xc006_1060 | 0xc002_7904);
            if dispatch || memory {
                let index = self.action;
                self.action += 1;
                let action = self
                    .plan
                    .actions
                    .get(index)
                    .ok_or(Error::Unsupported("ordered compute action alignment"))?;
                return Ok(Some(match action {
                    super::commands::Action::Dispatch(_) if dispatch => {
                        let (groups, mode, indirect) = if header.opcode() == 0x15 {
                            ([word(1), word(2), word(3)], word(4), None)
                        } else {
                            let (address, mode) = if packet.words == 4 {
                                (u64::from(word(1)) | (u64::from(word(2)) << 32), word(3))
                            } else {
                                let base = self
                                    .state
                                    .indirect_base
                                    .filter(|base| *base != 0)
                                    .ok_or(Error::Unsupported("missing indirect args base"))?;
                                (
                                    base.checked_add(u64::from(word(1)))
                                        .ok_or(Error::Unsupported("indirect args extent"))?,
                                    word(2),
                                )
                            };
                            if address == 0
                                || !address.is_multiple_of(4)
                                || address.checked_add(12).is_none_or(|end| end > 1 << 47)
                            {
                                return Err(Error::Unsupported("indirect args extent"));
                            }
                            ([0; 3], mode, Some(address))
                        };
                        OrderedStep::Dispatch(Dispatch {
                            state: self.state,
                            groups,
                            mode,
                            indirect,
                        })
                    }
                    super::commands::Action::Write(_)
                    | super::commands::Action::Wait(_)
                    | super::commands::Action::Release(_)
                    | super::commands::Action::Drain
                        if memory =>
                    {
                        OrderedStep::Memory(index)
                    }
                    _ => return Err(Error::Unsupported("ordered compute action kind")),
                }));
            }
        }
        if self.action != self.plan.actions.len() {
            return Err(Error::Unsupported("unconsumed compute actions"));
        }
        self.finished = true;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    struct Backing {
        base: u64,
        bytes: RefCell<Vec<u8>>,
        reads: Cell<usize>,
    }
    impl Memory for Backing {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            self.reads.set(self.reads.get() + 1);
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
    fn prepare(words: &[u32]) -> OrderedCommands {
        let data = bytes(words);
        OrderedCommands::prepare(
            &data,
            CommandRange {
                address: 0x1000,
                size: data.len() as u64,
            },
            ComputeState::empty(),
            &CommandLimits::default(),
        )
        .unwrap()
    }
    #[test]
    fn current_array_and_generic_tag_zero_override_dispatch_state_in_source_order() {
        let base = 0x3_0000_2000;
        let backing = Backing {
            base,
            bytes: RefCell::new(vec![0; 32]),
            reads: Cell::new(0),
        };
        let words = [
            0xc006_3700,
            0x100,
            base as u32,
            (base >> 32) as u32,
            0x7000_020c,
            9,
            0x20d,
            3,
            0xc003_6300,
            base as u32 | 3,
            (base >> 32) as u32,
            0x8000_0000,
            0x4002,
            0xc001_7600,
            0x20c,
            17,
            0xc003_1500,
            0,
            1,
            1,
            0x8041,
        ];
        let mut plan = prepare(&words);
        assert_eq!(backing.reads.get(), 0);
        assert!(plan.final_state().is_err());
        let mut dispatches = 0;
        while let Some(step) = plan.next_step().unwrap() {
            match step {
                OrderedStep::Array => plan.resolve_array(&backing).unwrap(),
                OrderedStep::Memory(index) => {
                    let Command::Write(write) = plan.memory_command(index).unwrap() else {
                        panic!("original write")
                    };
                    let start = (write.address() - base) as usize;
                    backing.bytes.borrow_mut()[start..start + write.bytes().len()]
                        .copy_from_slice(write.bytes());
                }
                OrderedStep::Dispatch(dispatch) => {
                    dispatches += 1;
                    assert_eq!(dispatch.shader_address(), Ok((17 << 8) | (3 << 40)));
                    let product =
                        super::super::prepare_dispatch(&dispatch, &backing, &Default::default())
                            .unwrap();
                    assert!(product.into_parts().0.is_none()); // Genuine zero group; no GPU work fabricated.
                }
            }
        }
        assert_eq!(dispatches, 1);
        assert_eq!(backing.reads.get(), 1);
        assert_eq!(plan.command_ranges().len(), 2);
        assert_eq!(plan.final_state().unwrap().get(0x20c), Ok(17));
    }
    #[test]
    fn unknown_spaces_offsets_and_new_array_ib_combinations_remain_explicit() {
        let backing = Backing {
            base: 0x2000,
            bytes: RefCell::new(bytes(&[0x20c, 7])),
            reads: Cell::new(0),
        };
        for opcode in [0x9f, 0x64] {
            let mut plan = prepare(&[0xc003_0000 | opcode << 8, 0x2000, 0, 0x8000_0000, 1]);
            assert!(matches!(plan.next_step(), Ok(Some(OrderedStep::Array))));
            assert_eq!(
                plan.resolve_array(&backing),
                Err(Error::Unsupported("compute indirect register space"))
            );
            assert!(plan.next_step().is_err());
        }
        backing.bytes.borrow_mut()[..4].copy_from_slice(&0x8000_020cu32.to_le_bytes());
        let mut plan = prepare(&[0xc003_6300, 0x2000, 0, 0x8000_0000, 1]);
        assert!(matches!(plan.next_step(), Ok(Some(OrderedStep::Array))));
        assert_eq!(
            plan.resolve_array(&backing),
            Err(Error::Unsupported("compute SH register"))
        );
        let data = bytes(&[
            0xc003_6300,
            0x2000,
            0,
            0x8000_0000,
            1,
            0xc002_3f02,
            0x3000,
            0,
            0x0f20_0001,
        ]);
        assert!(matches!(
            OrderedCommands::prepare(
                &data,
                CommandRange {
                    address: 0x1000,
                    size: data.len() as u64
                },
                ComputeState::empty(),
                &Default::default()
            ),
            Err(Error::Unsupported(
                "indirect buffers in ordered register stream"
            ))
        ));
    }
}
