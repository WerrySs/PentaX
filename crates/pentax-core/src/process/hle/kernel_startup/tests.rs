// SPDX-License-Identifier: GPL-2.0-only

//! Original data-only fixtures. No native block, CPU, file capability or host
//! execution participates; the internal stop is captured before SYSCALL.

#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

#[path = "tests/convert_keycode.rs"]
mod convert_keycode;
mod public_execution;

use super::*;
use crate::process::SessionLoadLimits;
use crate::process::hle::native::NativeServiceStop;
use crate::process::hle::provider::{program, put};
use crate::process::hle::{HleProfile, KnownFunction, build_hle_providers, build_kernel_provider};
use pentax_kernel::{Errno, GuestAccess, GuestError, GuestWrite, MemoryEffect, Service};
use pentax_linker::SymbolicError;
use pentax_platform::native_memory::{NativeMemoryError, NativeMemoryLimits, NativePermissions};

const RESERVATION: u64 = 0x10_0000;
const TLS_RVA: u64 = 0x4108;
const OUTPUT: u64 = 0x8000;
const NAME: u64 = OUTPUT + 0x40;
const TLS_OFFSET: u64 = 56;
const ENTRY: usize = 0x400;

#[test]
fn image_protection_tracks_actual_loaded_pages_without_allocating_or_reimporting() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let mapped = session.memory().mapped_bytes();
            let data = base + 0x4000;
            for protection in [0x8000_0003, 0, 1, 2] {
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [data + 1, 1, protection, 0, 0, 0],
                    ))
                    .unwrap();
                session
                    .kernel_stop(stop(
                        plan,
                        Service::QueryProtection,
                        [
                            data,
                            base + OUTPUT,
                            base + OUTPUT + 8,
                            base + OUTPUT + 16,
                            0,
                            0,
                        ],
                    ))
                    .unwrap();
                assert_eq!(word(&mut session, base + OUTPUT), data);
                assert_eq!(word(&mut session, base + OUTPUT + 8), data + PAGE_SIZE);
                assert_eq!(word(&mut session, base + OUTPUT + 16), protection);
                assert_eq!(session.memory().mapped_bytes(), mapped);
                assert_eq!(session.kernel_usage().mapped_bytes, PAGE_SIZE);
                assert_eq!(session.kernel_usage().handles_live, 1);
            }
            session
                .pause()
                .unwrap()
                .write(data, &42_u64.to_le_bytes())
                .unwrap();
            assert_eq!(word(&mut session, data), 42);
            // An ordinary unmap releases exactly the adopted metadata/accounting.
            session
                .kernel_stop(stop(plan, Service::Munmap, [data, PAGE_SIZE, 0, 0, 0, 0]))
                .unwrap();
            assert_eq!(session.kernel_usage().mapped_bytes, 0);
            assert_eq!(session.kernel_usage().handles_live, 0);
            assert!(
                session
                    .kernel_stop(stop(plan, Service::Mprotect, [data, PAGE_SIZE, 1, 0, 0, 0]))
                    .is_err()
            );
            assert_eq!(session.kernel_usage().handles_live, 0);
        },
    );
}

#[test]
fn image_protection_refuses_late_holes_providers_runtime_storage_and_resource_limits_atomically() {
    for limit in [
        None,
        Some("handles"),
        Some("mapped"),
        Some("scratch"),
        Some("work"),
    ] {
        let mut limits = KernelStartupLimits::default();
        match limit {
            Some("handles") => limits.kernel.max_handles = 0,
            Some("mapped") => limits.kernel.max_mapped_bytes = PAGE_SIZE - 1,
            Some("scratch") => limits.kernel.max_scratch_bytes = 1,
            Some("work") => limits.kernel.max_work = 1,
            _ => {}
        }
        // Kernel construction requires scratch to cover name and I/O policy.
        if limit == Some("scratch") {
            limits.kernel.max_name_bytes = 1;
            limits.kernel.max_io_bytes = 1;
        }
        with_plan(limits, |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let before = session.memory().mapped_bytes();
            let attempts = if limit.is_none() {
                vec![
                    (base + OUTPUT, 2 * PAGE_SIZE),
                    (base + 0x2_0000, PAGE_SIZE),
                    (pointers[0] & !(PAGE_SIZE - 1), PAGE_SIZE),
                ]
            } else {
                vec![(base + OUTPUT, PAGE_SIZE)]
            };
            for (address, length) in attempts {
                assert!(
                    session
                        .kernel_stop(stop(plan, Service::Mprotect, [address, length, 1, 0, 0, 0]))
                        .is_err()
                );
                session
                    .pause()
                    .unwrap()
                    .write(base + OUTPUT + 0x80, &[42])
                    .unwrap();
                assert_eq!(session.memory().mapped_bytes(), before);
                assert_eq!(session.kernel_usage().mapped_bytes, 0);
                assert_eq!(session.kernel_usage().handles_live, 0);
            }
        });
    }
}

