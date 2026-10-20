// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: source-bound retained-file module reuse.
// Adapted from pinned KernelLoadStartModule; see docs/upstream-provenance.md.

//! Explicit retained files and the pinned already-loaded LoadStartModule branch.
//! File admission and an actual initializer return are separate requirements.

use super::{KernelStartupError, PreparedNativeKernelStartup, charge};
use crate::process::hle::kernel_adapter::KernelMemory;
use pentax_kernel::Failure;
use pentax_loader::ElfType;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

mod dynamic;
pub use dynamic::{DynamicModuleInput, DynamicModuleStart, NativeServiceAction, PendingModuleLoad};

pub(crate) struct LoadedModule {
    path: String,
    module: usize,
    file: File,
    initialized: Option<(u64, u64)>,
    load_count: u32,
}
#[derive(Default)]
pub(crate) struct LoadedModules {
    entries: Vec<LoadedModule>,
    handles: Vec<usize>,
    pub(in crate::process::hle::kernel_startup) dynamic: dynamic::DynamicFiles,
}
impl LoadedModules {
    pub(super) fn new(inputs: usize) -> Result<Self, KernelStartupError> {
        let mut handles = Vec::new();
        handles
            .try_reserve_exact(inputs)
            .map_err(|_| KernelStartupError::Allocation)?;
        handles.extend(0..inputs);
        Ok(Self {
            handles,
            ..Default::default()
        })
    }
    pub(crate) fn resolve_handle(&self, handle: i32) -> Option<usize> {
        usize::try_from(handle)
            .ok()
            .and_then(|value| value.checked_sub(1))
            .and_then(|index| self.handles.get(index))
            .copied()
    }
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path[1..]
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Bind one canonical guest path to an explicitly owned file whose entire
    /// original input matches this actual supplied module. The file is retained
    /// and never written. Its owner should deny replacement/writers during use.
    /// This operation neither loads new mappings nor marks initialization done.
    pub fn bind_loaded_module_file(
        &mut self,
        module: usize,
        path: &str,
        mut file: File,
    ) -> Result<(), KernelStartupError> {
        let source = self.memory().source_plan();
        if !std::ptr::eq(source, self.plan().link_plan())
            || module == 0
            || module >= self.plan.kernel_provider_index()
            || path.len() >= self.plan.limits.kernel.max_name_bytes
            || !valid_path(path)
        {
            return Err(KernelStartupError::InvalidProvider("module file binding"));
        }
        if self.loaded_modules.entries.len() >= self.plan.limits.kernel.max_handles {
            return Err(KernelStartupError::Budget("module file bindings"));
        }
        let selected = &source.modules()[module];
        if !matches!(
            selected.runtime_plan().metadata.elf_type,
            ElfType::SharedObject | ElfType::SceSharedObject
        ) {
            return Err(KernelStartupError::InvalidProvider("module file role"));
        }
        let bytes = selected.source_bytes();
        let work = (bytes.len() as u64)
            .checked_mul(2)
            .and_then(|value| {
                value.checked_add(
                    (path.len() as u64 + 1) * (self.loaded_modules.entries.len() as u64 + 1),
                )
            })
            .and_then(|value| value.checked_add(32))
            .ok_or(KernelStartupError::Budget("module file binding"))?;
        charge(
            &mut self.provider_work,
            work,
            self.plan.limits.max_provider_work,
        )?;
        if self
            .loaded_modules
            .entries
            .iter()
            .any(|entry| entry.module == module || entry.path == path)
            || self.loaded_modules.dynamic.contains(path)
        {
            return Err(KernelStartupError::InvalidProvider(
                "duplicate module file binding",
            ));
        }
        let failure = || KernelStartupError::InvalidProvider("module file source mismatch");
        let metadata = file.metadata().map_err(|_| failure())?;
        if !metadata.is_file() || metadata.len() != bytes.len() as u64 {
            return Err(failure());
        }
        file.seek(SeekFrom::Start(0)).map_err(|_| failure())?;
        let mut buffer = [0; 8192];
        for expected in bytes.chunks(buffer.len()) {
            file.read_exact(&mut buffer[..expected.len()])
                .map_err(|_| failure())?;
            if buffer[..expected.len()] != *expected {
                return Err(failure());
            }
        }
        if file.read(&mut buffer[..1]).map_err(|_| failure())? != 0 {
            return Err(failure());
        }
        let mut owned = String::new();
        owned
            .try_reserve_exact(path.len())
            .map_err(|_| KernelStartupError::Allocation)?;
        owned.push_str(path);
        self.loaded_modules
            .entries
            .try_reserve(1)
            .map_err(|_| KernelStartupError::Allocation)?;
        self.loaded_modules.entries.push(LoadedModule {
            path: owned,
            module,
            file,
            initialized: None,
            load_count: 0,
        });
        Ok(())
    }

