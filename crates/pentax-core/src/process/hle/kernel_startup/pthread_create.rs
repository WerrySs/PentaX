// SPDX-License-Identifier: GPL-2.0-only

//! Prepared thread creation: independent TLS and admitted stack before publication.

use super::*;
use pentax_kernel::{AdmittedPthreadCreate, PthreadCreateAction, ThreadAttributes};
use pentax_linker::SourceIdentity;
mod join;
#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
mod thread_exit;
pub(super) use join::{JoinAction, JoinTicket};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeOwnerId, NativePermissions, NativeRange,
};

fn invalid(reason: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(reason)
}
fn memory(error: pentax_platform::native_memory::NativeMemoryError) -> KernelStartupError {
    KernelStartupError::Hle(HleError::Memory(crate::memory::MemoryError::Native(error)))
}
fn add(a: u64, b: u64) -> Result<u64, KernelStartupError> {
    a.checked_add(b)
        .filter(|end| *end < ADDRESS_LIMIT)
        .ok_or_else(|| invalid("thread storage range"))
}
fn pages(bytes: u64) -> Result<u64, KernelStartupError> {
    Ok(add(bytes, PAGE_SIZE - 1)? & !(PAGE_SIZE - 1))
}

#[derive(Clone, Copy, Debug)]
pub struct ThreadEntryLayout {
    pub entry: u64,
    pub argument: u64,
    pub stack: NativeRange,
    /// None for an exact caller-owned stack; no implicit guard is installed.
    pub guards: Option<[NativeRange; 2]>,
    pub return_slot: u64,
    pub return_marker: u64,
    pub pthread_pointer: u64,
}
struct Record {
    tls_index: usize,
    thread_pointer: u64,
    joined: bool,
    join_claim: Option<(u64, u64)>,
    waiting_for: Option<usize>,
    layout: ThreadEntryLayout,
    attributes: ThreadAttributes,
    published: bool,
    returned: Option<u64>,
    execution: Option<(u64, u64)>,
    exit: Option<thread_exit::Exit>,
}
pub(super) struct ThreadRuntime {
    pub(super) window: Option<NativeRange>,
    pub(super) next: u64,
    records: Vec<Record>,
    maximum: usize,
    next_join_serial: u64,
}
impl ThreadRuntime {
    pub(super) fn stack_ranges(&self) -> impl Iterator<Item = NativeRange> + '_ {
        self.records.iter().map(|record| record.layout.stack)
    }
    pub(super) fn new(maximum: usize) -> Result<Self, KernelStartupError> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(maximum)
            .map_err(|_| KernelStartupError::Allocation)?;
        Ok(Self {
            window: None,
            next: 0,
            records,
            maximum,
            next_join_serial: 1,
        })
    }
}

pub struct PendingThreadCreate {
    owner: NativeOwnerId,
    source: SourceIdentity,
    parent: u64,
    pc: u64,
    id: u64,
    args: [u64; 6],
    request: AdmittedPthreadCreate,
}
impl fmt::Debug for PendingThreadCreate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingThreadCreate")
            .finish_non_exhaustive()
    }
}
pub struct PreparedThreadStart<'plan, 'image> {
    pending: PendingThreadCreate,
    thread: PreparedNativeThread<'plan, 'image>,
    index: usize,
    layout: ThreadEntryLayout,
}
impl fmt::Debug for PreparedThreadStart<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedThreadStart")
            .finish_non_exhaustive()
    }
}
impl<'plan, 'image> PreparedThreadStart<'plan, 'image> {
    pub const fn thread(&self) -> &PreparedNativeThread<'plan, 'image> {
        &self.thread
    }
    pub const fn layout(&self) -> &ThreadEntryLayout {
        &self.layout
    }
    pub fn attributes(&self) -> ThreadAttributes {
        self.pending.request.attributes()
    }
}

