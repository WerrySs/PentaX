// SPDX-License-Identifier: GPL-2.0-or-later

//! Original multi-module images verify typed TLS values without execution.

mod fixture;

use super::*;
use crate::memory::{GuestMemory, MemoryError, PAGE_SIZE};
use crate::process::tls::{load_symbolic_with_static_tls, plan_static_tls};
use crate::process::{RelocationStatus, load_symbolic};
use fixture::{
    BSS, DATA, DTPMOD, DTPOFF, ORIGINAL, PROVIDER_WORDS, RELATIVE, Record, TLS_HEADER, TPOFF, Tls,
    consumer, filler, provider, record,
};
use pentax_linker::{
    ModuleInput, SymbolicLimits, SymbolicLinkPlan, TableKind, TlsBindingLimits, plan_symbolic,
    plan_tls_bindings,
};
use pentax_loader::open_executable;

const CONSUMER_BIAS: u64 = 0x10_0000;
const FILLER_BIAS: u64 = 0x20_0000;
const PROVIDER_BIAS: u64 = 0x30_0000;
const TP: [u64; 2] = [0x80_0000, 0xa0_0000];
/// Alignment bias 8 within a 32-byte alignment; 16 initialized of 32 bytes.
const PROVIDER_TLS: Tls = Tls {
    address: DATA + 8,
    initialized: 16,
    memory: 32,
    alignment: 32,
};
/// Initialized [DATA+0x40, DATA+0x50), zero tail to DATA+0x60.
const CONSUMER_TLS: Tls = Tls {
    address: DATA + 0x40,
    initialized: 16,
    memory: 32,
    alignment: 16,
};

type Images = Vec<(Vec<u8>, u64)>;

fn link(images: &[(Vec<u8>, u64)]) -> SymbolicLinkPlan<'_> {
    let inputs: Vec<_> = images
        .iter()
        .map(|(bytes, bias)| ModuleInput::new(open_executable(bytes).unwrap(), *bias))
        .collect();
    plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap()
}

fn storage() -> StaticTlsStorage<'static> {
    StaticTlsStorage {
        thread_pointers: &TP,
    }
}

