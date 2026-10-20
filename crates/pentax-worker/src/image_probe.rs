// SPDX-License-Identifier: GPL-2.0-only

//! Bounded read-only intake and reviewed prepared-runtime composition.
//! Default diagnostics never print input paths, NIDs, PCs or raw source bytes.

use crate::native_session::{
    NativeSessionError, NativeSessionLimits, NativeSessionProgress, NativeSessionReport,
    run_prepared_with_initializer, run_prepared_with_window,
};
use pentax_codegen::Mnemonic;
use pentax_core::process::hle::{
    KernelStartupError, NativeImageError, NativeImageLimits, with_native_image_session_with_roots,
};
use pentax_platform::native_dispatch::{NativeCpuState, NativeDispatchError, NativeStackFrame};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::OpenOptionsExt;

mod backing;
mod data_files;
mod dispatch_diagnostics;

pub(super) enum ProbeError {
    Input(&'static str),
    Admission(NativeImageError<NativeSessionError>),
}
impl ProbeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Input(code) => code,
            Self::Admission(NativeImageError::Callback(error)) => match error {
                NativeSessionError::Core(KernelStartupError::UnavailableImport { .. }) => {
                    "unavailable-function"
                }
                NativeSessionError::Core(KernelStartupError::UnavailableExitCallback) => {
                    "module-exit-unavailable"
                }
                NativeSessionError::Core(KernelStartupError::UnavailableJsonAllocatorVirtual) => {
                    "json-allocator-virtual-unavailable"
                }
                NativeSessionError::Core(KernelStartupError::UnavailableJsonValueStorage) => {
                    "json-value-storage-unavailable"
                }
                NativeSessionError::Core(KernelStartupError::UnavailableVectorArguments) => {
                    "vector-arguments-unavailable"
                }
                NativeSessionError::Core(KernelStartupError::InvalidJsonString) => {
                    "invalid-json-string"
                }
                NativeSessionError::Core(KernelStartupError::UnavailableJsonFormat) => {
                    "json-format-unavailable"
                }
                NativeSessionError::Core(_) => "runtime-service",
                NativeSessionError::UnsupportedExit { .. } => "unsupported-instruction",
                NativeSessionError::Dispatch(NativeDispatchError::MemoryFootprint { .. }) => {
                    "unsupported-memory-footprint"
                }
                NativeSessionError::Dispatch(_) => "native-dispatch",
                NativeSessionError::Codegen(_) => "block-translation",
                NativeSessionError::Memory(_) => "guest-memory",
                NativeSessionError::Extension(_) => "module-linking",
                NativeSessionError::Budget(_) => "execution-budget",
                NativeSessionError::SourceWindow => "initialized-source-unavailable",
                NativeSessionError::WrongPreparedSource => "source-identity",
                NativeSessionError::OutputUnavailable => "protocol-output",
                NativeSessionError::Allocation => "execution-allocation",
                NativeSessionError::FixtureInvariant(_) => "execution-invariant",
                NativeSessionError::ThreadDeadlock => "thread-deadlock",
                NativeSessionError::Frame(_) => "frame-session",
            },
            Self::Admission(error) => error.code(),
        }
    }
}
pub(super) struct ProbeOutcome {
    pub result: Result<NativeSessionReport, ProbeError>,
    pub progress: NativeSessionProgress,
    pub unavailable_functions: Option<usize>,
}
impl ProbeOutcome {
    pub fn error_runtime_failure(&self) -> Option<&'static str> {
        if let Err(ProbeError::Admission(NativeImageError::Callback(
            NativeSessionError::Dispatch(error),
        ))) = &self.result
        {
            return Some(dispatch_diagnostics::classify(error));
        }
        let Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(error)))) =
            &self.result
        else {
            return None;
        };
        use pentax_kernel::{Failure, GuestError};
        Some(match error {
            KernelStartupError::Kernel(Failure::Guest(error)) => match error {
                GuestError::Allocation => "guest-allocation",
                GuestError::BudgetExceeded => "guest-memory-budget",
                GuestError::HostFailure => "guest-host-failure",
                GuestError::UnsupportedMapping => "guest-mapping",
                GuestError::Unmapped => "guest-unmapped",
                GuestError::ReadDenied => "guest-read-denied",
                GuestError::WriteDenied => "guest-write-denied",
                GuestError::AddressOverflow => "guest-address-range",
            },
            KernelStartupError::Kernel(Failure::Unsupported(reason)) => match *reason {
                "caller-provided thread stack" => "thread-custom-stack",
                "guest thread exit callbacks" => "thread-exit-callbacks",
                "guest-thread execution and lifecycle" => "thread-lifecycle",
                "contended pthread mutex wait" => "mutex-contended",
                "contended pthread priority protocol" => "mutex-priority-contended",
                "unowned pthread mutex token" => "mutex-unowned",
                "bound initial-thread pthread context" => "pthread-unbound",
                "source-aborting unmapped protection range" => "protection-untracked-range",
                "source-aborting protection range overflow" => "protection-range-overflow",
                "runtime-owned kernel mapping" => "protection-runtime-owned",
                _ => "unsupported-service",
            },
            KernelStartupError::Kernel(Failure::Allocation) | KernelStartupError::Allocation => {
                "runtime-allocation"
            }
            KernelStartupError::Kernel(Failure::Limit(_)) | KernelStartupError::Budget(_) => {
                "runtime-budget"
            }
            KernelStartupError::Kernel(Failure::Errno(_)) => "unconverted-errno",
            KernelStartupError::InvalidProvider(_) => "provider-validation",
            KernelStartupError::SiteMismatch { .. }
            | KernelStartupError::ServiceIdMismatch { .. } => "service-site-validation",
            KernelStartupError::NativeOwnerMismatch
            | KernelStartupError::ContinuationMismatch { .. } => "owner-continuation",
            KernelStartupError::UnavailableImport { .. }
            | KernelStartupError::UnavailableExitCallback
            | KernelStartupError::UnavailableJsonAllocatorVirtual
            | KernelStartupError::UnavailableJsonValueStorage => return None,
            KernelStartupError::UnavailableVectorArguments => return None,
            KernelStartupError::InvalidJsonString | KernelStartupError::UnavailableJsonFormat => {
                return None;
            }
            KernelStartupError::Graphics(pentax_core::gpu::driver::Failure::Shader(reason)) => {
                reason.label()
            }
            KernelStartupError::Graphics(pentax_core::gpu::driver::Failure::Memory) => {
                "graphics-memory"
            }
            KernelStartupError::Graphics(pentax_core::gpu::driver::Failure::Limit) => {
                "graphics-budget"
            }
            KernelStartupError::Graphics(_) => "graphics-service",
            _ => "runtime-preparation",
        })
    }

    pub fn error_reference_hint(&self) -> Option<(Option<&'static str>, &'static str)> {
        match &self.result {
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(
                KernelStartupError::UnavailableImport {
                    reference_name,
                    reference_match,
                    ..
                },
            )))) => Some((*reference_name, *reference_match)),
            _ => None,
        }
    }

    pub fn error_reference_library(&self) -> Option<&'static str> {
        match &self.result {
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(
                KernelStartupError::UnavailableImport {
                    reference_library, ..
                },
            )))) => *reference_library,
            _ => None,
        }
    }

    pub fn error_public_function(&self) -> Option<&'static str> {
        match &self.result {
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(
                KernelStartupError::UnavailableImport { public_name, .. },
            )))) => *public_name,
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(_)))) => {
                self.progress.failed_kernel_service
            }
            _ => None,
        }
    }

    /// Only public, closed resource categories may enter the diagnostic protocol.
    pub fn error_budget(&self) -> Option<&'static str> {
        match &self.result {
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(
                KernelStartupError::Budget(name),
            )))) => Some(match *name {
                "kernel stop attempts" => "kernel-stops",
                "native request attempts" => "native-service-requests",
                "provider work" => "provider-work",
                "graphics provider work" => "graphics-provider-work",
                "source declarations" => "source-declarations",
                "source declaration bytes" => "source-declaration-bytes",
                "source declaration work" => "source-declaration-work",
                "dynamic module work" => "dynamic-module-work",
                "module input bytes" | "module file size" => "module-input-bytes",
                "module read work" => "module-read-work",
                "module publication work" | "module TLS publication work" => {
                    "module-publication-work"
                }
                "dynamic window" => "dynamic-module-window",
                _ => "runtime-other",
            }),
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Core(
                KernelStartupError::Kernel(pentax_kernel::Failure::Limit(name)),
            )))) => Some(match *name {
                "attempts" => "kernel-attempts",
                "work" => "kernel-work",
                "guest bytes" => "kernel-guest-bytes",
                "scratch" => "kernel-scratch",
                "handles" | "mapping handles" => "kernel-handles",
                "mapped bytes" => "kernel-mapped-bytes",
                "directory bytes" => "kernel-directory-bytes",
                "name bytes" => "kernel-name-bytes",
                "output bytes" => "kernel-output-bytes",
                "output transfer" => "kernel-output-transfer",
                _ => "kernel-other",
            }),
            Err(ProbeError::Admission(NativeImageError::Callback(
                NativeSessionError::Dispatch(NativeDispatchError::Budget(name)),
            ))) => match *name {
                "blocks" => Some("dispatch-blocks"),
                "source bytes" => Some("dispatch-source-bytes"),
                "emitted bytes" => Some("dispatch-emitted-bytes"),
                "work" => Some("dispatch-work"),
                "memory analysis" => Some("dispatch-memory-analysis"),
                "memory checks" => Some("dispatch-memory-checks"),
                "memory bytes" => Some("dispatch-memory-bytes"),
                "native instructions" => Some("dispatch-instructions"),
                "call work" | "nested call work" => Some("dispatch-call-work"),
                "nested calls" => Some("dispatch-nested-calls"),
                "source declarations" => Some("dispatch-source-declarations"),
                _ => Some("dispatch-other"),
            },
            Err(ProbeError::Admission(NativeImageError::Callback(NativeSessionError::Budget(
                name,
            )))) => match *name {
                "source windows" => Some("source-windows"),
                "admission work" => Some("admission-work"),
                "window allocation" | "source window" => Some("source-window"),
                "initializer frames" => Some("initializer-frames"),
                "idle waits" => Some("idle-waits"),
                _ => Some("other"),
            },
            _ => None,
        }
    }

    pub fn error_instruction(&self) -> Option<Mnemonic> {
        match &self.result {
            Err(ProbeError::Admission(NativeImageError::Callback(
                NativeSessionError::UnsupportedExit { mnemonic, .. },
            ))) => *mnemonic,
            _ => None,
        }
    }
}

