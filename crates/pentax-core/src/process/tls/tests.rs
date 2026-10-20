// SPDX-License-Identifier: GPL-2.0-or-later

mod fixture;

use super::*;
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE, Permissions};
use crate::process::{SessionLoadLimits, load_symbolic};
use fixture::{ADDEND, DATA, DATA_FILE, Options, TEMPLATE, image, program, put};
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

const TP: [u64; 2] = [0x80_0000, 0xa0_0000];
const BIAS: u64 = 0x10_0000;

#[test]
fn observed_thread_control_storage_keeps_self_pointer_zero_padding_and_separate_upper_guard() {
    let images = vec![image(Options::default())];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = StaticTlsLimits {
        thread_control: ThreadControlBlockPolicy::KytyObservedSelfPointerV1,
        ..Default::default()
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    let prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    for instance in prepared.instances() {
        let header = instance.thread_control_block.unwrap();
        assert_eq!(header.address, instance.thread_pointer);
        assert_eq!(header.size, 64);
        assert_eq!(word(prepared.memory(), header.address), header.address);
        let mut rest = vec![1; PAGE_SIZE as usize - 8];
        prepared
            .memory()
            .read(header.address + 8, &mut rest)
            .unwrap();
        assert!(rest.iter().all(|value| *value == 0));
        let guards = instance.guard_pages.unwrap();
        assert_eq!(guards[1], header.address + PAGE_SIZE);
        for guard in guards {
            assert!(matches!(
                prepared.memory().read(guard, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ));
        }
        assert!(matches!(
            prepared.memory().fetch(header.address, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ));
    }
    let ordinary = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    let old = load_symbolic_with_static_tls(
        &ordinary,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(
        prepared.memory().mapped_bytes() - old.memory().mapped_bytes(),
        2 * PAGE_SIZE
    );
    assert!(
        old.instances()
            .iter()
            .all(|instance| instance.thread_control_block.is_none())
    );
}

#[test]
fn explicit_thread_control_pages_participate_in_storage_mapping_and_address_budgets() {
    let images = vec![image(Options::default())];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = StaticTlsLimits {
        thread_control: ThreadControlBlockPolicy::KytyObservedSelfPointerV1,
        max_storage_bytes: PAGE_SIZE,
        ..Default::default()
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    let storage = StaticTlsStorage {
        thread_pointers: &TP[..1],
    };
    assert!(matches!(
        load_symbolic_with_static_tls(&plan, &storage, &SessionLoadLimits::default()),
        Err(TlsError::Budget {
            kind: TlsBudget::StorageBytes,
            ..
        })
    ));
    let limits = StaticTlsLimits {
        max_storage_bytes: 4 * PAGE_SIZE,
        ..limits
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    let memory = SessionLoadLimits {
        mapped_limit: plan.image_mapped_bytes + 2 * PAGE_SIZE - 1,
        ..Default::default()
    };
    assert!(matches!(
        load_symbolic_with_static_tls(&plan, &storage, &memory),
        Err(TlsError::MemoryBudget {
            resident: false,
            ..
        })
    ));
    assert!(matches!(
        load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &[ADDRESS_LIMIT - PAGE_SIZE]
            },
            &SessionLoadLimits::default()
        ),
        Err(TlsError::InvalidStorage { .. })
    ));
}

#[test]
fn thread_control_without_static_templates_is_only_created_by_explicit_profile() {
    let images = vec![image(Options {
        tls: false,
        ..Default::default()
    })];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = StaticTlsLimits {
        thread_control: ThreadControlBlockPolicy::KytyObservedSelfPointerV1,
        ..Default::default()
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    let prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP[..1],
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    let instance = &prepared.instances()[0];
    assert!(instance.storage.is_none() && instance.blocks.is_empty());
    assert!(instance.thread_control_block.is_some() && instance.guard_pages.is_some());
    assert_eq!(word(prepared.memory(), TP[0]), TP[0]);
}

fn make_inputs(images: &[Vec<u8>]) -> Vec<ModuleInput<'_>> {
    images
        .iter()
        .enumerate()
        .map(|(index, bytes)| {
            ModuleInput::new(open_executable(bytes).unwrap(), BIAS * (index as u64 + 1))
        })
        .collect()
}

fn word(memory: &GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

#[test]
fn original_multi_module_layout_preserves_bias_order_and_independent_runtime_ids() {
    let images = vec![
        image(Options {
            tls: false,
            ..Options::default()
        }),
        image(Options::default()),
        image(Options {
            address: DATA,
            tls_offset: DATA_FILE as u64,
            initialized: 0,
            memory: 7,
            alignment: 16,
            ..Options::default()
        }),
        image(Options {
            address: DATA,
            tls_offset: DATA_FILE as u64,
            initialized: 0,
            memory: 0,
            alignment: 1,
            ..Options::default()
        }),
    ];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    assert_eq!(plan.policy(), StaticTlsPolicy::PentaXVariantIIInputOrderV1);
    assert_eq!(plan.maximum_alignment(), 32);
    assert_eq!(plan.static_extent(), 32);
    let layouts: Vec<_> = plan
        .templates()
        .iter()
        .map(|template| {
            (
                template.module.index(),
                template.runtime_id.get(),
                template.tp_offset,
                template.alignment,
                template.alignment_bias,
            )
        })
        .collect();
    assert_eq!(
        layouts,
        vec![(1, 1, 24, 32, 8), (2, 2, 32, 16, 0), (3, 3, 32, 1, 0)]
    );
    let repeat = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    assert_eq!(plan.templates(), repeat.templates());
    let prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    for instance in prepared.instances() {
        assert_eq!(instance.blocks.len(), 3);
        for (block, template) in instance.blocks.iter().zip(plan.templates()) {
            assert_eq!(block.module, template.module);
            assert_eq!(block.address, instance.thread_pointer - template.tp_offset);
            assert_eq!(block.address % template.alignment, template.alignment_bias);
        }
    }
}

#[test]
fn materialization_copies_relocated_pointers_from_the_retained_source_exactly_once() {
    let images = vec![image(Options {
        relocation: Some(8),
        ..Options::default()
    })];
    let snapshot = images.clone();
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    let source = plan.templates()[0].source_address;
    assert_eq!(source, BIAS + DATA + 8);
    let mut prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(prepared.applied_relocations(), 1);
    assert_eq!(word(prepared.memory(), source), BIAS + ADDEND);
    let first = prepared.instances()[0].blocks[0].address;
    let second = prepared.instances()[1].blocks[0].address;
    for address in [first, second] {
        assert_eq!(word(prepared.memory(), address), BIAS + ADDEND);
        assert_eq!(word(prepared.memory(), address + 8), 0);
        assert_eq!(
            prepared.memory().fetch(address, &mut [0]),
            Err(MemoryError::PermissionDenied(address))
        );
    }
    // Only this private child can mutate the owned session. Public users see &GuestMemory.
    prepared
        .session
        .memory
        .write(first, &99_u64.to_le_bytes())
        .unwrap();
    assert_eq!(word(prepared.memory(), first), 99);
    assert_eq!(word(prepared.memory(), second), BIAS + ADDEND);
    assert_eq!(word(prepared.memory(), source), BIAS + ADDEND);
    for instance in prepared.instances() {
        for guard in instance.guard_pages.unwrap() {
            assert!(matches!(
                prepared.memory().read(guard, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ));
        }
    }
    assert_eq!(images, snapshot);
}

#[test]
fn separate_plans_keep_different_image_biases_and_template_values_bound() {
    let a = vec![image(Options {
        relocation: Some(8),
        ..Options::default()
    })];
    let mut b = [image(Options {
        relocation: Some(8),
        ..Options::default()
    })];
    put(&mut b[0], DATA_FILE + 0x50, &0x9876_u64.to_le_bytes());
    let ai = make_inputs(&a);
    let bi = vec![ModuleInput::new(open_executable(&b[0]).unwrap(), BIAS * 3)];
    let al = plan_symbolic(&ai, &SymbolicLimits::default()).unwrap();
    let bl = plan_symbolic(&bi, &SymbolicLimits::default()).unwrap();
    let ap = plan_static_tls(&al, &StaticTlsLimits::default()).unwrap();
    let bp = plan_static_tls(&bl, &StaticTlsLimits::default()).unwrap();
    let storage = StaticTlsStorage {
        thread_pointers: &TP[..1],
    };
    let first =
        load_symbolic_with_static_tls(&ap, &storage, &SessionLoadLimits::default()).unwrap();
    let second =
        load_symbolic_with_static_tls(&bp, &storage, &SessionLoadLimits::default()).unwrap();
    assert_eq!(
        word(first.memory(), first.instances()[0].blocks[0].address),
        BIAS + ADDEND
    );
    assert_eq!(
        word(second.memory(), second.instances()[0].blocks[0].address),
        BIAS * 3 + 0x9876
    );
    // No materialization API argument can replace ap's retained link plan or memory with bl/second.
}

#[test]
fn initialized_coverage_accepts_adjacent_discontiguous_payloads_and_standalone_relro() {
    for options in [
        Options {
            split: true,
            ..Options::default()
        },
        Options {
            data_kind: 0x6100_0010,
            ..Options::default()
        },
    ] {
        let images = vec![image(options)];
        let inputs = make_inputs(&images);
        let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
        let prepared = load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP[..1],
            },
            &SessionLoadLimits::default(),
        )
        .unwrap();
        assert_eq!(
            word(prepared.memory(), prepared.instances()[0].blocks[0].address),
            TEMPLATE
        );
    }
}

#[test]
fn bss_padding_and_logical_gaps_never_supply_initialized_template_bytes() {
    for options in [
        Options {
            address: DATA + 0x308,
            ..Options::default()
        },
        Options {
            address: DATA + 0x208,
            data_mem_size: 0x100,
            ..Options::default()
        },
        Options {
            data_file_size: 12,
            ..Options::default()
        },
    ] {
        let images = vec![image(options)];
        let inputs = make_inputs(&images);
        let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        assert!(matches!(
            plan_static_tls(&link, &StaticTlsLimits::default()),
            Err(TlsError::SourceNotInitialized { module: 0, .. })
        ));
    }
    let mut bytes = image(Options {
        split: true,
        ..Options::default()
    });
    program(&mut bytes, 4, 1, 6, [0x700, DATA + 16, 0xf0, 0x3ff0, 1]);
    let images = vec![bytes];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    assert!(matches!(
        plan_static_tls(&link, &StaticTlsLimits::default()),
        Err(TlsError::SourceNotInitialized { .. })
    ));
}

#[test]
fn conflicting_pt_tls_payload_and_unreadable_source_are_explicit_errors() {
    let mut bytes = image(Options {
        tls_offset: 0x408,
        ..Options::default()
    });
    put(&mut bytes, 0x408, &98_u64.to_le_bytes());
    let images = vec![bytes];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    assert!(matches!(
        plan_static_tls(&link, &StaticTlsLimits::default()),
        Err(TlsError::TemplateBytesMismatch { module: 0 })
    ));
    let images = vec![image(Options {
        data_flags: 2,
        ..Options::default()
    })];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    assert!(matches!(
        plan_static_tls(&link, &StaticTlsLimits::default()),
        Err(TlsError::SourceNotReadable { module: 0, .. })
    ));
}

#[test]
fn zero_initialization_and_absent_tls_do_not_invent_template_reads_or_tcb_bytes() {
    for alignment in [0, 1, 32] {
        let images = vec![image(Options {
            initialized: 0,
            address: DATA + 0x308,
            alignment,
            ..Options::default()
        })];
        let inputs = make_inputs(&images);
        let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
        let prepared = load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP,
            },
            &SessionLoadLimits::default(),
        )
        .unwrap();
        for instance in prepared.instances() {
            let mut bytes = [1; 16];
            prepared
                .memory()
                .read(instance.blocks[0].address, &mut bytes)
                .unwrap();
            assert_eq!(bytes, [0; 16]);
            assert!(matches!(
                prepared.memory().read(instance.thread_pointer, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ));
        }
        assert_eq!(prepared.memory().resident_bytes(), 2 * PAGE_SIZE);
    }
    let images = vec![image(Options {
        tls: false,
        ..Options::default()
    })];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    assert!(plan.templates().is_empty());
    assert_eq!(plan.static_extent(), 0);
    let prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(prepared.memory().mapped_bytes(), 2 * PAGE_SIZE);
    assert!(
        prepared
            .instances()
            .iter()
            .all(|instance| instance.blocks.is_empty()
                && instance.storage.is_none()
                && instance.guard_pages.is_none())
    );
}

#[test]
fn deferred_records_refuse_tls_materialization_without_changing_generic_staging() {
    let images = vec![image(Options {
        relocation: Some(16),
        ..Options::default()
    })];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    assert_eq!(
        load_symbolic(&link, &SessionLoadLimits::default())
            .unwrap()
            .deferred_relocations(),
        1
    );
    assert!(matches!(
        load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP
            },
            &SessionLoadLimits {
                resident_limit: 0,
                mapped_limit: 0
            }
        ),
        Err(TlsError::DeferredRelocations { count: 1 })
    ));
}

#[test]
fn planner_budgets_are_aggregate_and_charged_before_unbounded_work() {
    let images = vec![image(Options::default()), image(Options::default())];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let cases = [
        (
            StaticTlsLimits {
                max_modules: 1,
                ..StaticTlsLimits::default()
            },
            TlsBudget::Modules,
        ),
        (
            StaticTlsLimits {
                max_templates: 1,
                ..StaticTlsLimits::default()
            },
            TlsBudget::Templates,
        ),
        (
            StaticTlsLimits {
                max_segments: 3,
                ..StaticTlsLimits::default()
            },
            TlsBudget::Segments,
        ),
        (
            StaticTlsLimits {
                max_initialized_bytes: 15,
                ..StaticTlsLimits::default()
            },
            TlsBudget::InitializedBytes,
        ),
        (
            StaticTlsLimits {
                max_static_bytes: 55,
                ..StaticTlsLimits::default()
            },
            TlsBudget::StaticBytes,
        ),
        (
            StaticTlsLimits {
                max_alignment: 16,
                ..StaticTlsLimits::default()
            },
            TlsBudget::Alignment,
        ),
        (
            StaticTlsLimits {
                max_work_steps: 0,
                ..StaticTlsLimits::default()
            },
            TlsBudget::WorkSteps,
        ),
    ];
    for (limits, expected) in cases {
        assert!(
            matches!(plan_static_tls(&link, &limits), Err(TlsError::Budget { kind, .. }) if kind == expected)
        );
    }
    let exact = StaticTlsLimits {
        max_modules: 2,
        max_templates: 2,
        max_segments: 4,
        max_initialized_bytes: 16,
        max_static_bytes: 56,
        max_alignment: 32,
        ..StaticTlsLimits::default()
    };
    assert_eq!(plan_static_tls(&link, &exact).unwrap().static_extent(), 56);
}

#[test]
fn instance_storage_and_exact_shared_page_budgets_are_preflighted() {
    let images = vec![image(Options {
        split: true,
        relocation: Some(8),
        ..Options::default()
    })];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = StaticTlsLimits {
        max_instances: 1,
        ..StaticTlsLimits::default()
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    assert!(matches!(
        load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP
            },
            &SessionLoadLimits::default()
        ),
        Err(TlsError::Budget {
            kind: TlsBudget::Instances,
            ..
        })
    ));
    let limits = StaticTlsLimits {
        max_storage_bytes: PAGE_SIZE,
        ..StaticTlsLimits::default()
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    assert!(matches!(
        load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP
            },
            &SessionLoadLimits::default()
        ),
        Err(TlsError::Budget {
            kind: TlsBudget::StorageBytes,
            ..
        })
    ));
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    let storage = StaticTlsStorage {
        thread_pointers: &TP,
    };
    let exact = SessionLoadLimits {
        mapped_limit: 4 * PAGE_SIZE,
        resident_limit: 4 * PAGE_SIZE,
    };
    let prepared = load_symbolic_with_static_tls(&plan, &storage, &exact).unwrap();
    assert_eq!(prepared.memory().mapped_bytes(), exact.mapped_limit);
    assert_eq!(prepared.memory().resident_bytes(), exact.resident_limit);
    for limits in [
        SessionLoadLimits {
            mapped_limit: exact.mapped_limit - 1,
            ..exact.clone()
        },
        SessionLoadLimits {
            resident_limit: exact.resident_limit - 1,
            ..exact.clone()
        },
    ] {
        assert!(matches!(
            load_symbolic_with_static_tls(&plan, &storage, &limits),
            Err(TlsError::MemoryBudget { .. })
        ));
    }
}

