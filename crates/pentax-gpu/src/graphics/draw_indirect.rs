// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from public KytyPS5 7b7f9a70 agc.cpp/graphicsRun.cpp;
// original bounded live-argument and native-shader integration. See NOTICE.
use super::{Draw, Error, GraphicsState, RegisterSpace};
use crate::compute::{CommandRange, Memory};

/// Native shader inputs, not Vulkan offsets or guest-memory authority.
#[derive(Clone, Copy)]
pub struct NativeIndirectOffsets {
    base_vertex: i32,
    first_instance: u32,
    first_index: u32,
}
impl NativeIndirectOffsets {
    pub fn base_vertex(self) -> i32 {
        self.base_vertex
    }
    pub fn first_instance(self) -> u32 {
        self.first_instance
    }
    pub fn first_index(self) -> u32 {
        self.first_index
    }
}

/// Sealed descriptor from the exact single indexed-indirect packet.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IndirectDraw {
    offset: u32,
    vertex: u32,
    instance: u32,
    index: u32,
}
impl IndirectDraw {
    pub(super) fn decode(words: [u32; 5]) -> Result<Self, Error> {
        if words[0] != 0xc003_2500 || words[4] & !0x20 != 0 || words[3] & !0x1000_ffff != 0 {
            return Err(Error::Unsupported("indexed indirect packet profile"));
        }
        let result = Self {
            offset: words[1],
            vertex: words[2] & 0xffff,
            instance: words[3] & 0xffff,
            index: if words[3] & (1 << 28) != 0 {
                words[2] >> 16
            } else {
                0x280
            },
        };
        for location in [result.vertex, result.instance, result.index] {
            if location != 0x280 && !(0x8c..=0x9b).contains(&location) {
                return Err(Error::Unsupported("indexed indirect vertex user SGPR"));
            }
        }
        Ok(result)
    }
    pub(super) fn resolve(
        self,
        state: &mut GraphicsState,
        memory: &impl Memory,
        max_vertices: u64,
    ) -> Result<Option<Draw>, Error> {
        let base = state
            .draw_indirect_base
            .filter(|base| *base != 0)
            .ok_or(Error::Unsupported("unknown draw indirect base"))?;
        // The source validates the indexed type before reading the arguments.
        let kind = state
            .index_type
            .ok_or(Error::Unsupported("unknown INDEX_TYPE"))?;
        let address = base
            .checked_add(u64::from(self.offset))
            .filter(|address| address.is_multiple_of(4))
            .filter(|address| address.checked_add(20).is_some_and(|end| end <= 1 << 47))
            .ok_or(Error::Unsupported("indexed indirect argument extent"))?;
        let mut raw = [0; 20];
        memory.read(address, &mut raw).map_err(Error::Memory)?;
        let args: [u32; 5] =
            std::array::from_fn(|i| u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap()));
        // Keep the exact source patch order, including aliased locations.
        let mut next = *state;
        for (location, value) in [
            (self.vertex, args[3]),
            (self.instance, args[4]),
            (self.index, args[2]),
        ] {
            if location != 0x280 {
                next.write(RegisterSpace::Sh, location, value)?;
            }
        }
        let bound = next
            .index_count
            .ok_or(Error::Unsupported("unknown INDEX_BUFFER_SIZE"))?;
        let count = if bound == 0 {
            args[0]
        } else {
            args[0].min(bound)
        };
        next.instances = Some(args[1]);
        if count == 0 || args[1] == 0 {
            *state = next;
            return Ok(None);
        }
        u64::from(count)
            .checked_mul(u64::from(args[1]))
            .filter(|vertices| *vertices <= max_vertices)
            .ok_or(Error::Limit("indirect draw vertices"))?;
        let index_address = next
            .index_base
            .ok_or(Error::Unsupported("unknown INDEX_BASE"))?
            .checked_add(u64::from(args[2]) * kind.byte_width() as u64)
            .ok_or(Error::Unsupported("index offset extent"))?;
        let size = u64::from(count) * kind.byte_width() as u64;
        if index_address == 0
            || !index_address.is_multiple_of(kind.byte_width() as u64)
            || index_address
                .checked_add(size)
                .is_none_or(|end| end > 1 << 47)
        {
            return Err(Error::Unsupported("index extent"));
        }
        let draw = Draw {
            state: next,
            vertex_count: count,
            instance_count: args[1],
            indices: Some(super::commands::IndexSource {
                address: index_address,
                index_type: kind,
            }),
            native_offsets: Some(NativeIndirectOffsets {
                base_vertex: args[3] as i32,
                first_instance: args[4],
                first_index: args[2],
            }),
            indirect_arguments: Some(CommandRange { address, size: 20 }),
        };
        *state = next;
        Ok(Some(draw))
    }
}

impl std::fmt::Debug for IndirectDraw {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndirectDraw").finish_non_exhaustive()
    }
}
