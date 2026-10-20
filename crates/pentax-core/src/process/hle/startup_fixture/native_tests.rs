// SPDX-License-Identifier: GPL-2.0-only

//! Data-only native preparation of the existing original three-image profile.
//! No CPU, native entry, host FS or executable conversion participates.

use super::*;
use crate::memory::native::NativeLoadLimits;
use crate::process::hle::native::NativeServiceStop;
use crate::process::hle::native_startup::{PreparedNativeHleStartup, prepare_native_startup_in};
use crate::process::tls::{TlsBudget, TlsError};
use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryError, NativeMemoryLimits};

const RESERVATION: u64 = 0x8_0000;

fn limits() -> NativeLoadLimits {
    NativeLoadLimits {
        session: SessionLoadLimits {
            resident_limit: 8 * PAGE_SIZE,
            mapped_limit: 8 * PAGE_SIZE,
        },
        reserved_limit: RESERVATION,
    }
}

fn with_plan(
    policy: HleStartupLimits,
    test: impl FnOnce(&HleStartupPlan<'_>, NativeAddressSpace, u64, [u64; 2]),
) {
    with_custom_plan(policy, |_| {}, test);
}

fn with_custom_plan(
    policy: HleStartupLimits,
    change: impl FnOnce(&mut image::Images),
    test: impl FnOnce(&HleStartupPlan<'_>, NativeAddressSpace, u64, [u64; 2]),
) {
    let owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), RESERVATION).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let mut original = images().unwrap();
    change(&mut original);
    let before = (original.consumer.clone(), original.provider.clone());
    let providers =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &policy.hle).unwrap();
    let input = [
        ModuleInput::new(open_executable(&original.consumer).unwrap(), base),
        ModuleInput::new(
            open_executable(&original.provider).unwrap(),
            base + 0x2_0000,
        ),
    ];
    let plan = plan_hle_startup(
        &input,
        &providers,
        ProviderPlacement {
            load_bias: base + 0x4_0000,
        },
        &policy,
    )
    .unwrap();
    test(&plan, owner, base, [base + 0x6_8000, base + 0x7_8000]);
    assert_eq!((original.consumer, original.provider), before);
}

#[test]
fn native_budget_counts_uninitialized_image_pages_that_sparse_backing_leaves_lazy() {
    with_custom_plan(
        HleStartupLimits::default(),
        |original| {
            // Extend the original data LOAD by one wholly uninitialized page.
            let mem_size = 64 + 56 + 40;
            original.consumer[mem_size..mem_size + 8]
                .copy_from_slice(&(2 * PAGE_SIZE).to_le_bytes());
        },
        |plan, owner, _, pointers| {
            let cap = NativeLoadLimits {
                session: SessionLoadLimits {
                    resident_limit: 8 * PAGE_SIZE,
                    mapped_limit: 9 * PAGE_SIZE,
                },
                ..limits()
            };
            let sparse = prepare_hle_startup(
                plan,
                &StaticTlsStorage {
                    thread_pointers: &pointers,
                },
                &cap.session,
            )
            .unwrap();
            assert_eq!(sparse.memory().mapped_bytes(), 9 * PAGE_SIZE);
            assert_eq!(sparse.memory().resident_bytes(), 8 * PAGE_SIZE);
            assert!(matches!(prepare_native_startup_in(plan,
            &StaticTlsStorage { thread_pointers: &pointers }, &cap, Some(owner)),
            Err(HleError::Tls(TlsError::MemoryBudget {
                resident: true, needed, limit,
            })) if needed == 9 * PAGE_SIZE && limit == 8 * PAGE_SIZE));
        },
    );
}

fn prepare<'plan, 'image>(
    plan: &'plan HleStartupPlan<'image>,
    owner: NativeAddressSpace,
    pointers: &[u64],
) -> PreparedNativeHleStartup<'plan, 'image> {
    prepare_native_startup_in(
        plan,
        &StaticTlsStorage {
            thread_pointers: pointers,
        },
        &limits(),
        Some(owner),
    )
    .unwrap()
}

fn word(session: &mut PreparedNativeHleStartup<'_, '_>, address: u64) -> u64 {
    let mut value = [0; 8];
    session.pause().unwrap().read(address, &mut value).unwrap();
    u64::from_le_bytes(value)
}

fn stop(
    session: &PreparedNativeHleStartup<'_, '_>,
    function: KnownFunction,
    rdi: u64,
    rsi: u64,
    rdx: u64,
) -> NativeServiceStop {
    let site = session.plan().hle.sites()[function.index()];
    NativeServiceStop {
        rip: site.resume,
        rcx: site.resume,
        rax: u64::from(function.service_id()),
        rdi,
        rsi,
        rdx,
    }
}

