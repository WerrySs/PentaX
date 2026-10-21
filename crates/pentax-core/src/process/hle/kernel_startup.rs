// SPDX-License-Identifier: GPL-2.0-only

//! Exact generated kernel + libc providers with opt-in typed TLS preparation.
//! Service access is source-bound to this retained plan and its actual native
//! owner. Neither arbitrary native execution nor a console startup ABI follows.

use super::kernel_adapter::KernelMemory;
use super::kernel_provider::{
    CODE_MAPPED_BYTES, GeneratedKernelProvider, KernelMarker, KernelProviderError, MAX_IMAGE_BYTES,
    METADATA_RVA, PROGRAM_NAME_BUFFER_RVA, PROGRAM_NAME_OBJECT_RVA,
};
use super::{GeneratedProviders, HleError, HleStartupLimits, HleStartupPlan, ProviderPlacement};
use crate::memory::native::{NativeGuestMemory, NativeLoadLimits};
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE};
use crate::process::tls::StaticTlsStorage;
use pentax_kernel::{CallArgs, Failure, Kernel, Limits, ProcessIdentity, ReadOnlyRoot, Usage};
use pentax_linker::{ModuleInput, Resolution, SymbolicLinkPlan};
use pentax_loader::open_executable;
use pentax_platform::native_memory::{NativeAddressSpace, PausedNativeMemory};
use std::fmt;

mod libc_builtin;
mod native_execution;
pub use libc_builtin::{LibcBuiltinError, LibcBuiltinUsage, PreparedLibcBuiltins};
mod pthread_once;
pub use pthread_once::PreparedPthreadOnce;
mod netctl_callback;
pub use netctl_callback::PreparedNetCtlCallback;
mod font_callback;
pub use font_callback::PreparedFontParserCallback;
mod ime_callback;
pub use ime_callback::PreparedImeCallback;
mod graphics_runtime;
mod json_runtime;
mod json_strings;
mod json_values;
mod public_names;
mod unavailable_runtime;
pub use graphics_runtime::GraphicsStartupOptions;
pub use json_runtime::JsonStartupOptions;
pub use native_execution::{NativeServiceRequest, PreparedNativeSource};
use unavailable_runtime::UnavailableBinding;
pub use unavailable_runtime::UnavailableProviderPlacement;
mod native_thread;
pub use native_thread::PreparedNativeThread;
mod pthread_create;
pub use pthread_create::{PendingThreadCreate, PreparedThreadStart, ThreadEntryLayout};
mod pthread_wait;
pub use pthread_wait::PendingThreadWait;
mod native_entry;
pub use native_entry::{
    NativeEntryError, NativeEntryLayout, NativeEntryLimits, NativeEntryPolicy, NativeEntryRequest,
    PreparedNativeEntry,
};
mod native_module;
pub use native_module::{NativeModuleStartError, PreparedNativeModuleStart};
pub(super) mod module_lifecycle;
pub use module_lifecycle::{
    DynamicModuleInput, DynamicModuleStart, NativeServiceAction, PendingModuleLoad,
};
mod original_fixture;
pub use original_fixture::{
    OriginalPreparedError, OriginalPreparedLayout, OriginalPreparedReport,
    run_original_prepared_fixture,
};

/// Independent retained bounds for exact linking/TLS, provider validation and
/// stateful kernel services. Validation work is bounded separately in each
/// planning/preparation phase and cumulatively across service stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KernelStartupLimits {
    pub startup: HleStartupLimits,
    pub kernel: Limits,
    pub system: pentax_kernel::SystemConfig,
    pub user: pentax_kernel::UserConfig,
    pub max_kernel_provider_bytes: u64,
    pub max_provider_work: u64,
    /// Optional work bound for one JSON body or qualified module lookup,
    /// independent of the cumulative provider ledger. None retains the
    /// diagnostic profile's original shared allowance.
    pub max_provider_call_work: Option<u64>,
    /// Optional retained published-stop metadata capacity, in bytes. None
    /// preserves the original bound inherited from max_provider_work.
    pub max_provider_metadata_bytes: Option<u64>,
    /// Optional independent cumulative native-request ledger. None retains
    /// the original checked sum of kernel and required-libc attempt limits.
    pub max_native_service_requests: Option<u64>,
}

impl Default for KernelStartupLimits {
    fn default() -> Self {
        Self {
            startup: HleStartupLimits::default(),
            kernel: Limits::default(),
            system: pentax_kernel::SystemConfig::default(),
            user: pentax_kernel::UserConfig::default(),
            max_kernel_provider_bytes: MAX_IMAGE_BYTES,
            max_provider_work: 16 * 1024 * 1024,
            max_provider_call_work: None,
            max_provider_metadata_bytes: None,
            max_native_service_requests: None,
        }
    }
}

impl KernelStartupLimits {
    fn provider_call_maximum(&self, used: u64) -> Result<u64, KernelStartupError> {
        match self.max_provider_call_work {
            Some(work) => used
                .checked_add(work)
                .map(|maximum| maximum.min(self.max_provider_work))
                .ok_or(KernelStartupError::Budget("provider call work")),
            None => Ok(self.max_provider_work),
        }
    }

