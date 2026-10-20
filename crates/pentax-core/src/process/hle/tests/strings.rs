// SPDX-License-Identifier: GPL-2.0-only
// Original integration fixtures for the Kyty-derived string adapter, 2026-10-17.
// Source notices for translated string behavior: process/hle_strings.rs.

use super::super::dispatch::SITE_LOOKUP_WORK;
use super::super::fixture::{Asm, DATA, FixtureLimits, Import, Launched, RESULTS, SHIM, launch};
use super::super::provider::stub;
use super::*;
use crate::cpu::StopReason;
use crate::memory::{PAGE_SIZE, Permissions};
use crate::process::SessionLoadLimits;
use pentax_linker::DescriptorEncoding;

const FIRST: u64 = DATA + 0x100;
const SECOND: u64 = DATA + 0x140;
const NEEDLE: u64 = DATA + 0x180;
const OUTPUT: u64 = DATA + 0x200;

fn word(memory: &crate::memory::GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

fn read(memory: &crate::memory::GuestMemory, address: u64, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    memory.read(address, &mut bytes).unwrap();
    bytes
}

fn emit(asm: &mut Asm, function: KnownFunction, args: [u64; 3], result: usize, tail: bool) {
    for (register, argument) in [7, 6, 2].into_iter().zip(args) {
        asm.mov(register, argument);
    }
    if tail {
        asm.call(SHIM);
    } else {
        asm.call_slot(function.index());
    }
    asm.store_rax(RESULTS as u32 + result as u32 * 8);
}

fn consumer(main: Asm, shim: Option<Asm>) -> Consumer {
    let mut text = vec![(CODE, main.bytes)];
    if let Some(shim) = shim {
        text.push((SHIM, shim.bytes));
    }
    let mut consumer = Consumer::importing(text);
    consumer.imports = KnownFunction::LIBC.map(Import::function).to_vec();
    consumer
}

fn one(function: KnownFunction, args: [u64; 3], limits: &FixtureLimits) -> Launched {
    let mut asm = Asm::new(CODE);
    emit(&mut asm, function, args, 0, false);
    asm.int3();
    launch(&consumer(asm, None).build().unwrap(), limits).unwrap()
}

fn stop_before_service(launched: &mut Launched) {
    loop {
        if launched
            .gateway
            .cpu
            .step(&mut launched.gateway.memory)
            .unwrap()
            == Some(StopReason::Syscall)
        {
            return;
        }
    }
}

#[test]
fn all_thirteen_generated_exports_bind_in_both_domains_and_both_encodings() {
    let providers = providers();
    let original = providers.image().to_vec();
    for dynlib in [false, true] {
        for encoding in [
            DescriptorEncoding::Classic,
            DescriptorEncoding::Variant1Observed,
        ] {
            let mut asm = Asm::new(CODE);
            asm.int3();
            let mut description = consumer(asm, None);
            description.dynlib = dynlib;
            description.encoding = encoding;
            let image = description.build().unwrap();
            let before = image.clone();
            let plan = plan_one(&image, &providers).unwrap();
            assert_eq!(plan.provider_bindings(), 13);
            let prepared = prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
            assert_eq!(prepared.applied_relocations(), 13);
            for site in plan
                .sites()
                .iter()
                .filter(|site| KnownFunction::LIBC.contains(&site.function))
            {
                assert_eq!(
                    word(
                        prepared.memory(),
                        CONSUMER_BIAS + 0x4000 + 8 * site.function.index() as u64
                    ),
                    site.entry
                );
                let mut code = [0; 8];
                prepared.memory().fetch(site.entry, &mut code).unwrap();
                assert_eq!(code, stub(site.function));
            }
            assert_eq!(image, before);
        }
    }
    assert_eq!(providers.image(), original);
}

#[test]
fn retained_provider_cannot_bypass_a_tighter_planning_byte_limit() {
    let providers = providers();
    let image = minimal_consumer().build().unwrap();
    let inputs = [ModuleInput::new(
        open_executable(&image).unwrap(),
        CONSUMER_BIAS,
    )];
    let limits = HleLimits {
        max_provider_bytes: providers.image().len() as u64 - 1,
        ..HleLimits::default()
    };
    assert_eq!(
        plan_hle_session(&inputs, &providers, placement(), &limits).unwrap_err(),
        HleError::LimitExceeded {
            kind: HleBudget::ProviderBytes,
            requested: providers.image().len() as u64,
            maximum: limits.max_provider_bytes,
        }
    );
}

#[test]
fn closed_string_fixture_publishes_only_completed_repeatable_diagnostics() {
    let report = run_hle_string_fixture().unwrap();
    assert_eq!(
        (
            report.registered_functions,
            report.string_calls,
            report.memory_calls
        ),
        (15, 10, 1)
    );
    assert_eq!(
        (
            report.service_attempts,
            report.retired,
            report.provider_bindings,
            report.applied_relocations
        ),
        (11, 90, 13, 13)
    );
    assert_eq!(
        (
            report.charged_bytes,
            report.string_read_bytes,
            report.string_write_bytes
        ),
        (136, 118, 14)
    );
    assert_eq!(
        (report.work_units, report.string_scratch_peak_bytes),
        (82402, 14)
    );
    assert_eq!(
        (report.mapped_bytes, report.resident_bytes),
        (10 * PAGE_SIZE, 6 * PAGE_SIZE)
    );
    assert_eq!(report.provider_bias, PROVIDER_BIAS);
    assert_eq!(
        report.results,
        [
            6,
            u64::from(u32::MAX),
            0,
            OUTPUT,
            OUTPUT + 0x40,
            OUTPUT,
            OUTPUT + 1,
            OUTPUT + 8,
            OUTPUT + 1,
            9,
            OUTPUT + 0x60
        ]
    );
    assert_eq!(run_hle_string_fixture().unwrap(), report);
}

#[test]
fn original_linked_consumer_observes_all_nine_strings_repetition_tail_call_and_memory_service() {
    let ops = [
        (KnownFunction::Strlen, [FIRST, 0, u64::MAX], 6),
        (
            KnownFunction::Strcmp,
            [FIRST, SECOND, 0],
            u64::from(u32::MAX),
        ),
        (KnownFunction::Strncmp, [FIRST, SECOND, 6], 0),
        (KnownFunction::Strcpy, [OUTPUT, FIRST, 0], OUTPUT),
        (
            KnownFunction::Strncpy,
            [OUTPUT + 0x40, SECOND, 3],
            OUTPUT + 0x40,
        ),
        (KnownFunction::Strcat, [OUTPUT, NEEDLE, 0], OUTPUT),
        (
            KnownFunction::Strchr,
            [OUTPUT, u64::from(b'a'), 0],
            OUTPUT + 1,
        ),
        (
            KnownFunction::Strrchr,
            [OUTPUT, u64::from(b'a'), 0],
            OUTPUT + 8,
        ),
        (KnownFunction::Strstr, [OUTPUT, NEEDLE, 0], OUTPUT + 1),
        (KnownFunction::Strlen, [OUTPUT, 0, u64::MAX], 9),
        (
            KnownFunction::Memset,
            [OUTPUT + 0x60, u64::from(b'Q'), 4],
            OUTPUT + 0x60,
        ),
    ];
    let mut main = Asm::new(CODE);
    for (index, &(function, args, _)) in ops.iter().enumerate() {
        emit(&mut main, function, args, index, index == 9);
    }
    main.int3();
    let halt = CONSUMER_BIAS + main.rva() as u64;
    let mut shim = Asm::new(SHIM);
    shim.jmp_slot(KnownFunction::Strlen.index());
    let retired = main.retired + shim.retired + ops.len() as u64 * 3;
    let image = consumer(main, Some(shim)).build().unwrap();
    let before_image = image.clone();
    let mut launched = launch(&image, &FixtureLimits::default()).unwrap();
    for (address, bytes) in [
        (FIRST, &b"banana\0"[..]),
        (SECOND, &b"bananas\0"[..]),
        (NEEDLE, &b"ana\0"[..]),
    ] {
        launched.gateway.memory.write(address, bytes).unwrap();
    }
    launched
        .gateway
        .memory
        .write(OUTPUT, &[0xcc; 0x80])
        .unwrap();
    assert_eq!(launched.gateway.run(), Ok(halt));
    assert_eq!(launched.gateway.cpu.retired, retired);
    assert_eq!(launched.bindings, 13);
    assert_eq!(launched.applied, 13);
    for (index, &(_, _, expected)) in ops.iter().enumerate() {
        assert_eq!(
            word(&launched.gateway.memory, DATA + RESULTS + index as u64 * 8),
            expected
        );
    }
    assert_eq!(
        read(&launched.gateway.memory, OUTPUT, 11),
        b"bananaana\0\xcc"
    );
    assert_eq!(read(&launched.gateway.memory, OUTPUT + 0x40, 4), b"ban\xcc");
    assert_eq!(
        read(&launched.gateway.memory, OUTPUT + 0x60, 5),
        b"QQQQ\xcc"
    );
    assert_eq!(read(&launched.gateway.memory, FIRST, 7), b"banana\0");
    for register in [3, 4, 5, 12, 13, 14, 15] {
        assert_eq!(
            launched.gateway.cpu.registers[register],
            launched.initial.registers[register]
        );
    }
    assert_eq!(launched.gateway.cpu.fs_base, launched.initial.fs_base);
    let budget = &launched.gateway.budget;
    assert_eq!(budget.attempts(), ops.len() as u64);
    assert_eq!(budget.completed(), ops.len() as u64);
    assert_eq!(budget.calls(KnownFunction::Strlen), 2);
    for function in KnownFunction::LIBC {
        assert_eq!(
            budget.calls(function),
            ops.iter().filter(|op| op.0 == function).count() as u64
        );
    }
    assert!(budget.string_reads() > 0 && budget.string_writes() > 0);
    assert_eq!(
        budget.total_bytes(),
        4 + budget.string_reads() + budget.string_writes()
    );
    assert!(budget.string_scratch_peak() >= 10);
    assert_eq!(image, before_image);
}

#[test]
fn mixed_memory_then_string_calls_share_the_remaining_byte_budget_on_error() {
    let mut main = Asm::new(CODE);
    emit(&mut main, KnownFunction::Memset, [OUTPUT, 7, 4], 0, false);
    emit(
        &mut main,
        KnownFunction::Strlen,
        [FIRST, 0, u64::MAX],
        1,
        false,
    );
    main.int3();
    let mut limits = FixtureLimits::default();
    limits.hle.services.max_total_bytes = 5;
    let mut launched = launch(&consumer(main, None).build().unwrap(), &limits).unwrap();
    launched.gateway.memory.write(FIRST, b"ab\0").unwrap();
    assert_eq!(
        launched.gateway.run(),
        Err(HleError::Service(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::TotalBytes,
            requested: 6,
            maximum: 5
        }))
    );
    assert_eq!(launched.gateway.budget.total_bytes(), 5);
    assert_eq!(launched.gateway.budget.string_reads(), 1);
    assert_eq!(launched.gateway.budget.completed(), 1);
    assert_eq!(launched.gateway.budget.attempts(), 2);
    assert_eq!(read(&launched.gateway.memory, OUTPUT, 4), [7; 4]);
    assert_eq!(read(&launched.gateway.memory, FIRST, 3), b"ab\0");
}