#[test]
fn guards_destinations_and_guest_address_arithmetic_cannot_alias_images_or_instances() {
    let images = vec![image(Options::default())];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    for pointers in [
        vec![0],
        vec![PAGE_SIZE],
        vec![ADDRESS_LIMIT],
        vec![ADDRESS_LIMIT - 1],
        vec![TP[0] + 1],
    ] {
        assert!(matches!(
            load_symbolic_with_static_tls(
                &plan,
                &StaticTlsStorage {
                    thread_pointers: &pointers
                },
                &SessionLoadLimits::default()
            ),
            Err(TlsError::InvalidStorage { .. })
        ));
    }
    for pointers in [
        vec![BIAS + PAGE_SIZE],
        vec![BIAS + 2 * PAGE_SIZE],
        vec![TP[0], TP[0]],
        vec![TP[0], TP[0] + 2 * PAGE_SIZE],
    ] {
        assert!(matches!(
            load_symbolic_with_static_tls(
                &plan,
                &StaticTlsStorage {
                    thread_pointers: &pointers
                },
                &SessionLoadLimits::default()
            ),
            Err(TlsError::StorageOverlap { .. })
        ));
    }
}

#[test]
fn rejected_preparation_leaves_existing_sessions_and_source_bytes_untouched() {
    let images = vec![image(Options {
        relocation: Some(8),
        ..Options::default()
    })];
    let snapshots = images.clone();
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    let existing = load_symbolic(&link, &SessionLoadLimits::default()).unwrap();
    for limits in [
        SessionLoadLimits {
            resident_limit: 0,
            ..SessionLoadLimits::default()
        },
        SessionLoadLimits {
            mapped_limit: 0,
            ..SessionLoadLimits::default()
        },
    ] {
        assert!(
            load_symbolic_with_static_tls(
                &plan,
                &StaticTlsStorage {
                    thread_pointers: &TP
                },
                &limits
            )
            .is_err()
        );
        assert_eq!(word(existing.memory(), BIAS + DATA + 8), BIAS + ADDEND);
        assert!(matches!(
            existing.memory().read(TP[0] - PAGE_SIZE, &mut [0]),
            Err(MemoryError::Unmapped(_))
        ));
        assert_eq!(images, snapshots);
    }
    // A failure after owned image staging also publishes no partially prepared result.
    let mut altered = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    altered.templates[0].source_address = ADDRESS_LIMIT - 8;
    assert!(matches!(
        load_symbolic_with_static_tls(
            &altered,
            &StaticTlsStorage {
                thread_pointers: &TP
            },
            &SessionLoadLimits::default()
        ),
        Err(TlsError::Memory(MemoryError::Unmapped(_)))
    ));
    let mut complete = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    let range = complete.instances()[0].storage.unwrap();
    complete
        .session
        .memory
        .protect(range.address, range.size, Permissions::READ)
        .unwrap();
    assert!(
        complete
            .session
            .memory
            .write(complete.instances()[0].blocks[0].address, &[0])
            .is_err()
    );
    assert_eq!(images, snapshots);
}