    fn provider_call_remaining(&self, used: u64) -> Result<u64, KernelStartupError> {
        Ok(self.provider_call_maximum(used)?.saturating_sub(used))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KernelStartupError {
    Hle(HleError),
    Provider(KernelProviderError),
    Kernel(Failure),
    Codegen(pentax_codegen::CodegenError),
    Mapping(crate::process::ProcessError),
    Allocation,
    Budget(&'static str),
    InvalidProvider(&'static str),
    SiteMismatch {
        syscall_pc: u64,
    },
    ServiceIdMismatch {
        expected: u32,
        actual: u64,
    },
    NativeOwnerMismatch,
    ContinuationMismatch {
        syscall_pc: u64,
        next_pc: u64,
    },
    UnavailableImport {
        ordinal: u32,
        public_name: Option<&'static str>,
        reference_name: Option<&'static str>,
        reference_match: &'static str,
        reference_library: Option<&'static str>,
    },
    UnavailableExitCallback,
    UnavailableJsonAllocatorVirtual,
    UnavailableJsonValueStorage,
    UnavailableVectorArguments,
    InvalidJsonString,
    UnavailableJsonFormat,
    Graphics(crate::gpu::driver::Failure),
}

impl fmt::Display for KernelStartupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graphics(error) => write!(f, "graphics submit: {error:?}"),
            Self::Hle(e) => write!(f, "kernel startup preparation: {e}"),
            Self::Provider(e) => write!(f, "kernel provider: {e}"),
            Self::Kernel(e) => write!(f, "kernel service: {e:?}"),
            Self::Codegen(e) => write!(f, "prepared native source: {e}"),
            Self::Mapping(e) => write!(f, "native module mapping: {e}"),
            Self::Allocation => f.write_str("kernel startup allocation failed"),
            Self::Budget(kind) => write!(f, "kernel startup {kind} budget exceeded"),
            Self::InvalidProvider(reason) => write!(f, "kernel provider: {reason}"),
            Self::SiteMismatch { syscall_pc } => write!(
                f,
                "kernel stop {syscall_pc:#x} is not an owned provider site"
            ),
            Self::ServiceIdMismatch { expected, actual } => write!(
                f,
                "kernel service ID {actual:#x} differs from expected {expected:#x}"
            ),
            Self::NativeOwnerMismatch => {
                f.write_str("native request belongs to another backing owner")
            }
            Self::UnavailableImport { ordinal, .. } => {
                write!(f, "unavailable imported function {ordinal}")
            }
            Self::UnavailableExitCallback => f.write_str("module exit callback is unavailable"),
            Self::UnavailableJsonAllocatorVirtual => {
                f.write_str("JSON allocator virtual method is unavailable")
            }
            Self::UnavailableJsonValueStorage => {
                f.write_str("owned JSON value storage is unavailable")
            }
            Self::UnavailableVectorArguments => {
                f.write_str("captured vector arguments are unavailable")
            }
            Self::InvalidJsonString => f.write_str("JSON string storage is invalid"),
            Self::UnavailableJsonFormat => f.write_str("JSON formatting is unavailable"),
            Self::ContinuationMismatch {
                syscall_pc,
                next_pc,
            } => write!(
                f,
                "native continuation {next_pc:#x} is not immediately after SYSCALL at {syscall_pc:#x}"
            ),
        }
    }
}
impl std::error::Error for KernelStartupError {}

fn require(condition: bool, reason: &'static str) -> Result<(), KernelStartupError> {
    if condition {
        Ok(())
    } else {
        Err(KernelStartupError::InvalidProvider(reason))
    }
}

fn charge(used: &mut u64, count: u64, maximum: u64) -> Result<(), KernelStartupError> {
    let next = used
        .checked_add(count)
        .filter(|v| *v <= maximum)
        .ok_or(KernelStartupError::Budget("provider work"))?;
    *used = next;
    Ok(())
}

/// Input zero stays primary, followed by supplied images, then this factory's
/// libkernel image, then the existing libc factory's image. The exact linker
/// remains the sole resolver: no priority, alias or global NID fallback exists.
#[derive(Debug)]
pub struct KernelStartupPlan<'image> {
    hle: HleStartupPlan<'image>,
    factory: &'image GeneratedKernelProvider,
    provider: usize,
    bias: u64,
    bindings: usize,
    limits: KernelStartupLimits,
    unavailable: Option<UnavailableBinding<'image>>,
    graphics: Option<graphics_runtime::GraphicsBinding<'image>>,
    json: Option<json_runtime::JsonBinding<'image>>,
}

impl<'image> KernelStartupPlan<'image> {
    pub fn unavailable_functions(&self) -> usize {
        self.unavailable
            .as_ref()
            .map_or(0, |binding| binding.factory.function_count())
    }
    pub fn unavailable_exit_callback(&self) -> Option<u64> {
        self.unavailable
            .as_ref()
            .map(|binding| binding.bias + binding.factory.exit_callback_rva())
    }
    pub const fn link_plan(&self) -> &SymbolicLinkPlan<'image> {
        self.hle.link_plan()
    }
    pub const fn hle_startup_plan(&self) -> &HleStartupPlan<'image> {
        &self.hle
    }
    pub const fn limits(&self) -> KernelStartupLimits {
        self.limits
    }
    pub const fn kernel_provider_index(&self) -> usize {
        self.provider
    }
    pub const fn kernel_provider_bias(&self) -> u64 {
        self.bias
    }
    pub const fn kernel_bindings(&self) -> usize {
        self.bindings
    }
    pub fn registration_count(&self) -> usize {
        self.factory.registration_count()
    }

    fn entry(&self, marker: KernelMarker) -> u64 {
        self.bias + marker.rva
    }
}

