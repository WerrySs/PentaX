// SPDX-License-Identifier: GPL-2.0-only

//! Closed original preparation for a separately reviewed host block runner.
//! Core supplies owned source/storage and verifies observations; it neither
//! executes instructions nor certifies what a caller's host callback executed.

use super::{
    KernelStartupError, KernelStartupLimits, NativeEntryError, NativeEntryLimits,
    NativeEntryPolicy, NativeEntryRequest, PreparedNativeKernelStartup, plan_kernel_startup,
    prepare_native_kernel_startup_in,
};
use crate::memory::PAGE_SIZE;
use crate::memory::native::NativeLoadLimits;
use crate::process::SessionLoadLimits;
use crate::process::hle::{
    HleError, HleProfile, ProviderPlacement, build_hle_providers, build_kernel_provider,
};
use crate::process::tls::StaticTlsStorage;
use pentax_kernel::ProcessIdentity;
use pentax_linker::ModuleInput;
use pentax_loader::open_executable;
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryError, NativeMemoryLimits, NativeRange,
};
use std::fmt;

mod image;
use image::{EXIT_CALLBACK, OUTPUT, SYMBOL_OFFSET, TLS_RVA, original_image};

const ARENA: u64 = 0x10_0000;
const KERNEL: u64 = 0x2_0000;
const LIBC: u64 = 0x5_0000;
const TPS: [u64; 2] = [0x6_8000, 0x7_8000];
const PREFIX: u64 = 0x8_0000;
const STACK: u64 = 0xc_4000;
const RETURN_MARKER: u64 = 0x7fff_ffff_f000;
const EXPECTED: u64 = 42;

/// Original numeric stack/entry layout, not native execution authority.
/// The return marker is mediated only at the exact outer slot, never fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalPreparedLayout {
    pub entry: u64,
    pub stack: NativeRange,
    pub guards: [NativeRange; 2],
    pub return_slot: u64,
    pub return_marker: u64,
    pub frame_pointer: u64,
    pub arguments: u64,
    pub argument_name: u64,
    pub exit_handler: u64,
}

/// Verified callback observations on the actual retained native pages. These
/// values alone are not evidence of native instruction execution or PS5 ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OriginalPreparedReport {
    pub value: u64,
    pub result: u64,
    pub first_tls_value: u64,
    pub second_tls_value: u64,
    pub thread_id: u64,
    pub semaphore_handle: u64,
    pub modules: usize,
    pub applied_relocations: usize,
    pub kernel_services: u64,
    pub libc_services: u64,
    pub native_requests: u64,
    pub mapped_pages: u64,
    pub resident_pages: u64,
}