fn word(memory: &GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

/// A TLS-free primary, a metadata-free filler and a provider at module 2.
fn foreign(records: &[Record]) -> Images {
    vec![
        (consumer(None, records, false), CONSUMER_BIAS),
        (filler(), FILLER_BIAS),
        (provider(PROVIDER_TLS, [0, 8]), PROVIDER_BIAS),
    ]
}

/// A TLS-bearing RELRO primary at runtime ID 1 and a provider at module 1.
fn templated(records: &[Record]) -> Images {
    vec![
        (consumer(Some(CONSUMER_TLS), records, true), CONSUMER_BIAS),
        (provider(PROVIDER_TLS, [0, 8]), PROVIDER_BIAS),
    ]
}

fn foreign_records() -> Vec<Record> {
    vec![
        record(DATA + 0x80, 1, DTPMOD, 0),
        record(DATA + 0x88, 1, DTPOFF, 0),
        record(DATA + 0x90, 1, TPOFF, 0),
        record(DATA + 0x98, 2, DTPOFF, 4),
        record(DATA + 0xa0, 2, TPOFF, -8),
        record(DATA + 0xa8, 2, TPOFF, 23),
        record(DATA + 0xb0, 0, RELATIVE, 0x1234),
        record(BSS, 2, DTPMOD, 0),
    ]
}

fn negative(value: i64) -> u64 {
    value as u64
}

#[test]
fn foreign_provider_values_come_from_layout_ids_and_tp_offsets() {
    let images = foreign(&foreign_records());
    let snapshots = images.clone();
    let base = link(&images);
    assert_eq!(base.deferred_count(), 7);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    assert!(std::ptr::eq(plan.bindings(), &bindings));
    assert!(std::ptr::eq(plan.bindings().base_plan(), &base));
    assert_eq!(plan.policy(), StaticTlsPolicy::PentaXVariantIIInputOrderV1);
    // The TLS-free consumer and filler consume no IDs: ModuleId 2 receives ID 1.
    // Bias 8 within alignment 32 raises O from 32 to 56.
    let layout = plan.templates()[0];
    assert_eq!(plan.templates().len(), 1);
    assert_eq!(layout.module.index(), 2);
    assert_eq!(layout.runtime_id.get(), 1);
    assert_eq!((layout.alignment, layout.alignment_bias), (32, 8));
    assert_eq!(layout.tp_offset, 56);
    assert_eq!((plan.static_extent(), plan.maximum_alignment()), (56, 32));
    let expected = [
        (DATA + 0x80, 1),
        (DATA + 0x88, 0), // Offset zero, addend zero.
        (DATA + 0x90, negative(-56)),
        (DATA + 0x98, 12),
        (DATA + 0xa0, negative(-56)), // Negative addend reaching offset zero.
        (DATA + 0xa8, negative(-25)), // S+A = p_memsz - 1.
        (BSS, 1),
    ]
    .map(|(target, value)| (CONSUMER_BIAS + target, value));
    let values: Vec<_> = plan
        .tls_patches()
        .iter()
        .map(|patch| (patch.address(), patch.value()))
        .collect();
    assert_eq!(values, expected);
    for patch in plan.tls_patches() {
        let binding = bindings.bindings()[patch.binding_index()];
        assert_eq!(binding.address(), patch.address());
        assert_eq!(binding.source(), patch.source());
        assert_eq!(patch.source().table, TableKind::Rela);
        assert_eq!(patch.consumer().index(), 0);
        assert_eq!(patch.definition().module().index(), 2);
        assert_eq!(patch.definition().program_header_index(), TLS_HEADER);
        assert_eq!(patch.definition().template_mem_size(), 32);
        let kind = match patch.kind() {
            TlsRelocationKind::DtpMod64 => DTPMOD,
            TlsRelocationKind::DtpOff64 => DTPOFF,
            TlsRelocationKind::TpOff64 => TPOFF,
        };
        assert_eq!(patch.source().record.kind(), kind);
    }
    let prepared =
        load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
    assert_eq!(prepared.applied_relocations(), 8);
    assert_eq!(
        prepared.session.relocation_status(),
        RelocationStatus::CompleteWithinPolicy
    );
    for (address, value) in expected {
        assert_eq!(word(prepared.memory(), address), value);
    }
    assert_eq!(
        word(prepared.memory(), CONSUMER_BIAS + DATA + 0xb0),
        CONSUMER_BIAS + 0x1234
    );
    for instance in prepared.instances() {
        let block = instance.blocks[0];
        assert_eq!((block.module.index(), block.runtime_id.get()), (2, 1));
        assert_eq!(block.address, instance.thread_pointer - 56);
        assert_eq!(block.address % 32, 8);
        assert_eq!(word(prepared.memory(), block.address), PROVIDER_WORDS[0]);
        assert_eq!(
            word(prepared.memory(), block.address + 8),
            PROVIDER_WORDS[1]
        );
        assert_eq!(word(prepared.memory(), block.address + 16), 0);
        assert_eq!(word(prepared.memory(), block.address + 24), 0);
    }
    assert_eq!(images, snapshots);
}

#[test]
fn initialized_template_relocations_reach_both_copies_after_relro() {
    let records = [
        record(DATA + 0x40, 1, DTPMOD, 0),
        record(DATA + 0x48, 2, TPOFF, 0),
        record(DATA + 0x80, 2, DTPOFF, -8),
    ];
    let images = templated(&records);
    let snapshots = images.clone();
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    let layouts: Vec<_> = plan
        .templates()
        .iter()
        .map(|template| {
            (
                template.module.index(),
                template.runtime_id.get(),
                template.tp_offset,
            )
        })
        .collect();
    // The provider's ModuleId 1 differs from its runtime TLS ID 2.
    assert_eq!(layouts, vec![(0, 1, 32), (1, 2, 88)]);
    let source = CONSUMER_BIAS + CONSUMER_TLS.address;
    let mut prepared =
        load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
    assert_eq!(word(prepared.memory(), source), 2);
    assert_eq!(word(prepared.memory(), source + 8), negative(8 - 88));
    assert_eq!(word(prepared.memory(), CONSUMER_BIAS + DATA + 0x80), 0);
    for instance in prepared.instances() {
        let [own, provided] = [instance.blocks[0], instance.blocks[1]];
        assert_eq!(own.address, instance.thread_pointer - 32);
        assert_eq!(word(prepared.memory(), own.address), 2);
        assert_eq!(word(prepared.memory(), own.address + 8), negative(-80));
        assert_eq!(word(prepared.memory(), own.address + 16), 0);
        assert_eq!(word(prepared.memory(), own.address + 24), 0);
        assert_eq!(provided.address, instance.thread_pointer - 88);
        assert_eq!(provided.address % 32, 8);
        assert_eq!(word(prepared.memory(), provided.address), PROVIDER_WORDS[0]);
        for address in [own.address, provided.address] {
            assert_eq!(
                prepared.memory().fetch(address, &mut [0]),
                Err(MemoryError::PermissionDenied(address))
            );
        }
        for guard in instance.guard_pages.unwrap() {
            assert!(matches!(
                prepared.memory().read(guard, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ));
        }
    }
    // TLS writes preceded RELRO: the relocated source page is now read-only.
    assert_eq!(
        prepared.session.memory.write(source, &[0]),
        Err(MemoryError::PermissionDenied(source))
    );
    let first = prepared.instances()[0].blocks[0].address;
    let second = prepared.instances()[1].blocks[0].address;
    prepared
        .session
        .memory
        .write(first, &99_u64.to_le_bytes())
        .unwrap();
    assert_eq!(word(prepared.memory(), first), 99);
    assert_eq!(word(prepared.memory(), second), 2);
    assert_eq!(word(prepared.memory(), source), 2);
    assert_eq!(images, snapshots);
}

#[test]
fn typed_image_writes_crossing_tls_file_boundary_leave_instance_tails_zero() {
    for target in [DATA + 0x50, DATA + 0x4c, DATA + 0x58] {
        let images = templated(&[record(target, 2, TPOFF, 0)]);
        let snapshots = images.clone();
        let base = link(&images);
        // No ordinary write exists, so the old layout check alone accepts this.
        assert!(plan_static_tls(&base, &StaticTlsLimits::default()).is_ok());
        let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
        let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
        let prepared =
            load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
        assert_eq!(
            word(prepared.memory(), CONSUMER_BIAS + target),
            negative(-80)
        );
        let mut expected = [0_u8; 32];
        prepared
            .memory()
            .read(CONSUMER_BIAS + CONSUMER_TLS.address, &mut expected[..16])
            .unwrap();
        for instance in prepared.instances() {
            let mut actual = [0xff_u8; 32];
            prepared
                .memory()
                .read(instance.blocks[0].address, &mut actual)
                .unwrap();
            assert_eq!(actual, expected);
        }
        assert_eq!(images, snapshots);
    }
    // Half-open: ending at the initialized boundary or starting at p_memsz.
    for target in [DATA + 0x48, DATA + 0x60] {
        let images = templated(&[record(target, 2, TPOFF, 0)]);
        let base = link(&images);
        let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
        let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
        let prepared =
            load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
        assert_eq!(
            word(prepared.memory(), CONSUMER_BIAS + target),
            negative(-80)
        );
        for instance in prepared.instances() {
            let block = instance.blocks[0].address;
            // Only the in-template write is copied; DATA+0x60 lies beyond p_memsz.
            let copied = if target == DATA + 0x48 {
                negative(-80)
            } else {
                0
            };
            assert_eq!(word(prepared.memory(), block + 8), copied);
            assert_eq!(word(prepared.memory(), block + 16), 0);
            assert_eq!(word(prepared.memory(), block + 24), 0);
        }
    }
}

#[test]
fn block_relative_values_outside_the_provider_block_are_typed_failures() {
    for (symbol, kind, addend) in [
        (2, TPOFF, 24), // S+A = p_memsz.
        (1, DTPOFF, -1),
        (1, DTPOFF, i64::MIN),
        (2, TPOFF, i64::MAX),
    ] {
        let images = foreign(&[record(DATA + 0x80, symbol, kind, addend)]);
        let base = link(&images);
        let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
        match plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap_err() {
            TlsError::TlsAddendRange {
                consumer,
                source,
                definition,
            } => {
                assert_eq!(consumer.index(), 0);
                assert_eq!(source.index, 0);
                assert_eq!(source.record.addend, addend);
                assert_eq!(source.record.kind(), kind);
                assert_eq!(definition.unwrap().module.index(), 2);
                assert_eq!(definition.unwrap().symbol, symbol as usize);
            }
            other => panic!("unexpected relocated TLS error: {other}"),
        }
    }
}

#[test]
fn bss_tls_target_pages_join_the_resident_union_before_staging() {
    let images = foreign(&foreign_records());
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    // Six image pages (one only through the BSS TLS write; data-page writes
    // share their file page) plus one initialized storage page per instance.
    let exact = SessionLoadLimits {
        resident_limit: 8 * PAGE_SIZE,
        mapped_limit: 9 * PAGE_SIZE,
    };
    let prepared = load_relocated_static_tls(&plan, &storage(), &exact).unwrap();
    assert_eq!(prepared.memory().resident_bytes(), exact.resident_limit);
    assert_eq!(prepared.memory().mapped_bytes(), exact.mapped_limit);
    assert_eq!(word(prepared.memory(), CONSUMER_BIAS + BSS), 1);
    assert_eq!(
        load_relocated_static_tls(
            &plan,
            &storage(),
            &SessionLoadLimits {
                resident_limit: exact.resident_limit - 1,
                ..exact.clone()
            }
        )
        .unwrap_err(),
        TlsError::MemoryBudget {
            resident: true,
            needed: exact.resident_limit,
            limit: exact.resident_limit - 1,
        }
    );
    assert!(matches!(
        load_relocated_static_tls(
            &plan,
            &storage(),
            &SessionLoadLimits {
                mapped_limit: exact.mapped_limit - 1,
                ..exact.clone()
            }
        ),
        Err(TlsError::MemoryBudget {
            resident: false,
            ..
        })
    ));
}

#[test]
fn remaining_unknown_records_refuse_relocated_preparation_without_guessing() {
    let records = [
        record(DATA + 0x80, 1, DTPMOD, 0),
        record(DATA + 0x88, 0, 42, 7),
        record(DATA + 0x90, 2, TPOFF, 0),
        record(DATA + 0x98, 0, 43, 0),
    ];
    let images = foreign(&records);
    let snapshots = images.clone();
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    assert_eq!(bindings.remaining_deferred().len(), 2);
    match plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap_err() {
        TlsError::RemainingDeferredRelocations {
            count,
            module,
            source,
        } => {
            assert_eq!((count, module.index()), (2, 0));
            assert_eq!((source.table, source.index), (TableKind::Rela, 1));
            assert_eq!(source.record.kind(), 42);
            assert_eq!(source.record.addend, 7);
        }
        other => panic!("unexpected relocated TLS error: {other}"),
    }
    let staged = load_symbolic(&base, &SessionLoadLimits::default()).unwrap();
    assert_eq!(staged.deferred_relocations(), 4);
    assert_eq!(word(staged.memory(), CONSUMER_BIAS + DATA + 0x80), ORIGINAL);
    assert_eq!(images, snapshots);
}

#[test]
fn ordinary_staging_and_old_preparation_keep_tls_records_deferred() {
    let images = foreign(&foreign_records());
    let base = link(&images);
    let staged = load_symbolic(&base, &SessionLoadLimits::default()).unwrap();
    assert_eq!(staged.applied_relocations(), 1);
    assert_eq!(
        staged.relocation_status(),
        RelocationStatus::Deferred { count: 7 }
    );
    for offset in (0x80..0xb0).step_by(8) {
        assert_eq!(
            word(staged.memory(), CONSUMER_BIAS + DATA + offset),
            ORIGINAL
        );
    }
    assert_eq!(word(staged.memory(), CONSUMER_BIAS + BSS), 0);
    let old = plan_static_tls(&base, &StaticTlsLimits::default()).unwrap();
    assert!(matches!(
        load_symbolic_with_static_tls(&old, &storage(), &SessionLoadLimits::default()),
        Err(TlsError::DeferredRelocations { count: 7 })
    ));
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    assert_eq!(plan.templates(), old.templates());
    assert_eq!(plan.static_extent(), old.static_extent());
    // The base plan itself is never relabeled complete.
    assert_eq!(base.deferred_count(), 7);
}

#[test]
fn rejected_relocated_preparation_leaves_sessions_and_sources_untouched() {
    let records = [
        record(DATA + 0x40, 1, DTPMOD, 0),
        record(DATA + 0x48, 2, TPOFF, 0),
    ];
    let images = templated(&records);
    let snapshots = images.clone();
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    let existing =
        load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
    let block = existing.instances()[0].blocks[0].address;
    let overlapping = [CONSUMER_BIAS + 2 * PAGE_SIZE];
    let failures = [
        load_relocated_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &overlapping,
            },
            &SessionLoadLimits::default(),
        ),
        load_relocated_static_tls(
            &plan,
            &storage(),
            &SessionLoadLimits {
                resident_limit: 0,
                ..SessionLoadLimits::default()
            },
        ),
        load_relocated_static_tls(
            &plan,
            &storage(),
            &SessionLoadLimits {
                mapped_limit: 0,
                ..SessionLoadLimits::default()
            },
        ),
    ];
    assert!(matches!(
        failures[0],
        Err(TlsError::StorageOverlap { instance: 0 })
    ));
    assert!(
        failures[1..]
            .iter()
            .all(|result| matches!(result, Err(TlsError::MemoryBudget { .. })))
    );
    assert_eq!(word(existing.memory(), block), 2);
    assert_eq!(
        word(existing.memory(), CONSUMER_BIAS + CONSUMER_TLS.address),
        2
    );
    assert_eq!(images, snapshots);
    let again =
        load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
    assert_eq!(word(again.memory(), block + 8), negative(-80));
}