#[test]
fn zero_sized_tls_retains_identity_and_extreme_storage_respects_guest_bounds() {
    let images = vec![image(Options {
        initialized: 0,
        memory: 0,
        alignment: 1,
        address: DATA + 0x308,
        ..Options::default()
    })];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    assert_eq!(plan.templates().len(), 1);
    assert_eq!(plan.templates()[0].runtime_id.get(), 1);
    assert_eq!(plan.static_extent(), 0);
    let prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TP,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    assert!(
        prepared
            .instances()
            .iter()
            .all(|instance| instance.storage.is_none()
                && instance.blocks.len() == 1
                && instance.blocks[0].size == 0)
    );
    assert_eq!(prepared.memory().mapped_bytes(), 2 * PAGE_SIZE);
    let images = vec![image(Options::default())];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    let last = [ADDRESS_LIMIT - PAGE_SIZE];
    let prepared = load_symbolic_with_static_tls(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &last,
        },
        &SessionLoadLimits::default(),
    )
    .unwrap();
    assert_eq!(
        word(prepared.memory(), prepared.instances()[0].blocks[0].address),
        TEMPLATE
    );
    assert_eq!(
        prepared.memory().read(last[0], &mut [0]),
        Err(MemoryError::Unmapped(last[0]))
    );
    let too_high = [ModuleInput::new(
        open_executable(&images[0]).unwrap(),
        ADDRESS_LIMIT - PAGE_SIZE,
    )];
    let link = plan_symbolic(&too_high, &SymbolicLimits::default()).unwrap();
    assert!(matches!(
        plan_static_tls(&link, &StaticTlsLimits::default()),
        Err(TlsError::AddressOverflow)
    ));
}

