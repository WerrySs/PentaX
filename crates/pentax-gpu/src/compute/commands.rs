// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded iterative compute queue command admission.
// Adapted from pinned CpOpIndirectBuffer and ProcessPacket; see GPU NOTICE.

//! Complete command/state admission before any shader or resource read or GPU
//! effect. Indirect frames are immutable owned snapshots, never live pointers.
use super::{ComputeState, Error, Memory};
use crate::Header;
use std::{fmt, mem::size_of};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandRange {
    pub address: u64,
    pub size: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct CommandLimits {
    pub max_command_bytes: usize,
    pub max_host_bytes: usize,
    pub max_packets: u64,
    pub max_indirect_buffers: u64,
    pub max_depth: usize,
    pub max_dispatches: usize,
    pub max_actions: usize,
    pub max_write_bytes: usize,
    pub max_work: u64,
}
impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            max_command_bytes: 256 * 1024,
            max_host_bytes: 4 * 1024 * 1024,
            max_packets: 65536,
            max_indirect_buffers: 128,
            max_depth: 8,
            max_dispatches: 128,
            max_actions: 1024,
            max_write_bytes: 256 * 1024,
            max_work: 4 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandUsage {
    pub command_bytes: usize,
    pub host_bytes: usize,
    pub packets: u64,
    pub indirect_buffers: u64,
    pub work: u64,
    pub write_bytes: usize,
}
/// Immutable state at one actual DISPATCH_DIRECT packet. No public constructor
/// accepts arbitrary register snapshots, shader bytes or SPIR-V.
pub struct Dispatch {
    pub(super) state: ComputeState,
    pub(super) groups: [u32; 3],
    pub(super) mode: u32,
    pub(super) indirect: Option<u64>,
}
impl Dispatch {
    /// Read only the shader address encoded by the admitted queue snapshot.
    /// This grants no backing access and performs no shader fetch/compilation.
    pub fn shader_address(&self) -> Result<u64, Error> {
        let low = self.state.get(0x20c)?;
        let high = self.state.get(0x20d)?;
        if high > 255 {
            return Err(Error::Unsupported("shader address high bits"));
        }
        Ok((u64::from(low) << 8) | (u64::from(high) << 40))
    }
}
impl fmt::Debug for Dispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatch")
            .field("groups", &self.groups)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
pub struct CommandPlan {
    dispatches: Vec<Dispatch>,
    final_state: ComputeState,
    usage: CommandUsage,
    captured: Vec<CommandRange>,
    actions: Vec<Action>,
}
enum Action {
    Dispatch(usize),
    Write(MemoryWrite),
    Wait(MemoryWait),
    Release(Release),
}
/// Immutable ordered views. Only this planner can construct the write/wait
/// payloads, and the runtime never accepts detached command actions as input.
pub enum Command<'a> {
    Dispatch(&'a Dispatch),
    Write(&'a MemoryWrite),
    Wait(&'a MemoryWait),
    Release(&'a Release),
}
/// Immutable release admitted from actual packet bytes. Interrupts are emitted
/// only after prior work and the optional label write complete successfully.
pub struct Release {
    write: Option<MemoryWrite>,
    reference_clock: bool,
    context: Option<u32>,
}
impl Release {
    pub fn write(&self) -> Option<&MemoryWrite> {
        self.write.as_ref()
    }
    pub fn reference_clock(&self) -> bool {
        self.reference_clock
    }
    pub fn interrupt_context(&self) -> Option<u32> {
        self.context
    }
}
pub struct MemoryWrite {
    address: u64,
    bytes: Vec<u8>,
}
impl MemoryWrite {
    pub fn address(&self) -> u64 {
        self.address
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl fmt::Debug for MemoryWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryWrite")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
pub struct MemoryWait {
    address: u64,
    size: usize,
    reference: u64,
    mask: u64,
    function: u32,
}
impl MemoryWait {
    pub fn byte_count(&self) -> usize {
        self.size
    }
    pub fn satisfied(&self, memory: &impl Memory) -> Result<bool, Error> {
        let mut bytes = [0; 8];
        memory
            .read(self.address, &mut bytes[..self.size])
            .map_err(Error::Memory)?;
        let value = u64::from_le_bytes(bytes) & self.mask;
        Ok(match self.function {
            0 => true,
            1 => value < self.reference,
            2 => value <= self.reference,
            3 => value == self.reference,
            4 => value != self.reference,
            5 => value >= self.reference,
            6 => value > self.reference,
            _ => unreachable!(),
        })
    }
}
impl fmt::Debug for CommandPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandPlan")
            .field("dispatches", &self.dispatches.len())
            .field("usage", &self.usage)
            .finish()
    }
}
impl CommandPlan {
    pub fn dispatches(&self) -> &[Dispatch] {
        &self.dispatches
    }
    pub fn final_state(&self) -> ComputeState {
        self.final_state
    }
    pub fn commands(&self) -> impl Iterator<Item = Command<'_>> {
        self.actions.iter().map(|a| match a {
            Action::Dispatch(i) => Command::Dispatch(&self.dispatches[*i]),
            Action::Write(w) => Command::Write(w),
            Action::Wait(w) => Command::Wait(w),
            Action::Release(r) => Command::Release(r),
        })
    }
    pub fn usage(&self) -> CommandUsage {
        self.usage
    }
    /// All captured command spans, including chain sources and repeated calls.
    /// A snapshot executor must refuse GPU writes aliasing these live bytes.
    pub fn command_ranges(&self) -> &[CommandRange] {
        &self.captured
    }
}
struct Frame {
    bytes: Vec<u8>,
    at: usize,
    lineage_start: usize,
}
impl Frame {
    fn word(&self, at: usize) -> u32 {
        u32::from_le_bytes(self.bytes[at * 4..at * 4 + 4].try_into().unwrap())
    }
}
struct Budget<'a> {
    limits: &'a CommandLimits,
    usage: CommandUsage,
}
impl Budget<'_> {
    fn work(&mut self, n: u64) -> Result<(), Error> {
        self.usage.work = self
            .usage
            .work
            .checked_add(n)
            .filter(|v| *v <= self.limits.max_work)
            .ok_or(Error::Limit("command work"))?;
        Ok(())
    }
    fn host(&mut self, n: usize) -> Result<(), Error> {
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_add(n)
            .filter(|v| *v <= self.limits.max_host_bytes)
            .ok_or(Error::Limit("command host bytes"))?;
        Ok(())
    }
    fn reserve<T>(&mut self, vec: &mut Vec<T>, count: usize) -> Result<(), Error> {
        let expected = count
            .checked_mul(size_of::<T>())
            .ok_or(Error::Limit("command metadata"))?;
        self.host(expected)?;
        vec.try_reserve_exact(count)
            .map_err(|_| Error::Allocation)?;
        let actual = vec
            .capacity()
            .checked_mul(size_of::<T>())
            .ok_or(Error::Limit("command metadata"))?;
        self.host(actual - expected)
    }
    fn capture(
        &mut self,
        range: CommandRange,
        memory: &impl Memory,
        lineage_start: usize,
    ) -> Result<Frame, Error> {
        let length = usize::try_from(range.size).map_err(|_| Error::Limit("command bytes"))?;
        if range.address == 0
            || !range.address.is_multiple_of(4)
            || !length.is_multiple_of(4)
            || range
                .address
                .checked_add(range.size)
                .is_none_or(|v| v > 1 << 47)
        {
            return Err(Error::Unsupported("command address or alignment"));
        }
        self.usage.command_bytes = self
            .usage
            .command_bytes
            .checked_add(length)
            .filter(|v| *v <= self.limits.max_command_bytes)
            .ok_or(Error::Limit("command bytes"))?;
        self.work(range.size)?;
        let mut bytes = Vec::new();
        self.reserve(&mut bytes, length)?;
        bytes.resize(length, 0);
        if length != 0 {
            memory
                .read(range.address, &mut bytes)
                .map_err(Error::Memory)?;
        }
        Ok(Frame {
            bytes,
            at: 0,
            lineage_start,
        })
    }
}

/// Snapshot and admit an entire bounded stream, following ordinary call/chain
/// indirect buffers. Returns state-only plans too. Unported flags/packets or a
/// cycle return no plan and never read shader/resource bytes or publish state.
pub fn plan_commands(
    initial: &ComputeState,
    root: CommandRange,
    memory: &impl Memory,
    limits: &CommandLimits,
) -> Result<CommandPlan, Error> {
    if limits.max_depth == 0 {
        return Err(Error::Limit("command depth"));
    }
    let mut budget = Budget {
        limits,
        usage: CommandUsage::default(),
    };
    let mut frames = Vec::new();
    budget.reserve(&mut frames, limits.max_depth)?;
    let lineage_limit = usize::try_from(limits.max_indirect_buffers)
        .ok()
        .and_then(|n| n.checked_add(1))
        .ok_or(Error::Limit("indirect metadata"))?;
    let mut lineage = Vec::new();
    budget.reserve(&mut lineage, lineage_limit)?;
    let mut captured = Vec::new();
    budget.reserve(&mut captured, lineage_limit)?;
    let mut dispatches = Vec::new();
    budget.reserve(&mut dispatches, limits.max_dispatches)?;
    let mut actions = Vec::new();
    budget.reserve(&mut actions, limits.max_actions)?;
    frames.push(budget.capture(root, memory, 0)?);
    lineage.push(root);
    captured.push(root);
    let mut state = *initial;
    while let Some(frame) = frames.last_mut() {
        if frame.at == frame.bytes.len() / 4 {
            let start = frame.lineage_start;
            frames.pop();
            lineage.truncate(start);
            continue;
        }
        budget.usage.packets = budget
            .usage
            .packets
            .checked_add(1)
            .filter(|v| *v <= limits.max_packets)
            .ok_or(Error::Limit("command packets"))?;
        budget.work(1)?;
        let at = frame.at;
        let header = Header::new(frame.word(at));
        let fail = |reason| Error::Packet { dword: at, reason };
        let length = if header.raw() == 0x8000_0000 {
            1
        } else if header.raw() == 0xc006_1060 {
            8
        } else if header.packet_type() == 3 && matches!(header.low_byte(), 0 | 2) {
            if header.opcode() == 0x10 && header.count() == 0x3fff {
                1
            } else {
                usize::from(header.count()) + 2
            }
        } else {
            return Err(fail("packet header profile"));
        };
        let end = at
            .checked_add(length)
            .filter(|v| *v <= frame.bytes.len() / 4)
            .ok_or(fail("truncated packet"))?;
        frame.at = end;
        if length == 1 || (header.opcode() == 0x10 && header.raw() != 0xc006_1060) {
            continue;
        }
        if !matches!(header.opcode(), 0x76 | 0x11 | 0x3f) && actions.len() >= limits.max_actions {
            return Err(Error::Limit("command actions"));
        }
        match header.opcode() {
            0x11 if length == 4 && header.low_byte() == 2 && frame.word(at + 1) & 15 == 1 => {
                let address = u64::from(frame.word(at + 2) & !7)
                    | (u64::from(frame.word(at + 3) & 0xffff) << 32);
                if address >= 1 << 47 {
                    return Err(Error::Unsupported("indirect base address"));
                }
                state.indirect_base = Some(address);
            }
            0x76 if header.shader_type() && length >= 3 => {
                let first = frame.word(at + 1);
                if first > 0xffff {
                    return Err(fail("register selector"));
                }
                budget.work((length - 2) as u64)?;
                for index in 0..length - 2 {
                    state.write(first + index as u32, frame.word(at + 2 + index))?;
                }
            }
            0x15 if header.shader_type() && length == 5 => {
                if dispatches.len() >= limits.max_dispatches {
                    return Err(Error::Limit("command dispatches"));
                }
                dispatches.push(Dispatch {
                    state,
                    groups: [frame.word(at + 1), frame.word(at + 2), frame.word(at + 3)],
                    mode: frame.word(at + 4),
                    indirect: None,
                });
                actions.push(Action::Dispatch(dispatches.len() - 1));
            }
            0x16 if matches!(header.raw(), 0xc001_1600 | 0xc002_1600) => {
                if dispatches.len() >= limits.max_dispatches {
                    return Err(Error::Limit("command dispatches"));
                }
                let (address, mode) = if length == 4 {
                    (
                        u64::from(frame.word(at + 1)) | (u64::from(frame.word(at + 2)) << 32),
                        frame.word(at + 3),
                    )
                } else {
                    let base = state
                        .indirect_base
                        .filter(|v| *v != 0)
                        .ok_or(Error::Unsupported("missing indirect args base"))?;
                    (
                        base.checked_add(u64::from(frame.word(at + 1)))
                            .ok_or(Error::Unsupported("indirect args extent"))?,
                        frame.word(at + 2),
                    )
                };
                extent(address, 12, 4)?;
                dispatches.push(Dispatch {
                    state,
                    groups: [0; 3],
                    mode,
                    indirect: Some(address),
                });
                actions.push(Action::Dispatch(dispatches.len() - 1));
            }
            0x37 if length >= 5 => {
                let control = frame.word(at + 1);
                let destination = ((control >> 30) & 1) | ((control >> 7) & 0x1e);
                if !matches!(destination, 2 | 5)
                    || control & !(0x4000_0000 | 0xf00 | 0x10000 | 0x100000) != 0
                {
                    return Err(Error::Unsupported("write data control"));
                }
                let address = u64::from(frame.word(at + 2)) | (u64::from(frame.word(at + 3)) << 32);
                let begin = if control & (1 << 16) != 0 {
                    end - 1
                } else {
                    at + 4
                };
                let input = &frame.bytes[begin * 4..end * 4];
                actions.push(Action::Write(owned_write(address, input, &mut budget)?));
            }
            0x3c | 0x93
                if header.low_byte() == 0
                    && length == if header.opcode() == 0x3c { 7 } else { 9 } =>
            {
                let size = if header.opcode() == 0x3c { 4 } else { 8 };
                let control = frame.word(at + 1);
                if control & 0x10 == 0 || control & !0x117 != 0 || control & 7 == 7 {
                    return Err(Error::Unsupported("wait memory control"));
                }
                let address = u64::from(frame.word(at + 2) & !((size as u32) - 1))
                    | (u64::from(frame.word(at + 3) & 0x3ffff) << 32);
                extent(address, size, size)?;
                let read_value = |index| {
                    if size == 4 {
                        u64::from(frame.word(index))
                    } else {
                        u64::from(frame.word(index)) | (u64::from(frame.word(index + 1)) << 32)
                    }
                };
                let reference = read_value(at + 4);
                let mask = read_value(at + 4 + size / 4);
                actions.push(Action::Wait(MemoryWait {
                    address,
                    size,
                    reference,
                    mask,
                    function: control & 7,
                }));
            }
            0x10 | 0x49
                if header.raw() == 0xc006_1060 || (header.opcode() == 0x49 && length == 8) =>
            {
                let selection = frame.word(at + 2);
                let data = (selection >> 29) & 7;
                let interrupt = (selection >> 24) & 7;
                // The serialized executor fences every prior dispatch and
                // same-owner writeback. Source cache/barrier fields cannot
                // leave pending work here; GDS release data remains explicit.
                let destination = (selection >> 16) & 3;
                if destination > 1 || interrupt > 4 || data > 3 {
                    return Err(Error::Unsupported("release memory profile"));
                }
                let address = u64::from(frame.word(at + 3)) | (u64::from(frame.word(at + 4)) << 32);
                if address == 0 && destination == 1 && data != 0 && interrupt != 4 {
                    return Err(Error::Unsupported("release memory destination"));
                }
                if data >= 2 && address != 0 && interrupt != 4 {
                    // ExecReleaseMem normalizes the index except for the two
                    // native EOP profiles; ExecEndOfPipe still rejects unknown
                    // event combinations. Serial fencing does not waive those
                    // source semantic checks. INT_SEL4 uses its explicit
                    // dropped-label path instead of ExecEndOfPipe.
                    let event = frame.word(at + 1);
                    let kind = event & 0x3f;
                    let index = (event >> 8) & 7;
                    let writeback = event & (1 << 21) != 0;
                    let raises = matches!(interrupt, 1 | 2);
                    let allowed = if writeback {
                        match kind {
                            0x04 | 0x14 | 0x28 => true,
                            0x2b | 0x2d => !raises,
                            0x2f => index == 6 && !raises,
                            _ => false,
                        }
                    } else {
                        match kind {
                            0x04 => index == 5,
                            0x14 | 0x28 | 0x2f => true,
                            0x2b | 0x2d | 0x30 => !raises,
                            _ => false,
                        }
                    };
                    if !allowed {
                        return Err(Error::Unsupported("release event profile"));
                    }
                }
                let write = if data != 0 && address != 0 {
                    let length = if data == 1 { 4 } else { 8 };
                    extent(address, length, length)?;
                    let input = if data == 3 {
                        &[0; 8][..]
                    } else {
                        &frame.bytes[(at + 5) * 4..(at + 5) * 4 + length]
                    };
                    Some(owned_write(address, input, &mut budget)?)
                } else {
                    None
                };
                actions.push(Action::Release(Release {
                    write,
                    reference_clock: data == 3,
                    context: matches!(interrupt, 1 | 2 | 4)
                        .then_some(frame.word(at + 7) & 0x07ff_ffff),
                }));
            }
            0x3f if length == 4 => {
                let control = frame.word(at + 3);
                let size = u64::from(control & 0xfffff) * 4;
                if size == 0 {
                    continue;
                }
                // Exact pinned ordinary PS5 IB control profile; conditional,
                // predicated and broader VM/cache modes remain unsupported.
                if control & 0xffe0_0000 != 0x0f20_0000 {
                    return Err(Error::Unsupported("indirect control profile"));
                }
                let target = CommandRange {
                    address: u64::from(frame.word(at + 1)) | (u64::from(frame.word(at + 2)) << 32),
                    size,
                };
                budget.usage.indirect_buffers = budget
                    .usage
                    .indirect_buffers
                    .checked_add(1)
                    .filter(|v| *v <= limits.max_indirect_buffers)
                    .ok_or(Error::Limit("indirect buffers"))?;
                budget.work(lineage.len() as u64)?;
                if lineage.contains(&target) {
                    return Err(Error::Unsupported("indirect buffer cycle"));
                }
                let chain = control & (1 << 20) != 0;
                let start = if chain {
                    frame.lineage_start
                } else {
                    lineage.len()
                };
                if !chain && frames.len() >= limits.max_depth {
                    return Err(Error::Limit("command depth"));
                }
                let next = budget.capture(target, memory, start)?;
                captured.push(target);
                lineage.push(target);
                if chain {
                    *frames.last_mut().unwrap() = next;
                } else {
                    frames.push(next);
                }
            }
            _ => return Err(fail("command outside compute queue profile")),
        }
    }
    Ok(CommandPlan {
        dispatches,
        final_state: state,
        usage: budget.usage,
        captured,
        actions,
    })
}

fn extent(address: u64, size: usize, alignment: usize) -> Result<(), Error> {
    if address == 0
        || !address.is_multiple_of(alignment as u64)
        || address.checked_add(size as u64).is_none_or(|v| v > 1 << 47)
    {
        return Err(Error::Unsupported("memory command extent"));
    }
    Ok(())
}
fn owned_write(address: u64, input: &[u8], budget: &mut Budget<'_>) -> Result<MemoryWrite, Error> {
    extent(address, input.len(), 4)?;
    budget.usage.write_bytes = budget
        .usage
        .write_bytes
        .checked_add(input.len())
        .filter(|v| *v <= budget.limits.max_write_bytes)
        .ok_or(Error::Limit("command write bytes"))?;
    budget.work(input.len() as u64)?;
    let mut bytes = Vec::new();
    budget.reserve(&mut bytes, input.len())?;
    bytes.extend_from_slice(input);
    Ok(MemoryWrite { address, bytes })
}

#[cfg(test)]
mod tests;
