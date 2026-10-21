// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit file admission and actual nested module start.
// Adapted from pinned KernelLoadStartModule; see docs/upstream-provenance.md.

use super::super::unavailable_runtime::published::PublishedUnavailable;
use super::*;
use crate::memory::native::NativeLoadLimits;
use crate::process::hle::PreparedNativeSource;
use crate::process::hle::{GeneratedUnavailableProvider, UnavailableProviderLimits};
use pentax_codegen::{BlockLimits, DeclaredAppliedWrite, declare_materialized_source};
use pentax_kernel::{AdmittedModuleLoad, CallArgs, SysmoduleLoadCompletion};
use pentax_linker::{SourceIdentity, SymbolicExtensionLimits, SymbolicLinkPlan, plan_tls_bindings};
use pentax_loader::{RuntimeLoadPlan, open_executable};
use pentax_platform::native_memory::{NativeOwnerId, NativeRange};
use std::fmt;

mod fallbacks;
mod unavailable;

const MAX_FILES: usize = 125;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WORK: u64 = 64 * 1024 * 1024 * 1024;
const GRANULE: u64 = 65_536;

/// An admitted service result or an owned request that must suspend its caller.
#[derive(Debug)]
pub enum NativeServiceAction {
    Returned(u64),
    Load(PendingModuleLoad),
    Once(super::super::PreparedPthreadOnce),
    NetCtl(super::super::PreparedNetCtlCallback),
    FontParser(super::super::PreparedFontParserCallback),
    AgcCommandGrow(super::super::PreparedAgcCommandBufferCallback),
    ImeCallback(super::super::PreparedImeCallback),
    CreateThread(super::super::PendingThreadCreate),
    WaitThread(super::super::PendingThreadWait),
}
/// Private owner/source/file admission consumed by its bounded file read.
///
/// ```compile_fail
/// use pentax_core::process::hle::PendingModuleLoad;
/// fn retarget(ticket: PendingModuleLoad) {
///     let PendingModuleLoad { serial, .. } = ticket;
/// }
/// ```
pub struct PendingModuleLoad {
    owner: NativeOwnerId,
    serial: u64,
    file: usize,
    source: SourceIdentity,
    pc: u64,
    id: u64,
    request: AdmittedModuleLoad,
}
/// Immutable bytes from the exact admitted capability. They must remain alive
/// across every source view using them; no mutable byte slice is exposed.
pub struct DynamicModuleInput {
    pending: PendingModuleLoad,
    bytes: Vec<u8>,
}
impl DynamicModuleInput {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub const fn slot(&self) -> usize {
        self.pending.file
    }
}
/// Published mapping awaiting an actual nested initializer call and return.
/// Possessing or copying this value does not prove completion.
///
/// ```compile_fail
/// use pentax_core::process::hle::DynamicModuleStart;
/// fn retarget(start: DynamicModuleStart) -> DynamicModuleStart {
///     DynamicModuleStart { entry: 0x100, ..start }
/// }
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct DynamicModuleStart {
    owner: NativeOwnerId,
    serial: u64,
    file: usize,
    source: SourceIdentity,
    module: usize,
    handle: i32,
    entry: u64,
    args: CallArgs,
    initializer_args: [u64; 3],
    sysmodule: Option<SysmoduleLoadCompletion>,
    pc: u64,
    id: u64,
    result: Option<u64>,
}
impl DynamicModuleStart {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn args(&self) -> [u64; 3] {
        self.initializer_args
    }
}
impl fmt::Debug for PendingModuleLoad {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingModuleLoad").finish_non_exhaustive()
    }
}
impl fmt::Debug for DynamicModuleStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DynamicModuleStart")
            .field("module", &self.module)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy)]
