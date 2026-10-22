// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: immutable, bounded graphics IB call/chain capture.
// Source: KytyPS5 7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3,
// pm4Handlers.cpp CpOpIndirectBuffer; see README.md and NOTICE.

use super::{CommandLimits, CommandPlan, Error, GraphicsState, OrderedCommands};
use crate::{
    Header,
    compute::{CommandRange, Memory},
};
use std::mem::size_of;

/// Capture-only admission, independent of the subsequently prepared plan.
/// Host bytes conservatively include all admitted transient capture storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureUsage {
    pub command_bytes: usize,
    pub packets: u64,
    pub host_bytes: usize,
    pub work: u64,
    pub captured_buffers: usize,
    pub indirect_buffers: usize,
}

/// Monotonic admitted attempt usage survives refusal. No executable prefix or
/// partially captured command bytes escape through this failure product.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureFailure {
    pub error: Error,
    pub usage: CaptureUsage,
}

#[derive(Clone, Copy)]
pub(super) struct Origin {
    pub(super) at: usize,
    pub(super) source: usize,
    pub(super) dword: usize,
    pub(super) address: u64,
    pub(super) words: usize,
    pub(super) emitted: bool,
}

struct Frame {
    bytes: Vec<u8>,
    at: usize,
    source: usize,
    lineage_start: usize,
}

/// A sealed snapshot, never a native pointer or execution authority. Ordinary
/// IB packets are expanded in their actual call/chain order. Source ranges
/// include complete snapshots, including tails skipped by a CHAIN packet.
pub struct CapturedCommands {
    bytes: Vec<u8>,
    origins: Vec<Origin>,
    sources: Vec<CommandRange>,
    root_count: usize,
    arrays: bool,
    transferred: bool,
    usage: CaptureUsage,
}

struct Budget<'a> {
    limits: &'a CommandLimits,
    usage: CaptureUsage,
}
impl Budget<'_> {
    fn work(&mut self, amount: u64) -> Result<(), Error> {
        self.usage.work = self
            .usage
            .work
            .checked_add(amount)
            .filter(|n| *n <= self.limits.max_work)
            .ok_or(Error::Limit("graphics capture work"))?;
        Ok(())
    }
    fn host(&mut self, amount: usize) -> Result<(), Error> {
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_add(amount)
            .filter(|n| *n <= self.limits.max_host_bytes)
            .ok_or(Error::Limit("graphics capture host bytes"))?;
        Ok(())
    }
    // Admit each new capacity and relocation before allocation, then allocator
    // excess. Geometric growth keeps copy work bounded; cumulative host charge
    // conservatively includes old and new allocations during replacement.
    fn reserve<T>(&mut self, values: &mut Vec<T>, additional: usize) -> Result<(), Error> {
        let wanted = values
            .len()
            .checked_add(additional)
            .ok_or(Error::Limit("graphics capture allocation"))?;
        if wanted <= values.capacity() {
            return Ok(());
        }
        let old = values.capacity();
        let target = wanted.max(old.saturating_mul(2));
        let requested = target
            .checked_mul(size_of::<T>())
            .ok_or(Error::Limit("graphics capture allocation"))?;
        self.host(requested)?;
        self.work(requested as u64)?;
        values
            .try_reserve_exact(target - values.len())
            .map_err(|_| Error::Allocation)?;
        let extra = values
            .capacity()
            .checked_sub(target)
            .and_then(|n| n.checked_mul(size_of::<T>()))
            .ok_or(Error::Limit("graphics capture capacity"))?;
        self.host(extra)
    }
    fn snapshot(&mut self, range: CommandRange, memory: &impl Memory) -> Result<Vec<u8>, Error> {
        let length =
            usize::try_from(range.size).map_err(|_| Error::Limit("graphics source size"))?;
        if range.address == 0
            || !range.address.is_multiple_of(4)
            || !length.is_multiple_of(4)
            || range
                .address
                .checked_add(range.size)
                .is_none_or(|end| end > 1 << 47)
        {
            return Err(Error::SourcePacket {
                address: range.address,
                dword: 0,
                reason: "graphics command source extent",
            });
        }
        self.usage.command_bytes = self
            .usage
            .command_bytes
            .checked_add(length)
            .filter(|n| *n <= self.limits.max_command_bytes)
            .ok_or(Error::Limit("graphics captured command bytes"))?;
        self.usage.captured_buffers = self
            .usage
            .captured_buffers
            .checked_add(1)
            .filter(|n| *n <= self.limits.max_buffers)
            .ok_or(Error::Limit("graphics captured command buffers"))?;
        self.work(length as u64)?;
        let mut bytes = Vec::new();
        self.reserve(&mut bytes, length)?;
        bytes.resize(length, 0);
        memory
            .read(range.address, &mut bytes)
            .map_err(|error| Error::SourceMemory {
                address: range.address,
                error,
            })?;
        Ok(bytes)
    }
}