#[test]
fn retained_instance_limit_remains_configurable_for_relocated_plans() {
    let images = templated(&[record(DATA + 0x40, 1, DTPMOD, 0)]);
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let pointers = [TP[0], TP[1], 0xc0_0000];
    let storage = StaticTlsStorage {
        thread_pointers: &pointers,
    };
    let default = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    assert!(matches!(
        load_relocated_static_tls(&default, &storage, &SessionLoadLimits::default()),
        Err(TlsError::Budget {
            kind: TlsBudget::Instances,
            limit: 2
        })
    ));
    let limits = StaticTlsLimits {
        max_instances: 3,
        ..StaticTlsLimits::default()
    };
    let plan = plan_relocated_static_tls(&bindings, &limits).unwrap();
    let prepared =
        load_relocated_static_tls(&plan, &storage, &SessionLoadLimits::default()).unwrap();
    assert_eq!(prepared.instances().len(), 3);
    for instance in prepared.instances() {
        assert_eq!(word(prepared.memory(), instance.blocks[0].address), 2);
    }
}

#[test]
fn extra_tls_work_is_charged_within_the_same_per_call_budget() {
    let images = foreign(&foreign_records());
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let limits = |max_work_steps| StaticTlsLimits {
        max_work_steps,
        ..StaticTlsLimits::default()
    };
    // Layout: 3 modules, 5 segments, 1 patch, 2 provider segments and 16
    // initialized bytes. Extra: 7 visits, 7 lookups and 3 retained resident
    // intervals plus 7 TLS target intervals. No image/TLS-tail cross-product.
    assert!(plan_static_tls(&base, &limits(26)).is_err());
    assert!(plan_static_tls(&base, &limits(27)).is_ok());
    for steps in [0, 27, 50] {
        assert_eq!(
            plan_relocated_static_tls(&bindings, &limits(steps)).unwrap_err(),
            TlsError::Budget {
                kind: TlsBudget::WorkSteps,
                limit: steps
            }
        );
    }
    assert!(plan_relocated_static_tls(&bindings, &limits(51)).is_ok());
}

