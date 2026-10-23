// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de: frontend/translate/Memory.cpp
// (MemoryInfoFromDecoded, MemorySourceAt, GetBufferResource,
// ReadBufferAddress, S_LOAD for S_BUFFER_LOAD, BUFFER_LOAD/BUFFER_STORE for
// raw dwords, EmitMemory) and Attribute.cpp (V_INTERP_*, EXP and
// ExportTargetKindFromTarget).
// PentaX modifications: formatted, typed, sub-dword, atomic, TFE/LWE, FLAT,
// DS, unsupported MIMG modes and scalar-address forms are typed refusals; descriptor SGPR bases
// must be 4-aligned scalar registers.

use pentax_shader::{Family, Opcode as O, OperandKind, ShaderStage};

use super::{Opnd, Translator, u32v};
use crate::CompileError;
use crate::budget::LimitKind;
use crate::error::UnsupportedReason;
use crate::ir::{ExportInfo, ExportKind, MemoryInfo, Op, Value};

type R = Result<(), CompileError>;

/// Aux encoding of attribute reads: attribute, channel, mode.
pub(crate) fn attribute_aux(attribute: u32, channel: u32, mode: u32) -> u32 {
    (attribute & 0xff) | (channel & 0xff) << 8 | (mode & 0xff) << 16
}