pub(super) fn prepare_action(
    memory: &mut KernelMemory<'_, '_, '_>,
    action: PthreadCreateAction,
    pc: u64,
    id: u64,
    args: [u64; 6],
) -> Result<NativeServiceAction, KernelStartupError> {
    let request = match action {
        PthreadCreateAction::Returned(value) => return Ok(NativeServiceAction::Returned(value)),
        PthreadCreateAction::Create(request) => request,
    };
    let source = memory.source.ok_or(invalid("thread creation source"))?;
    let parent = memory
        .thread_tls
        .ok_or(invalid("thread creation parent"))?
        .thread_pointer;
    let maximum = memory
        .lookup_limits
        .ok_or(invalid("thread creation work"))?
        .max_work;
    let work = memory
        .lookup_work
        .as_deref_mut()
        .ok_or(invalid("thread creation work"))?;
    let mut found = false;
    for module in source.modules() {
        charge(work, 1, maximum)?;
        for segment in &module.runtime_plan().segments {
            charge(work, 1, maximum)?;
            if !segment.flags.executable() || segment.flags.writable() {
                continue;
            }
            let start = add(module.load_bias(), segment.vaddr)?;
            if start <= request.entry() && request.entry() - start < segment.file_bytes.len() as u64
            {
                if found {
                    return Err(invalid("ambiguous thread entry"));
                }
                found = true;
            }
        }
    }
    if !found {
        return Err(invalid("thread entry"));
    }
    memory
        .paused
        .preflight_fetch_source(request.entry(), 1)
        .map_err(self::memory)?;
    memory
        .paused
        .preflight_write(request.output(), 8)
        .map_err(self::memory)?;
    if !matches!(
        memory.paused.preflight_write(request.entry(), 1),
        Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
    ) {
        return Err(invalid("writable thread entry"));
    }
    Ok(NativeServiceAction::CreateThread(PendingThreadCreate {
        owner: memory.paused.owner_id(),
        source: source.identity(),
        parent,
        pc,
        id,
        args,
        request,
    }))
}

impl<'plan, 'image> PreparedNativeKernelStartup<'plan, 'image> {
    pub(crate) fn configure_thread_storage(
        &mut self,
        window: NativeRange,
    ) -> Result<(), KernelStartupError> {
        if self.thread_runtime.window.is_some()
            || !window.address.is_multiple_of(PAGE_SIZE)
            || !window.size.is_multiple_of(PAGE_SIZE)
        {
            return Err(invalid("thread storage configuration"));
        }
        if window.size == 0 {
            return Ok(());
        }
        let end = add(window.address, window.size)?;
        let kernel = self.plan.limits.kernel;
        if window.address < add(kernel.virtual_base, kernel.virtual_bytes)?
            && kernel.virtual_base < end
        {
            return Err(invalid("thread storage overlaps kernel allocator"));
        }
        self.pause()?
            .preflight_reserved_unmapped(window.address, window.size)
            .map_err(memory)?;
        self.thread_runtime.next = window.address;
        self.thread_runtime.window = Some(window);
        Ok(())
    }

    fn check_parent(
        &self,
        current: &PreparedNativeSource<'_, '_>,
        pending: &PendingThreadCreate,
    ) -> Result<(), KernelStartupError> {
        if pending.owner != self.owner_id()
            || current.owner_id() != self.owner_id()
            || pending.source != self.memory().active_source_identity()
            || pending.source != current.materialized_source().source_plan().identity()
            || self
                .tls()
                .instances()
                .get(self.active_tls)
                .is_none_or(|thread| thread.thread_pointer != pending.parent)
        {
            return Err(invalid("thread creation provenance"));
        }
        Ok(())
    }