    /// Successful already-loaded service calls, not initializer observations.
    pub fn module_load_count(&self, module: usize) -> Option<u32> {
        self.loaded_modules
            .entries
            .iter()
            .find(|entry| entry.module == module)
            .map(|entry| entry.load_count)
            .or_else(|| self.loaded_modules.dynamic.load_count(module))
    }

    /// Consume historical evidence of the explicit first-call preload returning
    /// through its exact process slot. Register no completion from public stop
    /// integers, metadata, an error, another owner/source/call or a replay.
    /// Unbound preloads need no registry and return false without adding one.
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn observe_module_initializer(
        &mut self,
        prepared: &super::PreparedNativeModuleStart<'_, '_>,
        returned: &pentax_platform::native_dispatch::NativeCallReturn<'_, '_>,
    ) -> Result<bool, KernelStartupError> {
        charge(
            &mut self.provider_work,
            self.loaded_modules.entries.len() as u64 + 32,
            self.plan.limits.max_provider_work,
        )?;
        let live_owner = self.pause()?.owner_id();
        let process = prepared.process_entry();
        let stack = returned.stack();
        let initial = returned.initial_state();
        let fs = if self.tls().thread_control_policy()
            == crate::process::tls::ThreadControlBlockPolicy::None
        {
            None
        } else {
            Some(
                self.tls()
                    .instances()
                    .first()
                    .ok_or(KernelStartupError::InvalidProvider("initializer thread"))?
                    .thread_pointer,
            )
        };
        if prepared.owner_id() != live_owner
            || returned.owner_id() != live_owner
            || !std::ptr::eq(prepared.source_plan(), self.plan().link_plan())
            || returned.source_plan().identity() != self.memory().active_source_identity()
            || returned.call_sequence() != 0
            || returned.entry() != prepared.entry()
            || stack.stack != process.stack
            || stack.guards != Some(process.guards)
            || stack.return_slot != process.return_slot
            || stack.return_marker != process.return_marker
            || initial.gpr[4] != process.return_slot
            || initial.gpr[5] != process.frame_pointer
            || [initial.gpr[7], initial.gpr[6], initial.gpr[2]] != [0; 3]
            || initial.fs_base != fs
            || initial.gs_base.is_some()
        {
            return Err(KernelStartupError::InvalidProvider(
                "initializer completion identity",
            ));
        }
        let Some(entry) = self
            .loaded_modules
            .entries
            .iter_mut()
            .find(|entry| entry.module == prepared.module_index())
        else {
            return Ok(false);
        };
        if entry.initialized.is_some() {
            return Err(KernelStartupError::InvalidProvider(
                "initializer completion replay",
            ));
        }
        entry.initialized = Some((returned.executor_id(), returned.call_sequence()));
        Ok(true)
    }
}

