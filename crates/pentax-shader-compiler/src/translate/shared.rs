// SPDX-License-Identifier: GPL-2.0-only
// Adapted from public KytyPS5 7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3,
// frontend/translate/Memory.cpp (DS_READ, DS_READ2, DS_WRITE, DS_WRITE2).
// PentaX: checked compute allocation, typed subset, and uniform workgroup synchronization.
use super::{Translator, u32v};
use crate::ir::Op;
use crate::{CompileError, UnsupportedReason};
use pentax_shader::Opcode as O;

impl Translator<'_> {
    pub(super) fn shared_memory(&mut self) -> Result<(), CompileError> {
        let inst = self.inst;
        let memory = inst
            .memory
            .ok_or(CompileError::Internal("DS memory metadata"))?;
        if memory.gds() {
            return Err(self.unsupported(UnsupportedReason::GlobalDataShare));
        }
        let Some(cs) = self.profile.compute() else {
            return Err(self.unsupported(UnsupportedReason::LocalDataShare));
        };
        if cs.shared_memory_bytes == 0 {
            return Err(self.unsupported(UnsupportedReason::SharedMemoryAllocation));
        }
        let paired = matches!(
            inst.opcode,
            O::DsRead2B32 | O::DsRead2st64B32 | O::DsWrite2B32 | O::DsWrite2st64B32
        );
        let read = matches!(
            inst.opcode,
            O::DsReadB32 | O::DsRead2B32 | O::DsRead2st64B32
        );
        // Snapshot address, EXEC and both source values before any destination
        // writes: a read's destination may alias its address VGPR.
        let address = self.read_u32(inst.src(0))?;
        let exec = self.e.emit(Op::GetExec, &[])?;
        let first_address = self.e.emit(Op::IAdd32, &[address, u32v(memory.offset())])?;
        let second_address = if paired {
            self.e
                .emit(Op::IAdd32, &[address, u32v(memory.secondary_offset())])?
        } else {
            first_address
        };
        if read {
            let first = self.e.emit(Op::LoadSharedU32, &[first_address, exec])?;
            let second = if paired {
                self.e.emit(Op::LoadSharedU32, &[second_address, exec])?
            } else {
                first
            };
            // All workgroup invocations rendezvous outside the EXEC-conditional access.
            self.e.emit(Op::SharedCompletion, &[])?;
            self.write(inst.dst, first)?;
            if paired {
                let mut dst = inst.dst;
                dst.reg = dst
                    .reg
                    .checked_add(1)
                    .filter(|&reg| reg < 256)
                    .ok_or_else(|| self.unsupported(UnsupportedReason::LocalDataShare))?;
                self.write(dst, second)?;
            }
        } else {
            let first = self.read_u32(inst.src(1))?;
            let second = if paired {
                self.read_u32(inst.src(2))?
            } else {
                first
            };
            self.e
                .emit(Op::StoreSharedU32, &[first_address, first, exec])?;
            // Source DS_WRITE2 keeps its first value if both offsets coincide.
            if paired && memory.secondary_offset() != memory.offset() {
                self.e
                    .emit(Op::StoreSharedU32, &[second_address, second, exec])?;
            }
            self.e.emit(Op::SharedCompletion, &[])?;
        }
        Ok(())
    }
}