impl Translator<'_> {
    fn add_memory_info(&mut self, info: MemoryInfo) -> Result<u32, CompileError> {
        let index = u32::try_from(self.e.ir.memory.len())
            .map_err(|_| CompileError::limit(LimitKind::IrInstructions))?;
        let limit = self.e.budget.limits.max_ir_instructions;
        self.e.budget.push(
            &mut self.e.ir.memory,
            info,
            limit,
            LimitKind::IrInstructions,
        )?;
        Ok(index)
    }

    /// Source `GetBufferResource` for a 4-aligned SGPR quad.
    fn buffer_resource(&mut self, base: Opnd) -> Result<Value, CompileError> {
        if base.kind != OperandKind::Sgpr || base.reg >= 104 {
            return Err(self.unsupported(UnsupportedReason::DynamicDescriptor));
        }
        if !base.reg.is_multiple_of(4) {
            return Err(self.unsupported(UnsupportedReason::MisalignedDescriptor));
        }
        let first = u32::from(base.reg);
        let mut dwords = [u32v(0); 4];
        for (index, dword) in dwords.iter_mut().enumerate() {
            *dword = self.read_scalar_code(first + index as u32)?;
        }
        self.e.emit(Op::GetBufferResource, &dwords)
    }

    /// Source `ScalarDestinationOperand`.
    fn scalar_destination(&self, op: Opnd, offset: u32) -> Result<Opnd, CompileError> {
        let code = match op.kind {
            OperandKind::Sgpr => u32::from(op.reg),
            OperandKind::VccLo => 106,
            OperandKind::VccHi => 107,
            _ => return Err(self.unsupported(UnsupportedReason::NotPorted)),
        } + offset;
        Ok(match code {
            0..=105 => Opnd {
                reg: code as u16,
                ..Opnd::special(OperandKind::Sgpr)
            },
            106 => Opnd::special(OperandKind::VccLo),
            107 => Opnd::special(OperandKind::VccHi),
            _ => return Err(self.unsupported(UnsupportedReason::NotPorted)),
        })
    }

    pub(super) fn emit_memory(&mut self) -> R {
        use UnsupportedReason as U;
        let inst = self.inst;
        match inst.opcode {
            O::SDcacheInv | O::SGl1Inv | O::BufferGl0Inv | O::BufferGl1Inv => {
                // Source: host shader memory has no guest-visible L0/GL1 cache.
                self.e.emit(Op::ControlNop, &[]).map(|_| ())
            }
            O::SLoadDword
            | O::SLoadDwordx2
            | O::SLoadDwordx4
            | O::SLoadDwordx8
            | O::SLoadDwordx16 => Err(self.unsupported(U::ScalarAddressLoad)),
            O::SBufferLoadDword
            | O::SBufferLoadDwordx2
            | O::SBufferLoadDwordx4
            | O::SBufferLoadDwordx8
            | O::SBufferLoadDwordx16 => self.s_buffer_load(),
            O::ImageSample | O::ImageGetLod => self.image_sample(),
            O::DsReadB32
            | O::DsRead2B32
            | O::DsRead2st64B32
            | O::DsWriteB32
            | O::DsWrite2B32
            | O::DsWrite2st64B32 => self.shared_memory(),
            O::SMemrealtime => Err(self.unsupported(U::RealtimeClock)),
            O::BufferLoadDword
            | O::BufferLoadDwordx2
            | O::BufferLoadDwordx3
            | O::BufferLoadDwordx4 => self.buffer_load(),
            O::BufferStoreDword
            | O::BufferStoreDwordx2
            | O::BufferStoreDwordx3
            | O::BufferStoreDwordx4 => self.buffer_store(),
            O::BufferLoadUbyte
            | O::BufferLoadSbyte
            | O::BufferLoadUshort
            | O::BufferLoadSshort
            | O::BufferStoreByte
            | O::BufferStoreShort => Err(self.unsupported(U::SubdwordBuffer)),
            O::BufferLoadFormatX
            | O::BufferLoadFormatXy
            | O::BufferLoadFormatXyz
            | O::BufferLoadFormatXyzw
            | O::BufferLoadFormatD16X
            | O::BufferStoreFormatX
            | O::BufferStoreFormatXy
            | O::BufferStoreFormatXyz
            | O::BufferStoreFormatXyzw
            | O::BufferStoreFormatD16X => Err(self.unsupported(U::FormattedBuffer)),
            _ => Err(self.unsupported(match inst.family {
                Family::Mtbuf => U::TypedBuffer,
                Family::Mubuf => U::BufferAtomic,
                Family::Flat => U::FlatOrGlobalMemory,
                Family::Ds if inst.memory.is_some_and(|m| m.gds()) => U::GlobalDataShare,
                Family::Ds => U::LocalDataShare,
                Family::Mimg => U::ImageOperation,
                _ => U::NotPorted,
            })),
        }
    }

    /// Source `S_LOAD(inst, raw = false)`.
    fn s_buffer_load(&mut self) -> R {
        let inst = self.inst;
        let access = inst
            .memory
            .ok_or(CompileError::Internal("SMEM without memory metadata"))?;
        let resource = self.buffer_resource(inst.src(0))?;
        let offset = self.read_u32(inst.src(1))?;
        let count = u32::from(access.data_dwords());
        let mut loaded = [u32v(0); 16];
        for component in 0..count {
            let memory = self.add_memory_info(MemoryInfo {
                scalar: true,
                offset: access.offset().wrapping_add(component * 4),
                data_dwords: 1,
                coherent: false,
                idxen: false,
                offen: false,
                resource: 0,
            })?;
            loaded[component as usize] =
                self.e
                    .emit_aux(Op::ReadConstBuffer, memory, &[resource, offset])?;
        }
        for component in 0..count {
            let dst = self.scalar_destination(inst.dst, component)?;
            self.write(dst, loaded[component as usize])?;
        }
        Ok(())
    }

    fn buffer_memory(&mut self) -> Result<(MemoryInfo, pentax_shader::MemoryAccess), CompileError> {
        let inst = self.inst;
        let access = inst
            .memory
            .ok_or(CompileError::Internal("MUBUF without memory metadata"))?;
        if access.tfe() || access.lwe() {
            return Err(self.unsupported(UnsupportedReason::BufferStatusReturn));
        }
        if access.formatted() || access.typed() {
            return Err(self.unsupported(UnsupportedReason::FormattedBuffer));
        }
        if access.data_bits() != 32 {
            return Err(self.unsupported(UnsupportedReason::SubdwordBuffer));
        }
        Ok((
            MemoryInfo {
                scalar: false,
                offset: access.offset(),
                data_dwords: u32::from(access.data_dwords()),
                // Source: GLC/DLC bypass L0/GL1 for non-atomic buffer accesses.
                coherent: access.glc() || access.dlc(),
                idxen: access.idxen(),
                offen: access.offen(),
                resource: 0,
            },
            access,
        ))
    }

    /// Source `ReadBufferAddress` through `MemorySourceAt`.
    fn buffer_address(&mut self, info: &MemoryInfo) -> Result<[Value; 3], CompileError> {
        let inst = self.inst;
        let index = if info.idxen {
            self.read_u32(inst.src(0))?
        } else {
            u32v(0)
        };
        let offset = if info.offen {
            let operand = if info.idxen {
                self.offset_operand(inst.src(0), 1)?
            } else {
                inst.src(0)
            };
            self.read_u32(operand)?
        } else {
            u32v(0)
        };
        let soffset_operand = inst.src(2);
        if soffset_operand.kind == OperandKind::LiteralConstant {
            // Selector 255 never reads a literal word in MUBUF (decoder note).
            return Err(self.unsupported(UnsupportedReason::NotPorted));
        }
        let soffset = self.read_u32(soffset_operand)?;
        Ok([index, offset, soffset])
    }

    fn buffer_load(&mut self) -> R {
        let inst = self.inst;
        let (info, _) = self.buffer_memory()?;
        let resource = self.buffer_resource(inst.src(1))?;
        let [index, offset, soffset] = self.buffer_address(&info)?;
        let exec = self.e.get(Op::GetExec, 0)?;
        let op = match info.data_dwords {
            1 => Op::LoadBufferU32,
            2 => Op::LoadBufferU32x2,
            3 => Op::LoadBufferU32x3,
            4 => Op::LoadBufferU32x4,
            _ => return Err(CompileError::Internal("buffer load dword count")),
        };
        let memory = self.add_memory_info(info)?;
        let loaded = self
            .e
            .emit_aux(op, memory, &[resource, index, offset, soffset, exec])?;
        if info.data_dwords == 1 {
            return self.write(inst.dst, loaded);
        }
        for component in 0..info.data_dwords {
            let value = self.e.extract(loaded, component)?;
            let dst = self.offset_operand(inst.dst, component as u16)?;
            self.write(dst, value)?;
        }
        Ok(())
    }

    fn buffer_store(&mut self) -> R {
        let inst = self.inst;
        let (info, _) = self.buffer_memory()?;
        let resource = self.buffer_resource(inst.src(1))?;
        let [index, offset, soffset] = self.buffer_address(&info)?;
        let data_src = inst.dst;
        let mut data = [u32v(0); 4];
        for (component, slot) in data.iter_mut().enumerate().take(info.data_dwords as usize) {
            let operand = self.offset_operand(data_src, component as u16)?;
            *slot = self.read_u32(operand)?;
        }
        let (op, value) = match info.data_dwords {
            1 => (Op::StoreBufferU32, data[0]),
            2 => (
                Op::StoreBufferU32x2,
                self.e.emit(Op::CompositeConstructU32x2, &data[..2])?,
            ),
            3 => (
                Op::StoreBufferU32x3,
                self.e.emit(Op::CompositeConstructU32x3, &data[..3])?,
            ),
            4 => (
                Op::StoreBufferU32x4,
                self.e.emit(Op::CompositeConstructU32x4, &data)?,
            ),
            _ => return Err(CompileError::Internal("buffer store dword count")),
        };
        let exec = self.e.get(Op::GetExec, 0)?;
        let memory = self.add_memory_info(info)?;
        self.e
            .emit_aux(op, memory, &[resource, index, offset, soffset, value, exec])
            .map(|_| ())
    }

    // ---------------------------------------------------- Attribute.cpp

    pub(super) fn emit_interpolation(&mut self) -> R {
        let inst = self.inst;
        if self.profile.stage() != ShaderStage::Pixel {
            return Err(self.unsupported(UnsupportedReason::InterpolationOnNonPixel));
        }
        let attribute = inst.src(1).value;
        let channel = inst.src(2).value;
        match inst.opcode {
            // Retain provenance of the real P1 partial until P2. Replacing
            // arbitrary modified coordinates with host interpolation is not
            // an implementation of the source arithmetic.
            O::VInterpP1F32 => {
                let i = self
                    .e
                    .get(Op::GetVectorRegister, u32::from(inst.src(0).reg))?;
                let value = self.e.emit_aux(
                    Op::InterpolationP1,
                    attribute_aux(attribute, channel, 0),
                    &[i],
                )?;
                self.write(inst.dst, value)
            }
            O::VInterpP2F32 => {
                let aux = attribute_aux(attribute, channel, 0);
                let value = if inst.src(0).kind == OperandKind::Vgpr {
                    let j = self
                        .e
                        .get(Op::GetVectorRegister, u32::from(inst.src(0).reg))?;
                    let partial = self.e.get(Op::GetVectorRegister, u32::from(inst.dst.reg))?;
                    self.e
                        .emit_aux(Op::GetAttributeWithBary, aux, &[partial, j])?
                } else {
                    return Err(self.unsupported(UnsupportedReason::PerVertexInterpolation));
                };
                self.write(inst.dst, value)
            }
            O::VInterpMovF32 => {
                let mode = inst.src(0).value;
                if mode >= 3 {
                    return Err(self.unsupported(UnsupportedReason::NotPorted));
                }
                let value = self.e.get(
                    Op::GetInterpolationParameter,
                    attribute_aux(attribute, channel, mode),
                )?;
                self.write(inst.dst, value)
            }
            _ => Err(self.unsupported(UnsupportedReason::NotPorted)),
        }
    }

    /// Source `EXP` and `AddExportInfo`.
    pub(super) fn export(&mut self) -> R {
        use UnsupportedReason as U;
        let inst = self.inst;
        let exp = inst
            .export
            .ok_or(CompileError::Internal("EXP without export metadata"))?;
        let stage = self.profile.stage();
        if stage != ShaderStage::Vertex && stage != ShaderStage::Pixel {
            return Err(self.unsupported(U::ExportOutsideGraphics));
        }
        let target = exp.target();
        let (kind, index) = match target {
            0x08 => (ExportKind::MrtZ, 0),
            0x09 => (ExportKind::Null, 0),
            0x00..=0x07 => (ExportKind::Mrt, u32::from(target)),
            0x0c..=0x0f => (ExportKind::Position, u32::from(target - 0x0c)),
            0x20..=0x3f => (ExportKind::Parameter, u32::from(target - 0x20)),
            _ => return Err(self.unsupported(U::ExportTarget(target))),
        };
        let legal = match kind {
            ExportKind::Null => true,
            ExportKind::Mrt => stage == ShaderStage::Pixel,
            ExportKind::Position | ExportKind::Parameter => stage == ShaderStage::Vertex,
            ExportKind::MrtZ => false,
        };
        if !legal {
            return Err(self.unsupported(U::ExportTarget(target)));
        }
        if kind == ExportKind::Position && index != 0 && exp.enabled() != 0 {
            return Err(self.unsupported(U::PositionAuxiliary));
        }
        if exp.compressed() && exp.enabled() != 0 && kind != ExportKind::Null {
            return Err(self.unsupported(U::CompressedExport));
        }
        let mut components = [u32v(0); 4];
        for (source, slot) in components
            .iter_mut()
            .enumerate()
            .take(inst.src_count.min(4))
        {
            *slot = self.read_raw(inst.src(source).plain())?;
        }
        let data = self.e.emit(Op::CompositeConstructU32x4, &components)?;
        let exec = self.e.get(Op::GetExec, 0)?;
        let export = u32::try_from(self.e.ir.exports.len())
            .map_err(|_| CompileError::limit(LimitKind::IrInstructions))?;
        let limit = self.e.budget.limits.max_ir_instructions;
        self.e.budget.push(
            &mut self.e.ir.exports,
            ExportInfo {
                kind,
                index,
                enabled: exp.enabled(),
                done: exp.done(),
                valid_mask: exp.valid_mask(),
                pc: inst.pc,
            },
            limit,
            LimitKind::IrInstructions,
        )?;
        self.e
            .emit_aux(Op::SetAttribute, export, &[data, exec])
            .map(|_| ())
    }
}
