// SPDX-License-Identifier: GPL-2.0-only
//! Bounded terminal permission observations without addresses or payload bytes.
use pentax_codegen::{Code, MaterializedBlockPlan, MaterializedSource, OpKind};
use pentax_core::process::hle::KernelStartupPlan;
use pentax_platform::{
    native_dispatch::NativeDispatchError,
    native_memory::{NativeMemoryError, PausedNativeMemory},
};

#[derive(Debug, Default)]
pub(super) struct DispatchContext {
    captured: bool,
    form: Option<Code>,
    planned_memory: &'static str,
    body_ran: bool,
    permissions: Option<(&'static str, &'static str, &'static str)>,
}
fn access(read: bool, write: bool, execute: bool) -> &'static str {
    match (read, write, execute) {
        (false, false, false) => "none",
        (true, false, false) => "read",
        (false, true, false) => "write",
        (true, true, false) => "read-write",
        (false, false, true) => "execute",
        (true, false, true) => "read-execute",
        (false, true, true) => "write-execute",
        (true, true, true) => "read-write-execute",
    }
}
impl DispatchContext {
    pub(super) fn capture(
        &mut self,
        error: &NativeDispatchError,
        block: &MaterializedBlockPlan<'_, '_, '_>,
        memory: &PausedNativeMemory<'_>,
        plan: &KernelStartupPlan<'_>,
        source: &MaterializedSource<'_, '_>,
        body_ran: bool,
    ) {
        self.captured = true;
        self.body_ran = body_ran;
        if let Some(instruction) = block.exit().instruction() {
            self.form = Some(instruction.code());
            self.planned_memory = if instruction.op0_kind() == OpKind::Memory {
                "destination"
            } else if instruction.op_kinds().any(|kind| kind == OpKind::Memory) {
                "source"
            } else {
                "none"
            };
        }
        let NativeDispatchError::Memory(
            NativeMemoryError::PermissionDenied(address) | NativeMemoryError::Unmapped(address),
        ) = error
        else {
            return;
        };
        let cpu = access(
            memory.preflight_read(*address, 1).is_ok(),
            memory.preflight_write(*address, 1).is_ok(),
            memory.preflight_fetch_source(*address, 1).is_ok(),
        );
        let gpu = access(
            memory.preflight_gpu_read(*address, 1).is_ok(),
            memory.preflight_gpu_write(*address, 1).is_ok(),
            false,
        );
        let mut origin = "outside-source";
        // Fixed metadata limits bound observation even if a future source plan
        // permits more modules/segments. No names, bytes or offsets are emitted.
        'modules: for (index, module) in source.source_plan().modules().iter().take(128).enumerate()
        {
            for segment in module.runtime_plan().segments.iter().take(256) {
                let Some(start) = module.load_bias().checked_add(segment.vaddr) else {
                    continue;
                };
                if start <= *address
                    && address
                        .checked_sub(start)
                        .is_some_and(|offset| offset < segment.mem_size)
                {
                    origin = if index == 0 {
                        "primary-image"
                    } else if index == plan.kernel_provider_index() {
                        "kernel-provider"
                    } else if index == plan.hle_startup_plan().provider_index() {
                        "libc-provider"
                    } else {
                        "other-image"
                    };
                    break 'modules;
                }
            }
        }
        self.permissions = Some((cpu, gpu, origin));
    }
    pub(super) fn protocol_fields(&self) -> String {
        if !self.captured {
            return String::new();
        }
        let mut fields = format!("\tdispatch_body_ran={}", u8::from(self.body_ran));
        if let Some(form) = self.form {
            fields.push_str(&format!(
                "\tplanned_form={form:?}\tplanned_memory={}",
                self.planned_memory
            ));
        }
        if let Some((cpu, gpu, origin)) = self.permissions {
            fields.push_str(&format!(
                "\tdispatch_cpu_access={cpu}\tdispatch_gpu_access={gpu}\tdispatch_origin={origin}"
            ));
        }
        fields
    }
}