#[test]
fn large_layout_alignment_and_materialization_work_stay_within_explicit_policy() {
    for options in [
        Options {
            memory: 4 * 1024 * 1024 + 1,
            ..Options::default()
        },
        Options {
            address: 8,
            tls_offset: 8,
            alignment: 2 * PAGE_SIZE,
            ..Options::default()
        },
    ] {
        let images = vec![image(options)];
        let inputs = make_inputs(&images);
        let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        let limits = StaticTlsLimits {
            max_alignment: 2 * PAGE_SIZE,
            ..StaticTlsLimits::default()
        };
        assert!(plan_static_tls(&link, &limits).is_err());
    }
    let mut bytes = image(Options {
        initialized: 0,
        ..Options::default()
    });
    program(&mut bytes, 4, 1, 4, [0x700, 0xc000, 1, PAGE_SIZE, 1]);
    let images = vec![bytes];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let plan = plan_static_tls(
        &link,
        &StaticTlsLimits {
            max_work_steps: 7,
            ..StaticTlsLimits::default()
        },
    )
    .unwrap();
    assert!(matches!(
        load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP
            },
            &SessionLoadLimits::default()
        ),
        Err(TlsError::Budget {
            kind: TlsBudget::WorkSteps,
            limit: 7
        })
    ));
}