#[test]
fn string_then_memory_uses_the_same_byte_counter_without_committing_denied_fill() {
    let mut main = Asm::new(CODE);
    emit(
        &mut main,
        KnownFunction::Strlen,
        [FIRST, 0, u64::MAX],
        0,
        false,
    );
    emit(&mut main, KnownFunction::Memset, [OUTPUT, 7, 4], 1, false);
    main.int3();
    let mut limits = FixtureLimits::default();
    limits.hle.services.max_total_bytes = 6;
    let mut launched = launch(&consumer(main, None).build().unwrap(), &limits).unwrap();
    launched.gateway.memory.write(FIRST, b"ab\0").unwrap();
    assert_eq!(
        launched.gateway.run(),
        Err(HleError::Service(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::TotalBytes,
            requested: 7,
            maximum: 6
        }))
    );
    assert_eq!(launched.gateway.budget.total_bytes(), 3);
    assert_eq!(launched.gateway.budget.completed(), 1);
    assert_eq!(read(&launched.gateway.memory, OUTPUT, 4), [0; 4]);
}

#[test]
fn repeated_string_calls_share_work_budget_and_keep_partial_error_charges() {
    let mut main = Asm::new(CODE);
    for result in 0..2 {
        emit(
            &mut main,
            KnownFunction::Strlen,
            [FIRST, 0, 0],
            result,
            false,
        );
    }
    main.int3();
    let mut limits = FixtureLimits::default();
    limits.hle.services.max_work_units = 2 * SITE_LOOKUP_WORK + 13;
    let mut launched = launch(&consumer(main, None).build().unwrap(), &limits).unwrap();
    launched.gateway.memory.write(FIRST, b"ab\0").unwrap();
    let maximum = limits.hle.services.max_work_units;
    assert_eq!(
        launched.gateway.run(),
        Err(HleError::Service(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::WorkUnits,
            requested: maximum + 1,
            maximum
        }))
    );
    assert_eq!(launched.gateway.budget.work(), maximum - 1);
    assert_eq!(launched.gateway.budget.string_reads(), 5);
    assert_eq!(launched.gateway.budget.attempts(), 2);
    assert_eq!(launched.gateway.budget.completed(), 1);
}