impl CapturedCommands {
    /// Snapshot all roots and every reachable ordinary graphics IB while the
    /// caller retains exclusive paused memory. No array, shader, target or
    /// indirect-draw argument contents are read by this operation.
    pub fn capture(
        roots: &[CommandRange],
        memory: &impl Memory,
        limits: &CommandLimits,
    ) -> Result<Self, Error> {
        Self::capture_attempt(roots, memory, limits).map_err(|failure| failure.error)
    }

    /// Stateful callers must commit this attempt's admitted bytes/buffers/work
    /// on both success and failure before propagating its causal refusal.
    pub fn capture_attempt(
        roots: &[CommandRange],
        memory: &impl Memory,
        limits: &CommandLimits,
    ) -> Result<Self, CaptureFailure> {
        let mut budget = Budget {
            limits,
            usage: CaptureUsage::default(),
        };
        Self::capture_inner(roots, memory, &mut budget).map_err(|error| CaptureFailure {
            error,
            usage: budget.usage,
        })
    }

    fn capture_inner(
        roots: &[CommandRange],
        memory: &impl Memory,
        budget: &mut Budget<'_>,
    ) -> Result<Self, Error> {
        let limits = budget.limits;
        if roots.len() > limits.max_buffers {
            return Err(Error::Limit("graphics captured command buffers"));
        }
        budget.host(size_of::<Self>())?;
        budget.work(size_of::<Self>() as u64)?;
        let mut sources = Vec::new();
        budget.reserve(&mut sources, roots.len())?;
        sources.extend_from_slice(roots);
        let mut bytes = Vec::new();
        let mut origins = Vec::new();
        let mut frames = Vec::<Frame>::new();
        let mut lineage = Vec::<CommandRange>::new();
        let mut arrays = false;
        for (root_index, root) in roots.iter().copied().enumerate() {
            let snapshot = budget
                .snapshot(root, memory)
                .map_err(|error| root_error(error, root_index))?;
            budget.reserve(&mut frames, 1)?;
            budget.reserve(&mut lineage, 1)?;
            frames.push(Frame {
                bytes: snapshot,
                at: 0,
                source: root_index,
                lineage_start: 0,
            });
            lineage.push(root);
            while !frames.is_empty() {
                let index = frames.len() - 1;
                if frames[index].at == frames[index].bytes.len() {
                    let start = frames[index].lineage_start;
                    frames.pop();
                    lineage.truncate(start);
                    continue;
                }
                let frame = &frames[index];
                let at = frame.at;
                let source = frame.source;
                let source_range = sources[source];
                let fail = |reason| {
                    source_packet(source, roots.len(), source_range.address, at / 4, reason)
                };
                let tail = &frame.bytes[at..];
                let words = packet_words(tail).map_err(&fail)?;
                let length = words
                    .checked_mul(4)
                    .ok_or(Error::Limit("graphics packet extent"))?;
                if length > tail.len() {
                    return Err(fail("truncated command packet"));
                }
                let header = Header::new(word(tail, 0));
                let packets = if header.raw() == 0xc002_7904 { 3 } else { 1 };
                budget.usage.packets = budget
                    .usage
                    .packets
                    .checked_add(packets)
                    .filter(|n| *n <= limits.max_packets)
                    .ok_or(Error::Limit("graphics captured packets"))?;
                budget.work(packets)?;
                let indirect = header.opcode() == 0x3f && header.packet_type() == 3;
                // Retain original IB packet locations too, although no IB
                // packet is reinterpreted as an emitted register operation.
                // Adjacent emitted packets from the same actual source have
                // identical address translation, so one span is lossless and
                // avoids per-packet storage for existing long flat streams.
                let previous: Option<&mut Origin> = origins.last_mut();
                if let Some(previous) = previous.filter(|previous| {
                    !indirect
                        && previous.emitted
                        && previous.source == source
                        && previous.at + previous.words * 4 == bytes.len()
                        && previous.dword + previous.words == at / 4
                }) {
                    previous.words = previous
                        .words
                        .checked_add(words)
                        .ok_or(Error::Limit("graphics provenance extent"))?;
                } else {
                    budget.reserve(&mut origins, 1)?;
                    origins.push(Origin {
                        at: bytes.len(),
                        source,
                        dword: at / 4,
                        address: source_range.address,
                        words,
                        emitted: !indirect,
                    });
                }
                if indirect {
                    if words != 4 {
                        return Err(fail("graphics indirect packet length"));
                    }
                    if header.low_byte() != 0 {
                        return Err(fail("graphics indirect header profile"));
                    }
                    let control = word(tail, 3);
                    let size = u64::from(control & 0x000f_ffff) * 4;
                    // Source zero-count semantics: no address/control dereference.
                    if size == 0 {
                        frames[index].at += length;
                        continue;
                    }
                    if control & 0xffe0_0000 != 0x0f20_0000 {
                        return Err(fail("graphics indirect control profile"));
                    }
                    let target = CommandRange {
                        address: u64::from(word(tail, 1)) | (u64::from(word(tail, 2)) << 32),
                        size,
                    };
                    budget.work(lineage.len() as u64)?;
                    if lineage.contains(&target) {
                        return Err(fail("graphics indirect buffer cycle"));
                    }
                    let chain = control & (1 << 20) != 0;
                    if !chain && frames.len() >= limits.max_buffers {
                        return Err(Error::Limit("graphics indirect call depth"));
                    }
                    let start = if chain {
                        frames[index].lineage_start
                    } else {
                        lineage.len()
                    };
                    budget.usage.indirect_buffers = budget
                        .usage
                        .indirect_buffers
                        .checked_add(1)
                        .ok_or(Error::Limit("graphics indirect buffers"))?;
                    let snapshot = budget.snapshot(target, memory)?;
                    budget.reserve(&mut sources, 1)?;
                    let next_source = sources.len();
                    sources.push(target);
                    budget.reserve(&mut lineage, 1)?;
                    lineage.push(target);
                    let next = Frame {
                        bytes: snapshot,
                        at: 0,
                        source: next_source,
                        lineage_start: start,
                    };
                    if chain {
                        frames[index] = next;
                    } else {
                        frames[index].at += length;
                        budget.reserve(&mut frames, 1)?;
                        frames.push(next);
                    }
                    continue;
                }
                arrays |= crate::registers::indirect_opcode(header);
                budget.reserve(&mut bytes, length)?;
                bytes.extend_from_slice(&frames[index].bytes[at..at + length]);
                budget.work(length as u64)?;
                frames[index].at += length;
            }
        }
        Ok(Self {
            bytes,
            origins,
            sources,
            root_count: roots.len(),
            arrays,
            transferred: false,
            usage: budget.usage,
        })
    }
    pub fn usage(&self) -> CaptureUsage {
        self.usage
    }
    pub fn command_ranges(&self) -> &[CommandRange] {
        &self.sources
    }
    pub fn into_command_ranges(self) -> Vec<CommandRange> {
        self.sources
    }
    pub fn contains_arrays(&self) -> bool {
        self.arrays
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Limits admit new plan allocations/work while this capture coexists.
    /// Original provenance is moved into the plan; its capacity is transferred
    /// from capture host usage to plan host usage without another allocation.
    pub fn plan(
        &mut self,
        initial: GraphicsState,
        limits: &CommandLimits,
    ) -> Result<CommandPlan, Error> {
        self.check_unconsumed()?;
        let mut plan = super::plan_commands(&[&self.bytes], initial, limits)
            .map_err(|error| self.source_error(error))?;
        plan.source_usage(self.usage.command_bytes, self.usage.packets);
        self.transfer_origins(&mut plan)?;
        Ok(plan)
    }
    pub fn prepare_ordered(
        &mut self,
        initial: GraphicsState,
        limits: &CommandLimits,
    ) -> Result<OrderedCommands, Error> {
        self.check_unconsumed()?;
        OrderedCommands::captured(self, initial, limits)
    }
    fn check_unconsumed(&self) -> Result<(), Error> {
        if self.transferred {
            Err(Error::Unsupported("consumed graphics capture"))
        } else {
            Ok(())
        }
    }
    pub(super) fn origins_host_bytes(&self) -> Result<usize, Error> {
        self.origins
            .capacity()
            .checked_mul(size_of::<Origin>())
            .ok_or(Error::Limit("graphics provenance capacity"))
    }
    pub(super) fn transfer_origins(&mut self, plan: &mut CommandPlan) -> Result<(), Error> {
        let bytes = self.origins_host_bytes()?;
        let remaining = self
            .usage
            .host_bytes
            .checked_sub(bytes)
            .ok_or(Error::Limit("graphics provenance transfer"))?;
        plan.retain_origins(std::mem::take(&mut self.origins))?;
        self.usage.host_bytes = remaining;
        self.transferred = true;
        Ok(())
    }
    pub(super) fn source_error(&self, error: Error) -> Error {
        let Error::Packet { dword, reason, .. } = error else {
            return error;
        };
        let at = dword.saturating_mul(4);
        let index = self.origins.partition_point(|origin| origin.at <= at);
        let Some(origin) = index
            .checked_sub(1)
            .and_then(|index| self.origins.get(index))
        else {
            return error;
        };
        source_packet(
            origin.source,
            self.root_count,
            self.sources[origin.source].address,
            origin.dword + (at - origin.at) / 4,
            reason,
        )
    }
}

fn root_error(error: Error, buffer: usize) -> Error {
    match error {
        Error::SourceMemory { error, .. } => Error::Memory(error),
        Error::SourcePacket { dword, reason, .. } => Error::Packet {
            buffer,
            dword,
            reason,
        },
        other => other,
    }
}
fn source_packet(
    source: usize,
    roots: usize,
    address: u64,
    dword: usize,
    reason: &'static str,
) -> Error {
    if source < roots {
        Error::Packet {
            buffer: source,
            dword,
            reason,
        }
    } else {
        Error::SourcePacket {
            address,
            dword,
            reason,
        }
    }
}
fn word(bytes: &[u8], index: usize) -> u32 {
    u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
}
fn packet_words(bytes: &[u8]) -> Result<usize, &'static str> {
    if bytes.len() < 4 {
        return Err("truncated command packet");
    }
    let header = Header::new(word(bytes, 0));
    if header.raw() == 0x8000_0000 {
        return Ok(1);
    }
    if header.raw() == 0xc004_105c {
        return Ok(6);
    }
    if header.raw() == 0xc006_1060 {
        return Ok(8);
    }
    if header.raw() == 0xc002_7904 {
        return crate::compute::tagged_wait_dwords(bytes)
            .map_err(|_| "graphics tagged wait packet");
    }
    if header.packet_type() != 3 || !matches!(header.low_byte(), 0 | 2) {
        return Err("graphics packet header");
    }
    if header.opcode() == 0x10 && header.count() == 0x3fff {
        Ok(1)
    } else {
        Ok(usize::from(header.count()) + 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RegisterSpace, compute::MemoryError, graphics::OrderedStep};
    use std::{
        cell::{Cell, RefCell},
        collections::BTreeMap,
    };

    #[derive(Default)]
    struct Backing {
        regions: RefCell<BTreeMap<u64, Vec<u8>>>,
        denied: Cell<Option<u64>>,
        reads: RefCell<Vec<u64>>,
    }
    impl Backing {
        fn put(&self, address: u64, words: &[u32]) -> CommandRange {
            let bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
            let size = bytes.len() as u64;
            self.regions.borrow_mut().insert(address, bytes);
            CommandRange { address, size }
        }
    }
    impl Memory for Backing {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
            self.reads.borrow_mut().push(address);
            if self.denied.get() == Some(address) {
                return Err(MemoryError::ReadDenied);
            }
            let regions = self.regions.borrow();
            let bytes = regions.get(&address).ok_or(MemoryError::Unavailable)?;
            output.copy_from_slice(bytes.get(..output.len()).ok_or(MemoryError::Unavailable)?);
            Ok(())
        }
    }
    fn ib(address: u64, dwords: u32, chain: bool) -> [u32; 4] {
        [
            0xc002_3f00,
            address as u32,
            (address >> 32) as u32,
            0x0f20_0000 | (u32::from(chain) << 20) | dwords,
        ]
    }
    fn shader(value: u32) -> [u32; 3] {
        [0xc001_7600, 8, value]
    }
    fn draw() -> [u32; 3] {
        [0xc001_2d00, 3, 2]
    }
    fn capture(memory: &Backing, roots: &[CommandRange]) -> CapturedCommands {
        CapturedCommands::capture(roots, memory, &CommandLimits::default()).unwrap()
    }