struct CallStamp {
    executor: u64,
    call: u64,
    stack: NativeRange,
    guards: Option<[NativeRange; 2]>,
    slot: u64,
    marker: u64,
}
#[derive(Clone, Copy)]
enum FileState {
    Registered,
    Reading(u64),
    Mapped {
        start: DynamicModuleStart,
        count: u32,
        call: Option<CallStamp>,
    },
    Ready {
        start: DynamicModuleStart,
        count: u32,
    },
    Failed,
}
struct ModuleFile {
    path: String,
    file: File,
    size: u64,
    state: FileState,
}
#[derive(Default)]
pub(in crate::process::hle::kernel_startup) struct DynamicFiles {
    files: Vec<ModuleFile>,
    window: Option<NativeRange>,
    native: Option<NativeLoadLimits>,
    cursor: u64,
    serial: u64,
    bytes: u64,
    registered_bytes: u64,
    work: u64,
    unavailable_limits: Option<UnavailableProviderLimits>,
    unavailable_functions: usize,
    unavailable_bytes: u64,
    unavailable_work: u64,
    pub(in crate::process::hle::kernel_startup) published: Vec<PublishedUnavailable>,
}
impl DynamicFiles {
    pub(super) fn load_count(&self, module: usize) -> Option<u32> {
        self.files.iter().find_map(|file| match file.state {
            FileState::Mapped { start, count, .. } | FileState::Ready { start, count }
                if start.module == module =>
            {
                Some(count)
            }
            _ => None,
        })
    }
    pub(super) fn contains(&self, path: &str) -> bool {
        self.files.iter().any(|file| file.path == path)
    }
    fn charge(&mut self, work: u64) -> Result<(), KernelStartupError> {
        self.work = self
            .work
            .checked_add(work)
            .filter(|value| *value <= MAX_WORK)
            .ok_or(KernelStartupError::Budget("dynamic module work"))?;
        Ok(())
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
fn invalid(code: &'static str) -> KernelStartupError {
    KernelStartupError::InvalidProvider(code)
}
fn failure(code: &'static str) -> Failure {
    Failure::Unsupported(code)
}
fn aligned(value: u64, unit: u64) -> Result<u64, KernelStartupError> {
    value
        .checked_add(unit - 1)
        .map(|value| value & !(unit - 1))
        .ok_or(KernelStartupError::Budget("module address"))
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Register an explicit capability, never a host path resolved from guest
    /// text. Its bytes remain unread until an admitted request selects it.
    pub fn register_dynamic_module_file(
        &mut self,
        path: &str,
        file: File,
    ) -> Result<(), KernelStartupError> {
        if self.native_service_requests != 0
            || self.loaded_modules.dynamic.window.is_none()
            || !valid_path(path)
            || path.len() >= self.plan.limits.kernel.max_name_bytes
            || self
                .loaded_modules
                .entries
                .iter()
                .any(|entry| entry.path == path)
            || self.loaded_modules.dynamic.contains(path)
        {
            return Err(invalid("dynamic file registration"));
        }
        let dynamic = &mut self.loaded_modules.dynamic;
        if dynamic.files.len() + self.plan.kernel_provider_index() >= MAX_FILES {
            return Err(KernelStartupError::Budget("module files"));
        }
        let metadata = file
            .metadata()
            .map_err(|_| invalid("module file metadata"))?;
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Err(invalid("module file size"));
        }
        let size = metadata.len();
        let total = dynamic
            .registered_bytes
            .checked_add(size)
            .filter(|value| *value <= MAX_INPUT_BYTES)
            .ok_or(KernelStartupError::Budget("module input bytes"))?;
        dynamic.charge(path.len() as u64 * (dynamic.files.len() as u64 + 1) + 32)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(path.len())
            .map_err(|_| KernelStartupError::Allocation)?;
        owned.push_str(path);
        dynamic
            .files
            .try_reserve_exact(1)
            .map_err(|_| KernelStartupError::Allocation)?;
        dynamic.files.push(ModuleFile {
            path: owned,
            file,
            size,
            state: FileState::Registered,
        });
        dynamic.registered_bytes = total;
        Ok(())
    }

    /// Metadata policy for the append planner; it grants no publication rights.
    pub fn dynamic_extension_limits(&self) -> SymbolicExtensionLimits {
        SymbolicExtensionLimits {
            symbolic: self.plan.limits.startup.hle.linking,
            ..Default::default()
        }
    }
    pub fn dynamic_file_count(&self) -> usize {
        self.loaded_modules.dynamic.files.len()
    }

    pub(crate) fn configure_dynamic_window(
        &mut self,
        window: NativeRange,
        native: NativeLoadLimits,
        unavailable: UnavailableProviderLimits,
    ) -> Result<(), KernelStartupError> {
        if self.native_service_requests != 0
            || self.loaded_modules.dynamic.window.is_some()
            || window.size == 0
            || !window.address.is_multiple_of(GRANULE)
            || !window.size.is_multiple_of(GRANULE)
            || self
                .plan
                .limits
                .kernel
                .virtual_base
                .checked_add(self.plan.limits.kernel.virtual_bytes)
                .zip(window.address.checked_add(window.size))
                .is_none_or(|(kernel_end, window_end)| {
                    window.address < kernel_end && self.plan.limits.kernel.virtual_base < window_end
                })
        {
            return Err(invalid("dynamic window"));
        }
        self.pause()?
            .preflight_reserved_unmapped(window.address, window.size)
            .map_err(|_| invalid("dynamic window backing"))?;
        let original = self.plan.link_plan().modules()[..self.plan.kernel_provider_index()]
            .iter()
            .try_fold(0_u64, |sum, module| {
                sum.checked_add(module.source_bytes().len() as u64)
            })
            .ok_or(KernelStartupError::Budget("module input bytes"))?;
        let dynamic = &mut self.loaded_modules.dynamic;
        dynamic.window = Some(window);
        dynamic.cursor = window.address;
        dynamic.native = Some(native);
        dynamic.registered_bytes = original;
        dynamic.unavailable_limits = Some(unavailable);
        if let Some(binding) = self.plan.unavailable {
            dynamic.unavailable_functions = binding.factory.function_count();
            dynamic.unavailable_bytes = binding.factory.image().len() as u64;
            dynamic.unavailable_work = binding.factory.work_steps();
        }
        self.dynamic_unavailable_limits()?;
        Ok(())
    }

    pub fn read_dynamic_module(
        &mut self,
        pending: PendingModuleLoad,
    ) -> Result<DynamicModuleInput, KernelStartupError> {
        self.validate_pending(&pending)?;
        let dynamic = &mut self.loaded_modules.dynamic;
        let size = dynamic.files[pending.file].size;
        let result = (|| {
            dynamic.bytes = dynamic
                .bytes
                .checked_add(size)
                .filter(|value| *value <= MAX_INPUT_BYTES)
                .ok_or(KernelStartupError::Budget("module input bytes"))?;
            dynamic.charge(
                size.checked_mul(2)
                    .and_then(|value| value.checked_add(32))
                    .ok_or(KernelStartupError::Budget("module read work"))?,
            )?;
            let file = &mut dynamic.files[pending.file];
            if file
                .file
                .metadata()
                .map_err(|_| invalid("module file read"))?
                .len()
                != size
            {
                return Err(invalid("module file changed"));
            }
            file.file
                .seek(SeekFrom::Start(0))
                .map_err(|_| invalid("module file read"))?;
            let size = usize::try_from(size)
                .map_err(|_| KernelStartupError::Budget("module file size"))?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size)
                .map_err(|_| KernelStartupError::Allocation)?;
            if bytes.capacity() as u64 > MAX_FILE_BYTES {
                return Err(KernelStartupError::Budget("module file allocation"));
            }
            bytes.resize(size, 0);
            file.file
                .read_exact(&mut bytes)
                .map_err(|_| invalid("module file read"))?;
            if file
                .file
                .read(&mut [0])
                .map_err(|_| invalid("module file read"))?
                != 0
            {
                return Err(invalid("module file changed"));
            }
            Ok(bytes)
        })();
        match result {
            Ok(bytes) => Ok(DynamicModuleInput { pending, bytes }),
            Err(error) => {
                dynamic.files[pending.file].state = FileState::Failed;
                Err(error)
            }
        }
    }

