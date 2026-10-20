// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned registration and checked surface geometry.
// Adapted from pinned videoOut.cpp, imageInfo.h and tile.cpp; see NOTICE.

use super::{Failure, INVALID_ADDRESS, INVALID_OPTION, INVALID_VALUE};
use pentax_platform::native_memory::{NativeRange, PausedNativeMemory};

const INVALID_INDEX: u64 = 0x8029_000a;
const SLOT_OCCUPIED: u64 = 0x8029_0010;
const INVALID_CATEGORY: u64 = 0x8029_001d;
const TRACKER_ADDRESS_SIZE: u64 = 1 << 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8Srgb,
    Bgra8Srgb,
    Rgb10A2,
    Bgr10A2,
    Rgb10A2Pq,
    Rgba16Float,
    Bgra16Float,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    None,
    Dcc256_256_0,
    Dcc256_64_64,
}
/// Retained geometry, not a memory-access capability or a presented image.
/// Consumers must revalidate the actual owning backing before reading pixels.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    pub data: NativeRange,
    pub metadata: Option<NativeRange>,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bytes_per_pixel: u32,
    pub format: PixelFormat,
    pub compression: Compression,
    pub options: u64,
    pub dcc_control: u32,
}

#[derive(Clone, Copy)]
struct Attribute([u8; 80]);
impl Attribute {
    fn read(memory: &PausedNativeMemory<'_>, address: u64) -> Result<Self, Failure> {
        let mut result = Self([0; 80]);
        memory
            .read(address, &mut result.0)
            .map_err(|_| Failure::Memory)?;
        Ok(result)
    }
    fn word(self, at: usize) -> u32 {
        u32::from_le_bytes(self.0[at..at + 4].try_into().unwrap())
    }
    fn wide(self, at: usize) -> u64 {
        u64::from_le_bytes(self.0[at..at + 8].try_into().unwrap())
    }
    fn surface(self, data: u64, metadata: u64, category: i32) -> Result<Surface, Failure> {
        let width = self.word(12);
        let height = self.word(16);
        if self.word(0) != 0
            || self.word(4) != 0
            || self.word(8) != 0
            || !(1..=16384).contains(&width)
            || !(1..=16384).contains(&height)
            || self.word(20) != 0
            || self.wide(24) & !40 != 0
            || self.word(52) != 0
            || self.0[56..] != [0; 24]
        {
            return Err(Failure::UnsupportedVideo);
        }
        let (format, bytes_per_pixel) = match self.wide(32) {
            0x8000_0000_2200_0000 => (PixelFormat::Rgba8Srgb, 4),
            0x8000_0000_0000_0000 => (PixelFormat::Bgra8Srgb, 4),
            0x8100_0000_2200_0000 => (PixelFormat::Rgb10A2, 4),
            0x8100_0000_0000_0000 => (PixelFormat::Bgr10A2, 4),
            0x8100_0704_2200_0000 => (PixelFormat::Rgb10A2Pq, 4),
            0xc001_0006_2200_0000 => (PixelFormat::Rgba16Float, 8),
            0xc001_0006_0000_0000 => (PixelFormat::Bgra16Float, 8),
            _ => return Err(Failure::UnsupportedVideo),
        };
        let control = self.word(48);
        let compression = match (category, metadata, control, self.wide(40)) {
            (0, 0, 0, 0) => Compression::None,
            (1, address, control, 0) if address != 0 && address.is_multiple_of(256) => {
                match control {
                    0x48 => Compression::Dcc256_256_0,
                    0x208 => Compression::Dcc256_64_64,
                    _ => return Err(Failure::UnsupportedVideo),
                }
            }
            _ => return Err(Failure::UnsupportedVideo),
        };
        // One level, one layer, one sample of the source RenderTarget64KB
        // layout. There is no mip tail for a single level, including tiny images.
        let pitch = width.next_multiple_of(128);
        let padded_height = height.next_multiple_of(if bytes_per_pixel == 4 { 128 } else { 64 });
        let size = u64::from(pitch) * u64::from(padded_height) * u64::from(bytes_per_pixel);
        if !data.is_multiple_of(65536) || !valid_range(data, size) {
            return Err(Failure::UnsupportedVideo);
        }
        let metadata = if compression == Compression::None {
            None
        } else {
            // Thin DCC: one 4KiB metadata block covers 1MiB of color data.
            let block_height = if bytes_per_pixel == 4 { 512 } else { 256 };
            let size =
                u64::from(width.div_ceil(512)) * u64::from(height.div_ceil(block_height)) * 4096;
            if !valid_range(metadata, size) {
                return Err(Failure::UnsupportedVideo);
            }
            Some(NativeRange {
                address: metadata,
                size,
            })
        };
        Ok(Surface {
            data: NativeRange {
                address: data,
                size,
            },
            metadata,
            width,
            height,
            pitch,
            bytes_per_pixel,
            format,
            compression,
            options: self.wide(24),
            dcc_control: control,
        })
    }
}
fn valid_range(address: u64, size: u64) -> bool {
    address != 0
        && address < TRACKER_ADDRESS_SIZE
        && size != 0
        && size <= TRACKER_ADDRESS_SIZE - address
}
#[derive(Clone, Copy)]
struct Group {
    attribute: Attribute,
    category: i32,
}
#[derive(Clone, Copy)]
struct Buffer {
    group: usize,
    surface: Surface,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Buffers {
    groups: [Option<Group>; 4],
    buffers: [Option<Buffer>; 16],
}
impl Buffers {
    pub(super) fn surface(&self, index: i32) -> Option<Surface> {
        self.buffers
            .get(usize::try_from(index).ok()?)?
            .map(|buffer| buffer.surface)
    }
    pub(super) fn register(
        &mut self,
        args: [u64; 6],
        stack: Option<u64>,
        memory: &PausedNativeMemory<'_>,
    ) -> Result<u64, Failure> {
        // Handle validation belongs to the owning port and precedes this call.
        if args[3] == 0 {
            return Ok(INVALID_ADDRESS);
        }
        if args[5] == 0 {
            return Ok(INVALID_OPTION);
        }
        let set = args[1] as u32 as i32;
        let start = args[2] as u32 as i32;
        let count = args[4] as u32 as i32;
        if !(0..4).contains(&set)
            || !(0..16).contains(&start)
            || !(1..=16).contains(&count)
            || start + count > 16
        {
            return Ok(INVALID_VALUE);
        }
        let address = stack
            .ok_or(Failure::UnsupportedVideo)?
            .checked_add(8)
            .ok_or(Failure::Memory)?;
        let mut extra = [0; 16];
        memory
            .read(address, &mut extra)
            .map_err(|_| Failure::Memory)?;
        if extra[8..] != [0; 8] {
            return Ok(INVALID_OPTION);
        }
        let category = i32::from_le_bytes(extra[..4].try_into().unwrap());
        if !matches!(category, 0 | 1) {
            return Ok(INVALID_CATEGORY);
        }
        let group = Group {
            attribute: Attribute::read(memory, args[5])?,
            category,
        };
        let mut staged = [None; 16];
        for (i, item) in staged.iter_mut().enumerate().take(count as usize) {
            let address = args[3].checked_add(i as u64 * 32).ok_or(Failure::Memory)?;
            let mut bytes = [0; 32];
            memory
                .read(address, &mut bytes)
                .map_err(|_| Failure::Memory)?;
            let data = u64::from_le_bytes(bytes[..8].try_into().unwrap());
            let metadata = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
            // Both reserved descriptor words are ignored by the pinned source.
            *item = Some(Buffer {
                group: set as usize,
                surface: group.attribute.surface(data, metadata, category)?,
            });
        }
        if self.groups[set as usize].is_some() {
            return Ok(INVALID_INDEX);
        }
        let destination = &mut self.buffers[start as usize..(start + count) as usize];
        if destination.iter().any(Option::is_some) {
            return Ok(SLOT_OCCUPIED);
        }
        self.groups[set as usize] = Some(group);
        destination.copy_from_slice(&staged[..count as usize]);
        Ok(0)
    }
    pub(super) fn change(
        &mut self,
        args: [u64; 6],
        memory: &PausedNativeMemory<'_>,
    ) -> Result<u64, Failure> {
        if args[2] == 0 {
            return Ok(INVALID_OPTION);
        }
        let set = args[1] as u32 as i32;
        if !(0..4).contains(&set) {
            return Ok(INVALID_INDEX);
        }
        if args[3] != 0 {
            return Ok(INVALID_OPTION);
        }
        let Some(current) = self.groups[set as usize] else {
            return Ok(INVALID_INDEX);
        };
        let replacement = Group {
            attribute: Attribute::read(memory, args[2])?,
            category: current.category,
        };
        let mut staged = self.buffers;
        for item in staged
            .iter_mut()
            .flatten()
            .filter(|buffer| buffer.group == set as usize)
        {
            item.surface = replacement.attribute.surface(
                item.surface.data.address,
                item.surface.metadata.map_or(0, |range| range.address),
                current.category,
            )?;
        }
        self.groups[set as usize] = Some(replacement);
        self.buffers = staged;
        Ok(0)
    }
    pub(super) fn unregister(&mut self, set: i32) -> u64 {
        if !(0..4).contains(&set) || self.groups[set as usize].is_none() {
            return INVALID_INDEX;
        }
        self.groups[set as usize] = None;
        for buffer in &mut self.buffers {
            if buffer.is_some_and(|buffer| buffer.group == set as usize) {
                *buffer = None;
            }
        }
        0
    }
}
