// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded complete flat draw admission and snapshots.

use super::{Error, GraphicsState};
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
    pub vertices: u64,
    pub host_bytes: usize,
    pub work: u64,
}

/// Private immutable snapshot from one actual DRAW_INDEX_AUTO packet.
pub struct Draw {
    pub(super) state: GraphicsState,
    pub(super) vertex_count: u32,
    pub(super) instance_count: u32,
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
    final_state: GraphicsState,
    usage: CommandUsage,
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
    pub fn draws(&self) -> &[Draw] {
        &self.draws
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
/// Unknown register writes, predicates, IBs, barriers and indexed/indirect draws
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
    usage.host_bytes = draw_bytes
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
    let mut state = initial;
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
            if length == 1 || header.opcode() == 0x10 {
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
                        draws.push(Draw {
                            state,
                            vertex_count,
                            instance_count,
                        });
                    }
                }
                _ => return Err(fail("command outside flat nonindexed graphics profile")),
            }
            at = end;
        }
    }
    Ok(CommandPlan {
        draws,
        final_state: state,
        usage,
    })
}