/// Callback failures retain their original type. No unsuccessful callback or
/// incomplete observed service sequence can publish an original fixture report.
#[derive(Debug)]
pub enum OriginalPreparedError<E> {
    Startup(KernelStartupError),
    Entry(NativeEntryError),
    Memory(NativeMemoryError),
    Fixture(&'static str),
    Callback(E),
}
impl<E: fmt::Display> fmt::Display for OriginalPreparedError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Startup(error) => write!(f, "original prepared startup: {error}"),
            Self::Entry(error) => write!(f, "original prepared entry: {error}"),
            Self::Memory(error) => write!(f, "original prepared memory: {error}"),
            Self::Fixture(reason) => write!(f, "original prepared fixture: {reason}"),
            Self::Callback(error) => write!(f, "original prepared callback: {error}"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for OriginalPreparedError<E> {}

fn startup<E>(error: HleError) -> OriginalPreparedError<E> {
    OriginalPreparedError::Startup(KernelStartupError::Hle(error))
}

fn word<E>(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    address: u64,
) -> Result<u64, OriginalPreparedError<E>> {
    let mut value = [0; 8];
    session
        .pause()
        .map_err(OriginalPreparedError::Startup)?
        .read(address, &mut value)
        .map_err(OriginalPreparedError::Memory)?;
    Ok(u64::from_le_bytes(value))
}

fn require<E>(condition: bool, reason: &'static str) -> Result<(), OriginalPreparedError<E>> {
    if condition {
        Ok(())
    } else {
        Err(OriginalPreparedError::Fixture(reason))
    }
}

fn image_mapped_bytes(image: &[u8]) -> Result<u64, HleError> {
    let view = open_executable(image).map_err(HleError::Load)?;
    let runtime = view.runtime_plan().map_err(HleError::Load)?;
    runtime.segments.iter().try_fold(0_u64, |total, segment| {
        // All three original images have disjoint page-aligned load segments.
        let end = segment
            .vaddr
            .checked_add(segment.mem_size)
            .and_then(|value| value.checked_add(PAGE_SIZE - 1))
            .ok_or(HleError::InvalidProvider("original image extent"))?
            & !(PAGE_SIZE - 1);
        total
            .checked_add(end - (segment.vaddr & !(PAGE_SIZE - 1)))
            .ok_or(HleError::InvalidProvider("original image byte sum"))
    })
}

fn load_limits(images: [&[u8]; 3]) -> Result<NativeLoadLimits, HleError> {
    // Actual image extents + two static TLS pages, two thread-header pages,
    // one entry stack and one dynamic prefix page, all eagerly resident.
    let mut bytes = 6 * PAGE_SIZE;
    for image in images {
        bytes = bytes
            .checked_add(image_mapped_bytes(image)?)
            .ok_or(HleError::InvalidProvider("original fixture byte sum"))?;
    }
    Ok(NativeLoadLimits {
        session: SessionLoadLimits {
            resident_limit: bytes,
            mapped_limit: bytes,
        },
        reserved_limit: ARENA,
    })
}

fn policy(base: u64) -> KernelStartupLimits {
    let mut limits = KernelStartupLimits::default();
    limits.startup.static_tls.thread_control =
        crate::process::tls::ThreadControlBlockPolicy::KytyObservedSelfPointerV1;
    limits.kernel.virtual_base = base + PREFIX;
    limits.kernel.virtual_bytes = 0x4_0000;
    limits.kernel.max_mapped_bytes = PAGE_SIZE;
    limits.kernel.max_attempts = 3;
    limits.kernel.max_handles = 2;
    limits.kernel.max_bytes = 8192;
    limits.kernel.max_work = 65536;
    limits.kernel.max_scratch_bytes = 4096;
    limits.kernel.max_io_bytes = 4096;
    limits.kernel.max_name_bytes = 64;
    limits.kernel.direct_memory_bytes = 0;
    limits.startup.hle.services.max_attempts = 1;
    limits.startup.hle.services.max_span_bytes = 8;
    limits.startup.hle.services.max_total_bytes = 8;
    limits.startup.hle.services.max_scratch_bytes = 8;
    // The shared libc policy charges up to two 16 KiB staging pages even for
    // this eight-byte copy, plus source/scratch visits and native attestation.
    limits.startup.hle.services.max_work_units = 2 * PAGE_SIZE + 64;
    limits
}

fn prepare_entry<E>(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    base: u64,
) -> Result<OriginalPreparedLayout, OriginalPreparedError<E>> {
    let proof = session
        .prepare_native_source(&Default::default())
        .map_err(OriginalPreparedError::Startup)?;
    let entry = session
        .prepare_process_entry(
            &proof,
            NativeEntryRequest {
                policy: NativeEntryPolicy::KytyObservedV1,
                stack: NativeRange {
                    address: base + STACK,
                    size: PAGE_SIZE,
                },
                program_name: b"PentaX",
                // Original no-op callback code, not a PS5 module-exit implementation.
                exit_handler: base + EXIT_CALLBACK as u64,
                return_marker: RETURN_MARKER,
            },
            &NativeEntryLimits::default(),
        )
        .map_err(OriginalPreparedError::Entry)?;
    let layout = *entry.layout();
    Ok(OriginalPreparedLayout {
        entry: layout.entry,
        stack: layout.stack,
        guards: layout.guards,
        return_slot: layout.return_slot,
        return_marker: layout.return_marker,
        frame_pointer: layout.frame_pointer,
        arguments: layout.arguments,
        argument_name: layout.argument_name,
        exit_handler: layout.exit_handler,
    })
}

fn tls_symbols<E>(
    session: &PreparedNativeKernelStartup<'_, '_>,
) -> Result<[u64; 2], OriginalPreparedError<E>> {
    let mut addresses = [0; 2];
    require(
        session.tls().instances().len() == addresses.len(),
        "TLS instance count",
    )?;
    for (address, instance) in addresses.iter_mut().zip(session.tls().instances()) {
        let block = instance
            .blocks
            .iter()
            .find(|block| block.module.index() == 0)
            .ok_or(OriginalPreparedError::Fixture("original TLS block"))?;
        *address = block
            .address
            .checked_add(SYMBOL_OFFSET)
            .filter(|value| {
                value
                    .checked_add(8)
                    .is_some_and(|end| end <= block.address + block.size)
            })
            .ok_or(OriginalPreparedError::Fixture("original TLS symbol range"))?;
    }
    Ok(addresses)
}

/// Build one closed original image and both exact providers inside a fresh
/// OS-selected owned arena, prepare typed TLS, then call the supplied host
/// runner once. All source, plans, factories and actual backing stay alive for
/// that call. This safe helper contains no CPU executor or native pointer.
///
/// The caller's separately reviewed runner must obey its own unsafe execution
/// contract. The returned report checks real service counts and backing effects;
/// it does not certify that a caller actually executed the original program.
/// Callback errors propagate unchanged, and every failure drops this local owner.
pub fn run_original_prepared_fixture<E>(
    callback: impl FnOnce(
        &mut PreparedNativeKernelStartup<'_, '_>,
        OriginalPreparedLayout,
    ) -> Result<u64, E>,
) -> Result<OriginalPreparedReport, OriginalPreparedError<E>> {
    let defaults = KernelStartupLimits::default();
    let kernel = build_kernel_provider(defaults.max_kernel_provider_bytes)
        .map_err(|error| OriginalPreparedError::Startup(KernelStartupError::Provider(error)))?;
    let libc = build_hle_providers(
        HleProfile::KytyObservedLibcInternalV1,
        &defaults.startup.hle,
    )
    .map_err(startup)?;
    // Base-dependent instruction immediates do not change this original image's
    // segment geometry; its actual payload is regenerated after reserving.
    let geometry = original_image(0).map_err(startup)?;
    let caps = load_limits([&geometry, kernel.image(), libc.image()]).map_err(startup)?;
    drop(geometry);
    let owner = NativeAddressSpace::reserve_any(
        NativeMemoryLimits {
            mapped_bytes: caps.session.mapped_limit,
            reserved_bytes: caps.reserved_limit,
        },
        ARENA,
    )
    .map_err(OriginalPreparedError::Memory)?;
    let base = owner
        .reserved_ranges()
        .next()
        .ok_or(OriginalPreparedError::Fixture("owned arena"))?
        .address;
    // reserve_any already checked base+ARENA against the guest address limit;
    // every original numeric address below is inside that complete owned range.
    let limits = policy(base);
    let bytes = original_image(base).map_err(startup)?;
    let input = [ModuleInput::new(
        open_executable(&bytes).map_err(|error| startup(HleError::Load(error)))?,
        base,
    )];
    let plan = plan_kernel_startup(
        &input,
        &kernel,
        ProviderPlacement {
            load_bias: base + KERNEL,
        },
        &libc,
        ProviderPlacement {
            load_bias: base + LIBC,
        },
        &limits,
    )
    .map_err(OriginalPreparedError::Startup)?;
    let pointers = TPS.map(|rva| base + rva);
    let mut session = prepare_native_kernel_startup_in(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &pointers,
        },
        &caps,
        ProcessIdentity {
            process_id: 41,
            initial_thread_id: 73,
        },
        vec![],
        owner,
    )
    .map_err(OriginalPreparedError::Startup)?;
    let layout = prepare_entry(&mut session, base)?;
    let symbols = tls_symbols(&session)?;
    let tpoff = word(&mut session, base + TLS_RVA + 16)?;
    require(
        pointers[0].wrapping_add(tpoff) == symbols[0],
        "typed TLS address",
    )?;
    require(
        word(&mut session, symbols[0])? == 37 && word(&mut session, symbols[1])? == 37,
        "initial TLS values",
    )?;
    let value = callback(&mut session, layout).map_err(OriginalPreparedError::Callback)?;
    require(value == EXPECTED, "callback result")?;
    let usage = session.kernel_usage();
    require(
        session.native_service_requests() == 4
            && session.kernel_stop_attempts() == 3
            && usage.attempts == 3
            && session.libc_service_attempts() == 1
            && session.hle.completed_services() == 1
            && usage.handles_live == 2
            && usage.mapped_bytes == PAGE_SIZE
            && usage.bytes_written == 4 + 8,
        "observed service sequence",
    )?;
    let result = word(&mut session, base + OUTPUT + 0x80)?;
    let first = word(&mut session, symbols[0])?;
    let second = word(&mut session, symbols[1])?;
    let thread_id = word(&mut session, base + PREFIX)?;
    let handle = word(&mut session, base + OUTPUT + 8)?;
    require(
        result == EXPECTED && first == EXPECTED && second == 37,
        "result and TLS isolation",
    )?;
    require(
        thread_id == 73 && handle != 0,
        "pthread prefix and semaphore handle",
    )?;
    let mut prefix = [0; 4096];
    session
        .pause()
        .map_err(OriginalPreparedError::Startup)?
        .read(base + PREFIX, &mut prefix)
        .map_err(OriginalPreparedError::Memory)?;
    require(
        prefix[4..].iter().all(|byte| *byte == 0),
        "pthread reserved prefix stays zero",
    )?;
    require(
        word(&mut session, base + TLS_RVA + SYMBOL_OFFSET)? == 37,
        "original TLS source unchanged",
    )?;
    require(
        word(&mut session, layout.return_slot)? == layout.return_marker,
        "outer return slot",
    )?;
    require(
        word(&mut session, base + OUTPUT + 0x90)? == layout.arguments
            && word(&mut session, base + OUTPUT + 0x98)? == layout.exit_handler
            && word(&mut session, base + OUTPUT + 0xa0)? == layout.frame_pointer
            && word(&mut session, base + OUTPUT + 0xa8)? == 1
            && word(&mut session, base + OUTPUT + 0xb0)? == u64::from_le_bytes(*b"PentaX\0\0")
            && word(&mut session, base + OUTPUT + 0xb8)? == 0
            && word(&mut session, base + OUTPUT + 0xc0)? == 0,
        "guest entry arguments, program name and root frame",
    )?;
    require(
        session.memory().mapped_bytes() == caps.session.mapped_limit
            && session.memory().committed_bytes() == caps.session.resident_limit,
        "actual eager page counts",
    )?;
    require(
        word(&mut session, pointers[0] + 0x28)? == 73
            && word(&mut session, pointers[1] + 0x28)? == 0,
        "modeled FS store and independent thread headers",
    )?;
    {
        let paused = session.pause().map_err(OriginalPreparedError::Startup)?;
        for guard in layout.guards {
            paused
                .preflight_reserved_unmapped(guard.address, guard.size)
                .map_err(OriginalPreparedError::Memory)?;
        }
        paused
            .preflight_write(layout.stack.address, layout.stack.size)
            .map_err(OriginalPreparedError::Memory)?;
        require(
            matches!(
                paused.preflight_fetch_source(layout.stack.address, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ),
            "stack stays NX",
        )?;
    }
    Ok(OriginalPreparedReport {
        value,
        result,
        first_tls_value: first,
        second_tls_value: second,
        thread_id,
        semaphore_handle: handle,
        modules: session.memory().module_count(),
        applied_relocations: session.memory().applied_relocations(),
        kernel_services: usage.attempts,
        libc_services: session.libc_service_attempts(),
        native_requests: session.native_service_requests(),
        mapped_pages: session.memory().mapped_bytes() / PAGE_SIZE,
        resident_pages: session.memory().committed_bytes() / PAGE_SIZE,
    })
}

#[cfg(test)]
mod tests;