fn read_bounded(file: &mut File, declared: u64, remaining: u64) -> Result<Vec<u8>, ProbeError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(usize::try_from(declared).map_err(|_| ProbeError::Input("input-size"))?)
        .map_err(|_| ProbeError::Input("input-allocation"))?;
    let mut chunk = [0; 65_536];
    loop {
        let allowed = (remaining - bytes.len() as u64 + 1).min(chunk.len() as u64) as usize;
        let count = file
            .read(&mut chunk[..allowed])
            .map_err(|_| ProbeError::Input("input-read"))?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() as u64 + count as u64 > remaining {
            return Err(ProbeError::Input("input-grew-beyond-limit"));
        }
        bytes
            .try_reserve_exact(count)
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        bytes.extend_from_slice(&chunk[..count]);
    }
}

struct Intake {
    save_directory: Option<File>,
    backing_directory: Option<backing::BackingDirectory>,
    system: pentax_kernel::SystemConfig,
    user: pentax_kernel::UserConfig,
    images: Vec<Vec<u8>>,
    startup_libc: Option<usize>,
    module_binding: Option<String>,
    files: Vec<File>,
    modules: Vec<(String, File)>,
    data_roots: Vec<pentax_kernel::ReadOnlyRoot>,
}
fn intake(args: &[OsString], limits: &NativeImageLimits) -> Result<Intake, ProbeError> {
    if args.is_empty()
        || !args.len().is_multiple_of(2)
        || args.len() / 2
            > limits
                .max_images
                .saturating_add(limits.startup.kernel.max_handles)
                .saturating_add(6)
        || args
            .chunks_exact(2)
            .filter(|pair| {
                pair[0] != "--data-file"
                    && pair[0] != "--config"
                    && pair[0] != "--save-directory"
                    && pair[0] != "--physical-backing-directory"
                    && pair[0] != "--data-directory"
                    && pair[0] != "--memory-profile"
                    && pair[0] != "--execution-profile"
                    && pair[0] != "--present-window"
            })
            .count()
            > limits.max_images
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--data-file" || pair[0] == "--data-directory")
            .count()
            > limits.startup.kernel.max_handles
        || args[0] != "--image"
        || args.chunks_exact(2).any(|pair| {
            pair[0] != "--image"
                && pair[0] != "--config"
                && pair[0] != "--save-directory"
                && pair[0] != "--physical-backing-directory"
                && pair[0] != "--preload-libc"
                && pair[0] != "--preload-libc-as"
                && pair[0] != "--module-file"
                && pair[0] != "--data-file"
                && pair[0] != "--data-directory"
                && pair[0] != "--memory-profile"
                && pair[0] != "--execution-profile"
                && pair[0] != "--present-window"
        })
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--preload-libc" || pair[0] == "--preload-libc-as")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--memory-profile")
            .count()
            > 1
        || args.chunks_exact(2).any(|pair| {
            pair[0] == "--memory-profile" && pair[1] != "compact" && pair[1] != "runtime"
        })
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--execution-profile")
            .count()
            > 1
        || args.chunks_exact(2).any(|pair| {
            pair[0] == "--execution-profile"
                && pair[1] != "probe"
                && pair[1] != "startup"
                && pair[1] != "extended-startup"
                && pair[1] != "long-startup"
        })
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--present-window")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--present-window" && pair[1] != "yes" && pair[1] != "no")
    {
        return Err(ProbeError::Input("image-arguments"));
    }
    if args
        .chunks_exact(2)
        .filter(|pair| pair[0] == "--physical-backing-directory")
        .count()
        > 1
        || (args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--physical-backing-directory")
            && !args
                .chunks_exact(2)
                .any(|pair| pair[0] == "--memory-profile" && pair[1] == "runtime"))
    {
        return Err(ProbeError::Input("backing-requires-runtime-once"));
    }
    if args
        .chunks_exact(2)
        .filter(|pair| pair[0] == "--config")
        .count()
        > 1
    {
        return Err(ProbeError::Input("image-arguments"));
    }
    let mut system = pentax_kernel::SystemConfig::default();
    let mut save_directory = None;
    let mut backing_directory = None;
    let mut user = pentax_kernel::UserConfig::default();
    let mut opened = Vec::new();
    opened
        .try_reserve_exact(args.len() / 2)
        .map_err(|_| ProbeError::Input("input-allocation"))?;
    let mut declared = 0_u64;
    let mut data = data_files::DataFiles::default();
    let mut startup_libc = None;
    let mut module_binding = None;
    let mut modules: Vec<(String, File)> = Vec::new();
    modules
        .try_reserve_exact(args.len() / 2)
        .map_err(|_| ProbeError::Input("input-allocation"))?;
    for pair in args.chunks_exact(2) {
        if pair[0] == "--physical-backing-directory" {
            backing_directory = Some(backing::BackingDirectory::open(args, &pair[1])?);
            continue;
        }
        if pair[0] == "--save-directory" {
            if save_directory.is_some() {
                return Err(ProbeError::Input("duplicate-save-directory"));
            }
            let selected = std::fs::canonicalize(&pair[1])
                .map_err(|_| ProbeError::Input("save-directory-path"))?;
            for binding in args
                .chunks_exact(2)
                .filter(|binding| binding[0] == "--data-directory")
            {
                let (_, host) = binding[1]
                    .to_str()
                    .and_then(|value| value.split_once('='))
                    .ok_or(ProbeError::Input("data-directory-binding"))?;
                let data_path = std::fs::canonicalize(host)
                    .map_err(|_| ProbeError::Input("data-directory-path"))?;
                if selected.starts_with(&data_path) || data_path.starts_with(&selected) {
                    return Err(ProbeError::Input("save-data-directory-overlap"));
                }
            }
            let directory = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .custom_flags(0x0200_0000)
                .open(&selected)
                .map_err(|_| ProbeError::Input("save-directory-open"))?;
            if !directory
                .metadata()
                .map_err(|_| ProbeError::Input("save-directory-metadata"))?
                .is_dir()
            {
                return Err(ProbeError::Input("save-directory-not-directory"));
            }
            save_directory = Some(directory);
            continue;
        }
        if pair[0] == "--config" {
            let file = File::open(&pair[1]).map_err(|_| ProbeError::Input("system-config-open"))?;
            let metadata = file
                .metadata()
                .map_err(|_| ProbeError::Input("system-config-metadata"))?;
            if !metadata.is_file() || metadata.len() > 65_536 {
                return Err(ProbeError::Input("system-config-size"));
            }
            let mut source = String::new();
            file.take(65_537)
                .read_to_string(&mut source)
                .map_err(|_| ProbeError::Input("system-config-read"))?;
            if source.len() > 65_536 {
                return Err(ProbeError::Input("system-config-size"));
            }
            let config = pentax_config::GlobalConfig::from_toml(&source)
                .map_err(|_| ProbeError::Input("system-config-invalid"))?;
            system = config.system;
            user = config.user;
            continue;
        }
        if pair[0] == "--memory-profile"
            || pair[0] == "--execution-profile"
            || pair[0] == "--present-window"
        {
            continue;
        }
        if pair[0] == "--data-file" {
            data.bind(&pair[1], limits)?;
            continue;
        }
        if pair[0] == "--data-directory" {
            data.bind_directory(&pair[1], limits)?;
            continue;
        }
        if pair[0] == "--preload-libc" || pair[0] == "--preload-libc-as" {
            startup_libc = Some(opened.len());
        }
        let selected;
        let mut module_path = None;
        let host_path = if pair[0] == "--preload-libc-as" || pair[0] == "--module-file" {
            let (path, host) = pair[1]
                .to_str()
                .and_then(|value| value.split_once('='))
                .filter(|(path, host)| {
                    !path.is_empty()
                        && !host.is_empty()
                        && path.len() < limits.startup.kernel.max_name_bytes
                })
                .ok_or(ProbeError::Input("module-path-binding"))?;
            let mut owned = String::new();
            owned
                .try_reserve_exact(path.len())
                .map_err(|_| ProbeError::Input("input-allocation"))?;
            owned.push_str(path);
            if modules.iter().any(|(existing, _)| existing == &owned)
                || module_binding
                    .as_ref()
                    .is_some_and(|existing| existing == &owned)
            {
                return Err(ProbeError::Input("duplicate-module-path"));
            }
            if pair[0] == "--preload-libc-as" {
                module_binding = Some(owned);
            } else {
                module_path = Some(owned);
            }
            selected = OsString::from(host);
            &selected
        } else {
            &pair[1]
        };
        // Read access only; preserve these handles and deny replacement/writers
        // during the immutable intake. Existing readers can keep using the file.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(host_path)
            .map_err(|_| ProbeError::Input("input-open"))?;
        let metadata = file
            .metadata()
            .map_err(|_| ProbeError::Input("input-metadata"))?;
        if !metadata.is_file() {
            return Err(ProbeError::Input("input-not-regular-file"));
        }
        declared = declared
            .checked_add(metadata.len())
            .filter(|value| *value <= limits.max_input_bytes)
            .ok_or(ProbeError::Input("input-byte-limit"))?;
        if let Some(path) = module_path {
            if metadata.len() > 128 * 1024 * 1024 {
                return Err(ProbeError::Input("module-file-size"));
            }
            modules.push((path, file));
        } else {
            opened.push((file, metadata.len()));
        }
    }
    let mut images = Vec::new();
    images
        .try_reserve_exact(opened.len())
        .map_err(|_| ProbeError::Input("input-allocation"))?;
    let mut actual = modules
        .iter()
        .try_fold(0_u64, |total, (_, file)| {
            file.metadata()
                .ok()
                .and_then(|metadata| total.checked_add(metadata.len()))
        })
        .ok_or(ProbeError::Input("input-metadata"))?;
    for (file, length) in &mut opened {
        let bytes = read_bounded(file, *length, limits.max_input_bytes - actual)?;
        actual += bytes.len() as u64;
        images.push(bytes);
    }
    let mut files = Vec::new();
    files
        .try_reserve_exact(opened.len())
        .map_err(|_| ProbeError::Input("input-allocation"))?;
    files.extend(opened.into_iter().map(|(file, _)| file));
    Ok(Intake {
        save_directory,
        backing_directory,
        system,
        user,
        images,
        startup_libc,
        module_binding,
        files,
        modules,
        data_roots: data.into_roots()?,
    })
}

