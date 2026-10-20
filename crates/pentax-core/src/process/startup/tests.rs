// SPDX-License-Identifier: GPL-2.0-or-later

use super::*;
use fixture::{CTOR, ENTRY, PROVIDER_ENTRY, TLS_HEADER, program, put};

fn images(layout: Layout) -> Images {
    fixture::images(layout).unwrap()
}

#[test]
fn closed_original_startup_observes_linked_calls_arguments_stack_and_independent_tls() {
    let report = run_startup_fixture().unwrap();
    assert_eq!(
        (report.value, report.provider_value, report.service_value),
        (42, 42, 42)
    );
    assert_eq!(
        (
            report.constructor_calls,
            report.service_calls,
            report.argc,
            report.argument_assertions
        ),
        (1, 1, 2, 7)
    );
    assert_eq!(
        (
            report.tls_initialized,
            report.independent_tls_initialized,
            report.independent_tls_zero_tail
        ),
        (37, 37, 0)
    );
    assert_eq!(
        report.provider_address,
        fixture::PROVIDER_BIAS + PROVIDER_ENTRY
    );
    assert_eq!(report.applied_relocations, 1);
    assert_eq!(report.retired, 42);
    assert_eq!(report.mapped_bytes, 10 * PAGE_SIZE);
    assert_eq!(report.resident_bytes, 7 * PAGE_SIZE);
    assert_eq!(run_startup_fixture().unwrap(), report);
}

#[test]
fn all_retired_instructions_share_one_budget_across_the_service_resume() {
    let layout = Layout::default();
    let original = images(layout);
    let report = run_original(&original, layout, &Limits::default()).unwrap();
    for budget in [0, 1, report.retired - 2, report.retired - 1] {
        let limits = Limits {
            instructions: budget,
            ..Limits::default()
        };
        assert!(matches!(run_original(&original, layout, &limits),
            Err(StartupFixtureError::InstructionBudget { limit, retired }) if limit == budget && retired == budget));
    }
    let exact = Limits {
        instructions: report.retired,
        ..Limits::default()
    };
    assert_eq!(run_original(&original, layout, &exact).unwrap(), report);
    let oversized = Limits {
        instructions: MAX_INSTRUCTIONS + 1,
        ..exact
    };
    assert!(matches!(
        run_original(&original, layout, &oversized),
        Err(StartupFixtureError::InvalidFixture(_))
    ));
}

#[test]
fn unknown_service_wrong_arguments_and_foreign_sites_never_complete_a_service() {
    let layout = Layout::default();
    let mut original = images(layout);
    put(
        &mut original.consumer,
        original.code.service_number_at,
        &0x5058_0002_u32.to_le_bytes(),
    );
    assert!(matches!(
        run_original(&original, layout, &Limits::default()),
        Err(StartupFixtureError::UnknownService(0x5058_0002))
    ));
    let mut original = images(layout);
    put(
        &mut original.consumer,
        original.code.service_argument_at,
        &20_u32.to_le_bytes(),
    );
    assert!(matches!(
        run_original(&original, layout, &Limits::default()),
        Err(StartupFixtureError::InvalidFixture(
            "service arguments or context"
        ))
    ));

    let mut cpu = ReferenceCpu::at(0x1234);
    cpu.registers[0] = SERVICE;
    cpu.registers[1] = 0x1234;
    cpu.registers[7] = 19;
    cpu.registers[6] = 23;
    cpu.registers[4] = 0x8000;
    cpu.fs_base = 0xa008;
    let before = cpu.clone();
    let mut count = 0;
    assert!(service(&mut cpu, 0x1235, 0xa008, 0x8000, &mut count).is_err());
    assert_eq!(cpu, before);
    assert_eq!(count, 0);
    service(&mut cpu, 0x1234, 0xa008, 0x8000, &mut count).unwrap();
    cpu.registers[0] = SERVICE;
    let before = cpu.clone();
    assert!(service(&mut cpu, 0x1234, 0xa008, 0x8000, &mut count).is_err());
    assert_eq!(cpu, before);
    assert_eq!(count, 1);
}

