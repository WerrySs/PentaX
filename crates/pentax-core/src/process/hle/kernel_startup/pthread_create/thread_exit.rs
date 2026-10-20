// SPDX-License-Identifier: GPL-2.0-only
//! Created-thread cleanup after actual native returns, on the same TLS/stack.
use super::*;
use pentax_kernel::{AdmittedThreadExitCallback, ThreadExitAction};

#[derive(Clone, Copy)]
pub(super) struct Exit {
    value: u64,
    callback: AdmittedThreadExitCallback,
    execution: Option<(u64, u64)>,
    anchor: (u64, u64),
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
impl PreparedNativeKernelStartup<'_, '_> {
    /// Consume only the current outer return. A false result means a cleanup
    /// callback is prepared on this actual executor; the child remains runnable.
    /// No integer callback address is ever invoked as a host function pointer.
    pub fn advance_created_thread_exit(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: &PreparedThreadStart<'_, '_>,
        engine: &mut pentax_platform::native_dispatch::NativeBlockStepper<'_, '_, '_>,
    ) -> Result<bool, KernelStartupError> {
        let returned = engine
            .observed_return()
            .ok_or(invalid("thread exit actual return"))?;
        engine
            .validate_call_return(&returned)
            .map_err(dispatch_error)?;
        if start.pending.owner != self.owner_id()
            || start.thread.owner_id() != self.owner_id()
            || current.owner_id() != self.owner_id()
            || current.materialized_source().source_plan().identity()
                != self.memory().active_source_identity()
            || returned.source_plan().identity() != self.memory().active_source_identity()
            || engine.owner_id() != self.owner_id()
        {
            return Err(invalid("thread exit current source"));
        }
        charge(
            &mut self.provider_work,
            2048,
            self.plan.limits.max_provider_work,
        )?;
        let record = self
            .thread_runtime
            .records
            .get(start.index)
            .ok_or(invalid("thread exit record"))?;
        if !record.published
            || record.returned.is_some()
            || record.tls_index != self.active_tls
            || record.tls_index != start.thread.index
            || record.layout.pthread_pointer != start.layout.pthread_pointer
            || self
                .tls()
                .instances()
                .get(self.active_tls)
                .is_none_or(|thread| thread.thread_pointer != start.thread.thread_pointer())
        {
            return Err(invalid("thread exit active context"));
        }
        let pending = record.exit;
        let value = if let Some(exit) = pending {
            exit.value
        } else {
            returned.value()
        };
        let action = if let Some(exit) = pending {
            if let Some((executor, call)) = exit.execution {
                let initial = returned.initial_state();
                let stack = returned.stack();
                if returned.owner_id() != self.owner_id()
                    || returned.executor_id() != executor
                    || returned.call_sequence() != call
                    || returned.entry() != exit.callback.entry()
                    || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != exit.callback.args()
                    || initial.gpr[4] != start.layout.return_slot
                    || initial.fs_base != Some(start.thread.thread_pointer())
                    || initial.gs_base.is_some()
                    || stack.stack != start.layout.stack
                    || stack.guards != start.layout.guards
                    || stack.return_slot != start.layout.return_slot
                    || stack.return_marker != start.layout.return_marker
                {
                    return Err(invalid("thread exit callback return provenance"));
                }
                let (mut paused, threads) =
                    self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
                let guest = KernelMemory {
                    paused: &mut paused,
                    source: Some(current.materialized_source().source_plan()),
                    supplied_modules: self.plan.kernel_provider_index(),
                    lookup_limits: None,
                    lookup_work: None,
                    loaded_modules: None,
                    thread_tls: threads.get(self.active_tls),
                    initial_entry: None,
                };
                self.kernel
                    .complete_thread_exit_callback(&exit.callback, &guest)
                    .map_err(KernelStartupError::Kernel)?
            } else {
                // Failed target/stack admission retained the issued ticket;
                // retry the same pending call without replaying a callback.
                if (returned.executor_id(), returned.call_sequence()) != exit.anchor {
                    return Err(invalid("thread exit pending return provenance"));
                }
                ThreadExitAction::Callback(exit.callback)
            }
        } else {
            self.validate_created_thread_return(start, &returned)?;
            let (mut paused, threads) =
                self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
            let guest = KernelMemory {
                paused: &mut paused,
                source: Some(current.materialized_source().source_plan()),
                supplied_modules: self.plan.kernel_provider_index(),
                lookup_limits: None,
                lookup_work: None,
                loaded_modules: None,
                thread_tls: threads.get(self.active_tls),
                initial_entry: None,
            };
            self.kernel
                .begin_thread_exit(&guest)
                .map_err(KernelStartupError::Kernel)?
        };
        let ThreadExitAction::Callback(callback) = action else {
            let record = &mut self.thread_runtime.records[start.index];
            record.exit = None;
            record.returned = Some(value);
            return Ok(true);
        };
        // Retain the pending operation even if a later policy/source/stack
        // admission fails. A stopped callback never implies a clean thread.
        self.thread_runtime.records[start.index].exit = Some(Exit {
            value,
            callback,
            execution: None,
            anchor: (returned.executor_id(), returned.call_sequence()),
        });
        self.check_thread_exit_target(current, callback.entry())?;
        let mut next = *engine.state();
        next.guest_pc = callback.entry();
        next.gpr[4] = start.layout.return_slot;
        let args = callback.args();
        next.gpr[7] = args[0];
        next.gpr[6] = args[1];
        next.gpr[2] = args[2];
        engine
            .begin_call(&mut self.pause()?, next)
            .map_err(dispatch_error)?;
        self.thread_runtime.records[start.index]
            .exit
            .as_mut()
            .unwrap()
            .execution = Some((engine.executor_id(), engine.call_sequence()));
        Ok(false)
    }

    fn check_thread_exit_target(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        entry: u64,
    ) -> Result<(), KernelStartupError> {
        let source = current.materialized_source().source_plan();
        let maximum = self.plan.limits.max_provider_work;
        let mut found = false;
        for module in source.modules() {
            charge(&mut self.provider_work, 1, maximum)?;
            for segment in &module.runtime_plan().segments {
                charge(&mut self.provider_work, 1, maximum)?;
                if !segment.flags.executable() || segment.flags.writable() {
                    continue;
                }
                let start = module
                    .load_bias()
                    .checked_add(segment.vaddr)
                    .ok_or(invalid("thread exit callback range"))?;
                if start <= entry && entry - start < segment.file_bytes.len() as u64 {
                    if found {
                        return Err(invalid("ambiguous thread exit callback target"));
                    }
                    found = true;
                }
            }
        }
        if !found {
            return Err(invalid("thread exit callback target"));
        }
        let paused = self.pause()?;
        paused.preflight_fetch_source(entry, 1).map_err(memory)?;
        if !matches!(
            paused.preflight_write(entry, 1),
            Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(invalid("writable thread exit callback target"));
        }
        Ok(())
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn dispatch_error(
    error: pentax_platform::native_dispatch::NativeDispatchError,
) -> KernelStartupError {
    use pentax_platform::native_dispatch::NativeDispatchError as E;
    match error {
        E::Budget(_) => KernelStartupError::Budget("thread exit native call"),
        E::Allocation => KernelStartupError::Allocation,
        E::Memory(error) => memory(error),
        _ => invalid("thread exit executor admission"),
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