fn original_consumer() -> Vec<u8> {
    let mut image = vec![0; 0x8100];
    image[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut image, 16, &3_u16.to_le_bytes());
    put(&mut image, 18, &62_u16.to_le_bytes());
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    put(&mut image, 52, &64_u16.to_le_bytes());
    put(&mut image, 54, &56_u16.to_le_bytes());
    put(&mut image, 56, &6_u16.to_le_bytes());
    program(&mut image, 0, 1, 5, [0, 0, 0x1000, PAGE_SIZE, PAGE_SIZE]);
    program(
        &mut image,
        1,
        1,
        6,
        [0x4000, 0x4000, 0x200, PAGE_SIZE, PAGE_SIZE],
    );
    program(
        &mut image,
        2,
        1,
        6,
        [OUTPUT, OUTPUT, 0x100, PAGE_SIZE, PAGE_SIZE],
    );
    program(&mut image, 4, 7, 4, [TLS_RVA, TLS_RVA, 32, 48, 32]);
    program(
        &mut image,
        5,
        0x6474_e552,
        4,
        [0, 0x4000, 0, PAGE_SIZE, PAGE_SIZE],
    );
    image[ENTRY] = 0xc3; // Metadata entry only: never executed in these tests.
    put(&mut image, TLS_RVA as usize + 24, &37_u64.to_le_bytes());
    put(&mut image, NAME as usize, b"original-semaphore\0");

    let mut strings = vec![0];
    let mut string = |value: &[u8]| {
        let at = strings.len() as u64;
        strings.extend_from_slice(value);
        strings.push(0);
        at
    };
    let kernel_module = string(b"libkernel");
    let kernel_library = string(b"libkernel");
    let libc_module = string(b"LibcInternal");
    let libc_library = string(b"LibcInternal");
    let sema = Service::registrations()
        .iter()
        .find(|r| r.service == Service::CreateSema)
        .unwrap();
    let mut name = sema.nid.as_bytes().to_vec();
    name.extend_from_slice(b"#D#C"); // Consumer-local library3/module2.
    let kernel_name = string(&name);
    let mut name = KnownFunction::Memcpy.nid().to_vec();
    name.extend_from_slice(b"#F#E"); // Consumer-local library5/module4.
    let libc_name = string(&name);
    put(&mut image, 0x900, &strings);
    for (index, name) in [(1, kernel_name), (2, libc_name)] {
        let at = 0xb00 + index * 24;
        put(&mut image, at, &(name as u32).to_le_bytes());
        image[at + 4] = 0x12; // Undefined GLOBAL FUNC.
        put(
            &mut image,
            0xd00 + (index - 1) * 24,
            &(0x4000 + (index as u64 - 1) * 8).to_le_bytes(),
        );
        put(
            &mut image,
            0xd08 + (index - 1) * 24,
            &(((index as u64) << 32) | 7).to_le_bytes(),
        );
    }
    // A real image-owned LOCAL STT_TLS definition at module-relative offset24.
    image[0xb00 + 3 * 24 + 4] = 6;
    put(&mut image, 0xb00 + 3 * 24 + 6, &1_u16.to_le_bytes());
    put(&mut image, 0xb00 + 3 * 24 + 8, &24_u64.to_le_bytes());
    put(&mut image, 0xb00 + 3 * 24 + 16, &8_u64.to_le_bytes());
    for (index, kind) in [16_u64, 17, 18].into_iter().enumerate() {
        put(
            &mut image,
            0xc00 + index * 24,
            &(TLS_RVA + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut image,
            0xc08 + index * 24,
            &((3_u64 << 32) | kind).to_le_bytes(),
        );
    }
    let entries = [
        (5_i64, 0x900),
        (10, strings.len() as u64),
        (6, 0xb00),
        (39, 4 * 24),
        (11, 24),
        (23, 0xd00),
        (2, 2 * 24),
        (20, 7),
        (7, 0xc00),
        (8, 3 * 24),
        (9, 24),
        (
            0x6100_000f,
            kernel_module | (1 << 32) | (1 << 40) | (2 << 48),
        ),
        (0x6100_0015, kernel_library | (1 << 32) | (3 << 48)),
        (0x6100_000f, libc_module | (1 << 32) | (1 << 40) | (4 << 48)),
        (0x6100_0015, libc_library | (1 << 32) | (5 << 48)),
        (0, 0),
    ];
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(&mut image, 0x800 + index * 16, &tag.to_le_bytes());
        put(&mut image, 0x808 + index * 16, &value.to_le_bytes());
    }
    program(
        &mut image,
        3,
        2,
        4,
        [
            0x800,
            0x800,
            (entries.len() * 16) as u64,
            (entries.len() * 16) as u64,
            8,
        ],
    );
    image
}

// These original images have disjoint page-aligned load segments and one
// sub-page static TLS block per instance. Derive their real image cost instead
// of embedding a previous provider's number of pages in quota tests.
fn initial_mapped_bytes(plan: &KernelStartupPlan<'_>, instances: u64) -> u64 {
    let images: u64 = plan
        .link_plan()
        .modules()
        .iter()
        .flat_map(|module| &module.runtime_plan().segments)
        .map(|segment| {
            ((segment.vaddr + segment.mem_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1))
                - (segment.vaddr & !(PAGE_SIZE - 1))
        })
        .sum();
    let header = u64::from(
        plan.limits.startup.static_tls.thread_control
            != crate::process::tls::ThreadControlBlockPolicy::None,
    );
    images + instances * PAGE_SIZE * (1 + header)
}

fn load_limits(plan: &KernelStartupPlan<'_>) -> NativeLoadLimits {
    let bytes = initial_mapped_bytes(plan, 2) + 3 * PAGE_SIZE;
    NativeLoadLimits {
        session: SessionLoadLimits {
            resident_limit: bytes,
            mapped_limit: bytes,
        },
        reserved_limit: RESERVATION,
    }
}

fn identity() -> ProcessIdentity {
    ProcessIdentity {
        process_id: 41,
        initial_thread_id: 73,
    }
}

mod configured_memory;
mod entry;
mod event_flag_wait;
mod module_symbols;
mod module_unwind;
mod mutex_wait;
#[path = "tests/pad_runtime.rs"]
mod pad_runtime;
mod process_parameters;
mod pthread_semaphore_wait;
mod rwlock_wait;
mod thread_context;
mod user_privacy;