#[test]
fn string_service_failures_preserve_post_trap_cpu_and_destination() {
    for failure in [0, 1, 2, 3] {
        let mut limits = FixtureLimits::default();
        if failure == 2 {
            limits.hle.services.max_scratch_bytes = 0;
        }
        if failure == 3 {
            limits.hle.services.max_span_bytes = 2;
        }
        let source = if failure == 0 { OUTPUT + 1 } else { FIRST };
        let mut launched = one(KnownFunction::Strcpy, [OUTPUT, source, 0], &limits);
        launched.gateway.memory.write(FIRST, b"abcd\0").unwrap();
        launched
            .gateway
            .memory
            .write(OUTPUT, b"old\0canary")
            .unwrap();
        if failure == 1 {
            launched
                .gateway
                .memory
                .protect(DATA, PAGE_SIZE, Permissions::READ)
                .unwrap();
        }
        stop_before_service(&mut launched);
        let before_cpu = launched.gateway.cpu.clone();
        let before = read(&launched.gateway.memory, OUTPUT, 10);
        let resident = launched.gateway.memory.resident_bytes();
        let expected = match failure {
            0 => MemoryServiceError::UnsupportedOverlap,
            1 => MemoryServiceError::Memory(crate::memory::MemoryError::PermissionDenied(OUTPUT)),
            2 => MemoryServiceError::LimitExceeded {
                kind: ServiceLimit::ScratchBytes,
                requested: 5,
                maximum: 0,
            },
            _ => MemoryServiceError::UnterminatedString {
                argument: PointerArgument::Second,
                maximum: 2,
            },
        };
        assert_eq!(launched.gateway.service(), Err(HleError::Service(expected)));
        assert_eq!(launched.gateway.cpu, before_cpu);
        assert_eq!(read(&launched.gateway.memory, OUTPUT, 10), before);
        assert_eq!(launched.gateway.memory.resident_bytes(), resident);
        assert_eq!(launched.gateway.budget.completed(), 0);
        assert_eq!(launched.gateway.budget.attempts(), 1);
    }
}

#[test]
fn short_source_strncpy_padding_is_bounded_by_copy_budgets_separately_from_scan_bound() {
    let mut limits = FixtureLimits::default();
    limits.hle.services.max_span_bytes = 2;
    let mut launched = one(KnownFunction::Strncpy, [OUTPUT, FIRST, 8], &limits);
    launched.gateway.memory.write(FIRST, b"a\0").unwrap();
    assert!(launched.gateway.run().is_ok());
    assert_eq!(
        read(&launched.gateway.memory, OUTPUT, 8),
        b"a\0\0\0\0\0\0\0"
    );
    assert_eq!(launched.gateway.budget.string_reads(), 4);
    assert_eq!(launched.gateway.budget.string_writes(), 8);
    assert_eq!(launched.gateway.budget.total_bytes(), 12);
}