/// Derive authority only from both factories and the exact complete image plan.
/// The kernel image is borrowed immutably, and every binding is validated against
/// its private generated manifest. All deferred records still pass through the
/// existing typed TLS binder; unknown records and zero-tail writes fail closed.
pub fn plan_kernel_startup<'image>(
    inputs: &[ModuleInput<'image>],
    kernel: &'image GeneratedKernelProvider,
    kernel_placement: ProviderPlacement,
    libc: &'image GeneratedProviders,
    libc_placement: ProviderPlacement,
    limits: &KernelStartupLimits,
) -> Result<KernelStartupPlan<'image>, KernelStartupError> {
    plan_kernel_startup_inner(
        inputs,
        kernel,
        kernel_placement,
        libc,
        libc_placement,
        KernelStartupExtensions::default(),
        limits,
    )
}

/// Explicit stop-at-use provider. Exact classification is recomputed against
/// supplied images and real factories before adding unavailable exports;
/// version conflicts, ambiguity, data/TLS/weak imports cannot be masked.
pub fn plan_kernel_startup_with_unavailable<'image>(
    inputs: &[ModuleInput<'image>],
    kernel: &'image GeneratedKernelProvider,
    kernel_placement: ProviderPlacement,
    libc: &'image GeneratedProviders,
    libc_placement: ProviderPlacement,
    unavailable: UnavailableProviderPlacement<'image>,
    limits: &KernelStartupLimits,
) -> Result<KernelStartupPlan<'image>, KernelStartupError> {
    plan_kernel_startup_inner(
        inputs,
        kernel,
        kernel_placement,
        libc,
        libc_placement,
        KernelStartupExtensions {
            unavailable: Some(unavailable),
            ..KernelStartupExtensions::default()
        },
        limits,
    )
}

/// Add the fixed, separately scoped Graphics5Driver provider before classifying
/// unavailable imports. Supplied providers still use ordinary exact resolution.
pub fn plan_kernel_startup_with_graphics<'image>(
    inputs: &[ModuleInput<'image>],
    kernel: &'image GeneratedKernelProvider,
    kernel_placement: ProviderPlacement,
    libc: &'image GeneratedProviders,
    libc_placement: ProviderPlacement,
    graphics: GraphicsStartupOptions<'image>,
    limits: &KernelStartupLimits,
) -> Result<KernelStartupPlan<'image>, KernelStartupError> {
    plan_kernel_startup_inner(
        inputs,
        kernel,
        kernel_placement,
        libc,
        libc_placement,
        KernelStartupExtensions {
            graphics: Some(graphics),
            unavailable: graphics.unavailable,
            json: None,
        },
        limits,
    )
}

/// Optional separately scoped, factory-bound providers admitted before missing imports.
#[derive(Clone, Copy, Debug, Default)]
pub struct KernelStartupExtensions<'image> {
    pub graphics: Option<GraphicsStartupOptions<'image>>,
    pub unavailable: Option<UnavailableProviderPlacement<'image>>,
    pub json: Option<JsonStartupOptions<'image>>,
}

pub fn plan_kernel_startup_with_extensions<'image>(
    inputs: &[ModuleInput<'image>],
    kernel: &'image GeneratedKernelProvider,
    kernel_placement: ProviderPlacement,
    libc: &'image GeneratedProviders,
    libc_placement: ProviderPlacement,
    extensions: KernelStartupExtensions<'image>,
    limits: &KernelStartupLimits,
) -> Result<KernelStartupPlan<'image>, KernelStartupError> {
    plan_kernel_startup_inner(
        inputs,
        kernel,
        kernel_placement,
        libc,
        libc_placement,
        extensions,
        limits,
    )
}