    #[test]
    fn graphics_indirect_call_chain_order_state_and_immutable_complete_sources() {
        let memory = Backing::default();
        let child_b = memory.put(0x3000, &[shader(3).as_slice(), &draw()].concat());
        let child_c = memory.put(0x1_0000_4000, &[shader(7).as_slice(), &draw()].concat());
        let child_a = memory.put(
            0x2000,
            &[
                shader(2).as_slice(),
                &ib(child_b.address, (child_b.size / 4) as u32, false),
                &shader(5),
                &draw(),
                &ib(child_c.address, (child_c.size / 4) as u32, true),
                &[0], // CHAIN does not execute this malformed tail, but retains its source.
            ]
            .concat(),
        );
        let root = memory.put(
            0x1000,
            &[
                [0xc000_2f00, 2].as_slice(),
                &shader(1),
                &ib(child_a.address, (child_a.size / 4) as u32, false),
                &shader(9),
                &draw(),
            ]
            .concat(),
        );
        let mut captured = capture(&memory, &[root]);
        assert_eq!(
            captured.command_ranges(),
            &[root, child_a, child_b, child_c]
        );
        assert_eq!(captured.usage().captured_buffers, 4);
        assert_eq!(captured.usage().indirect_buffers, 3);
        assert_eq!(captured.usage().packets, 14);
        assert_eq!(
            captured.usage().command_bytes as u64,
            root.size + child_a.size + child_b.size + child_c.size
        );
        memory.put(child_b.address, &[shader(99).as_slice(), &draw()].concat());
        let capture_host = captured.usage().host_bytes;
        let origin_host = captured.origins_host_bytes().unwrap();
        let plan = captured
            .plan(GraphicsState::empty(), &CommandLimits::default())
            .unwrap();
        assert_eq!(captured.usage().host_bytes + origin_host, capture_host);
        assert_eq!(plan.provenance_bytes(), origin_host);
        assert_eq!(
            plan.command_location(11),
            Some(super::super::CommandLocation {
                address: child_b.address,
                dword: 3
            })
        );
        assert_eq!(plan.command_location(usize::MAX), None);
        assert!(matches!(
            captured.plan(GraphicsState::empty(), &CommandLimits::default()),
            Err(Error::Unsupported("consumed graphics capture"))
        ));
        let values: Vec<_> = plan
            .draws()
            .iter()
            .map(|draw| draw.state.get(RegisterSpace::Sh, 8).unwrap())
            .collect();
        assert_eq!(values, [3, 5, 7, 9]);
        assert!(plan.draws().iter().all(|draw| draw.instance_count() == 2));
        assert_eq!(plan.usage().packets, 14);
        assert_eq!(plan.usage().command_bytes, captured.usage().command_bytes);
        assert_eq!(
            captured.into_command_ranges(),
            [root, child_a, child_b, child_c]
        );
    }

