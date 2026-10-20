// SPDX-License-Identifier: GPL-2.0-only

#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

use super::*;
use crate::memory::{MemoryError, PAGE_SIZE};
use crate::process::hle::fixture::{CODE, Consumer, Import};
use crate::process::hle::{
    HleLimits, HleProfile, KnownFunction, MemoryServiceError, ProviderPlacement, ServiceLimit,
    build_hle_providers, plan_hle_session,
};
use crate::process::{ProcessError, SessionLoadLimits};
use pentax_linker::ModuleInput;
use pentax_loader::open_executable;
use pentax_platform::native_memory::{NativeMemoryLimits, NativePermissions};

fn with_native(
    limits: HleLimits,
    test: impl FnOnce(&mut PreparedNativeHleSession<'_, '_>, u64, u64),
) {
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let bias = owner.reserved_ranges().next().unwrap().address;
    let providers = build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits).unwrap();
    let mut consumer = Consumer::importing(vec![(CODE, vec![0xcc])]);
    consumer.imports = KnownFunction::LIBC.map(Import::function).to_vec();
    let bytes = consumer.build().unwrap();
    let before = bytes.clone();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), bias)];
    let plan = plan_hle_session(
        &inputs,
        &providers,
        ProviderPlacement {
            load_bias: bias + 0x20000,
        },
        &limits,
    )
    .unwrap();
    let mut prepared = prepare_native_in(
        &plan,
        &NativeLoadLimits {
            session: SessionLoadLimits {
                resident_limit: 8 * PAGE_SIZE,
                mapped_limit: 8 * PAGE_SIZE,
            },
            reserved_limit: 0x40000,
        },
        Some(owner),
    )
    .unwrap();
    let data = bias + 0x10000;
    prepared
        .memory
        .space
        .map_at(data, PAGE_SIZE, NativePermissions::READ_WRITE)
        .unwrap();
    test(&mut prepared, bias, data);
    assert_eq!(bytes, before);
}
fn stop(
    session: &PreparedNativeHleSession<'_, '_>,
    function: KnownFunction,
    rdi: u64,
    rsi: u64,
    rdx: u64,
) -> NativeServiceStop {
    let site = session
        .plan
        .sites()
        .iter()
        .find(|site| site.function == function)
        .unwrap();
    NativeServiceStop {
        rip: site.resume,
        rcx: site.resume,
        rax: u64::from(function.service_id()),
        rdi,
        rsi,
        rdx,
    }
}
fn call(
    session: &mut PreparedNativeHleSession<'_, '_>,
    function: KnownFunction,
    rdi: u64,
    rsi: u64,
    rdx: u64,
) -> Result<u64, HleError> {
    let stop = stop(session, function, rdi, rsi, rdx);
    session.service_stop(stop)
}
fn read(session: &mut PreparedNativeHleSession<'_, '_>, at: u64, size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    session
        .memory
        .space
        .pause()
        .unwrap()
        .read(at, &mut bytes)
        .unwrap();
    bytes
}

#[test]
fn all_thirteen_hle_functions_operate_on_the_actual_owned_native_pages() {
    with_native(HleLimits::default(), |s, bias, data| {
        {
            let mut p = s.pause().unwrap();
            p.write(data, b"hello\0").unwrap();
            p.write(data + 0x300, b"hello\0").unwrap();
            p.write(data + 0x600, b"pre:\0").unwrap();
            p.write(data + 0x700, b"ell\0").unwrap();
        }
        assert_eq!(
            call(s, KnownFunction::Memset, data + 0x100, 0x5a, 4).unwrap(),
            data + 0x100
        );
        assert_eq!(
            call(s, KnownFunction::Memcpy, data + 0x200, data, 6).unwrap(),
            data + 0x200
        );
        assert_eq!(
            call(s, KnownFunction::Memmove, data + 0x201, data + 0x200, 5).unwrap(),
            data + 0x201
        );
        assert_eq!(
            call(s, KnownFunction::Memcmp, data, data + 0x300, 6).unwrap(),
            0
        );
        assert_eq!(
            call(s, KnownFunction::Strcmp, data, data + 0x300, 0).unwrap(),
            0
        );
        assert_eq!(
            call(s, KnownFunction::Strncmp, data, data + 0x300, 3).unwrap(),
            0
        );
        assert_eq!(call(s, KnownFunction::Strlen, data, 0, 0).unwrap(), 5);
        assert_eq!(
            call(s, KnownFunction::Strcpy, data + 0x400, data, 0).unwrap(),
            data + 0x400
        );
        assert_eq!(
            call(s, KnownFunction::Strncpy, data + 0x500, data, 8).unwrap(),
            data + 0x500
        );
        assert_eq!(
            call(s, KnownFunction::Strcat, data + 0x600, data, 0).unwrap(),
            data + 0x600
        );
        assert_eq!(
            call(s, KnownFunction::Strchr, data, u64::from(b'l'), 0).unwrap(),
            data + 2
        );
        assert_eq!(
            call(s, KnownFunction::Strrchr, data, u64::from(b'l'), 0).unwrap(),
            data + 3
        );
        assert_eq!(
            call(s, KnownFunction::Strstr, data + 0x600, data + 0x700, 0).unwrap(),
            data + 0x605
        );
        assert_eq!(read(s, data + 0x100, 4), b"ZZZZ");
        assert_eq!(read(s, data + 0x200, 6), b"hhello");
        assert_eq!(read(s, data + 0x400, 6), b"hello\0");
        assert_eq!(read(s, data + 0x500, 8), b"hello\0\0\0");
        assert_eq!(read(s, data + 0x600, 10), b"pre:hello\0");
        assert_eq!(s.budget.attempts(), 13);
        assert_eq!(s.budget.completed(), 13);
        for f in KnownFunction::LIBC {
            assert_eq!(s.budget.calls(f), 1);
        }
        assert!(std::ptr::eq(s.memory().source_plan(), s.plan().link_plan()));
        assert_eq!(s.memory().applied_relocations(), 13);
        for f in KnownFunction::LIBC {
            let at = bias + 0x4000 + 8 * f.index() as u64;
            let expected = s.plan.sites()[f.index()].entry;
            assert_eq!(
                u64::from_le_bytes(read(s, at, 8).try_into().unwrap()),
                expected
            );
        }
        let provider = s.plan.provider_bias();
        let mut original = [0; 8];
        s.pause()
            .unwrap()
            .fetch_source(provider + 0x100, &mut original)
            .unwrap();
        assert_eq!(
            original,
            super::super::provider::stub(KnownFunction::Memcpy)
        );
        assert!(matches!(
            s.pause().unwrap().write(provider + 0x100, &[0]),
            Err(pentax_platform::native_memory::NativeMemoryError::PermissionDenied(_))
        ));
    });
}