pub(in crate::process::hle) fn retain(
    memory: &mut KernelMemory<'_, '_, '_>,
    path: &str,
    result: Option<u64>,
) -> Result<i32, Failure> {
    if let Some(result) = dynamic::retain_existing(memory, path, result) {
        return result;
    }
    let source = memory
        .source
        .ok_or(Failure::Unsupported("module file source"))?;
    let limits = memory
        .lookup_limits
        .ok_or(Failure::Unsupported("module file limits"))?;
    let work = memory
        .lookup_work
        .as_deref_mut()
        .ok_or(Failure::Unsupported("module file work"))?;
    let modules = memory
        .loaded_modules
        .as_deref_mut()
        .ok_or(Failure::Unsupported("module file registry"))?;
    let amount = (path.len() as u64 + 1)
        .checked_mul(modules.entries.len() as u64 + 1)
        .and_then(|value| value.checked_add(16))
        .ok_or(Failure::Limit("module file lookup"))?;
    *work = work
        .checked_add(amount)
        .filter(|value| *value <= limits.max_work)
        .ok_or(Failure::Limit("module file lookup"))?;
    let entry = modules
        .entries
        .iter_mut()
        .find(|entry| entry.path == path)
        .ok_or(Failure::Unsupported("new or unbound module file"))?;
    if entry.module == 0
        || entry.module >= memory.supplied_modules
        || entry.module >= source.modules().len()
        || entry.initialized.is_none()
    {
        return Err(Failure::Unsupported(
            "module has no observed initializer return",
        ));
    }
    if !entry
        .file
        .metadata()
        .map_err(|_| Failure::Unsupported("retained module file"))?
        .is_file()
    {
        return Err(Failure::Unsupported("retained module file"));
    }
    let count = entry
        .load_count
        .checked_add(1)
        .ok_or(Failure::Limit("module load count"))?;
    let handle = i32::try_from(entry.module + 1).map_err(|_| Failure::Limit("module handle"))?;
    // All fallible state/output admission precedes the commit. Guest backing
    // faults leave count and result unchanged, including a nullable result.
    if let Some(address) = result {
        memory
            .paused
            .write(address, &0_i32.to_le_bytes())
            .map_err(|value| Failure::Guest(super::super::kernel_adapter::error(value, true)))?;
    }
    entry.load_count = count;
    Ok(handle)
}

pub(in crate::process::hle) use dynamic::{
    has_game_module, prepare_action, prepare_sysmodule_action,
};