fn plan_kernel_startup_inner<'image>(
    inputs: &[ModuleInput<'image>],
    kernel: &'image GeneratedKernelProvider,
    kernel_placement: ProviderPlacement,
    libc: &'image GeneratedProviders,
    libc_placement: ProviderPlacement,
    extensions: KernelStartupExtensions<'image>,
    limits: &KernelStartupLimits,
) -> Result<KernelStartupPlan<'image>, KernelStartupError> {
    let KernelStartupExtensions {
        graphics,
        unavailable,
        json,
    } = extensions;
    if inputs.is_empty() {
        return Err(KernelStartupError::Hle(HleError::NoPrimaryInput));
    }
    let count = inputs
        .len()
        .checked_add(
            2 + usize::from(unavailable.is_some())
                + usize::from(graphics.is_some())
                + usize::from(json.is_some()),
        )
        .ok_or(KernelStartupError::Budget("modules"))?;
    if count as u64 > limits.startup.hle.linking.max_modules {
        return Err(KernelStartupError::Budget("modules"));
    }
    if kernel.image().len() as u64 > limits.max_kernel_provider_bytes {
        return Err(KernelStartupError::Budget("kernel provider bytes"));
    }
    let bias = kernel_placement.load_bias;
    if !bias.is_multiple_of(PAGE_SIZE)
        || bias
            .checked_add(kernel.mapped_bytes())
            .is_none_or(|v| v > ADDRESS_LIMIT)
    {
        return Err(KernelStartupError::Hle(HleError::InvalidPlacement {
            load_bias: bias,
        }));
    }
    let mut work = 0;
    charge(&mut work, count as u64, limits.max_provider_work)?;
    let mut supplied = Vec::new();
    supplied
        .try_reserve_exact(count - 1)
        .map_err(|_| KernelStartupError::Allocation)?;
    supplied.extend_from_slice(inputs);
    let view =
        open_executable(kernel.image()).map_err(|e| KernelStartupError::Hle(HleError::Load(e)))?;
    supplied.push(ModuleInput::new(view, bias));
    let graphics = graphics
        .map(|options| graphics_runtime::append(&mut supplied, options))
        .transpose()?;
    let json = json
        .map(|options| json_runtime::append(&mut supplied, options, limits))
        .transpose()?;
    let unavailable = match unavailable {
        Some(placement) => Some(unavailable_runtime::append_classified(
            &mut supplied,
            libc,
            libc_placement,
            placement,
            limits,
            &mut work,
        )?),
        None => None,
    };
    let hle = super::plan_hle_startup(&supplied, libc, libc_placement, &limits.startup)
        .map_err(KernelStartupError::Hle)?;
    if let Some(binding) = &unavailable {
        unavailable_runtime::validate_source_and_bindings(
            hle.link_plan(),
            binding,
            limits,
            &mut work,
        )?;
    }
    if let Some(binding) = &graphics {
        graphics_runtime::validate(hle.link_plan(), binding, limits, &mut work)?;
    }
    if let Some(binding) = &json {
        json_runtime::validate(hle.link_plan(), binding, limits, &mut work)?;
    }
    let provider = inputs.len();
    let module = &hle.link_plan().modules()[provider];
    let runtime = module.runtime_plan();
    require(
        module.load_bias() == bias
            && runtime.segments.len() == 3
            && runtime.tls.is_none()
            && runtime.relro.is_empty(),
        "kernel module mapping",
    )?;
    let segment = &runtime.segments[0];
    require(
        std::ptr::eq(segment.file_bytes, kernel.code_bytes())
            && segment.vaddr == 0
            && segment.mem_size == CODE_MAPPED_BYTES
            && segment.flags.raw() == 5,
        "kernel factory source identity",
    )?;
    let data = &runtime.segments[1];
    require(
        std::ptr::eq(data.file_bytes, kernel.data_bytes())
            && data.vaddr == CODE_MAPPED_BYTES
            && data.mem_size == PAGE_SIZE
            && data.flags.raw() == 6,
        "kernel factory data identity",
    )?;
    let metadata = &runtime.segments[2];
    require(
        std::ptr::eq(metadata.file_bytes, kernel.metadata_bytes())
            && metadata.vaddr == METADATA_RVA
            && metadata.mem_size == kernel.metadata_mapped_bytes()
            && metadata.flags.raw() == 4,
        "kernel factory metadata identity",
    )?;
    let markers = kernel.markers();
    charge(
        &mut work,
        hle.link_plan().patch_count() as u64,
        limits.max_provider_work,
    )?;
    let mut bindings = 0;
    for patch in hle.link_plan().patches() {
        if patch.consumer().index() == provider {
            let record = patch.source().record;
            require(
                patch.resolution() == Resolution::Relative
                    && record.kind() == 8
                    && record.symbol_index() == 0
                    && record.offset == PROGRAM_NAME_OBJECT_RVA
                    && record.addend == PROGRAM_NAME_BUFFER_RVA as i64
                    && patch.address() == bias + PROGRAM_NAME_OBJECT_RVA
                    && patch.value() == bias + PROGRAM_NAME_BUFFER_RVA,
                "kernel provider relocation",
            )?;
            continue;
        }
        let Resolution::Definition(symbol) = patch.resolution() else {
            continue;
        };
        if symbol.module.index() != provider {
            continue;
        }
        // Only bindings to this provider scan its manifest. Relative writes and
        // other providers still pay for their outer visit, but no unused scan.
        charge(
            &mut work,
            (markers.len() + kernel.objects().len()) as u64,
            limits.max_provider_work,
        )?;
        let record = patch.source().record;
        let addend = if record.kind() == 1 { record.addend } else { 0 };
        require(
            markers.iter().any(|marker| {
                marker.symbol == symbol.symbol
                    && (bias + marker.rva).wrapping_add_signed(addend) == patch.value()
            }) || kernel.objects().iter().any(|object| {
                object.symbol == symbol.symbol
                    && (bias + object.rva).wrapping_add_signed(addend) == patch.value()
            }),
            "kernel binding outside manifest",
        )?;
        bindings += 1;
    }
    Ok(KernelStartupPlan {
        hle,
        factory: kernel,
        provider,
        bias,
        bindings,
        limits: *limits,
        unavailable,
        graphics,
        json,
    })
}

fn attest_marker(
    memory: &PausedNativeMemory<'_>,
    plan: &KernelStartupPlan<'_>,
    marker: KernelMarker,
) -> Result<(), KernelStartupError> {
    let expected = marker.bytes();
    let offset = marker.rva as usize;
    require(
        plan.factory.image().get(offset..offset + expected.len()) == Some(&expected),
        "kernel source marker",
    )?;
    let mut actual = [0; 8];
    memory
        .fetch_source(plan.entry(marker), &mut actual)
        .map_err(|_| KernelStartupError::InvalidProvider("kernel marker fetch permission"))?;
    require(actual == expected, "kernel live marker bytes")?;
    Ok(())
}