fn with_plan(
    limits: KernelStartupLimits,
    test: impl FnOnce(&KernelStartupPlan<'_>, NativeAddressSpace, u64, [u64; 2]),
) {
    with_consumer(original_consumer(), limits, test)
}
fn with_consumer(
    bytes: Vec<u8>,
    mut limits: KernelStartupLimits,
    test: impl FnOnce(&KernelStartupPlan<'_>, NativeAddressSpace, u64, [u64; 2]),
) {
    let owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), RESERVATION).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    // Admit runtime maps only in this test's actual OS-selected owned arena.
    limits.kernel.virtual_base = base + 0x8_0000;
    limits.kernel.virtual_bytes = 0x4_0000;
    let before = bytes.clone();
    let kernel = build_kernel_provider(limits.max_kernel_provider_bytes).unwrap();
    let libc =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.startup.hle).unwrap();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), base)];
    let plan = plan_kernel_startup(
        &inputs,
        &kernel,
        ProviderPlacement {
            load_bias: base + 0x2_0000,
        },
        &libc,
        ProviderPlacement {
            load_bias: base + 0x5_0000,
        },
        &limits,
    )
    .unwrap();
    test(&plan, owner, base, [base + 0x6_8000, base + 0x7_8000]);
    assert_eq!(bytes, before);
}

fn prepared<'plan, 'image>(
    plan: &'plan KernelStartupPlan<'image>,
    owner: NativeAddressSpace,
    pointers: &[u64],
) -> PreparedNativeKernelStartup<'plan, 'image> {
    prepare_in(
        plan,
        &StaticTlsStorage {
            thread_pointers: pointers,
        },
        &load_limits(plan),
        identity(),
        vec![],
        Some(owner),
    )
    .unwrap()
}

fn word(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64) -> u64 {
    let mut bytes = [0; 8];
    session
        .hle
        .pause()
        .unwrap()
        .read(address, &mut bytes)
        .unwrap();
    u64::from_le_bytes(bytes)
}

fn stop(plan: &KernelStartupPlan<'_>, service: Service, args: [u64; 6]) -> KernelServiceStop {
    let marker = *plan
        .factory
        .markers()
        .iter()
        .find(|m| m.registration.service == service)
        .unwrap();
    KernelServiceStop {
        syscall_pc: plan.entry(marker) + 5,
        service_id: u64::from(marker.service_id),
        args: CallArgs(args),
    }
}

#[test]
fn indexed_kernel_stop_sites_match_the_linear_catalogue_for_every_byte_and_bound() {
    with_plan(KernelStartupLimits::default(), |plan, _, _, _| {
        let first = plan.entry(plan.factory.markers()[0]) + 5;
        let last = plan.entry(*plan.factory.markers().last().unwrap()) + 5;
        // Independent previous lookup is the oracle for every valid start,
        // seven interior bytes, both edges and the preceding stub padding.
        let range = first - 16..=last + 16;
        let extremes = [0, 4, 5, plan.bias - 1, plan.bias, u64::MAX - 5, u64::MAX];
        for pc in range.chain(extremes) {
            let old = plan
                .factory
                .markers()
                .iter()
                .find(|marker| plan.entry(**marker) + 5 == pc);
            let indexed = plan.marker_at_syscall_pc(pc);
            assert_eq!(
                indexed.map(|marker| (marker.rva, marker.service_id, marker.symbol)),
                old.map(|marker| (marker.rva, marker.service_id, marker.symbol)),
                "catalogue selection differs at {pc:#x}"
            );
            if let (Some(indexed), Some(old)) = (indexed, old) {
                assert!(std::ptr::eq(indexed, old));
            }
        }
    });
}

#[test]
fn exact_kernel_provider_catalogue_and_real_got_resolutions() {
    with_plan(KernelStartupLimits::default(), |plan, _, _, _| {
        assert_eq!(
            (
                plan.registration_count(),
                plan.kernel_bindings(),
                plan.hle.provider_bindings()
            ),
            (1399, 1, 1)
        );
        assert_eq!(
            (
                plan.link_plan().modules().len(),
                plan.kernel_provider_index(),
                plan.hle.provider_index()
            ),
            (3, 1, 2)
        );
        assert_eq!(plan.hle.tls_bindings(), 3);
        // Only the two reviewed lifecycle imports have a PentaX v2 binding;
        // neither other SSL functions nor arbitrary module/library versions do.
        for (nid, service) in [
            ("hdpVEUDFW3s", Service::SslInit),
            ("0K1yQ6Lv-Yc", Service::SslTerm),
        ] {
            assert_eq!(
                pentax_kernel::resolve("libSceSsl", (2, 1), "libSceSsl", 1, nid),
                Some(service)
            );
            assert_eq!(
                pentax_kernel::resolve("libSceSsl", (3, 1), "libSceSsl", 1, nid),
                None
            );
            assert_eq!(
                pentax_kernel::resolve("libSceSsl", (2, 1), "libSceSsl", 2, nid),
                None
            );
        }
        assert_eq!(
            pentax_kernel::resolve("libSceSsl", (2, 1), "libSceSsl", 1, "TDfQqO-gMbY"),
            None
        );
        assert_eq!(
            pentax_kernel::resolve("libSceHttp2", (1, 1), "libSceHttp2", 1, "3JCe3lCbQ8A"),
            Some(Service::Http2Init)
        );
        assert_eq!(
            pentax_kernel::resolve("libSceHttp2", (2, 1), "libSceHttp2", 1, "3JCe3lCbQ8A"),
            None
        );
        assert_eq!(
            pentax_kernel::resolve("libSceHttp2", (1, 1), "libSceHttp2", 1, "rbqZig38AT8"),
            None // HTTP2 send is deliberately unavailable, never a success stub.
        );
        assert!(
            plan.link_plan().patches().iter().any(
                |p| matches!(p.resolution(), Resolution::Definition(s) if s.module.index() == 1)
            )
        );
        assert_eq!(
            plan.factory
                .markers()
                .iter()
                .filter(|m| m.registration.arguments == 6)
                .count(),
            17
        );
    });
}

