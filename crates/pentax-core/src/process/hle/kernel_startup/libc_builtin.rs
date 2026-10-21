// SPDX-License-Identifier: GPL-2.0-only
//! Explicit standard-memory ABI substitution at exact supplied exports.
//! No linker preference, instruction fingerprint or detached numeric authority.

use super::{KernelStartupError, PreparedNativeKernelStartup, PreparedNativeSource, charge};
use crate::process::hle::{KnownFunction, MemoryServiceError};
use pentax_linker::{
    DescriptorVersions, ModuleExportQuery, ModuleLookupLimits, NidToken, SymbolicLinkPlan,
    lookup_qualified_module_export,
};
use pentax_platform::native_memory::{NativeOwnerId, NativeRange};
use std::fmt;

const FUNCTIONS: [KnownFunction; 3] = [
    KnownFunction::Memcpy,
    KnownFunction::Memmove,
    KnownFunction::Memset,
];
// Four existing finite declarations, plus one separate standard-memory entry
// profile. The libc pair does not participate in prefix/name crossproducts.
const SCOPES: [(&[u8], &[u8]); 5] = [
    (b"LibcInternal", b"LibcInternal"),
    (b"LibcInternal", b"libSceLibcInternal"),
    (b"libSceLibcInternal", b"LibcInternal"),
    (b"libSceLibcInternal", b"libSceLibcInternal"),
    (b"libc", b"libc"),
];

/// Helper attempts/completions in memcpy, memmove, memset order. A completed
/// helper can still encounter a terminal normal-RET fault; these are not native
/// instruction counts. Ordinary provider calls do not affect this snapshot.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LibcBuiltinUsage {
    pub attempted: [u64; 3],
    pub completed: [u64; 3],
}

#[derive(Clone, Copy)]
#[cfg_attr(
    not(all(target_os = "windows", target_arch = "x86_64")),
    allow(dead_code)
)]
struct Entry {
    address: u64,
    executable: NativeRange,
}

/// Fixed manifest tied to the current immutable source and actual owner. Only
/// preparation can create it. Presence is metadata, never live permission.
#[cfg_attr(
    not(all(target_os = "windows", target_arch = "x86_64")),
    allow(dead_code)
)]
pub struct PreparedLibcBuiltins<'source, 'image> {
    owner: NativeOwnerId,
    source: &'source SymbolicLinkPlan<'image>,
    module: usize,
    source_scan_work: u64,
    entries: [Option<Entry>; 3],
}
impl fmt::Debug for PreparedLibcBuiltins<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedLibcBuiltins")
            .finish_non_exhaustive()
    }
}
impl PreparedLibcBuiltins<'_, '_> {
    pub fn function_count(&self) -> u64 {
        self.entries.iter().flatten().count() as u64
    }
    /// Cheap selection only; actual invocation rechecks the complete binding.
    pub fn contains_pc(&self, pc: u64) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.address == pc)
    }
}

#[derive(Debug)]
pub enum LibcBuiltinError {
    Core(KernelStartupError),
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    Dispatch(pentax_platform::native_dispatch::NativeDispatchError),
    Service(MemoryServiceError),
}
impl fmt::Display for LibcBuiltinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("checked supplied-libc builtin refused")
    }
}
impl std::error::Error for LibcBuiltinError {}
impl From<KernelStartupError> for LibcBuiltinError {
    fn from(error: KernelStartupError) -> Self {
        Self::Core(error)
    }
}
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
impl From<pentax_platform::native_dispatch::NativeDispatchError> for LibcBuiltinError {
    fn from(error: pentax_platform::native_dispatch::NativeDispatchError) -> Self {
        Self::Dispatch(error)
    }
}

fn invalid() -> KernelStartupError {
    KernelStartupError::InvalidProvider("supplied libc builtin identity")
}

fn scan_work(source: &SymbolicLinkPlan<'_>) -> Result<u64, KernelStartupError> {
    source.modules().iter().try_fold(1_u64, |work, module| {
        work.checked_add(1)
            .and_then(|n| n.checked_add(module.runtime_plan().segments.len() as u64))
            .ok_or(KernelStartupError::Budget("libc source scan"))
    })
}

/// Preserve ordered, initialized RX membership; no BSS or writable executable
/// entry is eligible. All caller scans are independently precharged.
fn executable_interval(
    source: &SymbolicLinkPlan<'_>,
    pc: u64,
) -> Result<Option<(usize, NativeRange)>, KernelStartupError> {
    for (index, module) in source.modules().iter().enumerate() {
        for segment in &module.runtime_plan().segments {
            if !segment.flags.executable() || segment.flags.writable() {
                continue;
            }
            let start = module
                .load_bias()
                .checked_add(segment.vaddr)
                .ok_or_else(invalid)?;
            let size = segment.file_bytes.len() as u64;
            let end = start.checked_add(size).ok_or_else(invalid)?;
            if start <= pc && pc < end {
                return Ok(Some((
                    index,
                    NativeRange {
                        address: start,
                        size,
                    },
                )));
            }
        }
    }
    Ok(None)
}