#[test]
fn local_module_dtpmod_uses_derived_tls_id_and_reaches_each_relocated_copy() {
    let local = consumer(
        Some(CONSUMER_TLS),
        &[record(CONSUMER_TLS.address, 0, DTPMOD, 0)],
        true,
    );
    let images = vec![
        (consumer(None, &[], false), FILLER_BIAS),
        (filler(), PROVIDER_BIAS),
        (local, CONSUMER_BIAS),
    ];
    let original = images.clone();
    let base = link(&images);
    let bindings = plan_tls_bindings(&base, &TlsBindingLimits::default()).unwrap();
    let plan = plan_relocated_static_tls(&bindings, &StaticTlsLimits::default()).unwrap();
    assert_eq!(plan.templates()[0].module.index(), 2);
    assert_eq!(plan.templates()[0].runtime_id.get(), 1);
    let patch = plan.tls_patches()[0];
    assert_eq!(patch.definition().module().index(), 2);
    assert_eq!(patch.definition().symbol(), None);
    assert_eq!(patch.value(), 1);
    let prepared =
        load_relocated_static_tls(&plan, &storage(), &SessionLoadLimits::default()).unwrap();
    assert_eq!(prepared.applied_relocations(), 1);
    assert_eq!(
        word(prepared.memory(), CONSUMER_BIAS + CONSUMER_TLS.address),
        1
    );
    for instance in prepared.instances() {
        let block = instance.blocks[0];
        assert_eq!(block.module.index(), 2);
        assert_eq!(block.runtime_id.get(), 1);
        assert_eq!(word(prepared.memory(), block.address), 1);
        assert_eq!(word(prepared.memory(), block.address + 16), 0);
    }
    assert_eq!(images, original);
}
