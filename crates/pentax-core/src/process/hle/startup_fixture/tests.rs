// SPDX-License-Identifier: GPL-2.0-or-later

use super::*;
use crate::cpu::CpuError;
use crate::process::hle::{MemoryServiceError, ServiceLimit, plan_hle_session};
use crate::process::tls::{TlsBudget, TlsError};
use pentax_linker::{TlsBindingBudget, TlsBindingError};

fn providers(limits: &HleStartupLimits) -> super::super::GeneratedProviders {
    build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.hle).unwrap()
}

fn prepared(limits: &HleStartupLimits) -> PreparedHleStartup {
    let original = images().unwrap();
    let providers = providers(limits);
    let plan = plan(&original, &providers, limits).unwrap();
    prepare_hle_startup(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TLS_POINTERS,
        },
        &memory_limits(),
    )
    .unwrap()
}

fn put64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn original_combined_guest_observes_typed_tls_and_real_hle_calls() {
    let report = run_hle_startup_fixture().unwrap();
    assert_eq!(
        (
            report.module_count,
            report.tls_templates,
            report.tls_instances
        ),
        (3, 2, 2)
    );
    assert_eq!(
        (
            report.provider_bindings,
            report.tls_bindings,
            report.applied_relocations
        ),
        (2, 3, 6)
    );
    assert_eq!(
        (report.provider_tls_runtime_id, report.dtpoff, report.tpoff),
        (2, 8, -80)
    );
    assert_eq!(
        (
            report.relocated_template_word,
            report.final_value,
            report.independent_value
        ),
        (ORIGINAL_POINTER, 42, ORIGINAL_POINTER)
    );
    assert_eq!(
        (
            report.service_attempts,
            report.memcpy_calls,
            report.memcmp_calls,
            report.requested_bytes
        ),
        (2, 1, 1, 16)
    );
    assert_eq!(report.retired, images().unwrap().retired);
    assert_eq!(report.mapped_bytes, 9 * PAGE_SIZE);
    assert_eq!(report.resident_bytes, 9 * PAGE_SIZE);
    assert_eq!(report, run_hle_startup_fixture().unwrap());
}

#[test]
fn ordinary_hle_still_refuses_tls_without_any_opt_in() {
    let original = images().unwrap();
    let providers = providers(&HleStartupLimits::default());
    let inputs = [ModuleInput::new(
        open_executable(&original.consumer).unwrap(),
        CONSUMER_BIAS,
    )];
    assert!(matches!(
        plan_hle_session(
            &inputs,
            &providers,
            ProviderPlacement {
                load_bias: HLE_PROVIDER_BIAS
            },
            &super::super::HleLimits::default()
        ),
        Err(HleError::UnsupportedTls { module: 0 })
    ));
    let opt_in = plan(&original, &providers, &HleStartupLimits::default()).unwrap();
    assert!(matches!(
        super::super::prepare_hle_session(&opt_in.hle, &memory_limits()),
        Err(HleError::UnsupportedTls { module: 0 })
    ));
}