fn attest(
    memory: &PausedNativeMemory<'_>,
    plan: &KernelStartupPlan<'_>,
) -> Result<(), KernelStartupError> {
    let mut work = 0;
    charge(
        &mut work,
        plan.registration_count() as u64 * 8
            + plan.factory.metadata_bytes().len() as u64
            + 3 * (CODE_MAPPED_BYTES / PAGE_SIZE + 1)
            + 4 * (plan.factory.metadata_mapped_bytes() / PAGE_SIZE),
        plan.limits.max_provider_work,
    )?;
    for marker in plan.factory.markers() {
        attest_marker(memory, plan, *marker)?;
    }
    // Loader metadata is an immutable factory product, never executable code.
    // Check its initialized bytes once on the actual newly prepared owner.
    let mut actual = [0; 128];
    for (index, expected) in plan
        .factory
        .metadata_bytes()
        .chunks(actual.len())
        .enumerate()
    {
        memory
            .read(
                plan.bias + METADATA_RVA + (index * actual.len()) as u64,
                &mut actual[..expected.len()],
            )
            .map_err(|_| KernelStartupError::InvalidProvider("kernel metadata read permission"))?;
        require(
            &actual[..expected.len()] == expected,
            "kernel live metadata bytes",
        )?;
    }
    attest_metadata_permissions(memory, plan)?;
    for page in (plan.bias..plan.bias + CODE_MAPPED_BYTES).step_by(PAGE_SIZE as usize) {
        require(
            memory.read(page, &mut [0]).is_ok()
                && memory.fetch_source(page, &mut [0]).is_ok()
                && matches!(
                    memory.preflight_write(page, 1),
                    Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
                ),
            "kernel provider must remain logical RX and physically NX",
        )?;
    }
    let data = plan.bias + CODE_MAPPED_BYTES;
    require(
        memory.read(data, &mut [0]).is_ok()
            && memory.preflight_write(data, PAGE_SIZE).is_ok()
            && matches!(
                memory.fetch_source(data, &mut [0]),
                Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
            ),
        "kernel object data must remain RW and NX",
    )?;
    if let Some(binding) = &plan.unavailable {
        unavailable_runtime::attest_all(memory, binding, &mut work, plan.limits.max_provider_work)?;
    }
    if let Some(binding) = &plan.graphics {
        graphics_runtime::attest_all(memory, binding, &mut work, plan.limits.max_provider_work)?;
    }
    if let Some(binding) = &plan.json {
        json_runtime::attest_all(memory, binding, &mut work, plan.limits.max_provider_work)?;
    }
    Ok(())
}

fn attest_metadata_permissions(
    memory: &PausedNativeMemory<'_>,
    plan: &KernelStartupPlan<'_>,
) -> Result<(), KernelStartupError> {
    for offset in (0..plan.factory.metadata_mapped_bytes()).step_by(PAGE_SIZE as usize) {
        let page = plan.bias + METADATA_RVA + offset;
        require(
            memory.preflight_read(page, PAGE_SIZE).is_ok()
                && memory.preflight_non_executable(page, PAGE_SIZE).is_ok()
                && matches!(
                    memory.preflight_write(page, PAGE_SIZE),
                    Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
                )
                && matches!(
                    memory.preflight_gpu_write(page, PAGE_SIZE),
                    Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
                ),
            "kernel metadata must remain immutable and NX",
        )?;
    }
    Ok(())
}

/// A captured PRE-SYSCALL library-call stop. Original RCX is argument four in
/// `args`; it is never reconstructed from a post-SYSCALL continuation or R10.
/// Only a reviewed internal dispatcher may construct this message.
#[derive(Clone, Copy)]
pub(crate) struct KernelServiceStop {
    pub syscall_pc: u64,
    pub service_id: u64,
    pub args: CallArgs,
}

/// Actual typed-TLS/HLE backing plus one stateful kernel owner. Both source
/// plans and factory bytes remain borrowed for this owner's complete lifetime.
/// Public operations expose diagnostics, exclusive checked pauses and attested
/// PRE-SYSCALL service requests; no executor or mutable raw view is returned.
pub struct PreparedNativeKernelStartup<'plan, 'image> {
    plan: &'plan KernelStartupPlan<'image>,
    hle: super::PreparedNativeHleStartup<'plan, 'image>,
    kernel: Kernel,
    stop_attempts: u64,
    provider_work: u64,
    native_service_requests: u64,
    libc_builtins: LibcBuiltinUsage,
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    libc_builtin_budget: super::dispatch::ServiceBudget,
    last_admitted_kernel_service: Option<&'static str>,
    loaded_modules: module_lifecycle::LoadedModules,
    once: pthread_once::OnceState,
    netctl_callbacks: netctl_callback::NetCtlCallbackState,
    font_parsers: font_callback::FontParserCallbackState,
    ime_callbacks: ime_callback::ImeCallbackState,
    json: json_strings::JsonStrings,
    graphics: Option<crate::gpu::driver::Driver>,
    initial_entry: Option<NativeEntryLayout>,
    active_tls: usize,
    thread_runtime: pthread_create::ThreadRuntime,
}

impl fmt::Debug for PreparedNativeKernelStartup<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedNativeKernelStartup")
            .field("plan", &self.plan)
            .field("kernel_usage", &self.kernel.usage())
            .field("stop_attempts", &self.stop_attempts)
            .finish_non_exhaustive()
    }
}

impl<'plan, 'image> PreparedNativeKernelStartup<'plan, 'image> {
    pub const fn plan(&self) -> &'plan KernelStartupPlan<'image> {
        self.plan
    }
    pub fn memory(&self) -> &NativeGuestMemory<'plan, 'image> {
        self.hle.memory()
    }
    pub fn tls(&self) -> &crate::process::tls::PreparedNativeTlsSession<'plan, 'image> {
        self.hle.tls()
    }
    pub fn kernel_usage(&self) -> Usage {
        self.kernel.usage()
    }

    /// Closed owner-table counts; no guest identities or memory contents.
    pub fn kernel_handle_usage(&self) -> pentax_kernel::HandleUsage {
        self.kernel.handle_usage()
    }

