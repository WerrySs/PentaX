// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: checked ordered shader-half fusion.
// Pinned agc.cpp680..824, shader.h and pm4.h; see NOTICE.

use super::*;

const INVALID_HALVES: u64 = 0x8a6c_0008;
const FUSED_EDITS: usize = 96 + 255 * 8;

pub(in crate::gpu) fn size(
    memory: &mut PausedNativeMemory<'_>,
    args: [u64; 6],
    used: &mut u64,
    maximum: u64,
) -> Result<u64, Failure> {
    let [destination, front, back, ..] = args;
    nonnull(destination, front, back)?;
    let mut work = Work { used, maximum };
    let mut edits = Edits::<MAX_EDITS>::new();
    let front_kind = edits.read::<1>(memory, at(front, 90)?, &mut work)?[0];
    let back_kind = edits.read::<1>(memory, at(back, 90)?, &mut work)?[0];
    if !matches!((front_kind, back_kind), (4, 6) | (5, 7)) {
        return Ok(INVALID_HALVES);
    }
    let count = edits.read::<1>(memory, at(back, 92)?, &mut work)?[0];
    edits.write(
        memory,
        destination,
        &(u64::from(count) * 8).to_le_bytes(),
        &mut work,
    )?;
    edits.write(memory, at(destination, 8)?, &4_u64.to_le_bytes(), &mut work)?;
    edits.publish(memory, &mut work)?;
    Ok(0)
}

fn nonnull(destination: u64, front: u64, back: u64) -> Result<(), Failure> {
    if destination == 0 || front == 0 || back == 0 {
        return Err(Failure::Shader(ShaderFailure::NullArgument));
    }
    Ok(())
}

pub(super) struct Editor<'a, 'memory, 'work> {
    edits: Edits<FUSED_EDITS>,
    memory: &'a mut PausedNativeMemory<'memory>,
    work: Work<'work>,
}
impl<'a, 'memory, 'work> Editor<'a, 'memory, 'work> {
    pub(super) fn new(
        memory: &'a mut PausedNativeMemory<'memory>,
        used: &'work mut u64,
        maximum: u64,
    ) -> Self {
        Self {
            edits: Edits::new(),
            memory,
            work: Work { used, maximum },
        }
    }
    pub(super) fn byte(&mut self, address: u64) -> Result<u8, Failure> {
        Ok(self.edits.read::<1>(self.memory, address, &mut self.work)?[0])
    }
    pub(super) fn word(&mut self, address: u64) -> Result<u32, Failure> {
        Ok(u32::from_le_bytes(self.edits.read(
            self.memory,
            address,
            &mut self.work,
        )?))
    }
    pub(super) fn half(&mut self, address: u64) -> Result<u16, Failure> {
        Ok(u16::from_le_bytes(self.edits.read(
            self.memory,
            address,
            &mut self.work,
        )?))
    }
    pub(super) fn wide(&mut self, address: u64) -> Result<u64, Failure> {
        Ok(u64::from_le_bytes(self.edits.read(
            self.memory,
            address,
            &mut self.work,
        )?))
    }
    pub(super) fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), Failure> {
        self.edits
            .write(self.memory, address, bytes, &mut self.work)
    }
    fn copy(&mut self, destination: u64, source: u64, length: usize) -> Result<(), Failure> {
        // Both source copies are multiples of the eight-byte register width.
        // Snapshot the complete input before staging overlapping destinations.
        if length > 255 * 8 || !length.is_multiple_of(8) {
            return Err(Failure::Limit);
        }
        let mut snapshot = [0; 255 * 8];
        for offset in (0..length).step_by(8) {
            snapshot[offset..offset + 8].copy_from_slice(&self.edits.read::<8>(
                self.memory,
                at(source, offset as u64)?,
                &mut self.work,
            )?);
        }
        for offset in (0..length).step_by(8) {
            self.write(
                at(destination, offset as u64)?,
                &snapshot[offset..offset + 8],
            )?;
        }
        Ok(())
    }
    fn find(
        &mut self,
        registers: u64,
        count: u8,
        offset: u32,
        mut occurrence: u8,
    ) -> Result<Option<u64>, Failure> {
        if registers == 0 {
            return Ok(None);
        }
        for index in 0..u64::from(count) {
            let address = at(registers, index * 8)?;
            if self.word(address)? != offset {
                continue;
            }
            if occurrence == 0 {
                return Ok(Some(address));
            }
            occurrence -= 1;
        }
        Ok(None)
    }
    fn required(
        &mut self,
        registers: u64,
        count: u8,
        offset: u32,
        occurrence: u8,
    ) -> Result<u64, Failure> {
        let register = self
            .find(registers, count, offset, occurrence)?
            .ok_or(Failure::Shader(ShaderFailure::Registers))?;
        at(register, 4)
    }
    fn merge(
        &mut self,
        destination: u64,
        source: u64,
        shift: u32,
        mask: u32,
    ) -> Result<(), Failure> {
        let original = self.word(destination)?;
        let source = self.word(source)?;
        let field = ((original >> shift) & mask).max((source >> shift) & mask);
        self.write(
            destination,
            &((original & !(mask << shift)) | (field << shift)).to_le_bytes(),
        )
    }
    pub(super) fn finish(mut self, result: u64) -> Result<u64, Failure> {
        self.edits.publish(self.memory, &mut self.work)?;
        Ok(result)
    }
}