pub(super) fn run(args: &[OsString]) -> ProbeOutcome {
    let mut limits = NativeImageLimits::default();
    let mut progress = NativeSessionProgress::default();
    let mut unavailable_functions = None;
    let result = (|| {
        let mut intake = intake(args, &limits)?;
        if args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--memory-profile" && pair[1] == "runtime")
        {
            // Query after immutable intake so its actual storage is already
            // reflected in host availability. No global host settings change.
            let host = pentax_platform::host_memory::available_memory()
                .map_err(|_| ProbeError::Input("host-memory-query"))?;
            limits = match &intake.backing_directory {
                Some(directory) => {
                    limits.with_file_backed_runtime_memory(host, directory.path.clone())
                }
                None => limits.with_runtime_memory(host),
            }
            .map_err(|error| ProbeError::Input(error.code()))?;
        }
        let long_startup = args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--execution-profile" && pair[1] == "long-startup");
        let extended_startup = long_startup
            || args
                .chunks_exact(2)
                .any(|pair| pair[0] == "--execution-profile" && pair[1] == "extended-startup");
        if extended_startup {
            // These cumulative service/registration visits are independent of
            // native backing, handles, I/O and per-call memory permissions.
            limits.startup.kernel.max_attempts = 1_000_000;
            limits.startup.max_provider_work = 1024 * 1024 * 1024;
        }
        if long_startup {
            // The compact 256-object fixture allowance is shared by mappings,
            // pthread prefixes, files, modules and other retained owners. Full
            // startup gets a finite 4096-object metadata allowance; backing,
            // thread counts, I/O sizes and per-access permissions remain separate.
            limits.startup.kernel.max_handles = 4096;
            limits.startup.kernel.max_attempts = 4_000_000;
            limits.startup.kernel.max_work *= 4;
            limits.startup.kernel.max_bytes *= 4;
            limits.startup.max_provider_work *= 4;
            // Actual shader initialization exceeds the compact graphics-call
            // allowance. Only this explicit supervised profile gets a larger
            // finite call count; memory, work, shader slots and GPU caps stay.
            limits.graphics.max_calls = 4096;
        }
        limits.startup_libc = intake.startup_libc;
        limits.startup.system = intake.system;
        limits.startup.user = intake.user;
        if !intake.modules.is_empty() {
            limits.dynamic_window_bytes = 64 * 1024 * 1024;
        }
        let mut borrowed = Vec::new();
        borrowed
            .try_reserve_exact(intake.images.len())
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        borrowed.extend(intake.images.iter().map(Vec::as_slice));
        with_native_image_session_with_roots(
            &borrowed,
            &limits,
            intake.data_roots,
            |session, layout| {
                unavailable_functions = Some(layout.unavailable_functions);
                if let Some(directory) = intake.save_directory.take() {
                    session.configure_save_storage(directory)?;
                }
                if let (Some(module), Some(path)) =
                    (intake.startup_libc, intake.module_binding.as_deref())
                {
                    let file = intake.files[module].try_clone().map_err(|_| {
                        NativeSessionError::Core(KernelStartupError::InvalidProvider(
                            "module file handle",
                        ))
                    })?;
                    session.bind_loaded_module_file(module, path, file)?;
                }
                for (path, file) in &intake.modules {
                    let file = file.try_clone().map_err(|_| {
                        NativeSessionError::Core(KernelStartupError::InvalidProvider(
                            "module file handle",
                        ))
                    })?;
                    session.register_dynamic_module_file(path, file)?;
                }
                crate::protocol("RUNNING\tmode=entry-probe")
                    .map_err(|_| NativeSessionError::OutputUnavailable)?;
                let mut execution = NativeSessionLimits::default();
                if args
                    .chunks_exact(2)
                    .any(|pair| pair[0] == "--execution-profile" && pair[1] == "startup")
                {
                    execution.dispatch.max_blocks = 100_000;
                }
                if extended_startup {
                    execution.dispatch.max_blocks = 64_000_000;
                    execution.dispatch.max_source_bytes = 4 * 1024 * 1024 * 1024;
                    execution.dispatch.max_emitted_bytes = 4 * 1024 * 1024 * 1024;
                    execution.dispatch.max_work_units = 256 * 1024 * 1024 * 1024;
                    execution.max_total_window_bytes = 4 * 1024 * 1024 * 1024;
                    execution.max_admission_work = 16 * 1024 * 1024 * 1024;
                    execution.progress_interval_blocks = 2_097_152;
                }
                if long_startup {
                    execution.dispatch.max_blocks *= 4;
                    execution.dispatch.max_source_bytes *= 4;
                    execution.dispatch.max_emitted_bytes *= 4;
                    execution.dispatch.max_work_units *= 4;
                    execution.max_total_window_bytes *= 4;
                    execution.max_admission_work *= 4;
                    execution.progress_interval_blocks *= 4;
                }
                execution.report_progress = true;
                // Finite native slots retain exact attested code between steps.
                // Each 512-byte slot includes its own native trailer cell.
                execution.dispatch.cache_bytes = 2 * 1024 * 1024;
                execution.dispatch.cache_slots = 4096;
                // Search both sides of code within a finite 512 MiB radius.
                // Large data windows remain reserved; the cache can sit below.
                execution.dispatch.cache_search_granules = 16_384;
                let proof = session.prepare_native_source(&execution.block)?;
                let entry = layout.entry;
                let mut initial = NativeCpuState::new(entry.entry, entry.return_slot);
                initial.gpr[5] = entry.frame_pointer;
                initial.gpr[7] = entry.arguments;
                initial.gpr[6] = entry.exit_handler;
                let run = if args
                    .chunks_exact(2)
                    .any(|pair| pair[0] == "--present-window" && pair[1] == "yes")
                {
                    run_prepared_with_window
                } else {
                    run_prepared_with_initializer
                };
                let result = run(
                    session,
                    &proof,
                    initial,
                    NativeStackFrame {
                        stack: entry.stack,
                        guards: Some(entry.guards),
                        return_slot: entry.return_slot,
                        return_marker: entry.return_marker,
                    },
                    layout.initializer.as_ref(),
                    &execution,
                    &mut progress,
                );
                // Observe once after execution, including a typed refusal.
                // Scanning owner tables is outside the per-block hot path.
                progress.kernel_handles = Some(session.kernel_handle_usage());
                progress.audio_out2 = Some(session.audio_out2_usage());
                result
            },
        )
        .map_err(ProbeError::Admission)
    })();
    ProbeOutcome {
        result,
        progress,
        unavailable_functions,
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    #[test]
    fn budget_protocol_exposes_only_closed_categories() {
        for (error, expected) in [
            (
                NativeSessionError::Budget("source windows"),
                Some("source-windows"),
            ),
            (
                NativeSessionError::Budget("unlisted diagnostic"),
                Some("other"),
            ),
            (
                NativeSessionError::Dispatch(NativeDispatchError::Budget("blocks")),
                Some("dispatch-blocks"),
            ),
            (
                NativeSessionError::Dispatch(NativeDispatchError::Budget("work")),
                Some("dispatch-work"),
            ),
            (
                NativeSessionError::Dispatch(NativeDispatchError::Budget("unlisted diagnostic")),
                Some("dispatch-other"),
            ),
            (NativeSessionError::SourceWindow, None),
            (
                NativeSessionError::Core(KernelStartupError::Budget("kernel stop attempts")),
                Some("kernel-stops"),
            ),
            (
                NativeSessionError::Core(KernelStartupError::Budget("provider work")),
                Some("provider-work"),
            ),
            (
                NativeSessionError::Core(KernelStartupError::Budget("unlisted diagnostic")),
                Some("runtime-other"),
            ),
            (
                NativeSessionError::Core(KernelStartupError::Kernel(
                    pentax_kernel::Failure::Limit("attempts"),
                )),
                Some("kernel-attempts"),
            ),
            (
                NativeSessionError::Core(KernelStartupError::Kernel(
                    pentax_kernel::Failure::Limit("unlisted diagnostic"),
                )),
                Some("kernel-other"),
            ),
            (
                NativeSessionError::Core(KernelStartupError::Kernel(
                    pentax_kernel::Failure::Unsupported("unlisted diagnostic"),
                )),
                None,
            ),
        ] {
            let outcome = ProbeOutcome {
                result: Err(ProbeError::Admission(NativeImageError::Callback(error))),
                progress: NativeSessionProgress::default(),
                unavailable_functions: None,
            };
            assert_eq!(outcome.error_budget(), expected);
        }
    }
}
