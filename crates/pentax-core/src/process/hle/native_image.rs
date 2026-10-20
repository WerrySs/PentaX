// SPDX-License-Identifier: GPL-2.0-only

//! Scoped admission of supplied immutable images into the reusable owned runtime.
//! This stages an explicit observed main-call probe, not a completed firmware
//! startup. One explicitly selected preload can use observed initialization;
//! automatic discovery, process parameters and module exit remain work.

use super::*;
use crate::memory::PAGE_SIZE;
use crate::memory::native::NativeLoadLimits;
use crate::process::SessionLoadLimits;
use crate::process::tls::{StaticTlsStorage, ThreadControlBlockPolicy, plan_relocated_static_tls};
use pentax_kernel::{ProcessIdentity, ReadOnlyRoot};
use pentax_linker::{
    MissingFunctionReason, ModuleInput, SymbolicError, plan_missing_functions, plan_tls_bindings,
};
use pentax_loader::{ElfType, LoadError, RuntimeLoadPlan, open_executable};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryError, NativeMemoryLimits, NativeRange,
};
use std::fmt;

const GRANULE: u64 = 65_536;

mod resources;
pub use resources::NativeResourceError;

/// Explicit host storage authority for private temporary physical sections.
/// Guest paths never select this directory or any existing backing file.
#[derive(Debug, Clone)]
pub struct NativeImageFileBacking {
    pub directory: std::path::PathBuf,
    pub capacity_bytes: u64,
    pub private_commit_limit_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct NativeImageLimits {
    /// Explicit supplied provider index for the observed libc preload call.
    /// Input zero cannot be selected. None preserves the main-entry-only probe.
    pub startup_libc: Option<usize>,
    pub max_images: usize,
    pub max_input_bytes: u64,
    pub max_image_alignment: u64,
    pub max_layout_work_steps: u64,
    pub stack_bytes: u64,
    pub kernel_window_bytes: u64,
    /// Dedicated lazy-commit window for thread TLS and guarded stacks.
    pub thread_window_bytes: u64,
    /// Reserved exclusively for later guest-directed module admission. Zero
    /// preserves the initial-only probe; pages remain unmapped until requested.
    pub dynamic_window_bytes: u64,
    pub startup: KernelStartupLimits,
    pub graphics: crate::gpu::driver::Limits,
    pub unavailable: UnavailableProviderLimits,
    pub native: NativeLoadLimits,
    /// Commit physical-section offsets only when first mapped, retaining the
    /// full section-capacity budget. Compact fixtures keep eager allocation.
    pub defer_physical_commit: bool,
    pub file_backing: Option<NativeImageFileBacking>,
}
impl Default for NativeImageLimits {
    fn default() -> Self {
        let mut startup = KernelStartupLimits::default();
        startup.startup.hle.linking.missing_weak = pentax_linker::MissingWeakPolicy::NullAddressV1;
        startup.startup.static_tls.thread_control =
            ThreadControlBlockPolicy::KytyObservedSelfPointerV1;
        startup.startup.static_tls.max_instances = startup.kernel.max_thread_contexts as u64;
        startup.startup.static_tls.max_storage_bytes = 64 * 1024 * 1024;
        Self {
            startup_libc: None,
            max_images: 123,
            max_input_bytes: 512 * 1024 * 1024,
            max_image_alignment: 2 * 1024 * 1024,
            max_layout_work_steps: 1_000_000,
            stack_bytes: 8 * 1024 * 1024,
            kernel_window_bytes: 64 * 1024 * 1024,
            thread_window_bytes: 64 * 1024 * 1024,
            dynamic_window_bytes: 0,
            startup,
            defer_physical_commit: false,
            file_backing: None,
            graphics: crate::gpu::driver::Limits::default(),
            unavailable: UnavailableProviderLimits::default(),
            native: NativeLoadLimits {
                session: SessionLoadLimits {
                    mapped_limit: 512 * 1024 * 1024,
                    resident_limit: 512 * 1024 * 1024,
                },
                reserved_limit: 512 * 1024 * 1024,
            },
        }
    }
}

/// Diagnostics and the numeric entry call context. They are not execution
/// authority; a runner still needs exact core source/owner provenance and live
/// per-block admission. Unavailable exports remain missing implementations.
#[derive(Debug, Clone, Copy)]
pub struct NativeImageLayout<'plan, 'image> {
    pub entry: NativeEntryLayout,
    pub initializer: Option<PreparedNativeModuleStart<'plan, 'image>>,
    pub supplied_modules: usize,
    pub unavailable_functions: usize,
    pub reserved_bytes: u64,
}

/// Keep typed causes available to the caller, while default formatting emits
/// only stable categories rather than paths, input identifiers or source bytes.
pub enum NativeImageError<E> {
    Load(LoadError),
    Link(SymbolicError),
    Preparation(KernelStartupError),
    Memory(NativeMemoryError),
    Entry(NativeEntryError),
    Provider(UnavailableProviderError),
    Initializer(NativeModuleStartError),
    Invalid(&'static str),
    Budget(&'static str),
    Callback(E),
}
impl<E> NativeImageError<E> {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Load(_) => "executable-load",
            Self::Link(SymbolicError::VersionMismatch { .. }) => "import-version-mismatch",
            Self::Link(SymbolicError::AmbiguousProvider { .. }) => "ambiguous-import",
            Self::Link(SymbolicError::UnsupportedSymbol { reason, .. }) => match reason {
                pentax_linker::UnsupportedSymbol::WeakProvider => "unsupported-weak-provider",
                pentax_linker::UnsupportedSymbol::PreemptibleDefinition => "preemptible-definition",
                pentax_linker::UnsupportedSymbol::TypeMismatch => "import-type-mismatch",
                _ => "unsupported-link-symbol",
            },
            Self::Link(SymbolicError::UnsupportedMissingFunction { reason, .. }) => match reason {
                MissingFunctionReason::SymbolType => "missing-non-function-import",
                MissingFunctionReason::WeakBinding => "missing-weak-function-import",
                MissingFunctionReason::AddressAddend => "missing-function-address-addend",
            },
            Self::Link(_) => "image-link",
            Self::Preparation(KernelStartupError::Hle(HleError::TlsBinding(
                pentax_linker::TlsBindingError::UnsupportedSymbolZero { .. },
            ))) => "tls-symbol-zero-unsupported",
            Self::Preparation(_) => "native-preparation",
            Self::Memory(_) => "native-memory",
            Self::Entry(_) => "entry-preparation",
            Self::Provider(_) => "unavailable-provider",
            Self::Initializer(error) => error.code(),
            Self::Invalid(code) | Self::Budget(code) => code,
            Self::Callback(_) => "execution-stopped",
        }
    }
}
impl<E> fmt::Debug for NativeImageError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NativeImageError")
            .field(&self.code())
            .finish()
    }
}
impl<E> fmt::Display for NativeImageError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "image admission: {}", self.code())
    }
}
impl<E> std::error::Error for NativeImageError<E> {}