    /// Committed headless CPU mixing and logical batches; no playback claim.
    pub fn audio_out2_usage(&self) -> pentax_kernel::AudioOut2Usage {
        self.kernel.audio_out2_usage()
    }

    pub fn memory_map_refusal(&self) -> Option<pentax_kernel::MemoryMapRefusal> {
        self.kernel.memory_map_refusal()
    }

    pub fn output_usage(&self) -> pentax_kernel::OutputUsage {
        self.kernel.output_usage()
    }

    /// Borrow private output for a bounded application-owned classifier.
    pub fn output_tail(&self, stream: pentax_kernel::OutputStream, maximum: usize) -> &[u8] {
        self.kernel.output_tail(stream, maximum)
    }

    /// Retrieve private guest output for an application-owned consumer.
    pub fn take_output(&mut self, stream: pentax_kernel::OutputStream) -> Vec<u8> {
        self.kernel.take_output(stream)
    }
    /// Process-local input-order runtime identity, distinct from SCE selectors.
    /// Generated HLE/unavailable images are not externally loaded module handles.
    pub fn supplied_module_handle(&self, index: usize) -> Option<i32> {
        if index < self.plan.kernel_provider_index() {
            i32::try_from(index + 1).ok()
        } else {
            None
        }
    }
    pub const fn kernel_stop_attempts(&self) -> u64 {
        self.stop_attempts
    }
    pub fn libc_service_attempts(&self) -> u64 {
        self.hle.service_attempts()
    }