#[test]
fn pre_marker_snprintf_forwards_three_gp_arguments_and_guest_stack_without_inventing_legacy_context()
 {
    with_native(HleLimits::default(), |s, _, data| {
        let format = data + 0x100;
        let output = data + 0x200;
        let stack = data + 0x308;
        {
            let mut paused = s.pause().unwrap();
            paused.write(format, b"%u/%u/%u/%llu\0").unwrap();
            paused.write(output - 1, &[0xa7; 32]).unwrap();
            // This is a direct routing fixture over owned paused pages, not
            // execution evidence. The first overflow word follows callee RSP.
            paused.write(stack, &0x1234u64.to_le_bytes()).unwrap();
            paused.write(stack + 8, &44u64.to_le_bytes()).unwrap();
        }
        let site = *s
            .plan
            .sites()
            .iter()
            .find(|site| site.function == KnownFunction::Snprintf)
            .unwrap();
        let captured = NativeLibcCall {
            args: [output, 32, format, 11, 22, 33],
            vector_args: None, // Integer conversions never need an XMM read.
            stack_pointer: Some(stack),
        };
        assert_eq!(
            pre_service_stop(
                s.plan,
                &mut s.memory,
                &mut s.budget,
                site.trap,
                site.resume,
                u64::from(KnownFunction::Snprintf.service_id()),
                captured
            ),
            Ok(11)
        );
        assert_eq!(read(s, output, 12), b"11/22/33/44\0");
        assert_eq!(read(s, output - 1, 1), [0xa7]);
        assert_eq!(read(s, output + 12, 1), [0xa7]);
        let before = read(s, output - 1, 32);
        let incomplete = NativeLibcCall {
            stack_pointer: None,
            ..captured
        };
        assert!(matches!(
            pre_service_stop(
                s.plan,
                &mut s.memory,
                &mut s.budget,
                site.trap,
                site.resume,
                u64::from(KnownFunction::Snprintf.service_id()),
                incomplete
            ),
            Err(HleError::Service(MemoryServiceError::InvalidServiceState(
                _
            )))
        ));
        assert_eq!(read(s, output - 1, 32), before);
        // The old post-SYSCALL route has no original RCX/vector/stack capture.
        assert!(matches!(
            call(s, KnownFunction::Snprintf, output, 32, format),
            Err(HleError::Service(MemoryServiceError::InvalidServiceState(
                _
            )))
        ));
        assert_eq!(read(s, output - 1, 32), before);
        assert_eq!(s.budget.attempts(), 3);
        assert_eq!(s.budget.calls(KnownFunction::Snprintf), 1);
    });
}

#[test]
fn native_memcpy_and_string_late_faults_do_not_partially_write() {
    with_native(HleLimits::default(), |s, _, data| {
        s.pause().unwrap().write(data, b"hello\0").unwrap();
        let edge = data + PAGE_SIZE - 2;
        s.pause().unwrap().write(edge, b"XY").unwrap();
        assert!(matches!(
            call(s, KnownFunction::Memcpy, edge, data, 6),
            Err(HleError::Service(MemoryServiceError::Memory(
                MemoryError::Unmapped(_)
            )))
        ));
        assert_eq!(read(s, edge, 2), b"XY");
        assert!(matches!(
            call(s, KnownFunction::Strcpy, edge, data, 0),
            Err(HleError::Service(MemoryServiceError::Memory(
                MemoryError::Unmapped(_)
            )))
        ));
        assert_eq!(read(s, edge, 2), b"XY");
        assert_eq!(s.budget.completed(), 0);
        assert_eq!(s.budget.attempts(), 2);
        s.memory
            .space
            .map_at(data + PAGE_SIZE, PAGE_SIZE, NativePermissions::READ)
            .unwrap();
        assert!(matches!(
            call(s, KnownFunction::Memset, edge, 0, 4),
            Err(HleError::Service(MemoryServiceError::Memory(
                MemoryError::PermissionDenied(_)
            )))
        ));
        assert_eq!(read(s, edge, 2), b"XY");
    });
}