#[test]
fn image_relocations_crossing_the_tls_file_boundary_preserve_zero_instance_tails() {
    // The initialized source is [DATA+8, DATA+16), with zero tail to DATA+24.
    for (initialized, target) in [
        (8, DATA + 16),
        (8, DATA + 12),
        (8, DATA + 20),
        (0, DATA + 8),
    ] {
        let mut bytes = image(Options {
            initialized,
            relocation: Some(8),
            ..Options::default()
        });
        put(&mut bytes, DATA_FILE + 0x40, &target.to_le_bytes());
        let images = vec![
            image(Options {
                tls: false,
                ..Options::default()
            }),
            bytes,
        ];
        let snapshots = images.clone();
        let inputs = make_inputs(&images);
        let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        // The ordinary image write is real; its numerical overlap with a TLS
        // tail must neither reject it nor copy it into per-thread zero bytes.
        let staged = load_symbolic(&link, &SessionLoadLimits::default()).unwrap();
        assert_eq!(word(staged.memory(), BIAS * 2 + target), BIAS * 2 + ADDEND);
        let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
        let prepared = load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP,
            },
            &SessionLoadLimits::default(),
        )
        .unwrap();
        assert_eq!(
            word(prepared.memory(), BIAS * 2 + target),
            BIAS * 2 + ADDEND
        );
        let mut expected = vec![0_u8; 16];
        staged
            .memory()
            .read(BIAS * 2 + DATA + 8, &mut expected[..initialized as usize])
            .unwrap();
        for instance in prepared.instances() {
            let mut actual = vec![0xff_u8; 16];
            prepared
                .memory()
                .read(instance.blocks[0].address, &mut actual)
                .unwrap();
            assert_eq!(actual, expected);
        }
        assert_eq!(images, snapshots);
    }
}

