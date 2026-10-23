// SPDX-License-Identifier: GPL-2.0-only
// Original bounded lowering, informed by the publicly available
// Jetsku/KytyPS5 b719d67 spirvEmitterImage.cpp image sample/query operations.
// Descriptor bytes, texture storage and sampler state remain caller-owned.

use super::{Opnd, Translator};
use crate::{
    CompileError,
    error::UnsupportedReason,
    ir::{Op, Value},
};
use pentax_shader::{ImageDimension, ImageSampleFlags as F, Opcode, OperandKind, ShaderStage};

impl Translator<'_> {
    fn image_handle(&mut self, base: Opnd, image: bool) -> Result<Value, CompileError> {
        let count = if image { 8 } else { 4 };
        if base.kind != OperandKind::Sgpr
            || !base.reg.is_multiple_of(4)
            || u32::from(base.reg) + count > 106
        {
            return Err(self.unsupported(UnsupportedReason::DynamicDescriptor));
        }
        let mut words = [Value::U32(0); 8];
        for (offset, word) in words.iter_mut().take(count as usize).enumerate() {
            *word = self.read_scalar_code(u32::from(base.reg) + offset as u32)?;
        }
        self.e.emit(
            if image {
                Op::GetImageResource
            } else {
                Op::GetSamplerResource
            },
            &words[..count as usize],
        )
    }

    pub(super) fn image_sample(&mut self) -> Result<(), CompileError> {
        let inst = self.inst;
        let image = inst
            .image
            .ok_or(CompileError::Internal("image metadata missing"))?;
        let memory = inst
            .memory
            .ok_or(CompileError::Internal("image access missing"))?;
        let flags = image.sample_flags();
        if self.profile.stage() != ShaderStage::Pixel
            || image.dimension() != ImageDimension::Dim2D
            || image.nsa_dwords() != 0
            || image.r128()
            || memory.data_bits() != 32
            || memory.tfe()
            || memory.lwe()
            || inst.src(0).kind != OperandKind::Vgpr
            || !matches!(flags, F::NONE | F::LOD | F::LEVEL_ZERO)
            || image.dmask() == 0
            || (inst.opcode == Opcode::ImageGetLod && image.dmask() & !3 != 0)
        {
            return Err(self.unsupported(UnsupportedReason::ImageMode));
        }
        let texture = self.image_handle(inst.src(1), true)?;
        let sampler = self.image_handle(inst.src(2), false)?;
        let x = self.read_f32(inst.src(0))?;
        let y_op = self.offset_operand(inst.src(0), 1)?;
        let y = self.read_f32(y_op)?;
        let coords = self.e.emit(Op::CompositeConstructF32x2, &[x, y])?;
        let query = inst.opcode == Opcode::ImageGetLod;
        let result = if query {
            self.e
                .emit(Op::ImageQueryLod, &[texture, sampler, coords])?
        } else if flags == F::NONE {
            self.e
                .emit(Op::ImageSampleImplicit, &[texture, sampler, coords])?
        } else {
            let lod = if flags == F::LEVEL_ZERO {
                Value::F32(0)
            } else {
                let operand = self.offset_operand(inst.src(0), 2)?;
                self.read_f32(operand)?
            };
            self.e
                .emit(Op::ImageSampleLod, &[texture, sampler, coords, lod])?
        };
        // DMASK selects components, packed consecutively in destination VGPRs.
        // Snapshot every source before overwriting any potentially aliased VGPR.
        let mut output = [Value::U32(0); 4];
        let mut count = 0;
        let components = if query { 2 } else { 4 };
        for component in 0..components {
            if image.dmask() & (1 << component) != 0 {
                output[count] = self.e.emit_aux(
                    if query {
                        Op::CompositeExtractF32x2
                    } else {
                        Op::CompositeExtractF32x4
                    },
                    component,
                    &[result],
                )?;
                count += 1;
            }
        }
        for (offset, value) in output[..count].iter().enumerate() {
            let dst = self.offset_operand(inst.dst, offset as u16)?;
            self.write(dst, *value)?;
        }
        Ok(())
    }
}
