// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded complete flat draw admission and snapshots.

use super::{Error, GraphicsState, IndexType, MemoryAction};
use crate::{Header, RegisterSpace};
use std::{fmt, mem::size_of};

#[derive(Clone, Copy, Debug)]
pub struct CommandLimits {
    pub max_buffers: usize,
    pub max_command_bytes: usize,
    pub max_packets: u64,
    pub max_draws: usize,
    pub max_vertices: u64,
    pub max_host_bytes: usize,
    pub max_work: u64,
}
impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            max_buffers: 64,
            max_command_bytes: 256 * 1024,
            max_packets: 65536,
            max_draws: 64,
            max_vertices: 1024 * 1024,
            max_host_bytes: 4 * 1024 * 1024,
            max_work: 4 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommandUsage {
    pub command_bytes: usize,
    pub packets: u64,
    pub draw_packets: u64,
    pub zero_draws: u64,
    pub flips: u64,
    pub vertices: u64,
    pub host_bytes: usize,
    pub work: u64,
}

/// Exact source R_FLIP payload, decoded only from tagged0xc004105c.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flip {
    handle: i32,
    buffer: i32,
    mode: i32,
    argument: i64,
}
impl Flip {
    pub fn handle(self) -> i32 {
        self.handle
    }
    pub fn buffer(self) -> i32 {
        self.buffer
    }
    pub fn mode(self) -> i32 {
        self.mode
    }
    pub fn argument(self) -> i64 {
        self.argument
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Draw(usize),
    Flip(Flip),
    Memory(usize),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct IndexSource {
    pub address: u64,
    pub index_type: IndexType,
}

/// Private immutable snapshot from one actual AUTO or indexed draw packet.
pub struct Draw {
    pub(super) state: GraphicsState,
    pub(super) vertex_count: u32,
    pub(super) instance_count: u32,
    pub(super) indices: Option<IndexSource>,
}
impl Draw {
    pub fn vertex_count(&self) -> u32 {
        self.vertex_count
    }
    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }
}
impl fmt::Debug for Draw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Draw")
            .field("vertex_count", &self.vertex_count)
            .field("instance_count", &self.instance_count)
            .finish_non_exhaustive()
    }
}
pub struct CommandPlan {
    draws: Vec<Draw>,
    actions: Vec<Action>,
    memory: Vec<MemoryAction>,
    final_state: GraphicsState,
    usage: CommandUsage,
    origins: Vec<super::indirect::Origin>,
}

/// Original command location. This carries no memory or execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandLocation {
    pub address: u64,
    pub dword: usize,
}
impl fmt::Debug for CommandPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphicsCommandPlan")
            .field("draws", &self.draws.len())
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}
impl CommandPlan {
    pub(super) fn provenance_bytes(&self) -> usize {
        self.origins.capacity() * size_of::<super::indirect::Origin>()
    }
    pub(super) fn retain_origins(
        &mut self,
        origins: Vec<super::indirect::Origin>,
    ) -> Result<(), Error> {
        let bytes = origins
            .capacity()
            .checked_mul(size_of::<super::indirect::Origin>())
            .ok_or(Error::Limit("graphics provenance capacity"))?;
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_add(bytes)
            .ok_or(Error::Limit("graphics provenance capacity"))?;
        self.origins = origins;
        Ok(())
    }
    pub fn command_location(&self, expanded_dword: usize) -> Option<CommandLocation> {
        let at = expanded_dword.checked_mul(4)?;
        let index = self
            .origins
            .partition_point(|origin| origin.at <= at)
            .checked_sub(1)?;
        let origin = self.origins.get(index)?;
        if !origin.emitted || (at - origin.at) / 4 >= origin.words {
            return None;
        }
        Some(CommandLocation {
            address: origin.address,
            dword: origin.dword + (at - origin.at) / 4,
        })
    }
    pub(super) fn source_usage(&mut self, command_bytes: usize, packets: u64) {
        self.usage.command_bytes = command_bytes;
        self.usage.packets = packets;
    }
    pub fn draws(&self) -> &[Draw] {
        &self.draws
    }
    pub fn actions(&self) -> &[Action] {
        &self.actions
    }
    pub fn memory_action(&self, index: usize) -> &MemoryAction {
        &self.memory[index]
    }
    pub fn final_state(&self) -> GraphicsState {
        self.final_state
    }
    pub fn usage(&self) -> CommandUsage {
        self.usage
    }
}

