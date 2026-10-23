// SPDX-License-Identifier: GPL-2.0-only
//! Owned sampled resources captured from the admitted draw's descriptor snapshot.
use super::{Budget, Error, Memory};
use crate::{
    compute::CommandRange,
    image_descriptor::{Image2D, Sampler},
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UploadedMip {
    pub width: u32,
    pub height: u32,
    pub offset: usize,
    pub bytes: usize,
}

pub struct PreparedTexture {
    pub(super) binding: u32,
    pub(super) descriptor: Image2D,
    pub(super) source: CommandRange,
    pub(super) mips: [UploadedMip; 16],
    pub(super) pixels: Vec<u8>,
}
impl PreparedTexture {
    pub fn binding(&self) -> u32 {
        self.binding
    }
    pub fn descriptor(&self) -> &Image2D {
        &self.descriptor
    }
    pub fn source(&self) -> CommandRange {
        self.source
    }
    pub fn mips(&self) -> &[UploadedMip] {
        &self.mips[..usize::from(self.descriptor.level_count)]
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

pub struct PreparedSampler {
    pub(super) binding: u32,
    pub(super) descriptor: Sampler,
}
impl PreparedSampler {
    pub fn binding(&self) -> u32 {
        self.binding
    }
    pub fn descriptor(&self) -> &Sampler {
        &self.descriptor
    }
}

pub(super) fn capture(
    descriptor: Image2D,
    binding: u32,
    memory: &impl Memory,
    target: CommandRange,
    budget: &mut Budget<'_>,
) -> Result<PreparedTexture, Error> {
    let source = CommandRange {
        address: descriptor.address,
        size: descriptor.total_bytes,
    };
    let end = source
        .address
        .checked_add(source.size)
        .ok_or(Error::Unsupported("texture extent"))?;
    let target_end = target
        .address
        .checked_add(target.size)
        .ok_or(Error::Unsupported("target extent"))?;
    if source.address == 0
        || end > 1 << 47
        || source.size == 0
        || source.address < target_end && target.address < end
        || descriptor.level_count == 0
        || descriptor.level_count > 16
    {
        return Err(Error::Unsupported("sampled texture source/target extent"));
    }
    let mut mips = [UploadedMip::default(); 16];
    let mut tight_length = 0usize;
    for (input, output) in descriptor.mips[..usize::from(descriptor.level_count)]
        .iter()
        .zip(mips.iter_mut())
    {
        let bytes = input
            .width
            .checked_mul(input.height)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(Error::Limit("texture mip bytes"))?;
        if input.width == 0 || input.height == 0 || input.row_pitch_bytes < input.width * 4 {
            return Err(Error::Unsupported("texture mip geometry"));
        }
        *output = UploadedMip {
            width: input.width,
            height: input.height,
            offset: tight_length,
            bytes,
        };
        tight_length = tight_length
            .checked_add(bytes)
            .ok_or(Error::Limit("texture upload bytes"))?;
    }
    let source_length =
        usize::try_from(source.size).map_err(|_| Error::Limit("texture source bytes"))?;
    budget.host(source_length)?;
    budget.host(tight_length)?;
    budget.work(
        source
            .size
            .checked_add(tight_length as u64)
            .ok_or(Error::Limit("texture work"))?,
    )?;
    let mut physical = Vec::new();
    physical
        .try_reserve_exact(source_length)
        .map_err(|_| Error::Allocation)?;
    budget.host(physical.capacity() - source_length)?;
    physical.resize(source_length, 0);
    memory
        .snapshot_buffer(source.address, &mut physical, true, false)
        .map_err(Error::Memory)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(tight_length)
        .map_err(|_| Error::Allocation)?;
    budget.host(pixels.capacity() - tight_length)?;
    pixels.resize(tight_length, 0);
    for (input, output) in descriptor.mips[..usize::from(descriptor.level_count)]
        .iter()
        .zip(mips.iter())
    {
        let mip_end = input
            .offset
            .checked_add(input.size)
            .ok_or(Error::Unsupported("texture mip extent"))?;
        for row in 0..u64::from(input.height) {
            let start = input
                .offset
                .checked_add(
                    row.checked_mul(u64::from(input.row_pitch_bytes))
                        .ok_or(Error::Unsupported("texture row extent"))?,
                )
                .ok_or(Error::Unsupported("texture row extent"))?;
            let end = start
                .checked_add(u64::from(input.width) * 4)
                .filter(|end| *end <= mip_end)
                .ok_or(Error::Unsupported("texture row extent"))?;
            let start = usize::try_from(start).map_err(|_| Error::Limit("texture source bytes"))?;
            let end = usize::try_from(end).map_err(|_| Error::Limit("texture source bytes"))?;
            let row_length = input.width as usize * 4;
            let destination = output.offset + row as usize * row_length;
            pixels[destination..destination + row_length].copy_from_slice(
                physical
                    .get(start..end)
                    .ok_or(Error::Unsupported("texture row backing"))?,
            );
        }
    }
    let physical_capacity = physical.capacity();
    drop(physical);
    budget.release(physical_capacity)?;
    Ok(PreparedTexture {
        binding,
        descriptor,
        source,
        mips,
        pixels,
    })
}