    /// Prepare fresh TLS/prefix and admit stack backing without selecting the child or
    /// publishing the caller's output. The executor must still admit its CPU
    /// state and reserve a run-queue slot before committing creation.
    pub fn prepare_created_thread(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        pending: PendingThreadCreate,
    ) -> Result<PreparedThreadStart<'plan, 'image>, KernelStartupError> {
        self.check_parent(current, &pending)?;
        let attributes = pending.request.attributes();
        let borrowed = attributes.stack_address != 0;
        if self.thread_runtime.records.len() + 1 >= self.thread_runtime.maximum {
            return Err(KernelStartupError::Budget("thread contexts"));
        }
        let window = self
            .thread_runtime
            .window
            .ok_or(invalid("thread storage unavailable"))?;
        let stack_size = if borrowed {
            attributes.stack_size
        } else {
            pages(add(attributes.stack_size, 1024 * 1024)?)?
        };
        let guard_size = if borrowed {
            0
        } else {
            pages(attributes.guard_size)?.max(PAGE_SIZE)
        };
        if !(0x4000..=8 * 1024 * 1024).contains(&stack_size) || guard_size > 1024 * 1024 {
            return Err(KernelStartupError::Budget("thread stack"));
        }
        let extent = pages(
            self.tls()
                .templates()
                .iter()
                .map(|template| template.tp_offset)
                .max()
                .unwrap_or(0)
                .max(self.plan.limits.startup.static_tls.reserved_static_bytes),
        )?;
        let start = self.thread_runtime.next;
        let tp = add(add(start, PAGE_SIZE)?, extent)?;
        let tls_end = add(tp, 2 * PAGE_SIZE)?;
        let stack_address = if borrowed {
            attributes.stack_address
        } else {
            add(tls_end, guard_size)?
        };
        let stack_end = add(stack_address, stack_size)?;
        let next = add(if borrowed { tls_end } else { stack_end }, PAGE_SIZE)?;
        if next > add(window.address, window.size)? {
            return Err(KernelStartupError::Budget("thread storage window"));
        }
        let main = self
            .initial_entry
            .ok_or(invalid("thread creation before process entry"))?;
        let layout = ThreadEntryLayout {
            entry: pending.request.entry(),
            argument: pending.request.argument(),
            stack: NativeRange {
                address: stack_address,
                size: stack_size,
            },
            guards: (!borrowed).then_some([
                NativeRange {
                    address: tls_end,
                    size: guard_size,
                },
                NativeRange {
                    address: stack_end,
                    size: PAGE_SIZE,
                },
            ]),
            return_slot: (stack_end & !15) - 8,
            return_marker: main.return_marker,
            pthread_pointer: tp + 0x1000,
        };
        charge(
            &mut self.provider_work,
            stack_size + extent + PAGE_SIZE,
            self.plan.limits.max_provider_work,
        )?;
        self.pause()?
            .preflight_reserved_unmapped(start, next - start)
            .map_err(memory)?;
        if borrowed {
            let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
            self.fiber_runtime.preflight_thread_stack(
                &paused,
                layout.stack,
                &self.kernel,
                &mut self.provider_work,
                self.plan.limits.max_provider_work,
            )?;
        }
        if borrowed {
            // The game retains every byte and the original mapping. Guard-size
            // attributes are ignored for user stacks in the pinned source.
            let paused = self.pause()?;
            paused
                .preflight_read(stack_address, stack_size)
                .map_err(memory)?;
            paused
                .preflight_write(stack_address, stack_size)
                .map_err(memory)?;
            paused
                .preflight_non_executable(stack_address, stack_size)
                .map_err(memory)?;
        } else {
            self.pause()?
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::MapZeroed {
                            address: stack_address,
                            size: stack_size,
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .map_err(memory)?;
        }
        let owned_stack = (!borrowed).then_some(layout.stack);
        let tls_index = match self.hle.tls_mut().append_thread_instance(tp) {
            Ok(index) => index,
            Err(error) => {
                self.rollback_thread_stack(owned_stack)?;
                return Err(KernelStartupError::Hle(HleError::Tls(error)));
            }
        };
        let thread = match self.prepare_thread_context(tls_index) {
            Ok(thread) => thread,
            Err(error) => {
                self.rollback_thread_storage(tp, owned_stack)?;
                return Err(error);
            }
        };
        let registered = {
            let (mut paused, instances) =
                self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
            let mut guest = KernelMemory {
                paused: &mut paused,
                source: Some(current.materialized_source().source_plan()),
                supplied_modules: self.plan.kernel_provider_index(),
                lookup_limits: None,
                lookup_work: None,
                loaded_modules: None,
                thread_tls: instances.get(tls_index),
                initial_entry: None,
            };
            self.kernel.register_thread_context_with_attributes(
                tp,
                layout.pthread_pointer,
                attributes,
                &mut guest,
            )
        };
        if let Err(error) = registered {
            self.rollback_thread_storage(tp, owned_stack)?;
            return Err(KernelStartupError::Kernel(error));
        }
        let index = self.thread_runtime.records.len();
        self.thread_runtime.records.push(Record {
            tls_index,
            thread_pointer: tp,
            joined: false,
            join_claim: None,
            waiting_for: None,
            layout,
            attributes,
            published: false,
            returned: None,
            execution: None,
            exit: None,
        });
        self.thread_runtime.next = next;
        Ok(PreparedThreadStart {
            pending,
            thread,
            index,
            layout,
        })
    }

    fn rollback_thread_stack(
        &mut self,
        stack: Option<NativeRange>,
    ) -> Result<(), KernelStartupError> {
        let Some(stack) = stack else {
            return Ok(());
        };
        let result = self.pause()?.apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::Unmap {
                    address: stack.address,
                    size: stack.size,
                },
                writes: &[],
            },
            &Default::default(),
        );
        if let Err(error) = result {
            self.hle.tls_mut().memory.space.abandon();
            return Err(memory(error));
        }
        Ok(())
    }
    fn rollback_thread_storage(
        &mut self,
        tp: u64,
        stack: Option<NativeRange>,
    ) -> Result<(), KernelStartupError> {
        self.hle
            .tls_mut()
            .rollback_last_thread_instance(tp)
            .map_err(|error| KernelStartupError::Hle(HleError::Tls(error)))?;
        self.rollback_thread_stack(stack)
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_created_thread_return(
        &mut self,
        start: &PreparedThreadStart<'_, '_>,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
    ) -> Result<(), KernelStartupError> {
        self.validate_created_thread_return(start, returned)?;
        if self.kernel.active_thread_requires_cleanup() {
            return Err(KernelStartupError::Kernel(Failure::Unsupported(
                "guest thread exit callbacks",
            )));
        }
        self.thread_runtime.records[start.index].returned = Some(returned.value());
        Ok(())
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    fn validate_created_thread_return(
        &self,
        start: &PreparedThreadStart<'_, '_>,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
    ) -> Result<(), KernelStartupError> {
        let record = self
            .thread_runtime
            .records
            .get(start.index)
            .ok_or(invalid("thread return record"))?;
        if start.pending.owner != self.owner_id()
            || start.thread.owner_id() != self.owner_id()
            || !record.published
            || record.execution != Some((returned.executor_id(), returned.call_sequence()))
            || record.returned.is_some()
            || record.tls_index != self.active_tls
            || returned.owner_id() != self.owner_id()
            || returned.source_plan().identity() != self.memory().active_source_identity()
            || returned.entry() != start.layout.entry
            || returned.initial_state().gpr[7] != start.layout.argument
            || returned.initial_state().fs_base != Some(start.thread.thread_pointer())
            || returned.stack().stack != start.layout.stack
            || returned.stack().guards != start.layout.guards
            || returned.stack().return_slot != start.layout.return_slot
            || returned.stack().return_marker != start.layout.return_marker
        {
            return Err(invalid("thread entry return provenance"));
        }
        Ok(())
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn publish_created_thread(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: &PreparedThreadStart<'_, '_>,
        cpu: &pentax_platform::native_dispatch::NativeThreadContext,
        parent: pentax_platform::native_dispatch::PreSyscallToken,
    ) -> Result<u64, KernelStartupError> {
        self.check_parent(current, &start.pending)?;
        let record = self
            .thread_runtime
            .records
            .get(start.index)
            .ok_or(invalid("thread creation record"))?;
        if record.published
            || record.tls_index != start.thread.index
            || record.layout.pthread_pointer != start.layout.pthread_pointer
            || record.attributes != start.attributes()
            || cpu.owner_id() != self.owner_id()
            || cpu.executor_id() != parent.executor_id()
            || cpu.state().guest_pc != start.layout.entry
            || cpu.state().gpr[4] != start.layout.return_slot
            || cpu.state().gpr[7] != start.layout.argument
            || cpu.state().fs_base != Some(start.thread.thread_pointer())
            || cpu.state().gs_base.is_some()
            || parent.owner_id() != self.owner_id()
            || parent.syscall_pc() != start.pending.pc
            || parent.service_id() != start.pending.id
            || parent.args() != start.pending.args
        {
            return Err(invalid("thread creation executable context"));
        }
        let (mut paused, instances) = self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
        let mut guest = KernelMemory {
            paused: &mut paused,
            source: Some(current.materialized_source().source_plan()),
            supplied_modules: self.plan.kernel_provider_index(),
            lookup_limits: None,
            lookup_work: None,
            loaded_modules: None,
            thread_tls: instances.get(self.active_tls),
            initial_entry: self.initial_entry,
        };
        self.kernel
            .publish_pthread_create(
                &start.pending.request,
                start.thread.thread_pointer(),
                start.layout.pthread_pointer,
                pentax_kernel::GuestWrite {
                    address: start.layout.return_slot,
                    bytes: &start.layout.return_marker.to_le_bytes(),
                },
                &mut guest,
            )
            .map_err(KernelStartupError::Kernel)?;
        self.thread_runtime.records[start.index].published = true;
        self.thread_runtime.records[start.index].execution =
            Some((cpu.executor_id(), cpu.call_sequence()));
        Ok(0)
    }
}