    #[test]
    fn graphics_indirect_zero_counts_and_root_error_facade_remain_exact() {
        let memory = Backing::default();
        let root = memory.put(0x1000, &[0xc002_3f00, 3, u32::MAX, 0xfff0_0000]);
        let mut captured = capture(&memory, &[root]);
        assert!(
            captured
                .plan(GraphicsState::empty(), &CommandLimits::default())
                .unwrap()
                .actions()
                .is_empty()
        );
        assert_eq!(*memory.reads.borrow(), [0x1000]);
        assert_eq!(captured.usage().packets, 1);
        let bad = memory.put(0x2000, &[0]);
        assert!(matches!(
            CapturedCommands::capture(&[root, bad], &memory, &CommandLimits::default()),
            Err(Error::Packet {
                buffer: 1,
                dword: 0,
                ..
            })
        ));
    }

    #[test]
    fn graphics_indirect_cycles_repeated_reads_and_aggregate_caps_are_bounded() {
        let memory = Backing::default();
        for chain in [false, true] {
            let root = memory.put(0x1000, &ib(0x1000, 4, chain));
            assert!(matches!(
                CapturedCommands::capture(&[root], &memory, &CommandLimits::default()),
                Err(Error::Packet {
                    reason: "graphics indirect buffer cycle",
                    ..
                })
            ));
        }
        let leaf = memory.put(0x3000, &[0x8000_0000]);
        let child = memory.put(0x2000, &ib(leaf.address, 1, false));
        let root = memory.put(
            0x1000,
            &[
                ib(child.address, 4, false).as_slice(),
                &ib(child.address, 4, false),
            ]
            .concat(),
        );
        let captured = capture(&memory, &[root]);
        assert_eq!(captured.usage().captured_buffers, 5); // repeated non-active reads are legal
        let mut limits = CommandLimits::default();
        limits.max_buffers = 2;
        assert!(matches!(
            CapturedCommands::capture(&[root], &memory, &limits),
            Err(Error::Limit(_))
        ));
        limits = CommandLimits::default();
        limits.max_command_bytes = root.size as usize + child.size as usize;
        assert!(matches!(
            CapturedCommands::capture(&[root], &memory, &limits),
            Err(Error::Limit("graphics captured command bytes"))
        ));
        limits = CommandLimits::default();
        limits.max_packets = 1;
        assert!(matches!(
            CapturedCommands::capture(&[root], &memory, &limits),
            Err(Error::Limit("graphics captured packets"))
        ));
        limits = CommandLimits::default();
        limits.max_host_bytes = 1;
        assert!(matches!(
            CapturedCommands::capture(&[root], &memory, &limits),
            Err(Error::Limit(_))
        ));
        limits = CommandLimits::default();
        limits.max_work = 1;
        assert!(matches!(
            CapturedCommands::capture(&[root], &memory, &limits),
            Err(Error::Limit(_))
        ));
    }