#[test]
fn native_typed_tls_copies_relocated_templates_then_hle_changes_only_one_instance() {
    with_plan(
        HleStartupLimits::default(),
        |plan, owner, base, pointers| {
            let mut session = prepare(plan, owner, &pointers);
            assert!(std::ptr::eq(
                session.memory().source_plan(),
                plan.link_plan()
            ));
            assert!(std::ptr::eq(session.plan(), plan));
            assert_eq!(
                (
                    session.memory().module_count(),
                    session.memory().applied_relocations()
                ),
                (3, 6)
            );
            assert_eq!(session.memory().mapped_bytes(), 8 * PAGE_SIZE);
            assert_eq!(session.memory().committed_bytes(), 8 * PAGE_SIZE);
            assert_eq!(session.memory().reserved_bytes(), RESERVATION);
            assert_eq!(
                (
                    session.tls().templates().len(),
                    session.tls().instances().len()
                ),
                (2, 2)
            );
            let expected_pointer = base + 0x123;
            for tp in pointers {
                assert_eq!(word(&mut session, tp - CONSUMER_OFFSET), 2);
                assert_eq!(word(&mut session, tp - CONSUMER_OFFSET + 8), 8);
                assert_eq!(word(&mut session, tp - CONSUMER_OFFSET + 16) as i64, -80);
                assert_eq!(
                    word(&mut session, tp - CONSUMER_OFFSET + 24),
                    expected_pointer
                );
                assert_eq!(word(&mut session, tp - PROVIDER_OFFSET), 91);
                assert_eq!(word(&mut session, tp - PROVIDER_OFFSET + 8), 37);
                assert_eq!(word(&mut session, tp - CONSUMER_OFFSET + 32), 0);
                assert_eq!(word(&mut session, tp - PROVIDER_OFFSET + 16), 0);
            }
            assert_eq!(word(&mut session, base + TLS_RVA + 24), expected_pointer);
            let destination = pointers[0] - CONSUMER_OFFSET + 24;
            let source = pointers[0] - PROVIDER_OFFSET + 8;
            let copy = stop(&session, KnownFunction::Memcpy, destination, source, 8);
            assert_eq!(session.service_stop(copy).unwrap(), destination);
            let compare = stop(&session, KnownFunction::Memcmp, destination, source, 8);
            assert_eq!(session.service_stop(compare).unwrap(), 0);
            assert_eq!(word(&mut session, destination), 37);
            assert_eq!(
                word(&mut session, pointers[1] - CONSUMER_OFFSET + 24),
                expected_pointer
            );
            assert_eq!(word(&mut session, base + TLS_RVA + 24), expected_pointer);
            assert_eq!(
                (session.service_attempts(), session.completed_services()),
                (2, 2)
            );
        },
    );
}