#[derive(Clone, Copy)]
struct Shape {
    low: u64,
    span: u64,
    alignment: u64,
}
fn sum<E>(left: u64, right: u64) -> Result<u64, NativeImageError<E>> {
    left.checked_add(right)
        .ok_or(NativeImageError::Budget("layout-overflow"))
}
fn round<E>(value: u64, unit: u64) -> Result<u64, NativeImageError<E>> {
    if unit == 0 || !unit.is_power_of_two() {
        return Err(NativeImageError::Invalid("layout-alignment"));
    }
    Ok(sum(value, unit - 1)? & !(unit - 1))
}
fn charge<E>(
    work: &mut u64,
    amount: u64,
    limits: &NativeImageLimits,
) -> Result<(), NativeImageError<E>> {
    *work = sum(*work, amount)?;
    if *work > limits.max_layout_work_steps {
        return Err(NativeImageError::Budget("layout-work"));
    }
    Ok(())
}
fn shape<E>(
    runtime: &RuntimeLoadPlan<'_>,
    work: &mut u64,
    limits: &NativeImageLimits,
) -> Result<Shape, NativeImageError<E>> {
    if !matches!(
        runtime.metadata.elf_type,
        ElfType::SharedObject | ElfType::SceDynamicExecutable | ElfType::SceSharedObject
    ) {
        return Err(NativeImageError::Invalid("fixed-address-image-unsupported"));
    }
    let mut low = u64::MAX;
    let mut high = 0;
    let mut alignment = PAGE_SIZE;
    charge(work, runtime.segments.len() as u64, limits)?;
    for segment in &runtime.segments {
        if segment.mem_size == 0 {
            continue;
        }
        low = low.min(segment.vaddr / PAGE_SIZE * PAGE_SIZE);
        high = high.max(round(sum(segment.vaddr, segment.mem_size)?, PAGE_SIZE)?);
        alignment = alignment.max(segment.align);
    }
    if low == u64::MAX || alignment > limits.max_image_alignment || !alignment.is_power_of_two() {
        return Err(NativeImageError::Invalid("image-layout"));
    }
    Ok(Shape {
        low,
        span: high - low,
        alignment,
    })
}