#[test]
fn retained_owner_keeps_typed_copies_guards_and_read_only_hle_attestation() {
    let prepared = prepared(&HleStartupLimits::default());
    assert_eq!(
        prepared.policy(),
        super::super::HleStartupPolicy::GenericAmd64StaticTlsV1
    );
    assert_eq!(
        (prepared.provider_index(), prepared.provider_bias()),
        (2, HLE_PROVIDER_BIAS)
    );
    for &tp in &TLS_POINTERS {
        assert_eq!(word(prepared.memory(), tp - CONSUMER_OFFSET).unwrap(), 2);
        assert_eq!(
            word(prepared.memory(), tp - CONSUMER_OFFSET + 8).unwrap(),
            8
        );
        assert_eq!(
            word(prepared.memory(), tp - CONSUMER_OFFSET + 16).unwrap() as i64,
            -80
        );
        assert_eq!(
            word(prepared.memory(), tp - CONSUMER_OFFSET + 24).unwrap(),
            ORIGINAL_POINTER
        );
    }
    for instance in prepared.tls().instances() {
        for address in instance.guard_pages.unwrap() {
            assert!(matches!(
                prepared.memory().preflight_read(address, 1),
                Err(MemoryError::Unmapped(_))
            ));
        }
        assert!(matches!(
            prepared
                .memory()
                .fetch(instance.storage.unwrap().address, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ));
    }
    for site in &prepared.sites {
        assert!(prepared.memory().fetch(site.entry, &mut [0; 8]).is_ok());
        assert!(matches!(
            prepared.memory().preflight_write(site.entry, 1),
            Err(MemoryError::PermissionDenied(_))
        ));
    }
}

#[test]
fn unknown_deferred_record_fails_before_combined_plan_publication() {
    let mut original = images().unwrap();
    // First TLS record now has an unsupported relocation kind, never treated as NONE.
    put64(&mut original.consumer, image::RELA + 8, (3 << 32) | 42);
    let before = original.consumer.clone();
    let providers = providers(&HleStartupLimits::default());
    assert!(matches!(
        plan(&original, &providers, &HleStartupLimits::default()),
        Err(HleError::Tls(TlsError::RemainingDeferredRelocations {
            count: 1,
            ..
        }))
    ));
    assert_eq!(original.consumer, before);
}

#[test]
fn tls_resolution_uses_exact_retained_names_without_hle_or_nid_fallback() {
    let mut original = images().unwrap();
    // A valid different library name with the same NID/versions is not a provider.
    let name = b"pentax-original-tls-lib";
    let at = original
        .provider
        .windows(name.len())
        .position(|bytes| bytes == name)
        .unwrap();
    original.provider[at] = b'q';
    let providers = providers(&HleStartupLimits::default());
    assert!(matches!(
        plan(&original, &providers, &HleStartupLimits::default()),
        Err(HleError::TlsBinding(TlsBindingError::Resolution { .. }))
    ));
}

#[test]
fn typed_and_ordinary_image_writes_leave_independent_tls_zero_tails() {
    for record in [0, 3] {
        let mut original = images().unwrap();
        put64(
            &mut original.consumer,
            image::RELA + record * 24,
            TLS_RVA + 32,
        );
        let before = original.consumer.clone();
        let providers = providers(&HleStartupLimits::default());
        let plan = plan(&original, &providers, &HleStartupLimits::default()).unwrap();
        let prepared = prepare_hle_startup(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TLS_POINTERS,
            },
            &memory_limits(),
        )
        .unwrap();
        let expected = if record == 0 { 2 } else { ORIGINAL_POINTER };
        assert_eq!(
            word(prepared.memory(), CONSUMER_BIAS + TLS_RVA + 32).unwrap(),
            expected
        );
        for tp in TLS_POINTERS {
            assert_eq!(
                word(prepared.memory(), tp - CONSUMER_OFFSET + 32).unwrap(),
                0
            );
        }
        assert_eq!(original.consumer, before);
    }
}

#[test]
fn aggregate_binding_and_layout_budgets_are_retained_and_enforced() {
    let original = images().unwrap();
    let mut limits = HleStartupLimits::default();
    limits.tls_bindings.max_bindings = 2;
    let providers = providers(&limits);
    assert!(matches!(
        plan(&original, &providers, &limits),
        Err(HleError::TlsBinding(TlsBindingError::Budget {
            kind: TlsBindingBudget::Bindings,
            limit: 2
        }))
    ));
    limits.tls_bindings.max_bindings = 3;
    limits.static_tls.max_work_steps = 0;
    assert!(matches!(
        plan(&original, &providers, &limits),
        Err(HleError::Tls(TlsError::Budget {
            kind: TlsBudget::WorkSteps,
            limit: 0
        }))
    ));
}

#[test]
fn instance_allowance_stays_in_plan_even_after_caller_limits_change() {
    let original = images().unwrap();
    let mut limits = HleStartupLimits::default();
    limits.static_tls.max_instances = 1;
    let providers = providers(&limits);
    let plan = plan(&original, &providers, &limits).unwrap();
    limits.static_tls.max_instances = 2;
    assert_eq!(limits.static_tls.max_instances, 2);
    assert_eq!(plan.limits().static_tls.max_instances, 1);
    assert!(matches!(
        prepare_hle_startup(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TLS_POINTERS
            },
            &memory_limits()
        ),
        Err(HleError::Tls(TlsError::Budget {
            kind: TlsBudget::Instances,
            limit: 1
        }))
    ));
}

