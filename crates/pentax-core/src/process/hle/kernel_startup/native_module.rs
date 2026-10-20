// SPDX-License-Identifier: GPL-2.0-only

//! Observed StartModule(libc, 0, null, null) preparation, without execution.
//! Adapted from pinned Kyty runtimeLinker.cpp; see docs/upstream-provenance.md.

use super::{PreparedNativeEntry, PreparedNativeKernelStartup, PreparedNativeSource};
use pentax_linker::SymbolicLinkPlan;
use pentax_loader::ElfType;
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId};
use std::fmt;

/// History of an exact module's DT_INIT target and the prepared process entry.
/// It does not attest continuing liveness or establish that initialization ran.
/// The observed call takes three zero arguments in RDI/RSI/RDX. The caller must
/// retain the actual thread and cumulative execution budgets between calls.
///
/// ```compile_fail
/// use pentax_core::process::hle::PreparedNativeModuleStart;
/// fn forge<'p, 'i>() -> PreparedNativeModuleStart<'p, 'i> {
///     PreparedNativeModuleStart { entry: 0x10000, module: 1 }
/// }
/// ```
#[derive(Clone, Copy)]
pub struct PreparedNativeModuleStart<'plan, 'image> {
    owner: NativeOwnerId,
    source: &'plan SymbolicLinkPlan<'image>,
    process: super::NativeEntryLayout,
    module: usize,
    entry: u64,
}
impl<'plan, 'image> PreparedNativeModuleStart<'plan, 'image> {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn process_entry(&self) -> &super::NativeEntryLayout {
        &self.process
    }
    pub const fn module_index(&self) -> usize {
        self.module
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
}
impl fmt::Debug for PreparedNativeModuleStart<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedNativeModuleStart")
            .field("module_index", &self.module)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeModuleStartError {
    WrongPreparedSource,
    InvalidModule,
    DuplicateInitializer,
    InvalidTarget,
    Budget,
    Memory(NativeMemoryError),
}
impl NativeModuleStartError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::WrongPreparedSource => "initializer-source-identity",
            Self::InvalidModule => "initializer-module",
            Self::DuplicateInitializer => "duplicate-initializer",
            Self::InvalidTarget => "initializer-target",
            Self::Budget => "initializer-work",
            Self::Memory(_) => "initializer-memory",
        }
    }
}
impl fmt::Display for NativeModuleStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for NativeModuleStartError {}

impl<'plan, 'image> PreparedNativeKernelStartup<'plan, 'image> {
    /// Prepare one explicitly selected shared module under the observed libc
    /// preload convention. No names, automatic discovery or input-order sweep
    /// choose a library. Missing/zero DT_INIT means there is no call to make;
    /// it is not counted as successful initialization. Arrays remain guest
    /// runtime responsibilities, rather than guessed host-call sequences.
    pub fn prepare_module_initializer(
        &mut self,
        proof: &PreparedNativeSource<'plan, 'image>,
        process: &PreparedNativeEntry<'plan, 'image>,
        module_index: usize,
        max_work_steps: u64,
    ) -> Result<Option<PreparedNativeModuleStart<'plan, 'image>>, NativeModuleStartError> {
        let source = proof.materialized_source().source_plan();
        if proof.owner_id() != self.owner_id()
            || process.owner_id() != self.owner_id()
            || !std::ptr::eq(source, self.plan().link_plan())
            || !std::ptr::eq(source, self.memory().source_plan())
            || !std::ptr::eq(source, process.source_plan())
        {
            return Err(NativeModuleStartError::WrongPreparedSource);
        }
        let module = source
            .modules()
            .get(module_index)
            .filter(|_| module_index != 0)
            .ok_or(NativeModuleStartError::InvalidModule)?;
        let runtime = module.runtime_plan();
        if !matches!(
            runtime.metadata.elf_type,
            ElfType::SharedObject | ElfType::SceSharedObject
        ) {
            return Err(NativeModuleStartError::InvalidModule);
        }
        let entries = runtime
            .dynamic
            .as_ref()
            .map_or(&[][..], |table| table.entries.as_slice());
        if (entries.len() as u64)
            .checked_add(runtime.segments.len() as u64)
            .and_then(|work| work.checked_add(16))
            .is_none_or(|work| work > max_work_steps)
        {
            return Err(NativeModuleStartError::Budget);
        }
        let mut init = None;
        for entry in entries.iter().filter(|entry| entry.tag == 12) {
            if init.replace(entry.value).is_some() {
                return Err(NativeModuleStartError::DuplicateInitializer);
            }
        }
        let Some(relative) = init.filter(|value| *value != 0) else {
            return Ok(None);
        };
        let address = module
            .load_bias()
            .checked_add(relative)
            .ok_or(NativeModuleStartError::InvalidTarget)?;
        if !runtime.segments.iter().any(|segment| {
            segment.flags.executable()
                && !segment.flags.writable()
                && segment.vaddr <= relative
                && relative
                    .checked_sub(segment.vaddr)
                    .is_some_and(|offset| offset < segment.file_bytes.len() as u64)
        }) {
            return Err(NativeModuleStartError::InvalidTarget);
        }
        let paused = self
            .pause()
            .map_err(|_| NativeModuleStartError::WrongPreparedSource)?;
        // DT_INIT is an instruction target; execute-only segments need no
        // additional guest data-read permission.
        paused
            .preflight_fetch_source(address, 1)
            .map_err(NativeModuleStartError::Memory)?;
        if !matches!(
            paused.preflight_write(address, 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(NativeModuleStartError::InvalidTarget);
        }
        Ok(Some(PreparedNativeModuleStart {
            owner: paused.owner_id(),
            source,
            process: *process.layout(),
            module: module_index,
            entry: address,
        }))
    }
}
