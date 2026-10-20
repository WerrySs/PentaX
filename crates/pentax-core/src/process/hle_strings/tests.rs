// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX fixtures for the Rust adaptation, 2026-10-17.
// Adaptation source/copyright notices are retained in ../hle_strings.rs.

use super::*;
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE, Permissions};

const SOURCE: u64 = 0x4100;
const DESTINATION: u64 = 0x8100;

fn limits() -> StringLimits {
    StringLimits {
        max_scan_bytes: 4096,
        max_bytes: 65536,
        max_work_units: 2 * 1024 * 1024,
        max_scratch_bytes: 65536,
    }
}

fn memory() -> GuestMemory {
    let mut memory = GuestMemory::new(4 * PAGE_SIZE, 4 * PAGE_SIZE);
    memory
        .map(0x4000, 3 * PAGE_SIZE, Permissions::READ_WRITE)
        .unwrap();
    memory
}

fn args(first: u64, second: u64, third: u64) -> StringArguments {
    StringArguments {
        first,
        second,
        third,
    }
}

fn bytes(memory: &GuestMemory, address: u64, count: usize) -> Vec<u8> {
    let mut result = vec![0; count];
    memory.read(address, &mut result).unwrap();
    result
}

fn call(
    operation: StringOperation,
    arguments: StringArguments,
    memory: &mut GuestMemory,
) -> (Result<u64, StringError>, StringUsage) {
    invoke(operation, arguments, memory, &limits()).into_parts()
}

#[test]
fn comparisons_use_unsigned_bytes_and_normalized_signed_eax() {
    for operation in [StringOperation::Strcmp, StringOperation::Strncmp] {
        for (left, right, expected) in [
            (&b"same\0"[..], &b"same\0"[..], 0),
            (&b"prefix\0"[..], &b"prefix!\0"[..], u64::from(u32::MAX)),
            (&b"z\0"[..], &b"a\0"[..], 1),
            (&[0x80, 0][..], &[0x7f, 0][..], 1),
            (&[1, 0][..], &[255, 0][..], u64::from(u32::MAX)),
        ] {
            let mut memory = memory();
            memory.write(SOURCE, left).unwrap();
            memory.write(DESTINATION, right).unwrap();
            let (result, usage) = call(operation, args(SOURCE, DESTINATION, 100), &mut memory);
            assert_eq!(result, Ok(expected));
            assert_eq!(usage.write_bytes(), 0);
            assert_eq!(usage.scratch_peak_bytes(), 0);
            assert_eq!(bytes(&memory, SOURCE, left.len()), left);
        }
    }
}