fn work(usage: &mut CommandUsage, limits: &CommandLimits, count: u64) -> Result<(), Error> {
    usage.work = usage
        .work
        .checked_add(count)
        .filter(|n| *n <= limits.max_work)
        .ok_or(Error::Limit("command work"))?;
    Ok(())
}

/// Admit all supplied flat streams before shader/resource reads or effects.
/// Unknown register writes, predicates, IBs, barriers and indirect draws
/// are explicit refusals. The caller publishes final_state only after execution
/// of the entire admitted plan succeeds; no partial prefix is returned.
pub fn plan_commands(
    buffers: &[&[u8]],
    initial: GraphicsState,
    limits: &CommandLimits,
) -> Result<CommandPlan, Error> {
    if buffers.len() > limits.max_buffers {
        return Err(Error::Limit("command buffers"));
    }
    let mut usage = CommandUsage::default();
    let metadata_work = (buffers.len() as u64)
        .checked_mul(2)
        .ok_or(Error::Limit("command metadata work"))?;
    work(&mut usage, limits, metadata_work)?;
    for buffer in buffers {
        if !buffer.len().is_multiple_of(4) {
            return Err(Error::Unsupported("command byte alignment"));
        }
        usage.command_bytes = usage
            .command_bytes
            .checked_add(buffer.len())
            .filter(|n| *n <= limits.max_command_bytes)
            .ok_or(Error::Limit("command bytes"))?;
    }
    let command_bytes = usage.command_bytes;
    work(&mut usage, limits, command_bytes as u64)?;
    let draw_bytes = limits
        .max_draws
        .checked_mul(size_of::<Draw>())
        .ok_or(Error::Limit("draw storage"))?;
    let action_count = limits
        .max_draws
        .checked_mul(2)
        .ok_or(Error::Limit("action storage"))?;
    let action_bytes = action_count
        .checked_mul(size_of::<Action>())
        .ok_or(Error::Limit("action storage"))?;
    usage.host_bytes = draw_bytes
        .checked_add(action_bytes)
        .ok_or(Error::Limit("action storage"))?
        .checked_add(size_of::<CommandPlan>())
        .and_then(|n| n.checked_add(size_of::<GraphicsState>()))
        .filter(|n| *n <= limits.max_host_bytes)
        .ok_or(Error::Limit("command host bytes"))?;
    let storage_work = draw_bytes
        .checked_add(
            size_of::<GraphicsState>()
                .checked_mul(2)
                .ok_or(Error::Limit("command state work"))?,
        )
        .ok_or(Error::Limit("command state work"))?;
    work(&mut usage, limits, storage_work as u64)?;
    let mut draws = Vec::new();
    draws
        .try_reserve_exact(limits.max_draws)
        .map_err(|_| Error::Allocation)?;
    let excess = draws
        .capacity()
        .checked_sub(limits.max_draws)
        .and_then(|n| n.checked_mul(size_of::<Draw>()))
        .ok_or(Error::Limit("draw storage"))?;
    usage.host_bytes = usage
        .host_bytes
        .checked_add(excess)
        .filter(|n| *n <= limits.max_host_bytes)
        .ok_or(Error::Limit("command host bytes"))?;
    let mut actions = Vec::new();
    actions
        .try_reserve_exact(action_count)
        .map_err(|_| Error::Allocation)?;
    usage.host_bytes = usage
        .host_bytes
        .checked_add(
            (actions.capacity() - action_count)
                .checked_mul(size_of::<Action>())
                .ok_or(Error::Limit("action storage"))?,
        )
        .filter(|n| *n <= limits.max_host_bytes)
        .ok_or(Error::Limit("command host bytes"))?;
    work(&mut usage, limits, action_bytes as u64)?;
    let mut state = initial;
    let mut memory = Vec::<MemoryAction>::new();
    let mut fresh_draw = false;
    for (buffer_index, bytes) in buffers.iter().enumerate() {
        let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let mut at = 0;
        while at < bytes.len() / 4 {
            usage.packets = usage
                .packets
                .checked_add(1)
                .filter(|n| *n <= limits.max_packets)
                .ok_or(Error::Limit("command packets"))?;
            work(&mut usage, limits, 1)?;
            let header = Header::new(word(at));
            let fail = |reason| Error::Packet {
                buffer: buffer_index,
                dword: at,
                reason,
            };
            let length = if header.raw() == 0x8000_0000 {
                1
            } else if header.raw() == 0xc004_105c {
                6
            } else if header.raw() == 0xc006_1060 {
                8
            } else if header.raw() == 0xc002_7904 {
                work(&mut usage, limits, 8)?;
                crate::compute::tagged_wait_dwords(&bytes[at * 4..]).map_err(memory_error)?
            } else if header.packet_type() == 3 && header.low_byte() == 0 {
                if header.opcode() == 0x10 && header.count() == 0x3fff {
                    1
                } else {
                    usize::from(header.count()) + 2
                }
            } else {
                return Err(fail("graphics packet header"));
            };
            let end = at
                .checked_add(length)
                .filter(|n| *n <= bytes.len() / 4)
                .ok_or(fail("truncated packet"))?;
            if header.raw() == 0xc004_105c {
                if actions.len() >= action_count {
                    return Err(Error::Limit("command actions"));
                }
                if !fresh_draw {
                    return Err(Error::Unsupported("GPU flip needs a fresh draw"));
                }
                work(&mut usage, limits, size_of::<Action>() as u64)?;
                actions.push(Action::Flip(Flip {
                    handle: word(at + 1) as i32,
                    buffer: word(at + 2) as i32,
                    mode: word(at + 3) as i32,
                    argument: (u64::from(word(at + 4)) | (u64::from(word(at + 5)) << 32)) as i64,
                }));
                usage.flips += 1;
                fresh_draw = false;
                at = end;
                continue;
            }
            if length > 1
                && header.opcode() == 0x10
                && matches!(word(at + 1), 0x6875_0777 | 0x6875_0778 | 0x6875_0781)
            {
                return Err(Error::Unsupported("unsupported graphics flip marker"));
            }
            if length == 1 || (header.opcode() == 0x10 && header.raw() != 0xc006_1060) {
                at = end;
                continue;
            }
            if (matches!(
                header.opcode(),
                0x27 | 0x35 | 0x2d | 0x37 | 0x3c | 0x93 | 0x49 | 0x42
            ) || matches!(header.raw(), 0xc006_1060 | 0xc002_7904))
                && actions.len() >= action_count
            {
                return Err(Error::Limit("command actions"));
            }
            if matches!(header.opcode(), 0x37 | 0x3c | 0x93 | 0x49 | 0x42)
                || matches!(header.raw(), 0xc006_1060 | 0xc002_7904)
            {
                if memory.capacity() == 0 {
                    let expected = action_count
                        .checked_mul(size_of::<MemoryAction>())
                        .ok_or(Error::Limit("memory action storage"))?;
                    usage.host_bytes = usage
                        .host_bytes
                        .checked_add(expected)
                        .filter(|n| *n <= limits.max_host_bytes)
                        .ok_or(Error::Limit("command host bytes"))?;
                    work(&mut usage, limits, expected as u64)?;
                    memory
                        .try_reserve_exact(action_count)
                        .map_err(|_| Error::Allocation)?;
                    let excess = memory
                        .capacity()
                        .checked_sub(action_count)
                        .and_then(|n| n.checked_mul(size_of::<MemoryAction>()))
                        .ok_or(Error::Limit("memory action storage"))?;
                    usage.host_bytes = usage
                        .host_bytes
                        .checked_add(excess)
                        .filter(|n| *n <= limits.max_host_bytes)
                        .ok_or(Error::Limit("command host bytes"))?;
                }
                let (action, charged) = crate::compute::memory_packet(
                    &bytes[at * 4..end * 4],
                    limits
                        .max_host_bytes
                        .checked_sub(usage.host_bytes)
                        .ok_or(Error::Limit("command host bytes"))?,
                    limits
                        .max_work
                        .checked_sub(usage.work)
                        .ok_or(Error::Limit("command work"))?,
                )
                .map_err(memory_error)?;
                work(&mut usage, limits, charged.work)?;
                usage.host_bytes = usage
                    .host_bytes
                    .checked_add(charged.host_bytes)
                    .filter(|n| *n <= limits.max_host_bytes)
                    .ok_or(Error::Limit("command host bytes"))?;
                if header.raw() == 0xc002_7904 {
                    usage.packets = usage
                        .packets
                        .checked_add(2)
                        .filter(|n| *n <= limits.max_packets)
                        .ok_or(Error::Limit("command packets"))?;
                }
                actions.push(Action::Memory(memory.len()));
                memory.push(action);
                // A later flip needs a newly completed draw. A prior receipt
                // cannot certify pixels after a possibly aliasing memory action.
                fresh_draw = false;
                at = end;
                continue;
            }
            match header.opcode() {
                0x69 | 0x76 | 0x79 if length >= 3 => {
                    let space = match header.opcode() {
                        0x69 => RegisterSpace::Context,
                        0x76 => RegisterSpace::Sh,
                        _ => RegisterSpace::Uconfig,
                    };
                    let first = word(at + 1);
                    let count = u32::try_from(length - 2).map_err(|_| fail("register count"))?;
                    if first > 0xffff || first.checked_add(count).is_none_or(|n| n > 0x10000) {
                        return Err(fail("register selector"));
                    }
                    work(&mut usage, limits, u64::from(count))?;
                    for i in 0..count {
                        state.write(space, first + i, word(at + 2 + i as usize))?;
                    }
                }
                0x2f if length == 2 => state.instances = Some(word(at + 1)),
                0x7a if header.raw() == 0xc001_7a00 => {
                    let value = word(at + 2);
                    // Exact indexed register grammar emitted by SetIndexSize.
                    // Cache policy affects residency, not the immutable upload.
                    if word(at + 1) != 0x2000_0243 || value & !0x4c3 != 0 || value & 0x400 == 0 {
                        return Err(fail("indexed register profile"));
                    }
                    state.index_type = Some(match value & 3 {
                        0 => IndexType::U16,
                        1 => IndexType::U32,
                        _ => return Err(fail("index type profile")),
                    });
                }
                0x26 if header.raw() == 0xc001_2600 => {
                    let address = u64::from(word(at + 1)) | (u64::from(word(at + 2)) << 32);
                    if address & 1 != 0 || address >= 1 << 47 {
                        return Err(fail("index base profile"));
                    }
                    state.index_base = Some(address);
                }
                0x13 if header.raw() == 0xc000_1300 => state.index_count = Some(word(at + 1)),
                0x2a if header.raw() == 0xc000_2a00 => {
                    state.index_type = Some(match word(at + 1) {
                        0 => IndexType::U16,
                        1 => IndexType::U32,
                        _ => return Err(fail("index type profile")),
                    });
                }
                0x2d if length == 3 => {
                    // AUTO_INDEX source only. USE_OPAQUE/streamout draw counts
                    // cannot be replaced with the encoded count or guessed.
                    if word(at + 2) != 2 {
                        return Err(fail("draw initiator"));
                    }
                    let vertex_count = word(at + 1);
                    let instance_count = state
                        .instances
                        .ok_or(Error::Unsupported("unknown NUM_INSTANCES"))?;
                    usage.draw_packets = usage
                        .draw_packets
                        .checked_add(1)
                        .ok_or(Error::Limit("draw packets"))?;
                    if vertex_count == 0 || instance_count == 0 {
                        usage.zero_draws = usage
                            .zero_draws
                            .checked_add(1)
                            .ok_or(Error::Limit("zero draws"))?;
                    } else {
                        if draws.len() >= limits.max_draws {
                            return Err(Error::Limit("draw count"));
                        }
                        let vertices = u64::from(vertex_count)
                            .checked_mul(u64::from(instance_count))
                            .ok_or(Error::Limit("draw vertices"))?;
                        usage.vertices = usage
                            .vertices
                            .checked_add(vertices)
                            .filter(|n| *n <= limits.max_vertices)
                            .ok_or(Error::Limit("draw vertices"))?;
                        work(&mut usage, limits, size_of::<Draw>() as u64)?;
                        actions.push(Action::Draw(draws.len()));
                        fresh_draw = true;
                        draws.push(Draw {
                            state,
                            vertex_count,
                            instance_count,
                            indices: None,
                        });
                    }
                }
                0x27 | 0x35 if matches!(header.raw(), 0xc004_2700 | 0xc003_3500) => {
                    let maximum = word(at + 1);
                    let offset = header.opcode() == 0x35;
                    let count = word(at + if offset { 3 } else { 4 });
                    if count > maximum
                        || word(at + if offset { 4 } else { 5 }) != 0
                        || (!offset && word(at + 3) >= 0x8000)
                    {
                        return Err(fail("indexed draw profile"));
                    }
                    let instances = state
                        .instances
                        .ok_or(Error::Unsupported("unknown NUM_INSTANCES"))?;
                    usage.draw_packets = usage
                        .draw_packets
                        .checked_add(1)
                        .ok_or(Error::Limit("draw packets"))?;
                    if count == 0 || instances == 0 {
                        usage.zero_draws = usage
                            .zero_draws
                            .checked_add(1)
                            .ok_or(Error::Limit("zero draws"))?;
                    } else {
                        let index_type = state
                            .index_type
                            .ok_or(Error::Unsupported("unknown INDEX_TYPE"))?;
                        let address = if offset {
                            // The pinned direct OFFSET path uses the exact
                            // count, not INDEX_BUFFER_SIZE (an indirect bound).
                            state
                                .index_base
                                .ok_or(Error::Unsupported("unknown INDEX_BASE"))?
                                .checked_add(
                                    u64::from(word(at + 2)) * index_type.byte_width() as u64,
                                )
                                .ok_or(fail("index offset extent"))?
                        } else {
                            u64::from(word(at + 2)) | (u64::from(word(at + 3)) << 32)
                        };
                        let size = u64::from(count)
                            .checked_mul(index_type.byte_width() as u64)
                            .ok_or(Error::Limit("index bytes"))?;
                        if address == 0
                            || !address.is_multiple_of(index_type.byte_width() as u64)
                            || address.checked_add(size).is_none_or(|end| end > 1 << 47)
                        {
                            return Err(fail("index extent"));
                        }
                        if draws.len() >= limits.max_draws {
                            return Err(Error::Limit("draw count"));
                        }
                        let vertices = u64::from(count)
                            .checked_mul(u64::from(instances))
                            .ok_or(Error::Limit("draw vertices"))?;
                        usage.vertices = usage
                            .vertices
                            .checked_add(vertices)
                            .filter(|n| *n <= limits.max_vertices)
                            .ok_or(Error::Limit("draw vertices"))?;
                        work(&mut usage, limits, size_of::<Draw>() as u64)?;
                        actions.push(Action::Draw(draws.len()));
                        fresh_draw = true;
                        draws.push(Draw {
                            state,
                            vertex_count: count,
                            instance_count: instances,
                            indices: Some(IndexSource {
                                address,
                                index_type,
                            }),
                        });
                    }
                }
                _ => return Err(fail("command outside flat graphics profile")),
            }
            at = end;
        }
    }
    Ok(CommandPlan {
        draws,
        actions,
        memory,
        final_state: state,
        usage,
        origins: Vec::new(),
    })
}

fn memory_error(error: crate::compute::Error) -> Error {
    match error {
        crate::compute::Error::Limit(reason) => Error::Limit(reason),
        crate::compute::Error::Allocation => Error::Allocation,
        crate::compute::Error::Unsupported(reason) => Error::Unsupported(reason),
        crate::compute::Error::Packet { reason, .. } => Error::Unsupported(reason),
        _ => Error::Unsupported("graphics memory packet profile"),
    }
}