#[test]
fn initialized_only_boundary_and_unrelated_relocations_remain_accepted() {
    for (initialized, memory, target) in [
        (8, 16, DATA + 8),  // Ends exactly at the initialized boundary.
        (8, 16, DATA + 4),  // Crosses the template start, entirely before the tail.
        (8, 16, DATA + 24), // Starts exactly after the declared TLS memory.
        (8, 8, DATA + 8),   // Fully initialized template, with no tail.
        (0, 0, DATA + 8),   // Zero-sized template, with no bytes to lose.
    ] {
        let mut bytes = image(Options {
            initialized,
            memory,
            relocation: Some(8),
            ..Options::default()
        });
        put(&mut bytes, DATA_FILE + 0x40, &target.to_le_bytes());
        let images = vec![bytes];
        let inputs = make_inputs(&images);
        let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        let plan = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
        let prepared = load_symbolic_with_static_tls(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &TP,
            },
            &SessionLoadLimits::default(),
        )
        .unwrap();
        for instance in prepared.instances() {
            if initialized != 0 {
                assert_eq!(
                    word(prepared.memory(), instance.blocks[0].address),
                    word(prepared.memory(), BIAS + DATA + 8)
                );
            }
            if memory > initialized {
                assert_eq!(
                    word(prepared.memory(), instance.blocks[0].address + initialized),
                    0
                );
            }
        }
    }
}

