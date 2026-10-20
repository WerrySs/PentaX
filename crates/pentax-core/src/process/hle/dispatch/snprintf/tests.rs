// SPDX-License-Identifier: GPL-2.0-only
//! Original expectations over retained guest pages, not host variadic calls.
use super::*;
use crate::memory::{GuestMemory, Permissions};

const BASE: u64 = 0x10_0000;
const FORMAT: u64 = BASE + 0x100;
const TEXT: u64 = BASE + 0x400;
const DEST: u64 = BASE + PAGE_SIZE;
const STACK: u64 = BASE + 2 * PAGE_SIZE;

fn fixture(format: &[u8]) -> (GuestMemory, ServiceBudget) {
    let mut memory = GuestMemory::new(4 * PAGE_SIZE, 4 * PAGE_SIZE);
    memory
        .map(BASE, 4 * PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory.write(FORMAT, format).unwrap();
    memory.write(DEST, &[0xa7; 256]).unwrap();
    (memory, ServiceBudget::new(MemoryServiceLimits::default()))
}

fn context(gp: [u64; 3]) -> VariadicContext {
    VariadicContext {
        gp,
        vectors: None,
        stack_pointer: Some(STACK),
    }
}

fn stack(memory: &mut GuestMemory, values: &[u64]) {
    for (index, value) in values.iter().enumerate() {
        memory
            .write(STACK + 8 + index as u64 * 8, &value.to_le_bytes())
            .unwrap();
    }
}

fn call(
    memory: &mut GuestMemory,
    budget: &mut ServiceBudget,
    context: Option<VariadicContext>,
    size: u64,
) -> Result<u64, MemoryServiceError> {
    let attempt = budget.begin_attempt()?;
    invoke_with_variadic(
        attempt,
        KnownFunction::Snprintf,
        AbiArgs {
            rdi: DEST,
            rsi: size,
            rdx: FORMAT,
        },
        memory,
        budget,
        context,
    )
    .map(|value| value.rax)
}

fn bytes(memory: &GuestMemory, address: u64, length: usize) -> Vec<u8> {
    let mut bytes = vec![0; length];
    memory.read(address, &mut bytes).unwrap();
    bytes
}

fn output(memory: &GuestMemory, expected: &[u8]) {
    assert_eq!(bytes(memory, DEST, expected.len()), expected);
    assert_eq!(bytes(memory, DEST + expected.len() as u64, 1), [0]);
    assert_eq!(bytes(memory, DEST + expected.len() as u64 + 1, 1), [0xa7]);
}

#[test]
fn integer_narrowing_and_full_width_lengths_consume_actual_gp_and_stack_words() {
    let (mut memory, mut budget) = fixture(b"%hhd|%hhu|%hd|%hu|%ld|%lld|%zu\0");
    stack(&mut memory, &[0xffff, i64::MIN as u64, u64::MAX, 42]);
    let expected = b"-1|255|-1|65535|-9223372036854775808|-1|42";
    assert_eq!(
        call(
            &mut memory,
            &mut budget,
            Some(context([0xff, 0xff, 0xffff])),
            256
        ),
        Ok(expected.len() as u64)
    );
    output(&memory, expected);
    assert_eq!(budget.calls(KnownFunction::Snprintf), 1);
}

#[test]
fn source_flags_wrapping_fields_and_unknown_conversions_preserve_exact_bytes() {
    let (mut memory, mut budget) = fixture(b"%+08d|%#08x|%#.0x|%-8.4d|%.*s|%q|%n|%%\0");
    memory.write(TEXT, b"raw\xff\0").unwrap();
    stack(&mut memory, &[12, u64::MAX, TEXT]);
    let expected = b"-0000042|0x00002a||12      |raw\xff|q|n|%";
    assert_eq!(
        call(
            &mut memory,
            &mut budget,
            Some(context([(-42_i32) as u32 as u64, 42, 0])),
            256
        ),
        Ok(expected.len() as u64)
    );
    output(&memory, expected);
    let (mut memory, mut budget) = fixture(b"%4294967296u|%.4294967296u|%n:%u\0");
    assert_eq!(
        call(&mut memory, &mut budget, Some(context([42, 0, 77])), 256),
        Ok(8)
    );
    output(&memory, b"42||n:77");
}

#[test]
fn integer_binary_buffer_cap_pointer_case_and_padding_follow_source_profile() {
    let (mut memory, mut budget) = fixture(b"%llb|%p|%#p|%+u\0");
    stack(&mut memory, &[42]);
    let expected = b"00000000000000000000000000000001|00000000000000AB|0X000000000000AB|42";
    assert_eq!(
        call(
            &mut memory,
            &mut budget,
            Some(context([(1_u64 << 40) | 1, 0xab, 0xab])),
            256
        ),
        Ok(expected.len() as u64)
    );
    output(&memory, expected);
}

#[test]
fn gp_and_fp_exhaustion_share_one_overflow_cursor_in_consumption_order() {
    let (mut memory, mut budget) =
        fixture(b"%u/%u/%u/%u/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%u\0");
    let mut capture = context([10, 20, 30]);
    let mut vectors = [[0x9d; 16]; 8];
    for (index, vector) in vectors.iter_mut().enumerate() {
        vector[..8].copy_from_slice(&((index + 1) as f64).to_bits().to_le_bytes());
    }
    capture.vectors = Some(vectors);
    stack(&mut memory, &[40, 9_f64.to_bits(), 50]);
    let expected = b"10/20/30/40/1/2/3/4/5/6/7/8/9/50";
    assert_eq!(
        call(&mut memory, &mut budget, Some(capture), 256),
        Ok(expected.len() as u64)
    );
    output(&memory, expected);
}

#[test]
fn utf16_precision_checks_units_then_utf8_boundaries_without_splitting_characters() {
    let (mut memory, mut budget) = fixture(b"%.3ls|%.1ls|%ls\0");
    for (address, text) in [(TEXT, "éé"), (TEXT + 32, "😀"), (TEXT + 64, "hé😀")] {
        let encoded = text
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        memory.write(address, &encoded).unwrap();
    }
    let expected = "é||hé😀".as_bytes();
    assert_eq!(
        call(
            &mut memory,
            &mut budget,
            Some(context([TEXT, TEXT + 32, TEXT + 64])),
            256
        ),
        Ok(expected.len() as u64)
    );
    output(&memory, expected);
    let (mut memory, mut budget) = fixture(b"%.0s|%.0ls\0");
    assert_eq!(
        call(
            &mut memory,
            &mut budget,
            Some(context([u64::MAX, u64::MAX, 0])),
            256
        ),
        Ok(1)
    );
    output(&memory, b"|");
}

#[test]
fn embedded_nul_ends_final_copy_while_logical_return_counts_later_rendering() {
    let (mut memory, mut budget) = fixture(b"A%cB%u\0");
    assert_eq!(
        call(&mut memory, &mut budget, Some(context([0, 42, 0])), 2),
        Ok(5)
    );
    output(&memory, b"A");
}

#[test]
fn format_and_string_destination_overlap_are_read_completely_before_one_write() {
    let (mut memory, mut budget) = fixture(b"unused\0");
    memory.write(DEST, b"%u/%s\0").unwrap();
    memory.write(DEST + 64, b"test\0").unwrap();
    let attempt = budget.begin_attempt().unwrap();
    let result = invoke_with_variadic(
        attempt,
        KnownFunction::Snprintf,
        AbiArgs {
            rdi: DEST,
            rsi: 256,
            rdx: DEST,
        },
        &mut memory,
        &mut budget,
        Some(context([42, DEST + 64, 0])),
    );
    assert_eq!(result.unwrap().rax, 7);
    output(&memory, b"42/test");
}

#[test]
fn missing_captures_overflow_late_reads_and_undefined_formats_never_publish_output() {
    for (format, capture) in [
        (b"%u\0".as_slice(), None),
        (b"%f\0".as_slice(), Some(context([0; 3]))),
        (
            b"%u%u%u%u\0".as_slice(),
            Some(VariadicContext {
                stack_pointer: None,
                ..context([1; 3])
            }),
        ),
        (
            b"%u%u%u%u\0".as_slice(),
            Some(VariadicContext {
                stack_pointer: Some(u64::MAX),
                ..context([1; 3])
            }),
        ),
        (b"prefix:%s\0".as_slice(), Some(context([u64::MAX, 0, 0]))),
        (b"broken%\0".as_slice(), Some(context([0; 3]))),
        (b"%ls\0".as_slice(), Some(context([TEXT, 0, 0]))),
    ] {
        let (mut memory, mut budget) = fixture(format);
        memory.write(TEXT, &0xdc00_u16.to_le_bytes()).unwrap();
        assert!(call(&mut memory, &mut budget, capture, 256).is_err());
        assert_eq!(bytes(&memory, DEST, 256), [0xa7; 256]);
        assert_eq!((budget.completed(), budget.attempts()), (0, 1));
        assert!(budget.work() >= SITE_LOOKUP_WORK);
    }
}

#[test]
fn truncation_zero_capacity_and_float_source_undefined_precision_are_typed_atomic_refusals() {
    for size in [0, 4] {
        let (mut memory, mut budget) = fixture(b"1234\0");
        assert!(matches!(
            call(&mut memory, &mut budget, Some(context([0; 3])), size),
            Err(MemoryServiceError::UnsupportedFormat(_))
        ));
        assert_eq!(bytes(&memory, DEST, 256), [0xa7; 256]);
        assert_eq!(budget.completed(), 0);
    }
    let (mut memory, mut budget) = fixture(b"%.42f\0");
    let mut capture = context([0; 3]);
    capture.vectors = Some([[0; 16]; 8]);
    assert!(matches!(
        call(&mut memory, &mut budget, Some(capture), 256),
        Err(MemoryServiceError::UnsupportedFormat(_))
    ));
    assert_eq!(bytes(&memory, DEST, 256), [0xa7; 256]);
}

#[test]
fn budget_allocation_and_destination_failures_preserve_canaries_and_allow_owned_retry() {
    for limits in [
        MemoryServiceLimits {
            max_scratch_bytes: 0,
            ..Default::default()
        },
        MemoryServiceLimits {
            max_total_bytes: 1,
            ..Default::default()
        },
        MemoryServiceLimits {
            max_work_units: SITE_LOOKUP_WORK + 1,
            ..Default::default()
        },
        MemoryServiceLimits {
            max_span_bytes: 2,
            ..Default::default()
        },
    ] {
        let (mut memory, _) = fixture(b"value=%u\0");
        let mut budget = ServiceBudget::new(limits);
        assert!(call(&mut memory, &mut budget, Some(context([42, 0, 0])), 256).is_err());
        assert_eq!(bytes(&memory, DEST, 256), [0xa7; 256]);
        assert_eq!(budget.completed(), 0);
    }
    let (mut memory, mut budget) = fixture(b"%u\0");
    budget.refuse_reservation = Some(0);
    assert_eq!(
        call(&mut memory, &mut budget, Some(context([42, 0, 0])), 256),
        Err(MemoryServiceError::Allocation)
    );
    assert_eq!(bytes(&memory, DEST, 256), [0xa7; 256]);
    budget.refuse_reservation = None;
    memory.protect(DEST, PAGE_SIZE, Permissions::READ).unwrap();
    assert!(call(&mut memory, &mut budget, Some(context([42, 0, 0])), 256).is_err());
    assert_eq!(bytes(&memory, DEST, 256), [0xa7; 256]);
    memory
        .protect(DEST, PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    assert_eq!(
        call(&mut memory, &mut budget, Some(context([42, 0, 0])), 256),
        Ok(2)
    );
    output(&memory, b"42");
    assert_eq!((budget.completed(), budget.attempts()), (1, 3));
}
