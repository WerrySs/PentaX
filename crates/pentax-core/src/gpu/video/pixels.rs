// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked, bounded same-owner scanout snapshots.
// Adapted from pinned tile.cpp Gen5RenderTargetOffsetInBlock; see NOTICE.

use super::{Compression, Failure, PixelFormat, Surface};
use pentax_platform::native_memory::PausedNativeMemory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceLayout {
    Linear,
    RenderTarget64Kb,
}

/// Explicit geometry for a pure byte conversion, never a guest-memory capability.
#[derive(Clone, Copy, Debug)]
pub struct ImageLayout {
    pub kind: SurfaceLayout,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct PresentationLimits {
    pub max_frame_bytes: usize,
    pub max_held_bytes: usize,
    pub max_work: u64,
}
impl Default for PresentationLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 64 * 1024 * 1024,
            max_held_bytes: 128 * 1024 * 1024,
            max_work: 1024 * 1024 * 1024,
        }
    }
}
pub(super) fn charge(
    work: &mut u64,
    amount: u64,
    limits: &PresentationLimits,
) -> Result<(), Failure> {
    let next = work
        .checked_add(amount)
        .filter(|v| *v <= limits.max_work)
        .ok_or(Failure::Limit)?;
    *work = next;
    Ok(())
}
fn allocate(size: usize, held: usize, limits: &PresentationLimits) -> Result<Vec<u8>, Failure> {
    if size > limits.max_frame_bytes
        || held
            .checked_add(size)
            .is_none_or(|v| v > limits.max_held_bytes)
    {
        return Err(Failure::Limit);
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| Failure::Allocation)?;
    if bytes.capacity() > limits.max_frame_bytes
        || held
            .checked_add(bytes.capacity())
            .is_none_or(|v| v > limits.max_held_bytes)
    {
        return Err(Failure::Limit);
    }
    bytes.resize(size, 0);
    Ok(bytes)
}
impl ImageLayout {
    fn sizes(self) -> Result<(usize, usize), Failure> {
        if self.width == 0
            || self.height == 0
            || self.width > 16384
            || self.height > 16384
            || self.pitch < self.width
            || self.pitch > 16384
        {
            return Err(Failure::UnsupportedVideo);
        }
        let height = match self.kind {
            SurfaceLayout::Linear => self.height,
            SurfaceLayout::RenderTarget64Kb if self.pitch.is_multiple_of(128) => {
                self.height.next_multiple_of(128)
            }
            _ => return Err(Failure::UnsupportedVideo),
        };
        let source = u64::from(self.pitch) * u64::from(height) * 4;
        let tight = u64::from(self.width) * u64::from(self.height) * 4;
        Ok((
            usize::try_from(source).map_err(|_| Failure::Limit)?,
            usize::try_from(tight).map_err(|_| Failure::Limit)?,
        ))
    }
    fn offset(self, x: u32, y: u32) -> usize {
        match self.kind {
            SurfaceLayout::Linear => {
                ((u64::from(y) * u64::from(self.pitch) + u64::from(x)) * 4) as usize
            }
            SurfaceLayout::RenderTarget64Kb => {
                let block = u64::from(y / 128) * u64::from(self.pitch / 128) + u64::from(x / 128);
                let local = render_target_32_offset(x % 128, y % 128);
                let xor = render_target_32_offset(x / 128 * 128, y / 128 * 128);
                (block * 65536 + u64::from(local ^ xor)) as usize
            }
        }
    }
}
fn render_target_32_offset(x: u32, y: u32) -> u32 {
    ((y << 4) & 0x0070)
        ^ ((y << 5) & 0x0f00)
        ^ ((y << 9) & 0x1000)
        ^ ((y << 8) & 0x4000)
        ^ ((x << 2) & 0x000c)
        ^ ((x << 5) & 0x0380)
        ^ ((x << 4) & 0x0400)
        ^ ((x << 6) & 0x0800)
        ^ ((x << 9) & 0xa000)
}

/// Decode explicit linear or pinned one-level RenderTarget64KB 32bpp bytes.
/// `held` includes the input's actual capacity and all other live staging.
pub fn detile_32(
    input: &[u8],
    layout: ImageLayout,
    held: usize,
    limits: &PresentationLimits,
    work: &mut u64,
) -> Result<Vec<u8>, Failure> {
    let (source, tight) = layout.sizes()?;
    if input.len() != source {
        return Err(Failure::UnsupportedVideo);
    }
    charge(
        work,
        (tight as u64).checked_mul(4).ok_or(Failure::Limit)?,
        limits,
    )?;
    let mut output = allocate(tight, held, limits)?;
    for y in 0..layout.height {
        for x in 0..layout.width {
            let from = layout.offset(x, y);
            let to = ((u64::from(y) * u64::from(layout.width) + u64::from(x)) * 4) as usize;
            output[to..to + 4]
                .copy_from_slice(input.get(from..from + 4).ok_or(Failure::UnsupportedVideo)?);
        }
    }
    Ok(output)
}
/// Encode tight 32bpp bytes to explicit layout, zeroing all admitted padding.
/// Guest LOAD/writeback must overlay active bytes onto its retained original
/// full snapshot; this standalone product does not preserve unknown padding.
pub fn retile_32(
    input: &[u8],
    layout: ImageLayout,
    held: usize,
    limits: &PresentationLimits,
    work: &mut u64,
) -> Result<Vec<u8>, Failure> {
    let (source, tight) = layout.sizes()?;
    if input.len() != tight {
        return Err(Failure::UnsupportedVideo);
    }
    charge(
        work,
        (tight as u64).checked_mul(4).ok_or(Failure::Limit)?,
        limits,
    )?;
    let mut output = allocate(source, held, limits)?;
    retile_pixels(input, layout, &mut output)?;
    Ok(output)
}