#[cfg(test)]
mod tests {
    //! Data-only file/state/transaction checks. Synthetic private bookkeeping
    //! below is not an initializer-return proof or a native execution claim.
    use super::*;
    use crate::process::hle::{
        NativeImageLimits, build_kernel_provider, with_native_image_session,
    };
    use pentax_kernel::{CallArgs, Service};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct OriginalFile {
        path: PathBuf,
        bytes: Vec<u8>,
    }
    impl OriginalFile {
        fn new(bytes: &[u8]) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pentax-original-module-cap-{}-{}.elf",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            Self {
                path,
                bytes: bytes.to_vec(),
            }
        }
        fn open(&self) -> File {
            File::open(&self.path).unwrap()
        }
    }
    impl Drop for OriginalFile {
        fn drop(&mut self) {
            assert_eq!(std::fs::read(&self.path).unwrap(), self.bytes);
            std::fs::remove_file(&self.path).unwrap();
        }
    }
    fn loaded(
        test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64, u64, &OriginalFile),
    ) {
        let primary = crate::process::hle::unavailable::tests::original_input(0);
        let provider = build_kernel_provider(
            crate::process::hle::KernelStartupLimits::default().max_kernel_provider_bytes,
        )
        .unwrap();
        let file = OriginalFile::new(provider.image());
        with_native_image_session(
            &[&primary, provider.image()],
            &NativeImageLimits::default(),
            |session, layout| {
                test(
                    session,
                    layout.entry.stack.address + 0x100,
                    layout.entry.guards[1].address,
                    &file,
                );
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
    fn retain_call(
        session: &mut PreparedNativeKernelStartup<'_, '_>,
        name: u64,
        result: u64,
    ) -> Result<u64, KernelStartupError> {
        let marker = *session
            .plan
            .factory
            .markers()
            .iter()
            .find(|marker| marker.registration.service == Service::LoadStartModule)
            .unwrap();
        session.kernel_stop(super::super::KernelServiceStop {
            syscall_pc: session.plan.entry(marker) + 5,
            service_id: marker.service_id.into(),
            args: CallArgs([name, 17, 0xdead, 1_u64 << 32, 0, result]),
        })
    }
    const PATH: &str = "/app0/sce_module/original.sprx";

    #[test]
    fn module_file_binding_checks_complete_original_input_canonical_path_roles_and_duplicates() {
        loaded(|session, _, _, file| {
            let mut changed = file.bytes.clone();
            changed[0x700] ^= 1;
            let wrong = OriginalFile::new(&changed);
            assert!(
                session
                    .bind_loaded_module_file(1, PATH, wrong.open())
                    .is_err()
            );
            for (index, path) in [
                (0, PATH),
                (2, PATH),
                (1, "/app0/../original.sprx"),
                (1, "relative.sprx"),
            ] {
                assert!(
                    session
                        .bind_loaded_module_file(index, path, file.open())
                        .is_err()
                );
            }
            assert!(session.loaded_modules.entries.is_empty());
            session
                .bind_loaded_module_file(1, PATH, file.open())
                .unwrap();
            assert_eq!(session.module_load_count(1), Some(0));
            assert!(
                session
                    .bind_loaded_module_file(1, PATH, file.open())
                    .is_err()
            );
            assert_eq!(session.loaded_modules.entries.len(), 1);
        });
    }

    #[test]
    fn module_retain_requires_observation_and_commits_only_exact_int32_output_with_its_count() {
        loaded(|session, scratch, _, file| {
            session
                .bind_loaded_module_file(1, PATH, file.open())
                .unwrap();
            session
                .pause()
                .unwrap()
                .write(scratch, format!("{PATH}\0").as_bytes())
                .unwrap();
            let output = scratch + 128;
            let initial = 0x1122_3344_5566_7788_u64;
            session
                .pause()
                .unwrap()
                .write(output, &initial.to_le_bytes())
                .unwrap();
            assert!(matches!(
                retain_call(session, scratch, output),
                Err(KernelStartupError::Kernel(Failure::Unsupported(_)))
            ));
            assert_eq!(session.module_load_count(1), Some(0));
            let mut bytes = [0; 8];
            session.pause().unwrap().read(output, &mut bytes).unwrap();
            assert_eq!(bytes, initial.to_le_bytes());
            // Private synthetic state exercises a transaction only. Production
            // can set this field only with opaque actual-return evidence.
            session.loaded_modules.entries[0].initialized = Some((1, 0));
            assert_eq!(retain_call(session, scratch, output), Ok(2));
            session.pause().unwrap().read(output, &mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), 0x1122_3344_0000_0000);
            assert_eq!(retain_call(session, scratch, 0), Ok(2));
            assert_eq!(session.module_load_count(1), Some(2));
            session.loaded_modules.entries[0].load_count = u32::MAX;
            session
                .pause()
                .unwrap()
                .write(output, &initial.to_le_bytes())
                .unwrap();
            assert!(matches!(
                retain_call(session, scratch, output),
                Err(KernelStartupError::Kernel(Failure::Limit(_)))
            ));
            session.pause().unwrap().read(output, &mut bytes).unwrap();
            assert_eq!(bytes, initial.to_le_bytes());
            assert_eq!(session.module_load_count(1), Some(u32::MAX));
        });
    }

    #[test]
    fn failed_module_result_write_and_exhausted_file_admission_leave_registry_and_output_unchanged()
    {
        loaded(|session, scratch, guard, file| {
            session
                .bind_loaded_module_file(1, PATH, file.open())
                .unwrap();
            session.loaded_modules.entries[0].initialized = Some((1, 0));
            session
                .pause()
                .unwrap()
                .write(scratch, format!("{PATH}\0").as_bytes())
                .unwrap();
            // An RX original provider location is a readable but unwritable
            // result pointer. The error must not increment the load count.
            for (target, length) in [
                (session.plan.link_plan().modules()[1].load_bias(), 4),
                (guard - 2, 2),
            ] {
                let mut before = [0; 4];
                session
                    .pause()
                    .unwrap()
                    .read(target, &mut before[..length])
                    .unwrap();
                assert!(matches!(
                    retain_call(session, scratch, target),
                    Err(KernelStartupError::Kernel(Failure::Guest(_)))
                ));
                assert_eq!(session.module_load_count(1), Some(0));
                let mut after = [0; 4];
                session
                    .pause()
                    .unwrap()
                    .read(target, &mut after[..length])
                    .unwrap();
                assert_eq!(before, after);
            }
        });
        loaded(|session, _, _, file| {
            session.provider_work = session.plan.limits.max_provider_work - 1;
            assert!(matches!(
                session.bind_loaded_module_file(1, PATH, file.open()),
                Err(KernelStartupError::Budget(_))
            ));
            assert!(session.loaded_modules.entries.is_empty());
        });
    }
}
