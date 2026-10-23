// SPDX-License-Identifier: GPL-2.0-only
// Adapted from public KytyPS5 7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3,
// spirvEmitterMemory.cpp LoadWideShared/StoreWideShared and
// spirvEmitterMemoryHelpers.cpp EnsureLdsStorage/EmitMemoryElementPointer.
use super::*;

impl Emit<'_, '_> {
    pub(super) fn define_shared(&mut self) -> R<()> {
        let bytes = self
            .profile
            .compute()
            .map_or(0, |cs| cs.shared_memory_bytes);
        if bytes == 0 {
            return Ok(());
        }
        let u = self.ty_u32()?;
        let count = self.const_u32(bytes / 4)?;
        let array = self.b.ty(OpTypeArray, &[u, count])?;
        let pointer = self.ty_pointer(StorageClassWorkgroup, array)?;
        self.shared_var = self.b.global_variable(pointer, StorageClassWorkgroup)?;
        self.b.name(self.shared_var, "lds_dwords")?;
        // No initializer: LDS contents before a guest write are undefined.
        Ok(())
    }

    fn shared_index(&mut self, id: InstId) -> R<(u32, u32)> {
        let address = self.arg(id, 0)?;
        // The upstream dword path discards the low two address bits.
        let index = self.shr_const(address, 2)?;
        let bytes = self
            .profile
            .compute()
            .map_or(0, |cs| cs.shared_memory_bytes);
        let count = self.const_u32(bytes / 4)?;
        let valid = self.boolop(OpULessThan, &[index, count])?;
        Ok((index, valid))
    }

    fn shared_pointer(&mut self, index: u32) -> R<u32> {
        if self.shared_var == 0 {
            return Err(CompileError::Internal("LDS storage absent"));
        }
        let u = self.ty_u32()?;
        let pointer = self.ty_pointer(StorageClassWorkgroup, u)?;
        self.op(OpAccessChain, pointer, &[self.shared_var, index])
    }

    pub(super) fn emit_shared_load(&mut self, id: InstId) -> R<u32> {
        let exec = self.arg(id, 1)?;
        self.value_or_zero_if(exec, |e| {
            let (index, valid) = e.shared_index(id)?;
            e.value_or_zero_if(valid, |e| {
                let pointer = e.shared_pointer(index)?;
                e.u32op(OpLoad, &[pointer])
            })
        })
    }

    pub(super) fn emit_shared_store(&mut self, id: InstId) -> R<()> {
        let data = self.arg(id, 1)?;
        let exec = self.arg(id, 2)?;
        self.if_condition(exec, |e| {
            let (index, valid) = e.shared_index(id)?;
            e.if_condition(valid, |e| {
                let pointer = e.shared_pointer(index)?;
                e.b.func(OpStore, &[pointer, data])
            })
        })
    }

    pub(super) fn shared_completion(&mut self) -> R<()> {
        // A conservative rendezvous after every DS is outside the lane's EXEC
        // branch. Straight-line guest CFG guarantees all invocations reach it.
        // This permits logical Wave64 across two physical subgroups; it does
        // not implement ballots, shuffles, atomics or defined results for
        // guest programs with unsynchronized conflicting LDS accesses.
        let workgroup = self.const_u32(ScopeWorkgroup)?;
        let semantics =
            self.const_u32(MemorySemanticsAcquireReleaseMask | MemorySemanticsWorkgroupMemoryMask)?;
        self.b
            .func(OpControlBarrier, &[workgroup, workgroup, semantics])
    }
}