/// Overlay active tight pixels onto an existing complete source-layout image.
/// All padding is preserved. Exact geometry, both spans, conversion work and
/// every destination offset are admitted before the first output mutation.
/// This function allocates nothing and grants no native-memory authority.
pub fn retile_32_into(
    input: &[u8],
    layout: ImageLayout,
    output: &mut [u8],
    limits: &PresentationLimits,
    work: &mut u64,
) -> Result<(), Failure> {
    let (source, tight) = layout.sizes()?;
    if input.len() != tight || output.len() != source {
        return Err(Failure::UnsupportedVideo);
    }
    if source > limits.max_frame_bytes || tight > limits.max_frame_bytes {
        return Err(Failure::Limit);
    }
    charge(
        work,
        (tight as u64).checked_mul(4).ok_or(Failure::Limit)?,
        limits,
    )?;
    retile_pixels(input, layout, output)
}

fn retile_pixels(input: &[u8], layout: ImageLayout, output: &mut [u8]) -> Result<(), Failure> {
    for y in 0..layout.height {
        for x in 0..layout.width {
            let to = layout.offset(x, y);
            if output.get(to..to + 4).is_none() {
                return Err(Failure::UnsupportedVideo);
            }
        }
    }
    for y in 0..layout.height {
        for x in 0..layout.width {
            let from = ((u64::from(y) * u64::from(layout.width) + u64::from(x)) * 4) as usize;
            let to = layout.offset(x, y);
            output
                .get_mut(to..to + 4)
                .ok_or(Failure::UnsupportedVideo)?
                .copy_from_slice(&input[from..from + 4]);
        }
    }
    Ok(())
}

pub(super) fn snapshot(
    surface: Surface,
    memory: &PausedNativeMemory<'_>,
    held: usize,
    limits: &PresentationLimits,
    work: &mut u64,
) -> Result<Vec<u8>, Failure> {
    if surface.compression != Compression::None
        || surface.metadata.is_some()
        || surface.bytes_per_pixel != 4
        || !matches!(
            surface.format,
            PixelFormat::Rgba8Srgb
                | PixelFormat::Bgra8Srgb
                | PixelFormat::Rgb10A2
                | PixelFormat::Bgr10A2
        )
    {
        return Err(Failure::UnsupportedVideo);
    }
    let layout = ImageLayout {
        kind: SurfaceLayout::RenderTarget64Kb,
        width: surface.width,
        height: surface.height,
        pitch: surface.pitch,
    };
    let (size, _) = layout.sizes()?;
    if surface.data.size != size as u64 {
        return Err(Failure::UnsupportedVideo);
    }
    // Scanout authority follows GPU read permissions, including GPU-only pages.
    charge(
        work,
        surface.data.size.checked_add(128).ok_or(Failure::Limit)?,
        limits,
    )?;
    memory
        .preflight_gpu_read(surface.data.address, surface.data.size)
        .map_err(|_| Failure::Memory)?;
    let mut source = allocate(size, held, limits)?;
    memory
        .read_gpu(surface.data.address, &mut source)
        .map_err(|_| Failure::Memory)?;
    let live = held.checked_add(source.capacity()).ok_or(Failure::Limit)?;
    let mut output = detile_32(&source, layout, live, limits, work)?;
    if surface.format != PixelFormat::Rgba8Srgb {
        charge(work, output.len() as u64, limits)?;
        for pixel in output.chunks_exact_mut(4) {
            match surface.format {
                PixelFormat::Bgra8Srgb => pixel.swap(0, 2),
                PixelFormat::Rgb10A2 | PixelFormat::Bgr10A2 => {
                    let value = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                    let channel =
                        |shift: u32| ((((value >> shift) & 1023_u32) * 255 + 511) / 1023) as u8;
                    let (r, b) = if surface.format == PixelFormat::Rgb10A2 {
                        (channel(0), channel(20))
                    } else {
                        (channel(20), channel(0))
                    };
                    pixel.copy_from_slice(&[r, channel(10), b, ((value >> 30) * 85) as u8]);
                }
                _ => return Err(Failure::UnsupportedVideo),
            }
        }
    }
    Ok(output)
}