    #[test]
    fn graphics_indirect_full_addresses_permissions_and_nested_error_locations_survive() {
        let memory = Backing::default();
        for address in [0, 0x3001, 1 << 47, u64::MAX - 3] {
            let root = memory.put(0x1000, &ib(address, 1, false));
            assert!(
                matches!(CapturedCommands::capture(&[root], &memory, &CommandLimits::default()),
                Err(Error::SourcePacket { address: actual, dword: 0, .. }) if actual == address)
            );
        }
        let address = 0x1_0000_3000;
        let root = memory.put(0x1000, &ib(address, 1, false));
        assert!(
            matches!(CapturedCommands::capture(&[root], &memory, &CommandLimits::default()),
            Err(Error::SourceMemory { address: actual, error: MemoryError::Unavailable }) if actual == address)
        );
        memory.put(address, &[0x8000_0000]);
        memory.denied.set(Some(address));
        assert!(
            matches!(CapturedCommands::capture(&[root], &memory, &CommandLimits::default()),
            Err(Error::SourceMemory { address: actual, error: MemoryError::ReadDenied }) if actual == address)
        );
        memory.denied.set(None);
        memory.put(address, &[0xc000_7f00, 0]); // well-formed but unsupported graphics opcode
        let root = memory.put(0x1000, &ib(address, 2, false));
        let mut captured = capture(&memory, &[root]);
        assert!(
            matches!(captured.plan(GraphicsState::empty(), &CommandLimits::default()),
            Err(Error::SourcePacket { address: actual, dword: 0, reason: "command outside flat graphics profile" }) if actual == address)
        );
        memory.put(address, &[0]);
        let root = memory.put(0x1000, &ib(address, 1, false));
        assert!(
            matches!(CapturedCommands::capture(&[root], &memory, &CommandLimits::default()),
            Err(Error::SourcePacket { address: actual, dword: 0, .. }) if actual == address)
        );
    }