#[test]
fn expanded_provider_keeps_all_source_pages_immutable_and_data_separate() {
    for (metadata, permissions) in [
        (false, NativePermissions::READ_WRITE),
        (true, NativePermissions::READ_WRITE),
        (true, NativePermissions::READ_EXECUTE),
        (true, NativePermissions::READ.with_gpu(false, true)),
    ] {
        with_plan(
            KernelStartupLimits::default(),
            |plan, owner, _, pointers| {
                assert!(plan.factory.image().len() as u64 > PAGE_SIZE);
                let mut session = prepared(plan, owner, &pointers);
                let bias = plan.kernel_provider_bias();
                for offset in (0..CODE_MAPPED_BYTES).step_by(PAGE_SIZE as usize) {
                    let paused = session.pause().unwrap();
                    paused.preflight_read(bias + offset, PAGE_SIZE).unwrap();
                    paused
                        .preflight_fetch_source(bias + offset, PAGE_SIZE)
                        .unwrap();
                    assert!(matches!(
                        paused.preflight_write(bias + offset, 1),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                }
                let data = bias + CODE_MAPPED_BYTES;
                let mut zeros = [0xa5; 512];
                session
                    .pause()
                    .unwrap()
                    .read(data + 16, &mut zeros)
                    .unwrap();
                assert_eq!(zeros, [0; 512]);
                session
                    .pause()
                    .unwrap()
                    .preflight_write(data, PAGE_SIZE)
                    .unwrap();
                session
                    .pause()
                    .unwrap()
                    .preflight_non_executable(data, PAGE_SIZE)
                    .unwrap();
                for offset in (0..plan.factory.metadata_mapped_bytes()).step_by(PAGE_SIZE as usize)
                {
                    let paused = session.pause().unwrap();
                    let page = bias + METADATA_RVA + offset;
                    paused.preflight_read(page, PAGE_SIZE).unwrap();
                    paused.preflight_non_executable(page, PAGE_SIZE).unwrap();
                    assert!(matches!(
                        paused.preflight_write(page, 1),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                    assert!(matches!(
                        paused.preflight_gpu_write(page, 1),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                    assert!(matches!(
                        paused.preflight_fetch_source(page, 1),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                }
                // Even the final metadata/code page must retain its exact permission
                // class when the selected service marker is in the first RX page.
                let target = if metadata {
                    bias + METADATA_RVA + plan.factory.metadata_mapped_bytes() - PAGE_SIZE
                } else {
                    bias + CODE_MAPPED_BYTES - PAGE_SIZE
                };
                session
                    .pause()
                    .unwrap()
                    .apply_transaction(
                        pentax_platform::native_memory::NativeMemoryTransaction {
                            effect: pentax_platform::native_memory::NativeMemoryEffect::Protect {
                                address: target,
                                size: PAGE_SIZE,
                                permissions,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
                assert!(matches!(
                    session.kernel_stop(stop(plan, Service::PthreadGetthreadid, [0; 6])),
                    Err(KernelStartupError::InvalidProvider(_))
                ));
                assert_eq!(session.kernel_usage().attempts, 0);
            },
        );
    }
}

#[test]
fn stack_guard_is_a_real_writable_nonexecuting_object_private_to_each_owner() {
    use crate::process::hle::kernel_provider::{STACK_GUARD_INITIAL, STACK_GUARD_RVA};
    // Original fixture: replace its semaphore import with the pinned data object.
    let mut bytes = original_consumer();
    let row = 0xb00 + 24;
    let name = u32::from_le_bytes(bytes[row..row + 4].try_into().unwrap()) as usize;
    put(&mut bytes, 0x900 + name, b"f7uOxY9mM1U");
    bytes[row + 4] = 0x11; // GLOBAL OBJECT.
    put(&mut bytes, 0xd08, &((1_u64 << 32) | 6).to_le_bytes());
    for _ in 0..2 {
        with_consumer(
            bytes.clone(),
            KernelStartupLimits::default(),
            |plan, owner, base, pointers| {
                let mut session = prepared(plan, owner, &pointers);
                let address = word(&mut session, base + 0x4000);
                assert_eq!(address, plan.kernel_provider_bias() + STACK_GUARD_RVA);
                assert_eq!(word(&mut session, address), STACK_GUARD_INITIAL);
                {
                    let mut paused = session.pause().unwrap();
                    assert!(matches!(
                        paused.fetch_source(address, &mut [0]),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                    paused.write(address, &73_u64.to_le_bytes()).unwrap();
                }
                assert_eq!(word(&mut session, address), 73);
            },
        );
    }
}

#[test]
fn six_original_arguments_and_outputs_use_same_native_tls_and_libc_pages() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            assert!(std::ptr::eq(
                session.memory().source_plan(),
                plan.link_plan()
            ));
            assert_eq!(
                session.memory().mapped_bytes(),
                initial_mapped_bytes(plan, 2)
            );
            let create = stop(
                plan,
                Service::CreateSema,
                [base + OUTPUT, base + NAME, 1, 17, 23, 0],
            );
            assert_eq!(word(&mut session, base + 0x4000), create.syscall_pc - 5);
            assert_eq!(session.kernel_stop(create).unwrap(), 0);
            let handle = word(&mut session, base + OUTPUT);
            assert_ne!(handle, 0);
            // Argument4 really is original RCX=17, not a continuation address.
            assert_eq!(
                session
                    .kernel_stop(stop(plan, Service::PollSema, [handle, 17, 0, 0, 0, 0]))
                    .unwrap(),
                0
            );
            assert_eq!(
                session.kernel_stop(stop(plan, Service::PollSema, [handle, 1, 0, 0, 0, 0])),
                Ok(u64::from(Errno::Busy.kernel_code() as u32))
            );
            for tp in pointers {
                assert_eq!(word(&mut session, tp - TLS_OFFSET), 1);
                assert_eq!(word(&mut session, tp - TLS_OFFSET + 8), 24);
                assert_eq!(word(&mut session, tp - TLS_OFFSET + 16) as i64, -32);
                assert_eq!(word(&mut session, tp - TLS_OFFSET + 24), 37);
                assert_eq!(word(&mut session, tp - TLS_OFFSET + 32), 0);
            }
            let site = plan.hle.hle.sites()[KnownFunction::Memcpy.index()];
            assert_eq!(
                session
                    .libc_stop(NativeServiceStop {
                        rip: site.resume,
                        rcx: site.resume,
                        rax: u64::from(KnownFunction::Memcpy.service_id()),
                        rdi: pointers[0] - TLS_OFFSET + 24,
                        rsi: base + OUTPUT,
                        rdx: 8
                    })
                    .unwrap(),
                pointers[0] - TLS_OFFSET + 24
            );
            assert_eq!(word(&mut session, pointers[0] - TLS_OFFSET + 24), handle);
            assert_eq!(word(&mut session, pointers[1] - TLS_OFFSET + 24), 37);
            assert_eq!(word(&mut session, base + TLS_RVA + 24), 37);
            assert_eq!(
                (
                    session.kernel_usage().handles_live,
                    session.kernel_usage().attempts,
                    session.kernel_stop_attempts(),
                    session.libc_service_attempts()
                ),
                (1, 3, 3, 1)
            );
        },
    );
}

#[test]
fn full_id_exact_trap_and_stop_budgets_are_enforced_before_any_kernel_state() {
    let mut policy = KernelStartupLimits::default();
    policy.kernel.max_attempts = 2;
    with_plan(policy, |plan, owner, base, pointers| {
        let mut session = prepared(plan, owner, &pointers);
        let mut call = stop(
            plan,
            Service::CreateSema,
            [base + OUTPUT, base + NAME, 1, 17, 23, 0],
        );
        call.service_id |= 1 << 32;
        assert!(matches!(
            session.kernel_stop(call),
            Err(KernelStartupError::ServiceIdMismatch { .. })
        ));
        call.service_id &= u32::MAX as u64;
        call.syscall_pc += 2; // RET continuation is not the PRE-SYSCALL site.
        assert!(matches!(
            session.kernel_stop(call),
            Err(KernelStartupError::SiteMismatch { .. })
        ));
        call.syscall_pc -= 2;
        assert_eq!(
            session.kernel_stop(call),
            Err(KernelStartupError::Budget("kernel stop attempts"))
        );
        assert_eq!(
            (
                session.kernel_usage().attempts,
                session.kernel_usage().handles_live
            ),
            (0, 0)
        );
        assert_eq!(word(&mut session, base + OUTPUT), 0);
    });
}

#[test]
fn provider_relro_and_guards_stay_immutable_while_pthread_prefix_is_real() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            assert!(matches!(
                session.hle.pause().unwrap().write(plan.bias, &[0]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                session.hle.pause().unwrap().write(base + 0x4000, &[0]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            let prefix = session
                .kernel_stop(stop(plan, Service::PthreadSelf, [0; 6]))
                .unwrap();
            assert_eq!(prefix, base + 0x8_0000);
            assert_eq!(word(&mut session, prefix), 73);
            let mut tail = [1; 4092];
            session
                .hle
                .pause()
                .unwrap()
                .read(prefix + 4, &mut tail)
                .unwrap();
            assert_eq!(tail, [0; 4092]);
            assert_eq!(
                session
                    .kernel_stop(stop(plan, Service::PthreadSelf, [0; 6]))
                    .unwrap(),
                prefix
            );
            assert_eq!(session.kernel_usage().handles_live, 1);
            for index in 0..session.tls().instances().len() {
                let guards = session.tls().instances()[index].guard_pages.unwrap();
                for guard in guards {
                    assert!(
                        session
                            .hle
                            .memory()
                            .space
                            .reserved_ranges()
                            .any(|r| r.address <= guard && guard + PAGE_SIZE <= r.address + r.size)
                    );
                    assert!(matches!(
                        session.hle.pause().unwrap().read(guard, &mut [0]),
                        Err(NativeMemoryError::Unmapped(_))
                    ));
                    assert!(matches!(
                        session.hle.pause().unwrap().write(guard, &[0]),
                        Err(NativeMemoryError::Unmapped(_))
                    ));
                }
            }
        },
    );
}

#[test]
fn selected_unsupported_sixth_argument_and_bad_output_publish_no_handle() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            assert!(matches!(
                session.kernel_stop(stop(
                    plan,
                    Service::CreateSema,
                    [base + OUTPUT, base + NAME, 1, 17, 23, 1]
                )),
                Err(KernelStartupError::Kernel(Failure::Unsupported(_)))
            ));
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::CreateSema,
                    [base + 0x4000, base + NAME, 1, 17, 23, 0]
                )),
                Err(KernelStartupError::Kernel(Failure::Guest(
                    GuestError::WriteDenied
                )))
            );
            assert_eq!(session.kernel_usage().handles_live, 0);
            assert_eq!(word(&mut session, base + OUTPUT), 0);
        },
    );
}

#[test]
fn native_adapter_preflights_entire_output_batch_and_rejects_overlap_and_effects() {
    let mut space =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x1_0000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, PAGE_SIZE, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = space.pause().unwrap();
    paused.write(base, &[0x55; 16]).unwrap();
    let mut adapter = KernelMemory {
        paused: &mut paused,
        source: None,
        supplied_modules: 0,
        lookup_limits: None,
        lookup_work: None,
        loaded_modules: None,
        thread_tls: None,
        initial_entry: None,
    };
    let writes = [
        GuestWrite {
            address: base,
            bytes: &[1; 8],
        },
        GuestWrite {
            address: base + PAGE_SIZE,
            bytes: &[2; 8],
        },
    ];
    assert_eq!(
        adapter.apply(MemoryEffect::None, &writes),
        Err(GuestError::Unmapped)
    );
    let mut actual = [0; 16];
    adapter.read(base, &mut actual).unwrap();
    assert_eq!(actual, [0x55; 16]);
    let writes = [
        GuestWrite {
            address: base,
            bytes: &[1; 8],
        },
        GuestWrite {
            address: base + 4,
            bytes: &[2; 8],
        },
    ];
    assert_eq!(
        adapter.apply(MemoryEffect::None, &writes),
        Err(GuestError::WriteDenied)
    );
    assert_eq!(
        adapter.apply(
            MemoryEffect::MapDirect {
                address: base + PAGE_SIZE,
                length: PAGE_SIZE,
                protection: 3,
                physical: 0,
            },
            &writes
        ),
        Err(GuestError::AddressOverflow)
    );
    adapter.read(base, &mut actual).unwrap();
    assert_eq!(actual, [0x55; 16]);
    adapter
        .apply(
            MemoryEffect::None,
            &[
                GuestWrite {
                    address: base,
                    bytes: &[1; 8],
                },
                GuestWrite {
                    address: base + 8,
                    bytes: &[2; 8],
                },
            ],
        )
        .unwrap();
    adapter.read(base, &mut actual).unwrap();
    assert_eq!(actual, [1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2]);
}

#[test]
fn competing_exact_kernel_copy_is_ambiguous_and_tighter_byte_work_limits_refuse() {
    let bytes = original_consumer();
    let policy = KernelStartupLimits::default();
    let kernel =
        build_kernel_provider(KernelStartupLimits::default().max_kernel_provider_bytes).unwrap();
    let copy = kernel.image().to_vec();
    let libc =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &policy.startup.hle).unwrap();
    let inputs = [
        ModuleInput::new(open_executable(&bytes).unwrap(), 0x10_0000),
        ModuleInput::new(open_executable(&copy).unwrap(), 0x18_0000),
    ];
    assert!(matches!(
        plan_kernel_startup(
            &inputs,
            &kernel,
            ProviderPlacement {
                load_bias: 0x20_0000
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x30_0000
            },
            &policy
        ),
        Err(KernelStartupError::Hle(HleError::Link(
            SymbolicError::AmbiguousProvider { .. }
        )))
    ));
    let mut tight = policy;
    tight.max_kernel_provider_bytes = kernel.image().len() as u64 - 1;
    assert!(matches!(
        plan_kernel_startup(
            &inputs[..1],
            &kernel,
            ProviderPlacement {
                load_bias: 0x20_0000
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x30_0000
            },
            &tight
        ),
        Err(KernelStartupError::Budget("kernel provider bytes"))
    ));
    tight = policy;
    tight.max_provider_work = 0;
    assert!(matches!(
        plan_kernel_startup(
            &inputs[..1],
            &kernel,
            ProviderPlacement {
                load_bias: 0x20_0000
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x30_0000
            },
            &tight
        ),
        Err(KernelStartupError::Budget("provider work"))
    ));
}

#[test]
fn altered_live_provider_source_is_rejected_before_service_admission() {
    with_plan(KernelStartupLimits::default(), |plan, mut owner, _, _| {
        owner
            .map_at(plan.bias, CODE_MAPPED_BYTES, NativePermissions::READ_WRITE)
            .unwrap();
        let mut bytes = plan.factory.code_bytes().to_vec();
        let marker = plan.factory.markers()[0];
        bytes[marker.rva as usize + 1] ^= 1;
        owner.pause().unwrap().write(plan.bias, &bytes).unwrap();
        owner
            .protect(
                plan.bias,
                CODE_MAPPED_BYTES,
                NativePermissions::READ_EXECUTE,
            )
            .unwrap();
        assert_eq!(
            attest(&owner.pause().unwrap(), plan),
            Err(KernelStartupError::InvalidProvider(
                "kernel live marker bytes"
            ))
        );
    });
}

#[test]
fn unknown_typed_relocations_never_publish_a_startup_plan() {
    let policy = KernelStartupLimits::default();
    let kernel =
        build_kernel_provider(KernelStartupLimits::default().max_kernel_provider_bytes).unwrap();
    let libc =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &policy.startup.hle).unwrap();
    let mut bytes = original_consumer();
    put(&mut bytes, 0xc08, &((3_u64 << 32) | 19).to_le_bytes());
    let before = bytes.clone();
    let input = [ModuleInput::new(
        open_executable(&bytes).unwrap(),
        0x10_0000,
    )];
    assert!(matches!(
        plan_kernel_startup(
            &input,
            &kernel,
            ProviderPlacement {
                load_bias: 0x20_0000
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x30_0000
            },
            &policy
        ),
        Err(KernelStartupError::Hle(HleError::Tls(
            crate::process::tls::TlsError::RemainingDeferredRelocations { count: 1, .. }
        )))
    ));
    assert_eq!(bytes, before);
}

#[test]
fn unrelated_relative_writes_do_not_multiply_provider_manifest_work() {
    let mut bytes = original_consumer();
    // Move the original TLS records to a larger table in initialized RX data;
    // append 256 ordinary relative writes to the consumer's writable BSS.
    let original_tls = bytes[0xc00..0xc00 + 72].to_vec();
    put(&mut bytes, 0x1000, &original_tls);
    program(&mut bytes, 0, 1, 5, [0, 0, 0x3000, PAGE_SIZE, PAGE_SIZE]);
    put(&mut bytes, 0x800 + 8 * 16 + 8, &0x1000_u64.to_le_bytes());
    put(
        &mut bytes,
        0x800 + 9 * 16 + 8,
        &((3_u64 + 256) * 24).to_le_bytes(),
    );
    for index in 0..256 {
        let at = 0x1000 + (3 + index) * 24;
        put(
            &mut bytes,
            at,
            &(OUTPUT + 0x100 + index as u64 * 8).to_le_bytes(),
        );
        put(&mut bytes, at + 8, &8_u64.to_le_bytes());
        put(&mut bytes, at + 16, &0x123_u64.to_le_bytes());
    }
    let before = bytes.clone();
    let mut policy = KernelStartupLimits::default();
    let kernel =
        build_kernel_provider(KernelStartupLimits::default().max_kernel_provider_bytes).unwrap();
    let libc =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &policy.startup.hle).unwrap();
    let input = [ModuleInput::new(
        open_executable(&bytes).unwrap(),
        0x10_0000,
    )];
    // One module visit each, one outer visit per patch, and exactly ONE
    // manifest scan for the kernel binding. Relative writes must not scan it.
    policy.max_provider_work =
        (input.len() + 2) as u64 + 259 + (kernel.markers().len() + kernel.objects().len()) as u64;
    let check = |policy: &KernelStartupLimits| {
        plan_kernel_startup(
            &input,
            &kernel,
            ProviderPlacement {
                load_bias: 0x20_0000,
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x30_0000,
            },
            policy,
        )
    };
    let plan = check(&policy).unwrap();
    assert_eq!(plan.kernel_bindings(), 1);
    assert_eq!(plan.link_plan().patch_count(), 259);
    assert_eq!(
        plan.link_plan()
            .patches()
            .iter()
            .filter(|p| p.resolution() == Resolution::Relative)
            .count(),
        257
    );
    policy.max_provider_work -= 1;
    assert!(matches!(
        check(&policy),
        Err(KernelStartupError::Budget("provider work"))
    ));
    assert_eq!(bytes, before);
}

#[test]
fn native_eager_budget_failure_drops_unpublished_session_without_touching_sources() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, _, pointers| {
            let mut limits = load_limits(plan);
            limits.session.resident_limit = initial_mapped_bytes(plan, 2) - PAGE_SIZE;
            assert!(matches!(
                prepare_in(
                    plan,
                    &StaticTlsStorage {
                        thread_pointers: &pointers
                    },
                    &limits,
                    identity(),
                    vec![],
                    Some(owner)
                ),
                Err(KernelStartupError::Hle(HleError::Tls(_)))
            ));
        },
    );
}