/// Keep images, factories, plans, typed TLS, actual owner and main-call storage
/// alive through one callback. This does not execute instructions or attest the
/// callback's claimed result. The dedicated runner supplies that evidence.
///
/// Only position-independent inputs are rebased. Exact versions and complete
/// supported relocations/TLS remain required. Missing callable functions are
/// retained explicitly and stop at use; no unknown data/weak/TLS binding is
/// synthesized. Input zero remains primary. No automatic module discovery,
/// constructor skipping claimed as successful initialization or exit handling.
pub fn with_native_image_session<T, E>(
    images: &[&[u8]],
    limits: &NativeImageLimits,
    callback: impl FnOnce(
        &mut PreparedNativeKernelStartup<'_, '_>,
        NativeImageLayout<'_, '_>,
    ) -> Result<T, E>,
) -> Result<T, NativeImageError<E>> {
    with_native_image_session_with_roots(images, limits, Vec::new(), callback)
}

/// Prepare the same owned runtime with explicit read-only file capabilities.
/// The kernel consumes the finite inventory and retains its normal name,
/// handle, transfer and cumulative work bounds. Guest paths never name host
/// paths. This does not discover directories or execute any guest instruction.
pub fn with_native_image_session_with_roots<T, E>(
    images: &[&[u8]],
    limits: &NativeImageLimits,
    roots: Vec<ReadOnlyRoot>,
    callback: impl FnOnce(
        &mut PreparedNativeKernelStartup<'_, '_>,
        NativeImageLayout<'_, '_>,
    ) -> Result<T, E>,
) -> Result<T, NativeImageError<E>> {
    if images.is_empty() || images.len() > limits.max_images {
        return Err(NativeImageError::Invalid("image-count"));
    }
    if limits
        .startup_libc
        .is_some_and(|index| index == 0 || index >= images.len())
    {
        return Err(NativeImageError::Invalid("initializer-input-index"));
    }
    if limits.stack_bytes == 0
        || !limits.stack_bytes.is_multiple_of(PAGE_SIZE)
        || limits.kernel_window_bytes == 0
        || !limits.kernel_window_bytes.is_multiple_of(PAGE_SIZE)
        || !limits.dynamic_window_bytes.is_multiple_of(GRANULE)
        || !limits.thread_window_bytes.is_multiple_of(GRANULE)
    {
        return Err(NativeImageError::Invalid("runtime-layout"));
    }
    let mut work = 0;
    charge(&mut work, images.len() as u64, limits)?;
    let mut input_bytes = 0;
    for image in images {
        input_bytes = sum(input_bytes, image.len() as u64)?;
    }
    if input_bytes > limits.max_input_bytes {
        return Err(NativeImageError::Budget("input-bytes"));
    }
    let mut views = Vec::new();
    let mut shapes = Vec::new();
    views
        .try_reserve_exact(images.len())
        .map_err(|_| NativeImageError::Budget("layout-allocation"))?;
    shapes
        .try_reserve_exact(images.len())
        .map_err(|_| NativeImageError::Budget("layout-allocation"))?;
    let mut reservation = 0;
    for bytes in images {
        let view = open_executable(bytes).map_err(NativeImageError::Load)?;
        let runtime = view.runtime_plan().map_err(NativeImageError::Load)?;
        let shape = shape(&runtime, &mut work, limits)?;
        reservation = sum(
            reservation,
            sum(round(shape.span, GRANULE)?, shape.alignment.max(GRANULE))?,
        )?;
        views.push(view);
        shapes.push(shape);
    }
    for extra in [
        limits.unavailable.max_image_bytes,
        limits.startup.startup.static_tls.max_static_bytes,
        limits.stack_bytes,
        limits.kernel_window_bytes,
        limits.dynamic_window_bytes,
        limits.thread_window_bytes,
        GRANULE,
        1024 * 1024,
    ] {
        reservation = sum(reservation, extra)?;
    }
    reservation = round(reservation, GRANULE)?;
    if reservation > limits.native.reserved_limit {
        return Err(NativeImageError::Budget("reservation-bytes"));
    }
    let mut owner = NativeAddressSpace::reserve_any(
        NativeMemoryLimits {
            mapped_bytes: limits
                .native
                .session
                .mapped_limit
                .min(limits.native.session.resident_limit),
            reserved_bytes: limits.native.reserved_limit,
        },
        reservation,
    )
    .map_err(NativeImageError::Memory)?;
    if let Some(backing) = &limits.file_backing {
        let store = pentax_platform::native_memory::NativeFileBacking::open(
            &backing.directory,
            backing.capacity_bytes,
        )
        .map_err(NativeImageError::Memory)?;
        owner
            .use_file_backing(store, backing.private_commit_limit_bytes)
            .map_err(NativeImageError::Memory)?;
    } else if limits.defer_physical_commit {
        owner
            .defer_physical_commit()
            .map_err(NativeImageError::Memory)?;
    }
    if owner.allocation_granularity() != GRANULE {
        return Err(NativeImageError::Invalid("native-allocation-granularity"));
    }
    let base = owner
        .reserved_ranges()
        .next()
        .ok_or(NativeImageError::Invalid("native-reservation"))?
        .address;
    let arena_end = sum(base, reservation)?;
    let mut cursor = base;
    let mut inputs = Vec::new();
    inputs
        .try_reserve_exact(images.len() + 4)
        .map_err(|_| NativeImageError::Budget("layout-allocation"))?;
    for (view, shape) in views.into_iter().zip(shapes) {
        let bias = round(cursor.saturating_sub(shape.low), shape.alignment)?;
        let start = sum(bias, shape.low)?;
        cursor = round(sum(start, shape.span)?, GRANULE)?;
        if start < base || cursor > arena_end {
            return Err(NativeImageError::Budget("image-placement"));
        }
        inputs.push(ModuleInput::new(view, bias));
    }
    let kernel_bias = cursor;
    let kernel = build_kernel_provider(limits.startup.max_kernel_provider_bytes)
        .map_err(|e| NativeImageError::Preparation(KernelStartupError::Provider(e)))?;
    let libc_bias = round(sum(kernel_bias, kernel.mapped_bytes())?, GRANULE)?;
    let graphics_bias = sum(libc_bias, GRANULE)?;
    let json_bias = sum(graphics_bias, GRANULE)?;
    let unavailable_bias = sum(json_bias, GRANULE)?;
    let graphics = build_graphics_provider(PAGE_SIZE)
        .map_err(|e| NativeImageError::Preparation(KernelStartupError::Provider(e)))?;
    let json = build_json_provider(
        ProviderPlacement {
            load_bias: json_bias,
        },
        limits.startup.startup.hle.max_provider_bytes,
    )
    .map_err(|e| NativeImageError::Preparation(KernelStartupError::Provider(e)))?;
    let libc = build_hle_providers(
        HleProfile::KytyObservedLibcInternalV1,
        &limits.startup.startup.hle,
    )
    .map_err(|e| NativeImageError::Preparation(KernelStartupError::Hle(e)))?;
    // One classification pass with the actual real providers, followed by the
    // constructor's independent exact validation. No per-import retry/reparse.
    inputs.push(ModuleInput::new(
        open_executable(kernel.image()).map_err(NativeImageError::Load)?,
        kernel_bias,
    ));
    inputs.push(ModuleInput::new(
        open_executable(graphics.image()).map_err(NativeImageError::Load)?,
        graphics_bias,
    ));
    inputs.push(ModuleInput::new(
        open_executable(json.image()).map_err(NativeImageError::Load)?,
        json_bias,
    ));
    inputs.push(ModuleInput::new(
        open_executable(libc.image()).map_err(NativeImageError::Load)?,
        libc_bias,
    ));
    let missing = plan_missing_functions(
        &inputs,
        &limits.startup.startup.hle.linking,
        limits.unavailable.max_functions as u64,
    )
    .map_err(NativeImageError::Link)?;
    inputs.truncate(images.len());
    let unavailable = build_unavailable_provider(missing, &limits.unavailable)
        .map_err(NativeImageError::Provider)?;
    cursor = round(sum(unavailable_bias, unavailable.mapped_bytes())?, GRANULE)?;
    // Kernel allocator lives beyond entry storage, inside this same reservation.
    // Compute its bounded location conservatively before retaining startup limits.
    let dynamic_start = round(
        sum(
            cursor,
            sum(
                limits.startup.startup.static_tls.max_static_bytes,
                sum(limits.stack_bytes, 8 * GRANULE)?,
            )?,
        )?,
        GRANULE,
    )?;
    // Keep later executable modules near initial code, before the large data
    // window, so their PC-relative data accesses remain within cache reach.
    let thread_start = sum(dynamic_start, limits.dynamic_window_bytes)?;
    let heap = sum(thread_start, limits.thread_window_bytes)?;
    if sum(heap, limits.kernel_window_bytes)? > arena_end {
        return Err(NativeImageError::Budget("runtime-placement"));
    }
    let mut startup = limits.startup;
    if limits.dynamic_window_bytes != 0 {
        startup.startup.static_tls.reserved_static_bytes =
            startup.startup.static_tls.max_static_bytes;
    }
    startup.kernel.virtual_base = heap;
    startup.kernel.virtual_bytes = limits.kernel_window_bytes;
    let plan = plan_kernel_startup_with_extensions(
        &inputs,
        &kernel,
        ProviderPlacement {
            load_bias: kernel_bias,
        },
        &libc,
        ProviderPlacement {
            load_bias: libc_bias,
        },
        KernelStartupExtensions {
            graphics: Some(GraphicsStartupOptions {
                provider: &graphics,
                placement: ProviderPlacement {
                    load_bias: graphics_bias,
                },
                limits: limits.graphics,
                unavailable: None,
            }),
            json: Some(JsonStartupOptions {
                provider: &json,
                placement: ProviderPlacement {
                    load_bias: json_bias,
                },
            }),
            unavailable: Some(UnavailableProviderPlacement {
                provider: &unavailable,
                placement: ProviderPlacement {
                    load_bias: unavailable_bias,
                },
            }),
        },
        &startup,
    )
    .map_err(NativeImageError::Preparation)?;
    let bindings =
        plan_tls_bindings(plan.link_plan(), &startup.startup.tls_bindings).map_err(|e| {
            NativeImageError::Preparation(KernelStartupError::Hle(HleError::TlsBinding(e)))
        })?;
    let tls = plan_relocated_static_tls(&bindings, &startup.startup.static_tls)
        .map_err(|e| NativeImageError::Preparation(KernelStartupError::Hle(HleError::Tls(e))))?;
    let extent = round(
        tls.static_extent()
            .max(startup.startup.static_tls.reserved_static_bytes),
        PAGE_SIZE,
    )?;
    // Keep both TLS guards inside granules contributed by actual storage. A
    // leading page and a TP outside the final granule's last page satisfy the
    // existing reservation-union preflight without inventing mapped guards.
    let mut first_storage = sum(cursor, PAGE_SIZE)?;
    if sum(first_storage, extent)? % GRANULE == GRANULE - PAGE_SIZE {
        first_storage = sum(first_storage, PAGE_SIZE)?;
    }
    let thread_pointer = sum(first_storage, extent)?;
    let stack_start = sum(
        round(sum(thread_pointer, 2 * PAGE_SIZE)?, GRANULE)?,
        PAGE_SIZE,
    )?;
    if sum(stack_start, sum(limits.stack_bytes, PAGE_SIZE)?)? > dynamic_start {
        return Err(NativeImageError::Budget("entry-placement"));
    }
    let mut session = prepare_native_kernel_startup_in(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &[thread_pointer],
        },
        &limits.native,
        ProcessIdentity {
            process_id: 1,
            initial_thread_id: 1,
        },
        roots,
        owner,
    )
    .map_err(NativeImageError::Preparation)?;
    session
        .configure_thread_storage(NativeRange {
            address: thread_start,
            size: limits.thread_window_bytes,
        })
        .map_err(NativeImageError::Preparation)?;
    if limits.dynamic_window_bytes != 0 {
        session
            .configure_dynamic_window(
                NativeRange {
                    address: dynamic_start,
                    size: limits.dynamic_window_bytes,
                },
                limits.native.clone(),
                limits.unavailable,
            )
            .map_err(NativeImageError::Preparation)?;
    }
    let source = session
        .prepare_native_source(&Default::default())
        .map_err(NativeImageError::Preparation)?;
    let entry = session
        .prepare_process_entry(
            &source,
            NativeEntryRequest {
                policy: NativeEntryPolicy::KytyObservedV1,
                stack: NativeRange {
                    address: stack_start,
                    size: limits.stack_bytes,
                },
                program_name: b"PentaX",
                exit_handler: plan
                    .unavailable_exit_callback()
                    .ok_or(NativeImageError::Invalid("exit-marker"))?,
                return_marker: 0x7fff_ffff_f000,
            },
            &NativeEntryLimits {
                max_stack_bytes: limits.stack_bytes,
                ..Default::default()
            },
        )
        .map_err(NativeImageError::Entry)?;
    let initializer = match limits.startup_libc {
        Some(index) => session
            .prepare_module_initializer(
                &source,
                &entry,
                index,
                limits.max_layout_work_steps.saturating_sub(work),
            )
            .map_err(NativeImageError::Initializer)?,
        None => None,
    };
    let layout = NativeImageLayout {
        entry: *entry.layout(),
        initializer,
        supplied_modules: images.len(),
        unavailable_functions: plan.unavailable_functions(),
        reserved_bytes: session.memory().reserved_bytes(),
    };
    callback(&mut session, layout).map_err(NativeImageError::Callback)
}

#[cfg(test)]
mod tests;