    /// Public catalogue label only, set after live marker admission for the most
    /// recent native request. No guest identifier or argument is retained.
    pub const fn last_admitted_kernel_service(&self) -> Option<&'static str> {
        self.last_admitted_kernel_service
    }

    #[allow(dead_code)]
    pub(crate) fn libc_stop(
        &mut self,
        stop: super::native::NativeServiceStop,
    ) -> Result<u64, HleError> {
        self.hle.service_stop(stop)
    }

    #[allow(dead_code)]
    pub(crate) fn kernel_stop(
        &mut self,
        stop: KernelServiceStop,
    ) -> Result<u64, KernelStartupError> {
        let source = self.plan.link_plan();
        if source.identity() != self.memory().active_source_identity() {
            return Err(KernelStartupError::InvalidProvider(
                "stale initial service source",
            ));
        }
        match self.kernel_stop_for(source, stop, false, None)? {
            NativeServiceAction::Returned(value) => Ok(value),
            NativeServiceAction::Load(_)
            | NativeServiceAction::Once(_)
            | NativeServiceAction::NetCtl(_)
            | NativeServiceAction::FontParser(_)
            | NativeServiceAction::ImeCallback(_)
            | NativeServiceAction::CreateThread(_)
            | NativeServiceAction::WaitThread(_) => {
                Err(KernelStartupError::InvalidProvider("pending module load"))
            }
        }
    }

    /// Bind explicitly supplied local addon metadata before the first native
    /// service request.
    /// This capability carries no account, purchase or PSN authorization claim.
    pub fn configure_local_entitlements(
        &mut self,
        metadata: pentax_kernel::LocalEntitlements,
    ) -> Result<(), KernelStartupError> {
        if self.native_service_requests != 0 {
            return Err(KernelStartupError::InvalidProvider(
                "late local entitlement binding",
            ));
        }
        self.kernel
            .configure_local_entitlements(metadata)
            .map_err(KernelStartupError::Kernel)
    }

    /// Bind the application's explicitly selected per-title save directory before execution.
    pub fn configure_save_storage(
        &mut self,
        directory: std::fs::File,
    ) -> Result<(), KernelStartupError> {
        if self.native_service_requests != 0 {
            return Err(KernelStartupError::InvalidProvider(
                "late save storage binding",
            ));
        }
        let storage =
            pentax_kernel::SaveStorage::directory(directory).map_err(KernelStartupError::Kernel)?;
        self.kernel
            .configure_save_storage(storage)
            .map_err(KernelStartupError::Kernel)
    }

    pub(crate) fn kernel_stop_for(
        &mut self,
        source: &pentax_linker::SymbolicLinkPlan<'_>,
        stop: KernelServiceStop,
        actions: bool,
        stack_pointer: Option<u64>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        self.kernel_stop_for_with_vectors(source, stop, actions, stack_pointer, None)
    }

    pub(crate) fn kernel_stop_for_with_vectors(
        &mut self,
        source: &pentax_linker::SymbolicLinkPlan<'_>,
        stop: KernelServiceStop,
        actions: bool,
        stack_pointer: Option<u64>,
        vector_args: Option<[[u8; 16]; 8]>,
    ) -> Result<NativeServiceAction, KernelStartupError> {
        if source.identity() != self.memory().active_source_identity() {
            return Err(KernelStartupError::InvalidProvider(
                "stale kernel service source",
            ));
        }
        let next = self
            .stop_attempts
            .checked_add(1)
            .filter(|v| *v <= self.plan.limits.kernel.max_attempts)
            .ok_or(KernelStartupError::Budget("kernel stop attempts"))?;
        self.stop_attempts = next;
        charge(
            &mut self.provider_work,
            self.plan.registration_count() as u64
                + 8
                + CODE_MAPPED_BYTES / PAGE_SIZE * 3
                + self.plan.factory.metadata_mapped_bytes() / PAGE_SIZE * 4,
            self.plan.limits.max_provider_work,
        )?;
        let marker = self
            .plan
            .factory
            .markers()
            .iter()
            .find(|marker| self.plan.entry(**marker) + 5 == stop.syscall_pc)
            .ok_or(KernelStartupError::SiteMismatch {
                syscall_pc: stop.syscall_pc,
            })?;
        if stop.service_id != u64::from(marker.service_id) {
            return Err(KernelStartupError::ServiceIdMismatch {
                expected: marker.service_id,
                actual: stop.service_id,
            });
        }
        if marker.registration.service == pentax_kernel::Service::GetProcParam {
            charge(
                &mut self.provider_work,
                super::kernel_adapter::process_parameter_work(source),
                self.plan.limits.max_provider_work,
            )?;
        }
        let (mut paused, tls_instances) =
            self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
        attest_marker(&paused, self.plan, *marker)?;
        // Recheck page ownership/immutability before dispatch, including failed
        // calls. The NativeGuestMemory owner guarantees all physical views NX.
        for page in (self.plan.bias..self.plan.bias + CODE_MAPPED_BYTES).step_by(PAGE_SIZE as usize)
        {
            require(
                paused.read(page, &mut [0]).is_ok()
                    && paused.preflight_fetch_source(page, 1).is_ok()
                    && matches!(
                        paused.preflight_write(page, 1),
                        Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
                    ),
                "kernel provider lost immutable permission",
            )?;
        }
        attest_metadata_permissions(&paused, self.plan)?;
        self.last_admitted_kernel_service = Some(marker.registration.source_name);
        let mut memory = KernelMemory {
            paused: &mut paused,
            source: Some(source),
            supplied_modules: self.plan.kernel_provider_index(),
            lookup_limits: Some(pentax_linker::ModuleLookupLimits {
                symbolic: self.plan.limits.startup.hle.linking,
                max_name_bytes: self.plan.limits.kernel.max_name_bytes,
                max_work: self.plan.limits.provider_call_maximum(self.provider_work)?,
            }),
            lookup_work: Some(&mut self.provider_work),
            loaded_modules: Some(&mut self.loaded_modules),
            thread_tls: tls_instances.get(self.active_tls),
            initial_entry: if self.active_tls == 0 {
                self.initial_entry
            } else {
                None
            },
        };
        if matches!(
            marker.registration.service,
            pentax_kernel::Service::SysmoduleLoad | pentax_kernel::Service::SysmoduleLoadInternal
        ) && module_lifecycle::has_game_module(&memory, stop.args.0[0] as u16)
        {
            if !actions {
                return Err(KernelStartupError::Kernel(
                    pentax_kernel::Failure::Unsupported("pending system game module load"),
                ));
            }
            let request = self
                .kernel
                .admit_sysmodule_load(marker.registration.service, stop.args)
                .map_err(KernelStartupError::Kernel)?;
            module_lifecycle::prepare_sysmodule_action(
                &mut memory,
                &mut self.kernel,
                request,
                stop.syscall_pc,
                stop.service_id,
            )
            .map_err(KernelStartupError::Kernel)
        } else if actions && marker.registration.service == pentax_kernel::Service::LoadStartModule
        {
            let request = self
                .kernel
                .admit_module_load(stop.args, &memory)
                .map_err(KernelStartupError::Kernel)?;
            module_lifecycle::prepare_action(&mut memory, request, stop.syscall_pc, stop.service_id)
                .map_err(KernelStartupError::Kernel)
        } else if actions
            && matches!(
                marker.registration.service,
                pentax_kernel::Service::PthreadCreate
                    | pentax_kernel::Service::PosixPthreadCreate
                    | pentax_kernel::Service::PosixPthreadCreateName
            )
        {
            let action = self
                .kernel
                .admit_pthread_create(marker.registration.service, stop.args, &memory)
                .map_err(KernelStartupError::Kernel)?;
            pthread_create::prepare_action(
                &mut memory,
                action,
                stop.syscall_pc,
                stop.service_id,
                stop.args.0,
            )
        } else if actions
            && matches!(
                marker.registration.service,
                pentax_kernel::Service::Usleep
                    | pentax_kernel::Service::CondWait
                    | pentax_kernel::Service::PosixCondWait
                    | pentax_kernel::Service::WaitSema
                    | pentax_kernel::Service::WaitEqueue
                    | pentax_kernel::Service::WaitEventFlag
                    | pentax_kernel::Service::SetEventFlag
                    | pentax_kernel::Service::ClearEventFlag
                    | pentax_kernel::Service::CancelEventFlag
                    | pentax_kernel::Service::DeleteEventFlag
                    | pentax_kernel::Service::MutexLock
                    | pentax_kernel::Service::PosixMutexLock
                    | pentax_kernel::Service::MutexTimedLock
                    | pentax_kernel::Service::PosixMutexTimedLock
                    | pentax_kernel::Service::AudioOut2ContextCreate
                    | pentax_kernel::Service::AudioOut2ContextDestroy
                    | pentax_kernel::Service::AudioOut2ContextAdvance
                    | pentax_kernel::Service::AudioOut2ContextPush
                    | pentax_kernel::Service::AudioOut2ContextGetQueueLevel
                    | pentax_kernel::Service::AudioOut2UserCreate
                    | pentax_kernel::Service::AudioOut2UserDestroy
                    | pentax_kernel::Service::AudioOut2PortCreate
                    | pentax_kernel::Service::AudioOut2PortDestroy
                    | pentax_kernel::Service::AudioOut2PortGetState
                    | pentax_kernel::Service::AudioOut2PortSetAttributes
            )
        {
            pthread_wait::begin(
                &mut self.kernel,
                &mut memory,
                marker.registration.service,
                stop.args,
                stop.syscall_pc,
                stop.service_id,
            )
        } else if actions
            && marker.registration.service == pentax_kernel::Service::NetCtlCheckCallback
        {
            let action = self
                .kernel
                .admit_netctl_callbacks()
                .map_err(KernelStartupError::Kernel)?;
            self.netctl_callbacks.prepare(
                &mut memory,
                action,
                stop.syscall_pc,
                stop.service_id,
                stop.args.0,
            )
        } else if actions && marker.registration.service == pentax_kernel::Service::FontCreateString
        {
            let action = self
                .kernel
                .admit_font_string(stop.args, stack_pointer, &mut memory)
                .map_err(KernelStartupError::Kernel)?;
            self.font_parsers.prepare(
                &mut memory,
                action,
                stop.syscall_pc,
                stop.service_id,
                stop.args.0,
            )
        } else if actions && marker.registration.service == pentax_kernel::Service::ImeUpdate {
            let action = self
                .kernel
                .admit_ime_update(stop.args, &mut memory)
                .map_err(KernelStartupError::Kernel)?;
            self.ime_callbacks.prepare(
                &mut memory,
                action,
                stop.syscall_pc,
                stop.service_id,
                stop.args.0,
                stack_pointer,
            )
        } else if actions && marker.registration.service == pentax_kernel::Service::PthreadOnce {
            let action = self
                .kernel
                .admit_pthread_once(stop.args, &memory)
                .map_err(KernelStartupError::Kernel)?;
            self.once.prepare(
                &mut memory,
                action,
                stop.syscall_pc,
                stop.service_id,
                stop.args.0,
            )
        } else {
            self.kernel
                .invoke_library_with_vectors(
                    marker.registration.service,
                    stop.args,
                    stack_pointer,
                    vector_args,
                    &mut memory,
                )
                .map(NativeServiceAction::Returned)
                .map_err(KernelStartupError::Kernel)
        }
    }
}