#[test]
fn flexible_mapping_query_and_protect_change_the_same_actual_native_pages() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let prefix = session
                .kernel_stop(stop(plan, Service::PthreadSelf, [0; 6]))
                .unwrap();
            assert_eq!(prefix, base + 0x8_0000);
            assert_eq!(
                session
                    .kernel_stop(stop(
                        plan,
                        Service::MapFlexible,
                        [base + OUTPUT, 2 * PAGE_SIZE, 3, 0, 0, 0]
                    ))
                    .unwrap(),
                0
            );
            let mapped = word(&mut session, base + OUTPUT);
            assert_eq!(mapped, prefix + PAGE_SIZE);
            let mut actual = vec![1; 2 * PAGE_SIZE as usize];
            session
                .hle
                .pause()
                .unwrap()
                .read(mapped, &mut actual)
                .unwrap();
            assert!(actual.iter().all(|byte| *byte == 0));
            session
                .hle
                .pause()
                .unwrap()
                .write(mapped + PAGE_SIZE - 2, b"same-pages")
                .unwrap();
            assert_eq!(
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [mapped, 2 * PAGE_SIZE, 1, 0, 0, 0]
                    ))
                    .unwrap(),
                0
            );
            assert!(matches!(
                session.hle.pause().unwrap().write(mapped, &[1]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            // The fourth original SysV argument is the third real output pointer.
            assert_eq!(
                session
                    .kernel_stop(stop(
                        plan,
                        Service::QueryProtection,
                        [
                            mapped + 1,
                            base + OUTPUT + 16,
                            base + OUTPUT + 24,
                            base + OUTPUT + 32,
                            0,
                            0
                        ]
                    ))
                    .unwrap(),
                0
            );
            assert_eq!(word(&mut session, base + OUTPUT + 16), mapped);
            assert_eq!(
                word(&mut session, base + OUTPUT + 24),
                mapped + 2 * PAGE_SIZE
            );
            assert_eq!(word(&mut session, base + OUTPUT + 32), 1);
            assert_eq!(
                session
                    .kernel_stop(stop(
                        plan,
                        Service::Mprotect,
                        [mapped, 2 * PAGE_SIZE, 3, 0, 0, 0]
                    ))
                    .unwrap(),
                0
            );
            session
                .hle
                .pause()
                .unwrap()
                .write(mapped, b"writable")
                .unwrap();
            let mut tail = [0; 10];
            session
                .hle
                .pause()
                .unwrap()
                .read(mapped + PAGE_SIZE - 2, &mut tail)
                .unwrap();
            assert_eq!(&tail, b"same-pages");
            assert_eq!(
                session.memory().mapped_bytes(),
                initial_mapped_bytes(plan, 2) + 3 * PAGE_SIZE
            );
            assert_eq!(
                (
                    session.kernel_usage().handles_live,
                    session.kernel_usage().mapped_bytes
                ),
                (2, 3 * PAGE_SIZE)
            );
        },
    );
}