pub(in crate::gpu) fn fuse(
    memory: &mut PausedNativeMemory<'_>,
    args: [u64; 6],
    recompute: bool,
    used: &mut u64,
    maximum: u64,
) -> Result<u64, Failure> {
    let [destination, front, back, scratch, ..] = args;
    nonnull(destination, front, back)?;
    let mut f = Editor::new(memory, used, maximum);
    let front_kind = f.byte(at(front, 90)?)?;
    let geometry = match front_kind {
        4 => true,
        5 => false,
        _ => return Ok(INVALID_HALVES),
    };
    if f.byte(at(back, 90)?)? != if geometry { 6 } else { 7 } {
        return Ok(INVALID_HALVES);
    }
    f.copy(destination, back, 96)?;
    f.write(at(destination, 90)?, &[if geometry { 2 } else { 3 }])?;
    let back_specials = f.wide(at(back, 40)?)?;
    let back_stages = f.word(at(back_specials, 12)?)?;
    let front_specials = f.wide(at(front, 40)?)?;
    let front_stages = f.word(at(front_specials, 12)?)?;
    if (front_stages ^ back_stages) & (1 << if geometry { 22 } else { 21 }) != 0 {
        // This source guest error preserves the already-copied header/type.
        return f.finish(INVALID_HALVES);
    }
    if scratch != 0 {
        let source = f.wide(at(back, 32)?)?;
        let count = f.byte(at(back, 92)?)?;
        f.copy(scratch, source, usize::from(count) * 8)?;
        f.write(at(destination, 32)?, &scratch.to_le_bytes())?;
    }
    let registers = f.wide(at(destination, 32)?)?;
    let count = f.byte(at(destination, 92)?)?;
    let front_count = f.byte(at(front, 92)?)?;
    let checksum = if geometry { 0x80 } else { 0x100 };
    for occurrence in 0..2 {
        let source_regs = f.wide(at(front, 32)?)?;
        let source = f.required(source_regs, front_count, checksum, occurrence)?;
        let target = f.required(registers, count, checksum, occurrence)?;
        let value = f.word(source)?;
        f.write(target, &value.to_le_bytes())?;
    }
    let rsrc1 = if geometry { 0x8a } else { 0x10a };
    let rsrc2 = rsrc1 + 1;
    let source_regs = f.wide(at(front, 32)?)?;
    let front1 = f.required(source_regs, front_count, rsrc1, 0)?;
    let source_regs = f.wide(at(front, 32)?)?;
    let front2 = f.required(source_regs, front_count, rsrc2, 0)?;
    let fused1 = f.required(registers, count, rsrc1, 0)?;
    let fused2 = f.required(registers, count, rsrc2, 0)?;
    if recompute {
        let front_vgprs = ((f.word(front1)? & 0x3f) + 1) * 4;
        let back_vgprs = ((f.word(fused1)? & 0x3f) + 1) * 4;
        let front_total = front_vgprs + (f.word(front2)? >> 28) * 8;
        let original = f.word(fused2)?;
        let back_total = back_vgprs + (original >> 28) * 8;
        let maximum = front_total.max(back_total);
        let shared = if front_vgprs.max(back_vgprs) >= maximum {
            0
        } else {
            (maximum - front_total.min(back_total) + 7) / 64
        };
        f.write(
            fused2,
            &((original & 0x0fff_ffff) | ((shared & 15) << 28)).to_le_bytes(),
        )?;
    } else {
        f.merge(fused2, front2, 28, 15)?;
    }
    f.merge(fused1, front1, 0, 0x3f)?;
    if geometry {
        f.merge(fused1, front1, 29, 3)?;
        f.merge(fused2, front2, 16, 3)?;
        let target = f.word(fused2)?;
        let source = f.word(front2)?;
        f.write(
            fused2,
            &((target & 0xfffb_ffff) | (source & 0x0004_0000)).to_le_bytes(),
        )?;
    } else {
        f.merge(fused1, front1, 28, 3)?;
    }
    let target = f.word(fused2)?;
    let source = f.word(front2)?;
    f.write(
        fused2,
        &((target & 0xf7ff_ffc1) | (source & 0x0800_003e)).to_le_bytes(),
    )?;
    // Optional address registers preserve the source's absent/malformed no-op.
    let code = f.wide(at(front, 16)?)?;
    let low_offset = if geometry { 0xc8 } else { 0x148 };
    if let Some(low) = f.find(registers, count, low_offset, 0)? {
        let next = at(low, 8)?;
        if next < at(registers, u64::from(count) * 8)? && f.word(next)? == low_offset + 1 {
            f.write(at(low, 4)?, &((code >> 8) as u32).to_le_bytes())?;
            let high = f.word(at(next, 4)?)?;
            f.write(
                at(next, 4)?,
                &((high & 0xffff_ff00) | ((code >> 40) as u32 & 255)).to_le_bytes(),
            )?;
        }
    }
    let user_data = if recompute { 0 } else { f.wide(at(front, 8)?)? };
    f.write(at(destination, 8)?, &user_data.to_le_bytes())?;
    f.finish(0)
}