#[test]
fn breakpoint_opcode_and_constructor_counter_are_not_success_shortcuts() {
    let layout = Layout::default();
    for opcode in [0xcc, 0xf4] {
        let mut original = images(layout);
        original.consumer[ENTRY as usize] = opcode;
        let result = run_original(&original, layout, &Limits::default());
        if opcode == 0xcc {
            assert!(matches!(
                result,
                Err(StartupFixtureError::InvalidFixture(_))
            ));
        } else {
            assert!(matches!(
                result,
                Err(StartupFixtureError::Cpu(
                    CpuError::UnsupportedInstruction { .. }
                ))
            ));
        }
    }
    let mut original = images(layout);
    // Make the original constructor store two. A final breakpoint and correct
    // arithmetic still cannot satisfy the exactly-once constructor assertion.
    put(
        &mut original.consumer,
        CTOR as usize + 15,
        &2_u32.to_le_bytes(),
    );
    assert!(matches!(
        run_original(&original, layout, &Limits::default()),
        Err(StartupFixtureError::InvalidFixture(
            "final value or exactly-once counters"
        ))
    ));
    let mut original = images(layout);
    original.provider[PROVIDER_ENTRY as usize + 5] = 6;
    assert!(matches!(
        run_original(&original, layout, &Limits::default()),
        Err(StartupFixtureError::InvalidFixture(_))
    ));
}

#[test]
fn late_storage_budget_failure_drops_the_attempt_and_preserves_originals() {
    let layout = Layout::default();
    let original = images(layout);
    let before = (original.consumer.clone(), original.provider.clone());
    for (resident, mapped) in [(6, 10), (7, 9)] {
        let limits = Limits {
            memory: SessionLoadLimits {
                resident_limit: resident * PAGE_SIZE,
                mapped_limit: mapped * PAGE_SIZE,
            },
            ..Limits::default()
        };
        assert!(matches!(
            run_original(&original, layout, &limits),
            Err(StartupFixtureError::Memory(MemoryError::BudgetExceeded))
        ));
        assert_eq!(
            (&original.consumer, &original.provider),
            (&before.0, &before.1)
        );
    }
    assert_eq!(
        run_original(&original, layout, &Limits::default())
            .unwrap()
            .value,
        42
    );
    assert_eq!((original.consumer, original.provider), before);
}

#[test]
fn storage_cannot_share_ownership_with_images_stack_arguments_or_other_tls() {
    let original = images(Layout::default());
    let baseline = Layout::default();
    for layout in [
        Layout {
            tls_pages: [baseline.stack, baseline.tls_pages[1]],
            ..baseline
        },
        Layout {
            tls_pages: [baseline.stack - PAGE_SIZE, baseline.tls_pages[1]],
            ..baseline
        },
        Layout {
            tls_pages: [baseline.tls_pages[0]; 2],
            ..baseline
        },
        Layout {
            boot: fixture::CONSUMER_BIAS,
            ..baseline
        },
        Layout {
            tls_pages: [fixture::PROVIDER_BIAS, baseline.tls_pages[1]],
            ..baseline
        },
        Layout {
            stack: ADDRESS_LIMIT - PAGE_SIZE,
            ..baseline
        },
        Layout {
            tls_pages: [baseline.tls_pages[0] + 1, baseline.tls_pages[1]],
            ..baseline
        },
    ] {
        assert!(run_original(&original, layout, &Limits::default()).is_err());
    }
}

#[test]
fn malformed_or_unsupported_bound_tls_does_not_become_synthetic_storage() {
    for [offset, address, file_size, mem_size, alignment] in [
        [0x908, 0x908, 17, 16, 32],
        [0x908, 0x908, 8, 24, 32],
        [0x908, 0x908, 8, 16, 8],
        [0x900, 0x900, 8, 16, 32],
    ] {
        let mut original = images(Layout::default());
        program(
            &mut original.provider,
            TLS_HEADER,
            7,
            4,
            [offset, address, file_size, mem_size, alignment],
        );
        assert!(run_original(&original, Layout::default(), &Limits::default()).is_err());
    }
}