#[test]
fn layout_work_is_bounded_without_an_image_patch_by_tls_tail_cross_product() {
    let images = vec![
        image(Options {
            relocation: Some(8),
            ..Options::default()
        }),
        image(Options {
            relocation: Some(8),
            ..Options::default()
        }),
    ];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    // Actual layout planning consumes 28 units. Independent image patches
    // do not require visits to every per-thread zero tail.
    assert!(matches!(
        plan_static_tls(
            &link,
            &StaticTlsLimits {
                max_work_steps: 27,
                ..StaticTlsLimits::default()
            }
        ),
        Err(TlsError::Budget {
            kind: TlsBudget::WorkSteps,
            limit: 27
        })
    ));
    assert!(
        plan_static_tls(
            &link,
            &StaticTlsLimits {
                max_work_steps: 28,
                ..StaticTlsLimits::default()
            }
        )
        .is_ok()
    );
}

#[test]
fn caller_instance_limit_is_honored_with_shared_storage_and_memory_bounds() {
    let images = vec![image(Options::default())];
    let inputs = make_inputs(&images);
    let link = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let pointers = [TP[0], TP[1], 0xc0_0000];
    let storage = StaticTlsStorage {
        thread_pointers: &pointers,
    };
    let limits = StaticTlsLimits {
        max_instances: 3,
        ..StaticTlsLimits::default()
    };
    let plan = plan_static_tls(&link, &limits).unwrap();
    let memory_limits = SessionLoadLimits {
        mapped_limit: 5 * PAGE_SIZE,
        resident_limit: 5 * PAGE_SIZE,
    };
    let mut prepared = load_symbolic_with_static_tls(&plan, &storage, &memory_limits).unwrap();
    assert_eq!(prepared.instances().len(), 3);
    let addresses: Vec<_> = prepared
        .instances()
        .iter()
        .map(|instance| instance.blocks[0].address)
        .collect();
    prepared
        .session
        .memory
        .write(addresses[0], &99_u64.to_le_bytes())
        .unwrap();
    assert_eq!(word(prepared.memory(), addresses[0]), 99);
    for address in &addresses[1..] {
        assert_eq!(word(prepared.memory(), *address), TEMPLATE);
    }
    assert_eq!(prepared.memory().mapped_bytes(), memory_limits.mapped_limit);
    assert_eq!(
        prepared.memory().resident_bytes(),
        memory_limits.resident_limit
    );
    let default = plan_static_tls(&link, &StaticTlsLimits::default()).unwrap();
    assert!(matches!(
        load_symbolic_with_static_tls(&default, &storage, &memory_limits),
        Err(TlsError::Budget {
            kind: TlsBudget::Instances,
            limit: 2
        })
    ));
    let limited = plan_static_tls(
        &link,
        &StaticTlsLimits {
            max_storage_bytes: 2 * PAGE_SIZE,
            ..limits
        },
    )
    .unwrap();
    assert!(matches!(
        load_symbolic_with_static_tls(&limited, &storage, &memory_limits),
        Err(TlsError::Budget {
            kind: TlsBudget::StorageBytes,
            ..
        })
    ));
    assert!(matches!(
        load_symbolic_with_static_tls(
            &plan,
            &storage,
            &SessionLoadLimits {
                resident_limit: memory_limits.resident_limit - 1,
                ..memory_limits.clone()
            }
        ),
        Err(TlsError::MemoryBudget { resident: true, .. })
    ));
}