impl PreparedNativeKernelStartup<'_, '_> {
    pub fn libc_builtin_usage(&self) -> LibcBuiltinUsage {
        self.libc_builtins
    }

    /// Explicitly select ONE originally supplied module. Generated providers
    /// and the primary image cannot mint this optional semantic profile.
    /// Missing/wrong-scope exports remain ordinary native calls. Ambiguity and
    /// malformed metadata refuse preparation; no fallback definition is chosen.
    pub fn prepare_libc_builtins<'source, 'image>(
        &mut self,
        current: &PreparedNativeSource<'source, 'image>,
        module: usize,
    ) -> Result<PreparedLibcBuiltins<'source, 'image>, KernelStartupError> {
        let source = current.materialized_source().source_plan();
        if current.owner_id() != self.owner_id()
            || source.identity() != self.memory().active_source_identity()
            || module == 0
            || module >= self.plan.kernel_provider_index()
        {
            return Err(invalid());
        }
        charge(
            &mut self.provider_work,
            source.modules().len() as u64 + 1,
            self.plan.limits.max_provider_work,
        )?;
        let source_scan_work = scan_work(source)?;
        let mut manifest = PreparedLibcBuiltins {
            owner: self.owner_id(),
            source,
            module,
            source_scan_work,
            entries: [None; 3],
        };
        for (index, function) in FUNCTIONS.into_iter().enumerate() {
            for (module_name, library_name) in SCOPES {
                let remaining = self
                    .plan
                    .limits
                    .max_provider_work
                    .saturating_sub(self.provider_work);
                let limits = ModuleLookupLimits {
                    max_work: remaining,
                    ..Default::default()
                };
                let query = ModuleExportQuery {
                    nid: NidToken::parse(function.nid()).expect("public NID"),
                    module: module_name,
                    library: library_name,
                    versions: DescriptorVersions {
                        module_major: 1,
                        module_minor: 1,
                        library: 1,
                    },
                };
                let attempt = lookup_qualified_module_export(source, module, &query, &limits);
                charge(
                    &mut self.provider_work,
                    attempt.work,
                    self.plan.limits.max_provider_work,
                )?;
                let Some(export) = attempt.result.map_err(|_| invalid())? else {
                    continue;
                };
                let remaining = self
                    .plan
                    .limits
                    .max_provider_work
                    .saturating_sub(self.provider_work);
                let identity = export.qualified_identity(remaining);
                charge(
                    &mut self.provider_work,
                    identity.work,
                    self.plan.limits.max_provider_work,
                )?;
                let identity = identity.result.map_err(|_| invalid())?;
                // Recheck the exact selected pair; neither name alone admits
                // a hybrid between libc and the existing internal scopes.
                charge(
                    &mut self.provider_work,
                    128,
                    self.plan.limits.max_provider_work,
                )?;
                if !identity.global_function
                    || identity.nid != NidToken::parse(function.nid()).expect("public NID")
                    || identity.module != module_name
                    || identity.library != library_name
                    || identity.versions.module_major != 1
                    || identity.versions.module_minor != 1
                    || identity.versions.library != 1
                {
                    continue;
                }
                charge(
                    &mut self.provider_work,
                    source_scan_work,
                    self.plan.limits.max_provider_work,
                )?;
                let (selected, executable) =
                    executable_interval(source, export.address())?.ok_or_else(invalid)?;
                if selected != module || export.definition().module.index() != module {
                    return Err(invalid());
                }
                for (other, entry) in manifest.entries.iter().enumerate() {
                    if let Some(entry) = entry
                        && ((other == index && entry.address != export.address())
                            || (other != index && entry.address == export.address()))
                    {
                        return Err(invalid());
                    }
                }
                manifest.entries[index] = Some(Entry {
                    address: export.address(),
                    executable,
                });
            }
        }
        Ok(manifest)
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn try_libc_builtin<'view, 'source, 'image>(
        &mut self,
        manifest: &PreparedLibcBuiltins<'source, 'image>,
        current: &PreparedNativeSource<'source, 'image>,
        engine: &mut pentax_platform::native_dispatch::NativeBlockStepper<'view, 'source, 'image>,
        block: &pentax_codegen::MaterializedBlockPlan<'_, 'source, 'image>,
    ) -> Result<Option<pentax_platform::native_dispatch::NativeBlockStop>, LibcBuiltinError> {
        use crate::process::hle::dispatch::{self, AbiArgs};
        use pentax_platform::native_memory::NativeMemoryError;
        let pc = engine.state().guest_pc;
        let Some(index) = manifest
            .entries
            .iter()
            .position(|entry| entry.is_some_and(|e| e.address == pc))
        else {
            return Ok(None);
        };
        // A modeled repeated string can resume at the same PC after changing
        // ABI argument registers. Never reinterpret even its initial entry as
        // a whole-function call; retained compare state is a separate guard.
        if !engine.libc_builtin_entry_ready()
            || block.instructions().first().is_some_and(|record| {
                let instruction = record.instruction();
                record.guest_pc() == pc
                    && instruction.is_string_instruction()
                    && (instruction.has_rep_prefix() || instruction.has_repne_prefix())
            })
        {
            return Ok(None);
        }
        if manifest.owner != self.owner_id()
            || current.owner_id() != manifest.owner
            || engine.owner_id() != manifest.owner
            || !std::ptr::eq(manifest.source, current.materialized_source().source_plan())
            || !std::ptr::eq(block.declaration(), current.materialized_source())
            || block.module().index() != manifest.module
            || block.guest_pc() != pc
            || self.memory().active_source_identity() != manifest.source.identity()
        {
            return Err(invalid().into());
        }
        let state = engine.state();
        let args = AbiArgs {
            rdi: state.gpr[7],
            rsi: state.gpr[6],
            rdx: state.gpr[2],
        };
        if index == 0 && virtual_overlap(args.rdi, args.rsi, args.rdx) {
            return Ok(None);
        }
        // Split only disjoint owned fields: the exact paused owner, existing HLE
        // budget, provider scan budget and fixed profile counters.
        let maximum = self.plan.limits.max_provider_work;
        let (mut memory, budget) = self
            .hle
            .pause_with_service_budget()
            .map_err(KernelStartupError::Hle)?;
        let token = engine.begin_libc_builtin(&memory, block)?;
        let eligibility = (|| -> Result<bool, LibcBuiltinError> {
            if args.rdx == 0 {
                return Ok(true);
            }
            let output = NativeRange {
                address: args.rdi,
                size: args.rdx,
            };
            let mut protected = [None; 3];
            protected[0] = Some(NativeRange {
                address: token.return_slot(),
                size: 8,
            });
            protected[1] = Some(manifest.entries[index].expect("selected").executable);
            if let Some(target) = token.return_target() {
                charge(&mut self.provider_work, manifest.source_scan_work, maximum)?;
                protected[2] = Some(
                    executable_interval(manifest.source, target)?
                        .ok_or_else(invalid)?
                        .1,
                );
            }
            for span in protected
                .into_iter()
                .flatten()
                .chain((index == 0).then_some(NativeRange {
                    address: args.rsi,
                    size: args.rdx,
                }))
            {
                let work = alias_work(output, span)?;
                charge(&mut self.provider_work, work, maximum)?;
                match memory.preflight_disjoint_backing(output, span, work) {
                    Ok(_) => {}
                    Err(NativeMemoryError::Overlap) => return Ok(false),
                    Err(error) => {
                        return Err(LibcBuiltinError::Core(KernelStartupError::Hle(
                            super::super::HleError::Memory(crate::memory::MemoryError::Native(
                                error,
                            )),
                        )));
                    }
                }
            }
            Ok(true)
        })();
        match eligibility {
            Ok(true) => {}
            Ok(false) => {
                engine.cancel_libc_builtin(token)?;
                return Ok(None);
            }
            Err(error) => {
                engine.cancel_libc_builtin(token)?;
                return Err(error);
            }
        }
        engine.validate_libc_builtin(&memory, &token)?;
        self.libc_builtins.attempted[index] = self.libc_builtins.attempted[index].saturating_add(1);
        let result = budget.begin_attempt().and_then(|attempt| {
            dispatch::invoke(attempt, FUNCTIONS[index], args, &mut memory, budget)
        });
        match result {
            Err(error) => {
                engine.fail_libc_builtin(token)?;
                Err(LibcBuiltinError::Service(error))
            }
            Ok(result) => {
                self.libc_builtins.completed[index] =
                    self.libc_builtins.completed[index].saturating_add(1);
                Ok(Some(
                    engine.complete_libc_builtin(&memory, token, result.rax)?,
                ))
            }
        }
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn virtual_overlap(left: u64, right: u64, size: u64) -> bool {
    size != 0
        && left
            .checked_add(size)
            .zip(right.checked_add(size))
            .is_some_and(|(le, re)| left < re && right < le)
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn alias_work(left: NativeRange, right: NativeRange) -> Result<u64, KernelStartupError> {
    let pages = |span: NativeRange| {
        span.address
            .checked_add(span.size)
            .filter(|_| span.size != 0)
            .map(|end| {
                (end - 1) / crate::memory::PAGE_SIZE - span.address / crate::memory::PAGE_SIZE + 1
            })
    };
    let left = pages(left).ok_or_else(invalid)?;
    let right = pages(right).ok_or_else(invalid)?;
    left.checked_mul(right)
        .and_then(|n| n.checked_add(left.checked_add(right)?.checked_mul(3)?))
        .and_then(|n| n.checked_add(1))
        .ok_or(KernelStartupError::Budget("libc alias work"))
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