#[test]
fn invalid_map_outputs_and_invalid_protection_publish_no_allocations() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let before = session.memory().mapped_bytes();
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::MapFlexible,
                    [base + 0x4000, PAGE_SIZE, 3, 0, 0, 0]
                )),
                Err(KernelStartupError::Kernel(Failure::Guest(
                    GuestError::WriteDenied
                )))
            );
            assert_eq!(session.memory().mapped_bytes(), before);
            assert_eq!(
                (
                    session.kernel_usage().mapped_bytes,
                    session.kernel_usage().handles_live
                ),
                (0, 0)
            );
            assert!(matches!(
                session.hle.pause().unwrap().read(base + 0x8_0000, &mut [0]),
                Err(NativeMemoryError::Unmapped(_))
            ));
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::MapFlexible,
                    [base + OUTPUT, PAGE_SIZE, 0x08, 0, 0, 0]
                )),
                Ok(0x8002_0016)
            );
            assert_eq!(
                session.kernel_stop(stop(
                    plan,
                    Service::AllocateDirect,
                    [0, 0x10_0000, PAGE_SIZE, PAGE_SIZE, 3, base + 0x4000]
                )),
                Err(KernelStartupError::Kernel(Failure::Guest(
                    GuestError::WriteDenied
                )))
            );
            assert_eq!(word(&mut session, base + OUTPUT), 0);
            assert_eq!(
                session
                    .kernel_stop(stop(
                        plan,
                        Service::MapFlexible,
                        [base + OUTPUT, PAGE_SIZE, 3, 0, 0, 0]
                    ))
                    .unwrap(),
                0
            );
            assert_eq!(word(&mut session, base + OUTPUT), base + 0x8_0000);
        },
    );
}