#[test]
fn storage_collision_and_aggregate_memory_limit_never_publish_preparation() {
    let original = images().unwrap();
    let before = (original.consumer.clone(), original.provider.clone());
    let limits = HleStartupLimits::default();
    let providers = providers(&limits);
    let plan = plan(&original, &providers, &limits).unwrap();
    assert!(matches!(
        prepare_hle_startup(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &[HLE_PROVIDER_BIAS + PAGE_SIZE]
            },
            &memory_limits()
        ),
        Err(HleError::Tls(TlsError::StorageOverlap { instance: 0 }))
    ));
    let memory = SessionLoadLimits {
        mapped_limit: 6 * PAGE_SIZE,
        ..memory_limits()
    };
    assert!(matches!(
        prepare_hle_startup(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TLS_POINTERS
            },
            &memory
        ),
        Err(HleError::Tls(TlsError::MemoryBudget {
            resident: false,
            ..
        }))
    ));
    assert_eq!(
        (&original.consumer, &original.provider),
        (&before.0, &before.1)
    );
}

#[test]
fn combined_gateway_keeps_one_shared_service_attempt_budget() {
    let mut limits = HleStartupLimits::default();
    limits.hle.services.max_attempts = 1;
    let mut consumer = ClosedConsumer::initialize(prepared(&limits)).unwrap();
    assert!(matches!(
        consumer.run(INSTRUCTIONS),
        Err(HleError::Service(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::Attempts,
            ..
        }))
    ));
    assert_eq!(consumer.budget.completed(), 1);
    assert_eq!(consumer.budget.calls(KnownFunction::Memcpy), 1);
    assert_eq!(consumer.budget.calls(KnownFunction::Memcmp), 0);
    assert_eq!(
        word(
            consumer.prepared.memory(),
            TLS_POINTERS[0] - CONSUMER_OFFSET + 24
        )
        .unwrap(),
        37
    );
    assert_eq!(
        word(
            consumer.prepared.memory(),
            TLS_POINTERS[1] - CONSUMER_OFFSET + 24
        )
        .unwrap(),
        ORIGINAL_POINTER
    );
}

#[test]
fn combined_gateway_keeps_one_shared_byte_budget_across_libc_calls() {
    let mut limits = HleStartupLimits::default();
    limits.hle.services.max_total_bytes = 15;
    let mut consumer = ClosedConsumer::initialize(prepared(&limits)).unwrap();
    assert!(matches!(
        consumer.run(INSTRUCTIONS),
        Err(HleError::Service(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::TotalBytes,
            ..
        }))
    ));
    assert_eq!(consumer.budget.attempts(), 2);
    assert_eq!(consumer.budget.completed(), 1);
    assert_eq!(consumer.budget.total_bytes(), 8);
}

#[test]
fn combined_instruction_limit_is_shared_across_hle_resumes() {
    let mut consumer = ClosedConsumer::initialize(prepared(&HleStartupLimits::default())).unwrap();
    let limit = images().unwrap().retired - 1;
    assert!(
        matches!(consumer.run(limit), Err(HleError::InstructionLimit { retired, .. }) if retired == limit)
    );
    assert_eq!(consumer.budget.completed(), 2);
    assert_eq!(consumer.cpu.retired, limit);
}

#[test]
fn wrong_original_guest_value_halts_at_assertion_and_publishes_no_success() {
    let mut original = images().unwrap();
    put64(&mut original.provider, TLS_RVA as usize + 8, 38);
    let limits = HleStartupLimits::default();
    let providers = providers(&limits);
    let plan = plan(&original, &providers, &limits).unwrap();
    let prepared = prepare_hle_startup(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TLS_POINTERS,
        },
        &memory_limits(),
    )
    .unwrap();
    let mut consumer = ClosedConsumer::initialize(prepared).unwrap();
    let halt = consumer.run(INSTRUCTIONS).unwrap();
    assert!(matches!(
        verify(&consumer, &original, halt),
        Err(HleError::Fixture("combined guest completion"))
    ));
    assert_eq!(consumer.budget.attempts(), 0);
}

#[test]
fn modeled_fs_guard_fault_does_not_dispatch_any_service() {
    let mut consumer = ClosedConsumer::initialize(prepared(&HleStartupLimits::default())).unwrap();
    consumer.cpu.fs_base = TLS_POINTERS[0] + PAGE_SIZE;
    let before = consumer.cpu.clone();
    assert!(matches!(
        consumer.run(INSTRUCTIONS),
        Err(HleError::Cpu(CpuError::Memory(MemoryError::Unmapped(_))))
    ));
    assert_eq!(consumer.cpu, before);
    assert_eq!(consumer.budget.attempts(), 0);
}