#[test]
fn bound_staged_template_is_used_and_boot_terminators_and_storage_permissions_are_checked() {
    let layout = Layout::default();
    let mut original = images(layout);
    put(&mut original.provider, 0x908, &13_u64.to_le_bytes());
    // Add a genuine supported RELATIVE write to this original TLS source.
    // Both copies must contain its staged result, not the ELF's thirteen.
    put(&mut original.provider, 0xb80, &0x908_u64.to_le_bytes());
    put(&mut original.provider, 0xb88, &8_u64.to_le_bytes());
    put(
        &mut original.provider,
        0xb90,
        &(37_i64 - fixture::PROVIDER_BIAS as i64).to_le_bytes(),
    );
    for (index, (tag, value)) in [(7_i64, 0xb80_u64), (8, 24), (9, 24), (0, 0)]
        .into_iter()
        .enumerate()
    {
        put(
            &mut original.provider,
            0x870 + index * 16,
            &tag.to_le_bytes(),
        );
        put(
            &mut original.provider,
            0x878 + index * 16,
            &value.to_le_bytes(),
        );
    }
    program(&mut original.provider, 1, 2, 4, [0x800, 0x800, 176, 176, 8]);
    let before = (original.consumer.clone(), original.provider.clone());
    let plan = link_original(&original.consumer, &original.provider).unwrap();
    assert_eq!(plan.patch_count(), 2);
    let tls_plan = plan_static_tls(&plan, &StaticTlsLimits::default()).unwrap();
    let template = fixture_template(&tls_plan).unwrap();
    assert_eq!(template.tp_offset, original.code.tls_offset);
    let tps = thread_pointers(layout).unwrap();
    let prepared = load_symbolic_with_static_tls(
        &tls_plan,
        &StaticTlsStorage {
            thread_pointers: &tps,
        },
        &Limits::default().memory,
    )
    .unwrap();
    let tls = fixture_blocks(&prepared, layout).unwrap();
    let mut session = prepared.session;
    initialize_storage(&mut session.memory, layout).unwrap();
    assert_eq!(
        word(&session.memory, fixture::PROVIDER_BIAS + 0x908).unwrap(),
        37
    );
    for address in tls {
        assert_eq!(address % 32, 8);
        assert_eq!(word(&session.memory, address).unwrap(), 37);
        assert_eq!(word(&session.memory, address + 8).unwrap(), 0);
        assert!(matches!(
            session.memory.fetch(address, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ));
    }
    session
        .memory
        .write(tls[0] + 8, &1_u64.to_le_bytes())
        .unwrap();
    assert_eq!(word(&session.memory, tls[1] + 8).unwrap(), 0);
    check_guards(&session.memory, layout).unwrap();
    check_arguments(&session.memory, layout).unwrap();
    session
        .memory
        .write(layout.boot + 24, &1_u64.to_le_bytes())
        .unwrap();
    assert!(check_arguments(&session.memory, layout).is_err());
    assert_eq!(&original.provider[0x908..0x910], &13_u64.to_le_bytes());
    assert_eq!((original.consumer, original.provider), before);
}

#[test]
fn final_original_instructions_use_the_planned_negative_offsets_without_tcb_bytes() {
    let layout = Layout::default();
    let original = images(layout);
    let plan = link_original(&original.consumer, &original.provider).unwrap();
    let tls_plan = plan_static_tls(&plan, &StaticTlsLimits::default()).unwrap();
    let template = fixture_template(&tls_plan).unwrap();
    assert_eq!(template.tp_offset, 24);
    assert_eq!(
        &original.provider[PROVIDER_ENTRY as usize..PROVIDER_ENTRY as usize + 4],
        &[0x64, 0x8b, 0x43, (-24_i8) as u8]
    );
    assert_eq!(
        &original.consumer[CTOR as usize..CTOR as usize + 5],
        &[0x64, 0x48, 0x8b, 0x43, (-16_i8) as u8]
    );
    let tps = thread_pointers(layout).unwrap();
    let prepared = load_symbolic_with_static_tls(
        &tls_plan,
        &StaticTlsStorage {
            thread_pointers: &tps,
        },
        &Limits::default().memory,
    )
    .unwrap();
    let blocks = fixture_blocks(&prepared, layout).unwrap();
    for (block, tp) in blocks.into_iter().zip(tps) {
        assert_eq!(tp - block, template.tp_offset);
        assert!(matches!(
            prepared.memory().read(tp, &mut [0]),
            Err(MemoryError::Unmapped(_))
        ));
    }
    let mut mismatch = original;
    mismatch.code.tls_offset += 1;
    assert!(matches!(
        run_original(&mismatch, layout, &Limits::default()),
        Err(StartupFixtureError::InvalidFixture(
            "final image TLS displacement ownership"
        ))
    ));
}