    #[test]
    fn graphics_indirect_nested_arrays_resolve_after_prior_write_and_preserve_all_sources() {
        let memory = Backing::default();
        let array_address = 0x1_0000_8000;
        memory.put(array_address, &[8, 11]);
        let child = memory.put(
            0x2000,
            &[
                0xc004_3700,
                0x100,
                array_address as u32,
                (array_address >> 32) as u32,
                8,
                77,
                0xc003_6300,
                array_address as u32,
                (array_address >> 32) as u32,
                0x8000_0000,
                1,
                0xc001_2d00,
                3,
                2,
            ],
        );
        let root = memory.put(
            0x1000,
            &[
                [0xc000_2f00, 1].as_slice(),
                &ib(child.address, (child.size / 4) as u32, false),
            ]
            .concat(),
        );
        let mut captured = capture(&memory, &[root]);
        assert!(captured.contains_arrays());
        assert!(!memory.reads.borrow().contains(&array_address));
        let mut ordered = captured
            .prepare_ordered(GraphicsState::empty(), &CommandLimits::default())
            .unwrap();
        assert_eq!(ordered.command_ranges(), &[root, child]);
        let Some(OrderedStep::Memory(index)) = ordered.next_step().unwrap() else {
            panic!("missing prior write");
        };
        let crate::compute::MemoryAction::Write(write) = ordered.memory_action(index) else {
            panic!("missing write action");
        };
        assert_eq!(write.address(), array_address);
        memory
            .regions
            .borrow_mut()
            .insert(array_address, write.bytes().to_vec());
        assert!(matches!(
            ordered.next_step().unwrap(),
            Some(OrderedStep::Array)
        ));
        assert_eq!(
            ordered.current_source(),
            Some(super::super::CommandLocation {
                address: child.address,
                dword: 6
            })
        );
        ordered.resolve_array(&memory).unwrap();
        let Some(OrderedStep::Draw(draw)) = ordered.next_step().unwrap() else {
            panic!("missing actual ordered draw");
        };
        assert_eq!(draw.state.get(RegisterSpace::Sh, 8).unwrap(), 77);
        assert_eq!(
            ordered.current_source(),
            Some(super::super::CommandLocation {
                address: child.address,
                dword: 11
            })
        );
        assert_eq!(
            ordered.command_ranges(),
            &[
                root,
                child,
                CommandRange {
                    address: array_address,
                    size: 8
                }
            ]
        );
        assert!(ordered.next_step().unwrap().is_none());
        assert_eq!(
            ordered
                .final_state()
                .unwrap()
                .get(RegisterSpace::Sh, 8)
                .unwrap(),
            77
        );
    }