/// Prepare the exact composed plan and actual native TLS owner, then attest
/// every kernel marker on those same pages. Errno, unavailable semantics and
/// backing faults remain distinct typed outcomes; no generic success exists.
pub fn prepare_native_kernel_startup<'plan, 'image>(
    plan: &'plan KernelStartupPlan<'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
    identity: ProcessIdentity,
    roots: Vec<ReadOnlyRoot>,
) -> Result<PreparedNativeKernelStartup<'plan, 'image>, KernelStartupError> {
    prepare_in(plan, storage, limits, identity, roots, None)
}

/// Stage into an already owned reservation, allowing its actual OS-selected
/// addresses to be chosen before building original profiles. Ownership moves
/// into the same checked staging path; supplied mappings are never overwritten.
pub fn prepare_native_kernel_startup_in<'plan, 'image>(
    plan: &'plan KernelStartupPlan<'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
    identity: ProcessIdentity,
    roots: Vec<ReadOnlyRoot>,
    owner: NativeAddressSpace,
) -> Result<PreparedNativeKernelStartup<'plan, 'image>, KernelStartupError> {
    prepare_in(plan, storage, limits, identity, roots, Some(owner))
}

fn prepare_in<'plan, 'image>(
    plan: &'plan KernelStartupPlan<'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
    identity: ProcessIdentity,
    roots: Vec<ReadOnlyRoot>,
    owner: Option<NativeAddressSpace>,
) -> Result<PreparedNativeKernelStartup<'plan, 'image>, KernelStartupError> {
    let mut work = 0;
    charge(
        &mut work,
        plan.registration_count() as u64 * 8 + 3,
        plan.limits.max_provider_work,
    )?;
    let mut kernel =
        Kernel::new(plan.limits.kernel, identity, roots).map_err(KernelStartupError::Kernel)?;
    kernel
        .configure_system(plan.limits.system)
        .map_err(KernelStartupError::Kernel)?;
    kernel
        .configure_user(plan.limits.user)
        .map_err(KernelStartupError::Kernel)?;
    let mut hle =
        super::native_startup::prepare_native_startup_in(&plan.hle, storage, limits, owner)
            .map_err(KernelStartupError::Hle)?;
    attest(&hle.pause().map_err(KernelStartupError::Hle)?, plan)?;
    if let Some(thread) = hle.tls().instances().first() {
        kernel
            .activate_thread_context(thread.thread_pointer)
            .map_err(KernelStartupError::Kernel)?;
    }
    Ok(PreparedNativeKernelStartup {
        plan,
        hle,
        kernel,
        stop_attempts: 0,
        provider_work: 0,
        native_service_requests: 0,
        libc_builtins: LibcBuiltinUsage::default(),
        #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
        libc_builtin_budget: super::dispatch::ServiceBudget::new(plan.hle.service_limits()),
        last_admitted_kernel_service: None,
        loaded_modules: module_lifecycle::LoadedModules::new(plan.kernel_provider_index())?,
        once: pthread_once::OnceState::new()?,
        netctl_callbacks: netctl_callback::NetCtlCallbackState::new(),
        font_parsers: font_callback::FontParserCallbackState::new()?,
        ime_callbacks: ime_callback::ImeCallbackState::new()?,
        json: json_strings::JsonStrings::new(),
        graphics: plan
            .graphics
            .map(|binding| crate::gpu::driver::Driver::new(binding.limits)),
        initial_entry: None,
        active_tls: 0,
        thread_runtime: pthread_create::ThreadRuntime::new(plan.limits.kernel.max_thread_contexts)?,
    })
}

#[cfg(test)]
mod tests;