    fn validate_pending(&self, pending: &PendingModuleLoad) -> Result<(), KernelStartupError> {
        if pending.owner != self.owner_id()
            || pending.source != self.memory().active_source_identity()
            || !matches!(self.loaded_modules.dynamic.files.get(pending.file).map(|file| file.state), Some(FileState::Reading(serial)) if serial == pending.serial)
        {
            return Err(invalid("stale module admission"));
        }
        Ok(())
    }

    pub fn dynamic_load_bias(&self, input: &DynamicModuleInput) -> Result<u64, KernelStartupError> {
        self.validate_pending(&input.pending)?;
        let runtime = open_executable(input.bytes())
            .and_then(|view| view.runtime_plan())
            .map_err(|_| invalid("module input"))?;
        let (low, _, alignment) = shape(&runtime)?;
        aligned(
            self.loaded_modules
                .dynamic
                .cursor
                .checked_sub(low)
                .ok_or(invalid("module placement"))?,
            alignment,
        )
    }

    /// Publish only the exact selected input against the actual current source.
    /// This is mapped/initializing state; actual return evidence is still needed.
    pub fn publish_dynamic_module(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        input: &DynamicModuleInput,
        source: &SymbolicLinkPlan<'_>,
        block: &BlockLimits,
    ) -> Result<DynamicModuleStart, KernelStartupError> {
        self.publish_dynamic_batch(current, input, source, None, block)
    }