#[test]
fn comparison_short_circuits_before_unused_bytes_or_count() {
    let mut memory = memory();
    let edge = 0x10000 - 1;
    memory.write(edge, b"x").unwrap();
    let (result, usage) = call(StringOperation::Strncmp, args(edge, edge, 1), &mut memory);
    assert_eq!(result, Ok(0));
    assert_eq!(usage.read_bytes(), 2);
    memory.write(edge, b"a").unwrap();
    memory.write(SOURCE, b"b").unwrap();
    assert_eq!(
        call(StringOperation::Strcmp, args(edge, SOURCE, 0), &mut memory).0,
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(
        call(
            StringOperation::Strncmp,
            args(edge, SOURCE, u64::MAX),
            &mut memory
        )
        .0,
        Ok(u64::from(u32::MAX))
    );
    memory.write(edge - 1, b"a\0").unwrap();
    memory.write(SOURCE, b"a\0").unwrap();
    assert_eq!(
        call(
            StringOperation::Strcmp,
            args(edge - 1, SOURCE, 0),
            &mut memory
        )
        .0,
        Ok(0)
    );
}

#[test]
fn zero_count_validates_pointer_integers_without_mapping_or_null_rejection() {
    for operation in [StringOperation::Strncmp, StringOperation::Strncpy] {
        for (first, second) in [(0, 0), (0x18000, 0x1c000), (ADDRESS_LIMIT - 1, 0)] {
            let mut memory = memory();
            let (result, usage) = call(operation, args(first, second, 0), &mut memory);
            assert_eq!(
                result,
                Ok(if operation == StringOperation::Strncpy {
                    first
                } else {
                    0
                })
            );
            assert_eq!(usage.bytes(), 0);
            assert_eq!(usage.work_units(), 1);
            assert_eq!(memory.resident_bytes(), 0);
        }
        for (first, second, argument) in [
            (ADDRESS_LIMIT, 0, StringArgument::First),
            (0, u64::MAX, StringArgument::Second),
        ] {
            let mut memory = memory();
            assert_eq!(
                call(operation, args(first, second, 0), &mut memory).0,
                Err(StringError::AddressOverflow { argument })
            );
        }
    }
}

#[test]
fn strlen_accepts_a_nul_in_the_final_guest_byte_and_exact_scan_bound() {
    let mut memory = memory();
    memory
        .map(
            ADDRESS_LIMIT - PAGE_SIZE,
            PAGE_SIZE,
            Permissions::READ_WRITE,
        )
        .unwrap();
    memory.write(ADDRESS_LIMIT - 4, b"abc\0").unwrap();
    let mut bound = limits();
    bound.max_scan_bytes = 4;
    let (result, usage) = invoke(
        StringOperation::Strlen,
        args(ADDRESS_LIMIT - 4, 0, 0),
        &mut memory,
        &bound,
    )
    .into_parts();
    assert_eq!(result, Ok(3));
    assert_eq!(usage.read_bytes(), 4);
    assert_eq!(usage.bytes(), 4);
    assert_eq!(usage.work_units(), 9);
    bound.max_scan_bytes = 3;
    let (result, usage) = invoke(
        StringOperation::Strlen,
        args(ADDRESS_LIMIT - 4, 0, 0),
        &mut memory,
        &bound,
    )
    .into_parts();
    assert_eq!(
        result,
        Err(StringError::Unterminated {
            argument: StringArgument::First,
            limit: 3
        })
    );
    assert_eq!(usage.read_bytes(), 3);
}

#[test]
fn character_search_handles_repeats_nul_low_int_byte_and_missing_values() {
    let mut memory = memory();
    memory
        .write(SOURCE, &[b'a', 0xff, b'a', b'b', b'a', 0])
        .unwrap();
    for (operation, character, expected) in [
        (StringOperation::Strchr, u64::from(b'a'), SOURCE),
        (StringOperation::Strrchr, u64::from(b'a'), SOURCE + 4),
        (StringOperation::Strchr, 0xffff_ffff_0000_00ff, SOURCE + 1),
        (StringOperation::Strrchr, u64::from(b'z'), 0),
        (StringOperation::Strchr, 0x100, SOURCE + 5),
        (StringOperation::Strrchr, 0, SOURCE + 5),
    ] {
        assert_eq!(
            call(operation, args(SOURCE, character, 0), &mut memory).0,
            Ok(expected)
        );
    }
    let edge = 0x10000 - 1;
    memory.write(edge, b"q").unwrap();
    assert_eq!(
        call(
            StringOperation::Strchr,
            args(edge, u64::from(b'q'), 0),
            &mut memory
        )
        .0,
        Ok(edge)
    );
    assert!(matches!(
        call(
            StringOperation::Strrchr,
            args(edge, u64::from(b'q'), 0),
            &mut memory
        )
        .0,
        Err(StringError::Memory(MemoryError::Unmapped(0x10000)))
    ));
}

#[test]
fn strstr_returns_first_match_and_handles_empty_needle_without_haystack_read() {
    for (haystack, needle, expected) in [
        (&b"bananana\0"[..], &b"nana\0"[..], SOURCE + 2),
        (&b"aaaaab\0"[..], &b"aaab\0"[..], SOURCE + 2),
        (&b"short\0"[..], &b"too long\0"[..], 0),
        (&b"abc\0"[..], &b"z\0"[..], 0),
        (&b"\0"[..], &b"\0"[..], SOURCE),
        (&b"\0"[..], &b"x\0"[..], 0),
    ] {
        let mut memory = memory();
        memory.write(SOURCE, haystack).unwrap();
        memory.write(DESTINATION, needle).unwrap();
        assert_eq!(
            call(
                StringOperation::Strstr,
                args(SOURCE, DESTINATION, 0),
                &mut memory
            )
            .0,
            Ok(expected)
        );
    }
    let mut memory = memory();
    memory.write(DESTINATION, b"\0").unwrap();
    let (result, usage) = call(
        StringOperation::Strstr,
        args(0x18000, DESTINATION, 0),
        &mut memory,
    );
    assert_eq!(result, Ok(0x18000));
    assert_eq!(usage.read_bytes(), 1);
    assert_eq!(usage.scratch_peak_bytes(), 0);
}

#[test]
fn strcpy_snapshots_all_bytes_and_changes_only_the_terminated_output() {
    let mut memory = memory();
    memory.write(SOURCE, b"ported string\0").unwrap();
    memory.write(DESTINATION, &[0xcc; 32]).unwrap();
    let resident = memory.resident_bytes();
    let (result, usage) = call(
        StringOperation::Strcpy,
        args(DESTINATION, SOURCE, 0),
        &mut memory,
    );
    assert_eq!(result, Ok(DESTINATION));
    assert_eq!(&bytes(&memory, DESTINATION, 32)[..14], b"ported string\0");
    assert_eq!(&bytes(&memory, DESTINATION, 32)[14..], &[0xcc; 18]);
    assert_eq!(usage.read_bytes(), 28);
    assert_eq!(usage.write_bytes(), 14);
    assert!(usage.scratch_peak_bytes() >= 14);
    assert_eq!(memory.resident_bytes(), resident);
    assert_eq!(bytes(&memory, SOURCE, 14), b"ported string\0");
}

#[test]
fn strncpy_truncates_without_adding_nul_and_pads_after_the_first_nul() {
    for (source, count, expected) in [
        (&b"abcdef\0"[..], 3, &b"abc"[..]),
        (&b"ab\0unread"[..], 8, &b"ab\0\0\0\0\0\0"[..]),
        (&b"\0"[..], 4, &[0; 4][..]),
        (&b"abc\0"[..], 4, &b"abc\0"[..]),
    ] {
        let mut memory = memory();
        memory.write(SOURCE, source).unwrap();
        memory.write(DESTINATION, &[0xdd; 16]).unwrap();
        let (result, usage) = call(
            StringOperation::Strncpy,
            args(DESTINATION, SOURCE, count),
            &mut memory,
        );
        assert_eq!(result, Ok(DESTINATION));
        assert_eq!(bytes(&memory, DESTINATION, count as usize), expected);
        assert_eq!(bytes(&memory, DESTINATION + count, 1), [0xdd]);
        assert_eq!(usage.write_bytes(), count);
    }
}

#[test]
fn strncpy_never_reads_beyond_its_count_or_the_first_nul() {
    let mut memory = memory();
    let edge = 0x10000 - 1;
    memory.write(edge, b"z").unwrap();
    assert_eq!(
        call(
            StringOperation::Strncpy,
            args(DESTINATION, edge, 1),
            &mut memory
        )
        .0,
        Ok(DESTINATION)
    );
    assert_eq!(bytes(&memory, DESTINATION, 1), b"z");
    memory.write(edge, b"\0").unwrap();
    let (result, usage) = call(
        StringOperation::Strncpy,
        args(DESTINATION, edge, 8),
        &mut memory,
    );
    assert_eq!(result, Ok(DESTINATION));
    assert_eq!(bytes(&memory, DESTINATION, 8), [0; 8]);
    assert_eq!(usage.read_bytes(), 2); // Scan + snapshot, never the seven padding bytes.
}

#[test]
fn strcat_retains_the_prefix_and_returns_the_original_destination() {
    let mut memory = memory();
    memory.write(SOURCE, b"suffix\0").unwrap();
    memory.write(DESTINATION, b"pre:\0XXXXXXXXXXXX").unwrap();
    let (result, usage) = call(
        StringOperation::Strcat,
        args(DESTINATION, SOURCE, 0),
        &mut memory,
    );
    assert_eq!(result, Ok(DESTINATION));
    assert_eq!(bytes(&memory, DESTINATION, 12), b"pre:suffix\0X");
    assert_eq!(usage.write_bytes(), 7);
    assert_eq!(usage.read_bytes(), 19); // Five prefix bytes, seven scan + seven snapshot.
    memory.write(SOURCE, b"\0").unwrap();
    assert_eq!(
        call(
            StringOperation::Strcat,
            args(DESTINATION, SOURCE, 0),
            &mut memory
        )
        .0,
        Ok(DESTINATION)
    );
    assert_eq!(bytes(&memory, DESTINATION, 11), b"pre:suffix\0");
}

#[test]
fn every_copy_refuses_overlapping_strings_and_leaves_the_entire_input_unchanged() {
    for operation in [
        StringOperation::Strcpy,
        StringOperation::Strncpy,
        StringOperation::Strcat,
    ] {
        for offset in [0_i64, -1, 1] {
            let mut memory = memory();
            memory
                .write(DESTINATION - 4, b"aaaabcdefgh\0yyyyyyyyyyyy")
                .unwrap();
            let source = DESTINATION.checked_add_signed(offset).unwrap();
            let before = bytes(&memory, DESTINATION - 4, 24);
            let resident = memory.resident_bytes();
            assert_eq!(
                call(operation, args(DESTINATION, source, 6), &mut memory).0,
                Err(StringError::UnsupportedOverlap)
            );
            assert_eq!(bytes(&memory, DESTINATION - 4, 24), before);
            assert_eq!(memory.resident_bytes(), resident);
        }
    }
    // The append itself is disjoint, but source aliases the existing prefix.
    let mut memory = memory();
    memory.write(DESTINATION, b"abc\0xxxxxxxx").unwrap();
    assert_eq!(
        call(
            StringOperation::Strcat,
            args(DESTINATION, DESTINATION + 1, 0),
            &mut memory
        )
        .0,
        Err(StringError::UnsupportedOverlap)
    );
    assert_eq!(bytes(&memory, DESTINATION, 12), b"abc\0xxxxxxxx");
}

#[test]
fn unterminated_source_or_destination_does_not_start_a_partial_copy() {
    for operation in [StringOperation::Strcpy, StringOperation::Strcat] {
        let mut memory = memory();
        memory.write(SOURCE, b"abcd").unwrap();
        memory.write(DESTINATION, b"old\0sentinel").unwrap();
        let before = bytes(&memory, DESTINATION, 12);
        let mut bound = limits();
        bound.max_scan_bytes = 4;
        assert_eq!(
            invoke(operation, args(DESTINATION, SOURCE, 0), &mut memory, &bound)
                .into_parts()
                .0,
            Err(StringError::Unterminated {
                argument: StringArgument::Second,
                limit: 4
            })
        );
        assert_eq!(bytes(&memory, DESTINATION, 12), before);
    }
    let mut memory = memory();
    memory.write(SOURCE, b"ok\0").unwrap();
    memory.write(DESTINATION, b"no nul here").unwrap();
    let mut bound = limits();
    bound.max_scan_bytes = 4;
    assert_eq!(
        invoke(
            StringOperation::Strcat,
            args(DESTINATION, SOURCE, 0),
            &mut memory,
            &bound
        )
        .into_parts()
        .0,
        Err(StringError::Unterminated {
            argument: StringArgument::First,
            limit: 4
        })
    );
    assert_eq!(bytes(&memory, DESTINATION, 11), b"no nul here");
}

#[test]
fn late_source_faults_preserve_destination_and_keep_usage_on_error() {
    for operation in [
        StringOperation::Strcpy,
        StringOperation::Strncpy,
        StringOperation::Strcat,
    ] {
        let mut memory = memory();
        memory.write(0xffff, b"x").unwrap();
        memory.write(DESTINATION, b"old\0canary").unwrap();
        let before = bytes(&memory, DESTINATION, 10);
        let resident = memory.resident_bytes();
        let (result, usage) = call(operation, args(DESTINATION, 0xffff, 2), &mut memory);
        assert_eq!(
            result,
            Err(StringError::Memory(MemoryError::Unmapped(0x10000)))
        );
        assert!(usage.read_bytes() >= 2);
        assert!(usage.work_units() > 1);
        assert_eq!(bytes(&memory, DESTINATION, 10), before);
        assert_eq!(memory.resident_bytes(), resident);
    }
}

#[test]
fn cross_page_write_permissions_and_residency_fail_before_any_guest_change() {
    for budget in [false, true] {
        let mut memory = GuestMemory::new(
            if budget { 2 * PAGE_SIZE } else { 3 * PAGE_SIZE },
            3 * PAGE_SIZE,
        );
        memory
            .map(0x4000, 3 * PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        memory.write(SOURCE, b"abc\0").unwrap();
        memory.write(0xbffe, &[0x55; 2]).unwrap();
        if !budget {
            memory
                .protect(0xc000, PAGE_SIZE, Permissions::READ)
                .unwrap();
        }
        let before = bytes(&memory, 0xbffe, 4);
        let resident = memory.resident_bytes();
        let mut reservations = 0;
        let (result, _) = engine::invoke_with_gate(
            StringOperation::Strcpy,
            args(0xbffe, SOURCE, 0),
            &mut memory,
            &limits(),
            &mut || {
                reservations += 1;
                Ok(())
            },
        )
        .into_parts();
        assert_eq!(
            result,
            Err(StringError::Memory(if budget {
                MemoryError::BudgetExceeded
            } else {
                MemoryError::PermissionDenied(0xc000)
            }))
        );
        assert_eq!(reservations, 0);
        assert_eq!(bytes(&memory, 0xbffe, 4), before);
        assert_eq!(memory.resident_bytes(), resident);
    }
}

#[test]
fn source_requires_read_and_destination_requires_write() {
    let mut memory = memory();
    memory.write(SOURCE, b"abc\0").unwrap();
    memory.write(DESTINATION, b"old\0").unwrap();
    memory
        .protect(0x4000, PAGE_SIZE, Permissions::WRITE)
        .unwrap();
    assert_eq!(
        call(
            StringOperation::Strcpy,
            args(DESTINATION, SOURCE, 0),
            &mut memory
        )
        .0,
        Err(StringError::Memory(MemoryError::PermissionDenied(SOURCE)))
    );
    assert_eq!(bytes(&memory, DESTINATION, 4), b"old\0");
    memory
        .protect(0x4000, PAGE_SIZE, Permissions::READ)
        .unwrap();
    memory
        .protect(0x8000, PAGE_SIZE, Permissions::READ)
        .unwrap();
    assert_eq!(
        call(
            StringOperation::Strcpy,
            args(DESTINATION, SOURCE, 0),
            &mut memory
        )
        .0,
        Err(StringError::Memory(MemoryError::PermissionDenied(
            DESTINATION
        )))
    );
    assert_eq!(bytes(&memory, DESTINATION, 4), b"old\0");
}

#[test]
fn null_noncanonical_and_huge_spans_have_typed_errors_before_allocation() {
    for operation in [
        StringOperation::Strcmp,
        StringOperation::Strlen,
        StringOperation::Strcpy,
        StringOperation::Strcat,
        StringOperation::Strchr,
        StringOperation::Strrchr,
        StringOperation::Strstr,
    ] {
        let mut memory = memory();
        assert_eq!(
            call(operation, args(0, SOURCE, 0), &mut memory).0,
            Err(StringError::NullPointer {
                argument: StringArgument::First
            })
        );
        assert_eq!(
            call(operation, args(ADDRESS_LIMIT, SOURCE, 0), &mut memory).0,
            Err(StringError::AddressOverflow {
                argument: StringArgument::First
            })
        );
    }
    let mut memory = memory();
    assert_eq!(
        call(
            StringOperation::Strncpy,
            args(DESTINATION, SOURCE, u64::MAX),
            &mut memory
        )
        .0,
        Err(StringError::AddressOverflow {
            argument: StringArgument::First
        })
    );
    assert_eq!(memory.resident_bytes(), 0);
}

#[test]
fn write_span_past_guest_end_is_rejected_without_changing_old_bytes() {
    let mut memory = memory();
    memory
        .map(
            ADDRESS_LIMIT - PAGE_SIZE,
            PAGE_SIZE,
            Permissions::READ_WRITE,
        )
        .unwrap();
    memory.write(SOURCE, b"abc\0").unwrap();
    memory.write(ADDRESS_LIMIT - 2, &[0x55; 2]).unwrap();
    let resident = memory.resident_bytes();
    assert_eq!(
        call(
            StringOperation::Strcpy,
            args(ADDRESS_LIMIT - 2, SOURCE, 0),
            &mut memory
        )
        .0,
        Err(StringError::AddressOverflow {
            argument: StringArgument::First
        })
    );
    assert_eq!(bytes(&memory, ADDRESS_LIMIT - 2, 2), [0x55; 2]);
    assert_eq!(memory.resident_bytes(), resident);
}

#[test]
fn scratch_reservation_failure_leaves_copy_and_search_inputs_unchanged() {
    for (operation, refusals) in [
        (StringOperation::Strcpy, 1),
        (StringOperation::Strncpy, 1),
        (StringOperation::Strcat, 1),
        (StringOperation::Strstr, 2),
    ] {
        for refused in 0..refusals {
            let mut memory = memory();
            memory.write(SOURCE, b"aab\0").unwrap();
            memory.write(DESTINATION, b"aaaab\0canary").unwrap();
            let source = bytes(&memory, SOURCE, 4);
            let destination = bytes(&memory, DESTINATION, 12);
            let resident = memory.resident_bytes();
            let mut index = 0;
            let arguments = if operation == StringOperation::Strstr {
                args(DESTINATION, SOURCE, 0)
            } else {
                args(DESTINATION, SOURCE, 8)
            };
            let (result, usage) =
                engine::invoke_with_gate(operation, arguments, &mut memory, &limits(), &mut || {
                    let current = index;
                    index += 1;
                    if current == refused {
                        Err(StringError::Allocation)
                    } else {
                        Ok(())
                    }
                })
                .into_parts();
            assert_eq!(result, Err(StringError::Allocation));
            assert_eq!(index, refused + 1);
            assert!(usage.work_units() > 1);
            assert_eq!(bytes(&memory, SOURCE, 4), source);
            assert_eq!(bytes(&memory, DESTINATION, 12), destination);
            assert_eq!(memory.resident_bytes(), resident);
        }
    }
}

#[test]
fn all_nine_operations_obey_inclusive_byte_work_and_actual_scratch_allowances() {
    for operation in [
        StringOperation::Strcmp,
        StringOperation::Strncmp,
        StringOperation::Strlen,
        StringOperation::Strcpy,
        StringOperation::Strncpy,
        StringOperation::Strcat,
        StringOperation::Strchr,
        StringOperation::Strrchr,
        StringOperation::Strstr,
    ] {
        let setup = || {
            let mut memory = memory();
            memory.write(SOURCE, b"aba\0").unwrap();
            memory.write(DESTINATION, b"ababa\0canary").unwrap();
            memory
        };
        let arguments = match operation {
            StringOperation::Strchr | StringOperation::Strrchr => args(SOURCE, u64::from(b'a'), 0),
            StringOperation::Strcpy
            | StringOperation::Strncpy
            | StringOperation::Strcat
            | StringOperation::Strstr => args(DESTINATION, SOURCE, 8),
            _ => args(SOURCE, DESTINATION, 8),
        };
        let (expected, usage) = call(operation, arguments, &mut setup());
        assert!(expected.is_ok(), "{operation:?} {expected:?}");
        let tight = StringLimits {
            max_scan_bytes: 4096,
            max_bytes: usage.bytes(),
            max_work_units: usage.work_units(),
            max_scratch_bytes: usage.scratch_peak_bytes(),
        };
        let (result, exact) = invoke(operation, arguments, &mut setup(), &tight).into_parts();
        assert_eq!(result, expected, "{operation:?}");
        assert_eq!(exact, usage);
        for kind in [
            StringLimit::Bytes,
            StringLimit::WorkUnits,
            StringLimit::ScratchBytes,
        ] {
            let mut bound = tight;
            let field = match kind {
                StringLimit::Bytes => &mut bound.max_bytes,
                StringLimit::WorkUnits => &mut bound.max_work_units,
                StringLimit::ScratchBytes => &mut bound.max_scratch_bytes,
            };
            if *field == 0 {
                continue;
            }
            *field -= 1;
            let mut memory = setup();
            let before = bytes(&memory, DESTINATION, 12);
            let (result, consumed) = invoke(operation, arguments, &mut memory, &bound).into_parts();
            assert!(
                matches!(result, Err(StringError::LimitExceeded { kind: actual, .. }) if actual == kind),
                "{operation:?} {kind:?}: {result:?}"
            );
            assert!(consumed.bytes() <= bound.max_bytes);
            assert!(consumed.work_units() <= bound.max_work_units);
            assert!(consumed.scratch_peak_bytes() <= bound.max_scratch_bytes);
            assert_eq!(bytes(&memory, DESTINATION, 12), before);
        }
    }
}

#[test]
fn repetitive_strstr_comparisons_are_charged_before_exhaustion() {
    let mut memory = memory();
    let haystack = [vec![b'a'; 128], vec![0]].concat();
    let needle = [vec![b'a'; 31], vec![b'b', 0]].concat();
    memory.write(SOURCE, &haystack).unwrap();
    memory.write(DESTINATION, &needle).unwrap();
    let (result, full) = call(
        StringOperation::Strstr,
        args(SOURCE, DESTINATION, 0),
        &mut memory,
    );
    assert_eq!(result, Ok(0));
    assert!(full.work_units() > 128 * 10);
    let mut bound = limits();
    bound.max_work_units = full.work_units() - 1;
    let (result, partial) = invoke(
        StringOperation::Strstr,
        args(SOURCE, DESTINATION, 0),
        &mut memory,
        &bound,
    )
    .into_parts();
    assert!(matches!(
        result,
        Err(StringError::LimitExceeded {
            kind: StringLimit::WorkUnits,
            ..
        })
    ));
    assert_eq!(partial.work_units(), bound.max_work_units);
    assert_eq!(bytes(&memory, SOURCE, haystack.len()), haystack);
    assert_eq!(bytes(&memory, DESTINATION, needle.len()), needle);
}

#[test]
fn zero_limits_cannot_bypass_accounting_or_start_allocation() {
    let mut memory = memory();
    memory.write(SOURCE, b"x\0").unwrap();
    let mut bound = limits();
    bound.max_work_units = 0;
    let (result, usage) =
        invoke(StringOperation::Strncpy, args(0, 0, 0), &mut memory, &bound).into_parts();
    assert!(matches!(
        result,
        Err(StringError::LimitExceeded {
            kind: StringLimit::WorkUnits,
            ..
        })
    ));
    assert_eq!(usage, StringUsage::default());
    bound = limits();
    bound.max_bytes = 0;
    let (result, usage) = invoke(
        StringOperation::Strlen,
        args(SOURCE, 0, 0),
        &mut memory,
        &bound,
    )
    .into_parts();
    assert!(matches!(
        result,
        Err(StringError::LimitExceeded {
            kind: StringLimit::Bytes,
            ..
        })
    ));
    assert_eq!(usage.bytes(), 0);
    assert_eq!(usage.work_units(), 1);
    bound = limits();
    bound.max_scan_bytes = 0;
    assert!(matches!(
        invoke(
            StringOperation::Strlen,
            args(SOURCE, 0, 0),
            &mut memory,
            &bound
        )
        .into_parts()
        .0,
        Err(StringError::Unterminated { limit: 0, .. })
    ));
}

#[test]
fn retained_remaining_allowances_bound_multiple_invocations_without_resetting_usage() {
    let mut memory = memory();
    memory.write(SOURCE, b"abc\0").unwrap();
    let mut remaining = limits();
    remaining.max_bytes = 8;
    remaining.max_work_units = 18;
    for _ in 0..2 {
        let (result, usage) = invoke(
            StringOperation::Strlen,
            args(SOURCE, 0, 0),
            &mut memory,
            &remaining,
        )
        .into_parts();
        assert_eq!(result, Ok(3));
        remaining.max_bytes -= usage.bytes();
        remaining.max_work_units -= usage.work_units();
    }
    assert_eq!(remaining.max_bytes, 0);
    assert_eq!(remaining.max_work_units, 0);
    let (result, usage) = invoke(
        StringOperation::Strlen,
        args(SOURCE, 0, 0),
        &mut memory,
        &remaining,
    )
    .into_parts();
    assert!(matches!(
        result,
        Err(StringError::LimitExceeded {
            kind: StringLimit::WorkUnits,
            ..
        })
    ));
    assert_eq!(usage, StringUsage::default());
}