#[test]
fn runtime_prefix_maps_obey_the_retained_total_eager_native_budget() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, _, pointers| {
            let mut limits = load_limits(plan);
            let baseline = initial_mapped_bytes(plan, 2);
            limits.session.resident_limit = baseline;
            limits.session.mapped_limit = baseline;
            let mut session = prepare_in(
                plan,
                &StaticTlsStorage {
                    thread_pointers: &pointers,
                },
                &limits,
                identity(),
                vec![],
                Some(owner),
            )
            .unwrap();
            assert_eq!(
                session.kernel_stop(stop(plan, Service::PthreadSelf, [0; 6])),
                Err(KernelStartupError::Kernel(Failure::Guest(
                    GuestError::BudgetExceeded
                )))
            );
            assert_eq!(session.memory().mapped_bytes(), baseline);
            let statistics = session.memory().statistics();
            assert_eq!(statistics.mapped_bytes, baseline);
            assert_eq!(statistics.committed_bytes, baseline);
            assert_eq!(statistics.limits.mapped_bytes, baseline);
            assert_eq!(
                (
                    session.kernel_usage().mapped_bytes,
                    session.kernel_usage().handles_live
                ),
                (0, 0)
            );
        },
    );
}

#[test]
fn thread_destructor_registration_uses_the_attested_owner_and_preserves_its_actual_guest_target() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let callback = base + ENTRY as u64;
            assert_eq!(session.kernel.thread_lifecycle_callbacks().destructor, None);
            session
                .kernel_stop(stop(
                    plan,
                    Service::SetThreadDtors,
                    [callback, 0, 0, 0, 0, 0],
                ))
                .unwrap();
            assert_eq!(
                session.kernel.thread_lifecycle_callbacks().destructor,
                Some(callback)
            );
            assert!(matches!(
                session.kernel_stop(stop(plan, Service::SetThreadDtors, [0; 6])),
                Err(KernelStartupError::Kernel(Failure::Unsupported(
                    "thread lifecycle callback replacement"
                )))
            ));
            assert_eq!(
                session.kernel.thread_lifecycle_callbacks().destructor,
                Some(callback)
            );
        },
    );
}