#[test]
fn native_services_keep_source_bound_site_and_full_id_checks() {
    with_native(HleLimits::default(), |s, _, data| {
        s.pause().unwrap().write(data, b"keep\0").unwrap();
        let mut trap = stop(s, KnownFunction::Memset, data, 0, 4);
        trap.rax |= 1 << 32;
        assert!(matches!(
            s.service_stop(trap),
            Err(HleError::ServiceIdMismatch { .. })
        ));
        trap = stop(s, KnownFunction::Memset, data, 0, 4);
        trap.rcx += 1;
        assert!(matches!(
            s.service_stop(trap),
            Err(HleError::ContinuationMismatch { .. })
        ));
        trap = stop(s, KnownFunction::Memset, data, 0, 4);
        trap.rip = data;
        assert!(matches!(
            s.service_stop(trap),
            Err(HleError::SiteMismatch { .. })
        ));
        assert_eq!(read(s, data, 5), b"keep\0");
        assert_eq!(s.budget.attempts(), 3);
        assert_eq!(s.budget.completed(), 0);
    });
}

#[test]
fn native_services_share_one_retained_budget_and_allow_repeated_valid_calls() {
    let mut limits = HleLimits::default();
    limits.services.max_attempts = 2;
    with_native(limits, |s, _, data| {
        for _ in 0..2 {
            assert_eq!(call(s, KnownFunction::Memset, data, 0x5a, 1).unwrap(), data);
        }
        assert!(matches!(
            call(s, KnownFunction::Memset, data, 0, 1),
            Err(HleError::Service(MemoryServiceError::LimitExceeded {
                kind: ServiceLimit::Attempts,
                ..
            }))
        ));
        assert_eq!(read(s, data, 1), b"Z");
        assert_eq!(s.budget.completed(), 2);
    });
}

#[test]
fn native_preparation_refuses_deferred_records() {
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let bias = owner.reserved_ranges().next().unwrap().address;
    let limits = HleLimits::default();
    let providers = build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits).unwrap();
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.imports[0].relocation = 42;
    let bytes = original.build().unwrap();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), bias)];
    let plan = plan_hle_session(
        &inputs,
        &providers,
        ProviderPlacement {
            load_bias: bias + 0x20000,
        },
        &limits,
    )
    .unwrap();
    assert!(matches!(
        prepare_native_in(&plan, &NativeLoadLimits::default(), Some(owner)),
        Err(HleError::Staging(ProcessError::NativeDeferredRelocations {
            count: 1
        }))
    ));
}

#[test]
fn eager_commit_and_reserved_budgets_are_enforced_before_native_publication() {
    for reserved_limit in [0x40000, 0x30000] {
        let owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
        let bias = owner.reserved_ranges().next().unwrap().address;
        let limits = HleLimits::default();
        let providers =
            build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits).unwrap();
        let original = Consumer::importing(vec![(CODE, vec![0xcc])])
            .build()
            .unwrap();
        let inputs = [ModuleInput::new(open_executable(&original).unwrap(), bias)];
        let plan = plan_hle_session(
            &inputs,
            &providers,
            ProviderPlacement {
                load_bias: bias + 0x20000,
            },
            &limits,
        )
        .unwrap();
        let resident_limit = if reserved_limit == 0x40000 {
            2 * PAGE_SIZE
        } else {
            8 * PAGE_SIZE
        };
        assert!(matches!(
            prepare_native_in(
                &plan,
                &NativeLoadLimits {
                    session: SessionLoadLimits {
                        resident_limit,
                        mapped_limit: 8 * PAGE_SIZE
                    },
                    reserved_limit
                },
                Some(owner)
            ),
            Err(HleError::Staging(ProcessError::Memory(
                MemoryError::BudgetExceeded
            )))
        ));
    }
}

#[test]
fn retained_tls_source_is_explicitly_refused_by_initial_native_builder() {
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.imports.clear();
    let mut bytes = original.build().unwrap();
    bytes[56..58].copy_from_slice(&5_u16.to_le_bytes());
    super::super::provider::program(&mut bytes, 4, 7, 4, [0x4080, 0x4080, 8, 16, 8]);
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let bias = owner.reserved_ranges().next().unwrap().address;
    let source = pentax_linker::plan_symbolic(
        &[ModuleInput::new(open_executable(&bytes).unwrap(), bias)],
        &pentax_linker::SymbolicLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        prepare_in(&source, &NativeLoadLimits::default(), Some(owner)),
        Err(ProcessError::NativeUnsupportedTls { module: 0 })
    ));
}