    #[test]
    fn graphics_indirect_refused_capture_retains_admitted_attempt_usage_without_a_plan() {
        let memory = Backing::default();
        let child = memory.put(0x2000, &[0xc000_2f00, 3, 0]);
        let root = memory.put(0x1000, &ib(child.address, 3, false));
        for denied in [false, true] {
            memory.denied.set(denied.then_some(child.address));
            let failure = match CapturedCommands::capture_attempt(
                &[root],
                &memory,
                &CommandLimits::default(),
            ) {
                Err(failure) => failure,
                Ok(_) => panic!("invalid nested capture produced a command product"),
            };
            assert_eq!(failure.usage.command_bytes as u64, root.size + child.size);
            assert_eq!(failure.usage.captured_buffers, 2);
            assert_eq!(failure.usage.indirect_buffers, 1);
            assert!(failure.usage.work >= root.size + child.size);
            assert!(failure.usage.host_bytes > 0);
            if denied {
                assert!(matches!(
                    failure.error,
                    Error::SourceMemory {
                        address: 0x2000,
                        error: MemoryError::ReadDenied
                    }
                ));
                assert_eq!(failure.usage.packets, 1);
            } else {
                assert!(matches!(
                    failure.error,
                    Error::SourcePacket {
                        address: 0x2000,
                        dword: 2,
                        ..
                    }
                ));
                assert_eq!(failure.usage.packets, 2);
            }
        }
    }

    #[test]
    fn graphics_indirect_full_flat_nop_profile_keeps_lossless_coalesced_provenance_within_old_caps()
    {
        let memory = Backing::default();
        let limits = CommandLimits::default();
        let words = vec![0x8000_0000; limits.max_command_bytes / 4];
        let root = memory.put(0x1000, &words);
        let mut captured = capture(&memory, &[root]);
        assert_eq!(captured.origins.len(), 1);
        assert_eq!(captured.usage().packets, limits.max_packets);
        let capture = captured.usage();
        let mut remaining = limits;
        remaining.max_host_bytes -= capture.host_bytes;
        remaining.max_work -= capture.work;
        let plan = captured.plan(GraphicsState::empty(), &remaining).unwrap();
        assert!(captured.usage().host_bytes + plan.usage().host_bytes <= limits.max_host_bytes);
        assert!(capture.work + plan.usage().work <= limits.max_work);
        for dword in [0, 1, words.len() / 2, words.len() - 1] {
            assert_eq!(
                plan.command_location(dword),
                Some(super::super::CommandLocation {
                    address: root.address,
                    dword
                })
            );
        }
        assert_eq!(plan.command_location(words.len()), None);
    }
}