#[test]
fn application_heap_registration_reads_the_actual_guest_table_and_preserves_snapshot_on_failure() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let address = base + OUTPUT + 0x80;
            let mut table = [0; 80];
            for (slot, value) in [
                (0, base + ENTRY as u64),
                (1, base + ENTRY as u64 + 1),
                (6, base + ENTRY as u64 + 2),
            ] {
                table[slot * 8..slot * 8 + 8].copy_from_slice(&value.to_le_bytes());
            }
            session.pause().unwrap().write(address, &table).unwrap();
            session
                .kernel_stop(stop(
                    plan,
                    Service::SetApplicationHeapApi,
                    [address, 0, 0, 0, 0, 0],
                ))
                .unwrap();
            let callbacks = session.kernel.application_heap_callbacks().unwrap();
            assert_eq!(callbacks.malloc, base + ENTRY as u64);
            assert_eq!(callbacks.free, base + ENTRY as u64 + 1);
            assert_eq!(callbacks.posix_memalign, Some(base + ENTRY as u64 + 2));
            assert!(
                session
                    .kernel_stop(stop(
                        plan,
                        Service::SetApplicationHeapApi,
                        [base + OUTPUT + PAGE_SIZE - 72, 0, 0, 0, 0, 0]
                    ))
                    .is_err()
            );
            assert!(session.kernel.application_heap_callbacks() == Some(callbacks));
            session.pause().unwrap().write(address, &[0; 80]).unwrap();
            assert!(session.kernel.application_heap_callbacks() == Some(callbacks));
            session
                .kernel_stop(stop(
                    plan,
                    Service::SetApplicationHeapApi,
                    [address, 0, 0, 0, 0, 0],
                ))
                .unwrap();
            assert!(session.kernel.application_heap_callbacks().is_none());
        },
    );
}

#[path = "tests/provider_permissions.rs"]
mod provider_permissions;