    /// Publish one selected guest and, if required, its exact classified stop
    /// provider in the same memory transaction. Only the guest receives a handle.
    pub fn publish_dynamic_batch(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        input: &DynamicModuleInput,
        source: &SymbolicLinkPlan<'_>,
        delta: Option<&GeneratedUnavailableProvider>,
        block: &BlockLimits,
    ) -> Result<DynamicModuleStart, KernelStartupError> {
        let result = self.publish_inner(current, input, source, delta, block);
        if result.is_err()
            && input.pending.owner == self.owner_id()
            && let Some(file) = self
                .loaded_modules
                .dynamic
                .files
                .get_mut(input.pending.file)
            && matches!(file.state, FileState::Reading(serial) if serial == input.pending.serial)
        {
            file.state = FileState::Failed;
        }
        result
    }

    fn publish_inner(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        input: &DynamicModuleInput,
        source: &SymbolicLinkPlan<'_>,
        delta: Option<&GeneratedUnavailableProvider>,
        block: &BlockLimits,
    ) -> Result<DynamicModuleStart, KernelStartupError> {
        self.validate_pending(&input.pending)?;
        self.validate_dynamic_fallbacks(current, source)?;
        if current.owner_id() != self.owner_id()
            || current.materialized_source().source_plan().identity()
                != self.memory().active_source_identity()
            || source.parent_identity() != Some(self.memory().active_source_identity())
            || source.modules().len()
                != self.memory().module_count() + 1 + usize::from(delta.is_some())
            || source
                .appended_guest()
                .is_none_or(|guest| guest.index() != self.memory().module_count())
        {
            return Err(invalid("module publication source"));
        }
        let module = source
            .modules()
            .get(self.memory().module_count())
            .ok_or(invalid("module publication source"))?;
        if !std::ptr::eq(module.source_bytes(), input.bytes())
            || module.load_bias() != self.dynamic_load_bias(input)?
        {
            return Err(invalid("module publication profile"));
        }
        let (_, high, _) = shape(module.runtime_plan())?;
        let guest_end = aligned(
            module
                .load_bias()
                .checked_add(high)
                .ok_or(invalid("module placement"))?,
            GRANULE,
        )?;
        let published = self.authenticate_dynamic_delta(current, input, source, delta)?;
        let end = match delta {
            Some(factory) => aligned(
                guest_end
                    .checked_add(factory.mapped_bytes())
                    .ok_or(invalid("module placement"))?,
                GRANULE,
            )?,
            None => guest_end,
        };
        let window = self
            .loaded_modules
            .dynamic
            .window
            .ok_or(invalid("dynamic window"))?;
        if end
            > window
                .address
                .checked_add(window.size)
                .ok_or(invalid("dynamic window"))?
        {
            return Err(KernelStartupError::Budget("dynamic window"));
        }
        let entry = initializer(module)?;
        let admitted = self.plan.hle.limits();
        let bindings = plan_tls_bindings(source, &admitted.tls_bindings).map_err(|value| {
            KernelStartupError::Hle(crate::process::hle::HleError::TlsBinding(value))
        })?;
        let layout =
            crate::process::tls::plan_relocated_static_tls(&bindings, &admitted.static_tls)
                .map_err(|value| {
                    KernelStartupError::Hle(crate::process::hle::HleError::Tls(value))
                })?;
        if !layout.templates().starts_with(self.tls().templates()) {
            return Err(invalid("retained TLS layout"));
        }
        let mut writes = Vec::new();
        writes
            .try_reserve_exact(layout.tls_patches().len())
            .map_err(|_| KernelStartupError::Allocation)?;
        for patch in layout.tls_patches() {
            let binding = &bindings.bindings()[patch.binding_index()];
            writes.push(DeclaredAppliedWrite {
                deferred_index: binding.deferred_index(),
                consumer: patch.consumer(),
                source: patch.source(),
                address: patch.address(),
                value: patch.value(),
            });
        }
        if !current
            .materialized_source()
            .declared_writes()
            .iter()
            .eq(writes
                .iter()
                .filter(|write| write.consumer.index() < self.memory().module_count()))
        {
            return Err(invalid("retained TLS writes"));
        }
        let preview = declare_materialized_source(source, &writes, block)
            .map_err(KernelStartupError::Codegen)?;
        let tls_bytes = layout.templates()[self.tls().templates().len()..]
            .iter()
            .try_fold(0_u64, |sum, template| sum.checked_add(template.mem_size))
            .and_then(|bytes| bytes.checked_mul(self.tls().instances().len() as u64))
            .and_then(|bytes| bytes.checked_mul(6))
            .and_then(|bytes| {
                bytes.checked_add(
                    (layout.templates().len() as u64)
                        .checked_mul(self.tls().instances().len() as u64 + 1)?
                        .checked_mul(64)?,
                )
            })
            .ok_or(KernelStartupError::Budget("module TLS publication work"))?;
        let fallback_work = (source.fallback_transitions().len() as u64)
            .checked_mul(self.tls().templates().len() as u64 + 1)
            .and_then(|work| work.checked_mul(128))
            .and_then(|work| {
                work.checked_add(
                    (source.fallback_transitions().len() as u64)
                        .checked_mul(8)?
                        .checked_mul(std::mem::size_of::<(usize, usize, u8)>() as u64)?,
                )
            })
            .ok_or(KernelStartupError::Budget(
                "module fallback publication work",
            ))?;
        self.loaded_modules.dynamic.charge(
            preview
                .declaration_usage()
                .work_units
                .checked_add(input.bytes.len() as u64)
                .and_then(|work| work.checked_add(tls_bytes))
                .and_then(|work| work.checked_add(fallback_work))
                .ok_or(KernelStartupError::Budget("module publication work"))?,
        )?;
        self.loaded_modules
            .handles
            .try_reserve_exact(1)
            .map_err(|_| KernelStartupError::Allocation)?;
        let handle = i32::try_from(self.loaded_modules.handles.len() + 1)
            .map_err(|_| KernelStartupError::Budget("module handles"))?;
        let native = self
            .loaded_modules
            .dynamic
            .native
            .clone()
            .ok_or(invalid("dynamic window"))?;
        if published.is_some() {
            self.loaded_modules
                .dynamic
                .published
                .try_reserve_exact(1)
                .map_err(|_| KernelStartupError::Allocation)?;
        }
        let maximum = self.plan.limits.max_provider_work;
        let work = &mut self.provider_work;
        self.hle
            .tls_mut()
            .append_batch(&layout, window, &native, |space| {
                if let Some(origin) = &published {
                    let memory = space
                        .pause()
                        .map_err(|_| crate::process::ProcessError::InvalidSegment)?;
                    origin
                        .attest_all(&memory, work, maximum)
                        .map_err(|_| crate::process::ProcessError::InvalidSegment)?;
                }
                Ok(())
            })
            .map_err(|error| KernelStartupError::Hle(crate::process::hle::HleError::Tls(error)))?;
        // Every allocation, source check and native write has completed. The
        // remaining registry/counter/handle publication is infallible.
        if let (Some(origin), Some(factory)) = (published, delta) {
            let dynamic = &mut self.loaded_modules.dynamic;
            dynamic.unavailable_functions += factory.function_count();
            dynamic.unavailable_bytes += factory.image().len() as u64;
            dynamic.unavailable_work += factory.work_steps();
            dynamic.published.push(origin);
        }
        let pending = &input.pending;
        let start = DynamicModuleStart {
            owner: pending.owner,
            serial: pending.serial,
            file: pending.file,
            source: source.identity(),
            module: module.id().index(),
            handle,
            entry,
            args: pending.request.raw_arguments(),
            initializer_args: [
                pending.request.module_args(),
                pending.request.argument_pointer(),
                0,
            ],
            sysmodule: pending.request.sysmodule_completion(),
            result: pending.request.result_pointer(),
            pc: pending.pc,
            id: pending.id,
        };
        self.loaded_modules.handles.push(start.module);
        self.loaded_modules.dynamic.files[start.file].state = FileState::Mapped {
            start,
            count: 1,
            call: None,
        };
        self.loaded_modules.dynamic.cursor = end;
        Ok(start)
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_dynamic_module_call(
        &mut self,
        start: DynamicModuleStart,
        call: &pentax_platform::native_dispatch::NativeNestedCall,
    ) -> Result<(), KernelStartupError> {
        let parent = call.parent_syscall();
        if start.owner != self.pause()?.owner_id()
            || call.owner_id() != start.owner
            || call.source_identity() != start.source
            || start.source != self.memory().active_source_identity()
            || call.entry() != start.entry
            || call.args() != start.args()
            || parent.syscall_pc() != start.pc
            || parent.service_id() != start.id
            || parent.args() != start.args.0
        {
            return Err(invalid("module initializer call"));
        }
        let file = self
            .loaded_modules
            .dynamic
            .files
            .get_mut(start.file)
            .ok_or(invalid("module initializer state"))?;
        let FileState::Mapped {
            start: expected,
            count,
            call: None,
        } = file.state
        else {
            return Err(invalid("module initializer replay"));
        };
        if expected != start {
            return Err(invalid("module initializer identity"));
        }
        let stack = call.stack();
        file.state = FileState::Mapped {
            start,
            count,
            call: Some(CallStamp {
                executor: call.executor_id(),
                call: call.call_sequence(),
                stack: stack.stack,
                guards: stack.guards,
                slot: stack.return_slot,
                marker: stack.return_marker,
            }),
        };
        Ok(())
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn complete_dynamic_module(
        &mut self,
        current: &PreparedNativeSource<'_, '_>,
        start: DynamicModuleStart,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
    ) -> Result<u64, KernelStartupError> {
        if self.pause()?.owner_id() != start.owner
            || current.owner_id() != start.owner
            || returned.owner_id() != start.owner
            || returned.source_plan().identity() != self.memory().active_source_identity()
            || current.materialized_source().source_plan().identity()
                != returned.source_plan().identity()
        {
            return Err(invalid("module initializer return source"));
        }
        let state = self
            .loaded_modules
            .dynamic
            .files
            .get(start.file)
            .ok_or(invalid("module initializer state"))?
            .state;
        let FileState::Mapped {
            start: expected,
            count,
            call: Some(call),
        } = state
        else {
            return Err(invalid("module initializer return replay"));
        };
        let stack = returned.stack();
        let initial = returned.initial_state();
        if expected != start
            || returned.executor_id() != call.executor
            || returned.call_sequence() != call.call
            || returned.entry() != start.entry
            || stack.stack != call.stack
            || stack.guards != call.guards
            || stack.return_slot != call.slot
            || stack.return_marker != call.marker
            || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != start.args()
        {
            return Err(invalid("module initializer return identity"));
        }
        let value = returned.value() as u32 as i32;
        if value < 0 {
            self.loaded_modules.dynamic.files[start.file].state = FileState::Failed;
            return Err(invalid("negative module initializer result"));
        }
        if let Some(completion) = start.sysmodule {
            let (mut paused, threads) =
                self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
            let mut memory = KernelMemory {
                paused: &mut paused,
                source: Some(current.materialized_source().source_plan()),
                supplied_modules: self.plan.kernel_provider_index(),
                lookup_limits: None,
                lookup_work: None,
                loaded_modules: None,
                thread_tls: threads.get(self.active_tls),
                initial_entry: None,
            };
            let completed = self.kernel.complete_sysmodule_load(completion, &mut memory);
            if let Err(error) = completed {
                self.loaded_modules.dynamic.files[start.file].state = FileState::Failed;
                return Err(KernelStartupError::Kernel(error));
            }
        } else if let Some(address) = start.result
            && let Err(error) = self.pause()?.write(address, &value.to_le_bytes())
        {
            self.loaded_modules.dynamic.files[start.file].state = FileState::Failed;
            return Err(KernelStartupError::Kernel(Failure::Guest(
                crate::process::hle::kernel_adapter::error(error, true),
            )));
        }
        self.loaded_modules.dynamic.files[start.file].state = FileState::Ready { start, count };
        Ok(if start.sysmodule.is_some() {
            0
        } else {
            start.handle as u64
        })
    }
}

fn shape(runtime: &RuntimeLoadPlan<'_>) -> Result<(u64, u64, u64), KernelStartupError> {
    if !matches!(
        runtime.metadata.elf_type,
        ElfType::SharedObject | ElfType::SceSharedObject
    ) {
        return Err(invalid("new module role"));
    }
    let mut low = u64::MAX;
    let mut high = 0;
    let mut alignment = GRANULE;
    for segment in &runtime.segments {
        if segment.mem_size == 0 {
            continue;
        }
        low = low.min(segment.vaddr / crate::memory::PAGE_SIZE * crate::memory::PAGE_SIZE);
        high = high.max(aligned(
            segment
                .vaddr
                .checked_add(segment.mem_size)
                .ok_or(invalid("module extent"))?,
            crate::memory::PAGE_SIZE,
        )?);
        alignment = alignment.max(segment.align);
    }
    if low == u64::MAX || !alignment.is_power_of_two() || alignment > 2 * 1024 * 1024 {
        return Err(invalid("module alignment"));
    }
    Ok((low, high, alignment))
}
fn initializer(module: &pentax_linker::PlannedModule<'_>) -> Result<u64, KernelStartupError> {
    let mut init = None;
    for record in module
        .runtime_plan()
        .dynamic
        .as_ref()
        .map_or(&[][..], |value| value.entries.as_slice())
        .iter()
        .filter(|record| record.tag == 12)
    {
        if init.replace(record.value).is_some() {
            return Err(invalid("duplicate module initializer"));
        }
    }
    let relative = init
        .filter(|value| *value != 0)
        .ok_or(invalid("missing module initializer"))?;
    if !module.runtime_plan().segments.iter().any(|segment| {
        segment.flags.executable()
            && !segment.flags.writable()
            && segment.vaddr <= relative
            && relative - segment.vaddr < segment.file_bytes.len() as u64
    }) {
        return Err(invalid("module initializer target"));
    }
    module
        .load_bias()
        .checked_add(relative)
        .ok_or(invalid("module initializer target"))
}

pub(in crate::process::hle) fn prepare_action(
    memory: &mut KernelMemory<'_, '_, '_>,
    request: AdmittedModuleLoad,
    pc: u64,
    id: u64,
) -> Result<NativeServiceAction, Failure> {
    let owner = memory.paused.owner_id();
    let source = memory.source.ok_or(failure("module source"))?.identity();
    let modules = memory
        .loaded_modules
        .as_deref_mut()
        .ok_or(failure("module registry"))?;
    modules
        .dynamic
        .charge((request.path().len() as u64 + 1) * (modules.dynamic.files.len() as u64 + 1) + 16)
        .map_err(|_| Failure::Limit("module lookup work"))?;
    if let Some(index) = modules
        .dynamic
        .files
        .iter()
        .position(|file| file.path == request.path())
        && matches!(modules.dynamic.files[index].state, FileState::Registered)
    {
        modules.dynamic.serial = modules
            .dynamic
            .serial
            .checked_add(1)
            .ok_or(Failure::Limit("module tickets"))?;
        let serial = modules.dynamic.serial;
        modules.dynamic.files[index].state = FileState::Reading(serial);
        return Ok(NativeServiceAction::Load(PendingModuleLoad {
            owner,
            source,
            serial,
            file: index,
            pc,
            id,
            request,
        }));
    }
    let value = super::retain(memory, request.path(), request.result_pointer())?;
    Ok(NativeServiceAction::Returned(value as u64))
}

pub(in crate::process::hle) fn has_game_module(memory: &KernelMemory<'_, '_, '_>, id: u16) -> bool {
    pentax_kernel::game_module_path(id).is_some_and(|path| {
        memory
            .loaded_modules
            .as_deref()
            .is_some_and(|modules| modules.dynamic.contains(path))
    })
}

pub(in crate::process::hle) fn prepare_sysmodule_action(
    memory: &mut KernelMemory<'_, '_, '_>,
    kernel: &mut pentax_kernel::Kernel,
    request: AdmittedModuleLoad,
    pc: u64,
    id: u64,
) -> Result<NativeServiceAction, Failure> {
    let completion = request
        .sysmodule_completion()
        .ok_or(failure("system module admission"))?;
    let modules = memory
        .loaded_modules
        .as_deref_mut()
        .ok_or(failure("module registry"))?;
    modules
        .dynamic
        .charge((request.path().len() as u64 + 1) * (modules.dynamic.files.len() as u64 + 1) + 16)
        .map_err(|_| Failure::Limit("module lookup work"))?;
    let file = modules
        .dynamic
        .files
        .iter()
        .find(|file| file.path == request.path())
        .ok_or(failure("system module capability"))?;
    match file.state {
        FileState::Registered => prepare_action(memory, request, pc, id),
        // A reentrant self-load may retain the already mapped module while its
        // outer initializer is running, without claiming that start completed.
        FileState::Mapped { .. } | FileState::Ready { .. } => kernel
            .complete_sysmodule_load(completion, memory)
            .map(NativeServiceAction::Returned),
        _ => Err(failure("system module not published")),
    }
}

pub(super) fn retain_existing(
    memory: &mut KernelMemory<'_, '_, '_>,
    path: &str,
    result: Option<u64>,
) -> Option<Result<i32, Failure>> {
    let modules = memory.loaded_modules.as_deref_mut()?;
    if modules
        .dynamic
        .charge((path.len() as u64 + 1) * (modules.dynamic.files.len() as u64 + 1) + 16)
        .is_err()
    {
        return Some(Err(Failure::Limit("module lookup work")));
    }
    let index = modules
        .dynamic
        .files
        .iter()
        .position(|file| file.path == path)?;
    let file = &mut modules.dynamic.files[index];
    let (start, count, call, ready) = match file.state {
        FileState::Mapped { start, count, call } => (start, count, call, false),
        FileState::Ready { start, count } => (start, count, None, true),
        _ => return Some(Err(failure("module not published"))),
    };
    let action = (|| {
        let count = count
            .checked_add(1)
            .ok_or(Failure::Limit("module load count"))?;
        if let Some(address) = result {
            memory
                .paused
                .write(address, &0_i32.to_le_bytes())
                .map_err(|value| {
                    Failure::Guest(crate::process::hle::kernel_adapter::error(value, true))
                })?;
        }
        file.state = if ready {
            FileState::Ready { start, count }
        } else {
            FileState::Mapped { start, count, call }
        };
        Ok(start.handle)
    })();
    Some(action)
}