#[test]
fn native_tls_guards_are_owned_holes_and_final_relro_and_provider_permissions_hold() {
    with_plan(HleStartupLimits::default(), |plan, owner, _, pointers| {
        let mut session = prepare(plan, owner, &pointers);
        let instances = session.tls().instances().to_vec();
        for instance in instances {
            for guard in instance.guard_pages.unwrap() {
                assert!(
                    session
                        .memory()
                        .space
                        .reserved_ranges()
                        .any(|r| r.address <= guard && guard + PAGE_SIZE <= r.address + r.size)
                );
                assert!(matches!(
                    session.pause().unwrap().read(guard, &mut [0]),
                    Err(NativeMemoryError::Unmapped(_))
                ));
                assert!(matches!(
                    session.pause().unwrap().write(guard, &[0]),
                    Err(NativeMemoryError::Unmapped(_))
                ));
            }
            assert!(matches!(
                session
                    .pause()
                    .unwrap()
                    .fetch_source(instance.storage.unwrap().address, &mut [0]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
        }
        let base = session.memory().source_plan().modules()[0].load_bias();
        assert!(matches!(
            session.pause().unwrap().write(base + GOT, &[0]),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
        for site in plan.hle.sites() {
            session
                .pause()
                .unwrap()
                .fetch_source(site.entry, &mut [0; 8])
                .unwrap();
            assert!(matches!(
                session.pause().unwrap().write(site.entry, &[0]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
        }
    });
}

#[test]
fn native_combined_services_keep_full_site_id_and_one_retained_budget() {
    let mut policy = HleStartupLimits::default();
    policy.hle.services.max_attempts = 2;
    with_plan(policy, |plan, owner, _, pointers| {
        let mut session = prepare(plan, owner, &pointers);
        let address = pointers[0] - CONSUMER_OFFSET + 32;
        let mut invalid = stop(&session, KnownFunction::Memset, address, 0xff, 4);
        invalid.rax |= 1 << 32;
        assert!(matches!(
            session.service_stop(invalid),
            Err(HleError::ServiceIdMismatch { .. })
        ));
        assert_eq!(word(&mut session, address), 0);
        let valid = stop(&session, KnownFunction::Memset, address, 0x5a, 4);
        session.service_stop(valid).unwrap();
        assert_eq!(word(&mut session, address), 0x5a5a_5a5a);
        assert_eq!(word(&mut session, pointers[1] - CONSUMER_OFFSET + 32), 0);
        assert!(matches!(
            session.service_stop(valid),
            Err(HleError::Service(
                super::super::MemoryServiceError::LimitExceeded {
                    kind: super::super::ServiceLimit::Attempts,
                    ..
                }
            ))
        ));
        assert_eq!(
            (session.service_attempts(), session.completed_services()),
            (2, 1)
        );
    });
}

#[test]
fn native_tls_preflights_total_eager_resident_mapped_and_reservation_budgets() {
    for kind in 0..3 {
        with_plan(HleStartupLimits::default(), |plan, owner, _, pointers| {
            let mut cap = limits();
            match kind {
                0 => cap.session.resident_limit = 8 * PAGE_SIZE - 1,
                1 => cap.session.mapped_limit = 8 * PAGE_SIZE - 1,
                _ => cap.reserved_limit = 5 * 0x1_0000 - 1,
            }
            let result = prepare_native_startup_in(
                plan,
                &StaticTlsStorage {
                    thread_pointers: &pointers,
                },
                &cap,
                Some(owner),
            );
            match kind {
                0 => assert!(matches!(
                    result,
                    Err(HleError::Tls(TlsError::MemoryBudget { resident: true, .. }))
                )),
                1 => assert!(matches!(
                    result,
                    Err(HleError::Tls(TlsError::MemoryBudget {
                        resident: false,
                        ..
                    }))
                )),
                _ => assert!(matches!(
                    result,
                    Err(HleError::Tls(TlsError::NativeReservationBudget { .. }))
                )),
            }
        });
    }
}

#[test]
fn native_tls_refuses_guard_boundary_and_image_storage_overlap() {
    for boundary in [false, true] {
        with_plan(HleStartupLimits::default(), |plan, owner, base, _| {
            let tp = if boundary {
                base + 0x7_0000
            } else {
                base + 2 * PAGE_SIZE
            };
            let result = prepare_native_startup_in(
                plan,
                &StaticTlsStorage {
                    thread_pointers: &[tp],
                },
                &limits(),
                Some(owner),
            );
            if boundary {
                assert!(matches!(
                    result,
                    Err(HleError::Tls(TlsError::NativeGuardOutsideReservation {
                        instance: 0
                    }))
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(HleError::Tls(TlsError::StorageOverlap { instance: 0 }))
                ));
            }
        });
    }
}

#[test]
fn native_combined_path_keeps_caller_instance_limit_retained_in_source_plan() {
    let mut policy = HleStartupLimits::default();
    policy.static_tls.max_instances = 1;
    with_plan(policy, |plan, owner, _, pointers| {
        let result = prepare_native_startup_in(
            plan,
            &StaticTlsStorage {
                thread_pointers: &pointers,
            },
            &limits(),
            Some(owner),
        );
        assert!(matches!(
            result,
            Err(HleError::Tls(TlsError::Budget {
                kind: TlsBudget::Instances,
                limit: 1
            }))
        ));
    });
}

#[test]
fn native_startup_admission_keeps_wrong_sources_and_unknown_records_fail_closed() {
    for variant in 0..2 {
        let mut original = images().unwrap();
        match variant {
            0 => {
                let name = b"pentax-original-tls-lib";
                let at = original
                    .provider
                    .windows(name.len())
                    .position(|s| s == name)
                    .unwrap();
                original.provider[at] = b'q';
            }
            1 => original.consumer[image::RELA + 8..image::RELA + 16]
                .copy_from_slice(&((3_u64 << 32) | 42).to_le_bytes()),
            _ => unreachable!(),
        }
        let before = (original.consumer.clone(), original.provider.clone());
        let policy = HleStartupLimits::default();
        let providers =
            build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &policy.hle).unwrap();
        let result = plan(&original, &providers, &policy);
        match variant {
            0 => assert!(matches!(result, Err(HleError::TlsBinding(_)))),
            1 => assert!(matches!(
                result,
                Err(HleError::Tls(TlsError::RemainingDeferredRelocations {
                    count: 1,
                    ..
                }))
            )),
            _ => unreachable!(),
        }
        assert_eq!((original.consumer, original.provider), before);
    }
}

#[test]
fn native_image_writes_preserve_each_independent_tls_zero_tail() {
    for record in [0, 3] {
        with_custom_plan(
            HleStartupLimits::default(),
            |original| {
                let at = image::RELA + 24 * record;
                original.consumer[at..at + 8].copy_from_slice(&(TLS_RVA + 32).to_le_bytes());
            },
            |plan, owner, base, pointers| {
                let mut session = prepare(plan, owner, &pointers);
                let expected = if record == 0 { 2 } else { base + 0x123 };
                assert_eq!(word(&mut session, base + TLS_RVA + 32), expected);
                for tp in pointers {
                    assert_eq!(word(&mut session, tp - CONSUMER_OFFSET + 32), 0);
                }
            },
        );
    }
}
