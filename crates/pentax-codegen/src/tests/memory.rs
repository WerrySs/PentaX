// SPDX-License-Identifier: GPL-2.0-only

use super::*;

fn footprint(
    code: &[u8],
    registers: [u64; 16],
) -> Result<InstructionMemoryAccesses, MemoryAccessError> {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let block = make(&source);
    MemoryAccessAnalyzer::default().analyze(&block.instructions()[0], &registers)
}
fn access(address: u64, size: u64, kind: MemoryAccessKind) -> DeclaredMemoryAccess {
    DeclaredMemoryAccess {
        address,
        size,
        kind,
    }
}

#[test]
fn locked_rmw_and_implicit_exchange_keep_prefixes_and_exact_read_write_footprints() {
    for size in [1, 2, 4, 8] {
        let mut prefix = vec![0xf0];
        if size == 2 {
            prefix.push(0x66);
        }
        if size == 8 {
            prefix.push(0x48);
        }
        let wide = u8::from(size != 1);
        let mut forms = Vec::new();
        for opcode in [0x00, 0x10, 0x20, 0x08, 0x30, 0x28, 0x18] {
            forms.push(vec![opcode + wide, 0x08]);
        }
        for extension in [0, 1] {
            forms.push(vec![if size == 1 { 0xfe } else { 0xff }, extension << 3]);
        }
        for extension in [2, 3] {
            forms.push(vec![if size == 1 { 0xf6 } else { 0xf7 }, extension << 3]);
        }
        forms.push(vec![0x0f, 0xc0 + wide, 0x08]);
        forms.push(vec![0x0f, 0xb0 + wide, 0x08]);
        forms.push(vec![0x86 + wide, 0x08]);
        for form in forms {
            let mut code = prefix.clone();
            code.extend(form);
            code.push(0xc3);
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let plan = make(&source);
            let emitted = ordinary_output(&plan);
            assert_eq!(emitted.len(), 1, "{code:x?}");
            assert!(emitted[0].has_lock_prefix(), "{code:x?}");
            assert!(plan.instructions()[0].has_atomic_memory());
            let mut gpr = [0; 16];
            gpr[0] = 0x8000;
            assert_eq!(
                footprint(&code, gpr).unwrap().accesses(),
                [access(0x8000, size, MemoryAccessKind::ReadWrite)]
            );
            if size != 1 {
                gpr[0] += 1;
                assert_eq!(
                    footprint(&code, gpr).unwrap_err(),
                    MemoryAccessError::UnalignedAtomic
                );
            }
        }
        let mut implicit = prefix[1..].to_vec();
        implicit.extend([0x86 + wide, 0x08, 0xc3]);
        let mut gpr = [0; 16];
        gpr[0] = 0x8000;
        assert_eq!(
            footprint(&implicit, gpr).unwrap().accesses(),
            [access(0x8000, size, MemoryAccessKind::ReadWrite)]
        );
        if size != 1 {
            gpr[0] += 1;
            assert_eq!(
                footprint(&implicit, gpr).unwrap_err(),
                MemoryAccessError::UnalignedAtomic
            );
        }
    }
    let mut gpr = [0; 16];
    gpr[0] = 0x8000;
    assert_eq!(
        footprint(&[0xf0, 0x0f, 0xc7, 0x08, 0xc3], gpr)
            .unwrap()
            .accesses(),
        [access(0x8000, 8, MemoryAccessKind::ReadWrite)]
    );
}

#[test]
fn atomics_split_even_default_policy_and_retry_after_register_address_changes() {
    for atomic in [&[0xf0, 0x48, 0x01, 0x08][..], &[0x48, 0x87, 0x08][..]] {
        let mut code = vec![0x48, 0x83, 0xc0, 8];
        code.extend(atomic);
        code.extend([0x48, 0x83, 0xc1, 1, 0xc3]);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let first = make(&source);
        assert_eq!(ordinary_output(&first).len(), 1);
        let pc = BIAS + ENTRY as u64 + 4;
        assert_eq!(first.exit().kind(), ExitKind::Fallthrough { target: pc });
        let second = plan_block(
            &source,
            0,
            pc,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(ordinary_output(&second).len(), 1);
        assert!(second.instructions()[0].has_atomic_memory());
        assert_eq!(
            second.exit().kind(),
            ExitKind::Fallthrough {
                target: pc + atomic.len() as u64
            }
        );
        let mut gpr = [0; 16];
        gpr[0] = 0x8008;
        assert_eq!(
            MemoryAccessAnalyzer::default()
                .analyze(&second.instructions()[0], &gpr)
                .unwrap()
                .accesses(),
            [access(0x8008, 8, MemoryAccessKind::ReadWrite)]
        );
    }
}

#[test]
fn locked_hle_segment_bit_string_and_feature_refusals_do_not_emit_atomic_bytes() {
    for code in [
        vec![0xf2, 0xf0, 0x48, 0x01, 0x08, 0xc3],
        vec![0xf3, 0xf0, 0x48, 0x01, 0x08, 0xc3],
        vec![0x64, 0xf0, 0x48, 0x01, 0x08, 0xc3],
        vec![0xf0, 0x48, 0x0f, 0xab, 0x08, 0xc3],
        vec![0xf0, 0x48, 0x0f, 0xc7, 0x08, 0xc3],
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert!(ordinary_output(&plan).is_empty(), "{code:x?}");
        assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
    }
}

#[test]
fn isolated_memory_splits_keep_register_batches_and_retry_the_unconsumed_memory_pc() {
    // MOV EAX,1; ADD EAX,2; MOV RCX,[RAX]; ADD ECX,3; RET.
    let code = [
        0xb8, 1, 0, 0, 0, 0x83, 0xc0, 2, 0x48, 0x8b, 0x08, 0x83, 0xc1, 3, 0xc3,
    ];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let policy = TranslationPolicy::x64_sse2().with_isolated_memory();
    let start = BIAS + ENTRY as u64;
    let mut pc = start;
    for (emitted, next) in [(2, start + 8), (1, start + 11), (1, start + 15)] {
        let plan = plan_block(
            &source,
            0,
            pc,
            placement(),
            &policy,
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(
            plan.instructions()
                .iter()
                .filter(|r| r.emitted_len() != 0)
                .count(),
            emitted
        );
        assert_eq!(plan.exit().next_guest_pc(), next);
        if next == start + 15 {
            assert!(matches!(plan.exit().kind(), ExitKind::Return { adjust: 0 }));
        } else {
            assert_eq!(plan.exit().kind(), ExitKind::Fallthrough { target: next });
        }
        pc = next;
    }
    assert!(!TranslationPolicy::x64_sse2().isolates_memory());
    assert_eq!(ordinary_output(&make(&source)).len(), 4);
}

#[test]
fn stack_footprints_use_original_push_rsp_post_increment_pop_and_leave_rbp() {
    let mut registers = [0; 16];
    registers[4] = 0x8000;
    registers[5] = 0xa000;
    assert_eq!(
        footprint(&[0xff, 0x34, 0x24, 0xc3], registers)
            .unwrap()
            .accesses(),
        [
            access(0x8000, 8, MemoryAccessKind::Read),
            access(0x7ff8, 8, MemoryAccessKind::Write)
        ]
    );
    assert_eq!(
        footprint(&[0x8f, 0x04, 0x24, 0xc3], registers)
            .unwrap()
            .accesses(),
        [
            access(0x8008, 8, MemoryAccessKind::Write),
            access(0x8000, 8, MemoryAccessKind::Read)
        ]
    );
    assert_eq!(
        footprint(&[0xc9, 0xc3], registers).unwrap().accesses(),
        [access(0xa000, 8, MemoryAccessKind::Read)]
    );
    assert_eq!(
        footprint(&[0x66, 0x50, 0xc3], registers)
            .unwrap()
            .accesses(),
        [access(0x7ffe, 2, MemoryAccessKind::Write)]
    );
}

#[test]
fn independent_memory_batches_preserve_address_dependencies_and_atomic_boundaries() {
    for (code, emitted) in [
        // MOV EAX,[RDI]; ADD EAX,1; MOV [RDI+4],EAX; MOV ECX,[RSI].
        (
            &[0x8b, 0x07, 0x83, 0xc0, 1, 0x89, 0x47, 4, 0x8b, 0x0e, 0xc3][..],
            4,
        ),
        // Partial, full-width, zero-extending and conditional address writes.
        (&[0xb0, 1, 0x8b, 0x08, 0xc3][..], 1),
        (&[0xb4, 1, 0x8b, 0x08, 0xc3][..], 1),
        (&[0xb8, 1, 0, 0, 0, 0x8b, 0x08, 0xc3][..], 1),
        (&[0x48, 0x89, 0xf8, 0x8b, 0x08, 0xc3][..], 1),
        (&[0x0f, 0x44, 0xc7, 0x8b, 0x08, 0xc3][..], 1),
        // A loaded pointer cannot authorize the following dereference.
        (&[0x48, 0x8b, 0x07, 0x8b, 0x08, 0xc3][..], 1),
        // Implicit RDX and RSP writes, plus an updated index register.
        (&[0xf7, 0xe7, 0x8b, 0x0a, 0xc3][..], 1),
        (&[0x50, 0x8b, 0x04, 0x24, 0xc3][..], 1),
        (&[0x83, 0xc1, 1, 0x8b, 0x04, 0x8f, 0xc3][..], 1),
        // Memory aliasing is allowed: sequential execution preserves ordering.
        (&[0x89, 0x07, 0x8b, 0x0f, 0xc3][..], 2),
        // An atomic must be alone even when its address is independent.
        (&[0x90, 0xf0, 0x83, 0x07, 1, 0xc3][..], 1),
        (&[0xf0, 0x83, 0x07, 1, 0x90, 0xc3][..], 1),
    ] {
        let bytes = image(code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2().with_independent_memory(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(ordinary_output(&plan).len(), emitted, "{code:x?}");
        if plan.exit().kind() == (ExitKind::Return { adjust: 0 }) {
            assert_eq!(
                plan.exit().next_guest_pc(),
                BIAS + ENTRY as u64 + code.len() as u64
            );
        } else {
            let length: usize = plan
                .instructions()
                .iter()
                .filter(|r| r.emitted_len() != 0)
                .map(|r| r.source_len())
                .sum();
            assert_eq!(plan.exit().guest_pc(), BIAS + ENTRY as u64 + length as u64);
        }
    }
}

#[test]
fn access_width_direction_sib_and_conditional_reads_are_conservative() {
    let mut registers = [0; 16];
    registers[0] = 0x8000;
    registers[1] = 2;
    registers[12] = 0x9000;
    for (code, expected) in [
        (
            vec![0x49, 0x8b, 0x44, 0xcc, 0xf8, 0xc3],
            access(0x9008, 8, MemoryAccessKind::Read),
        ),
        (
            vec![0x8b, 0x00, 0xc3],
            access(0x8000, 4, MemoryAccessKind::Read),
        ),
        (
            vec![0xf3, 0x0f, 0x6f, 0x00, 0xc3],
            access(0x8000, 16, MemoryAccessKind::Read),
        ),
        (
            vec![0x01, 0x08, 0xc3],
            access(0x8000, 4, MemoryAccessKind::ReadWrite),
        ),
        (
            vec![0x48, 0x0f, 0x45, 0x00, 0xc3],
            access(0x8000, 8, MemoryAccessKind::Read),
        ),
        (
            vec![0x0f, 0x95, 0x00, 0xc3],
            access(0x8000, 1, MemoryAccessKind::Write),
        ),
    ] {
        assert_eq!(footprint(&code, registers).unwrap().accesses(), [expected]);
    }
}

#[test]
fn rip_relative_footprint_is_original_guest_address_and_lea_nop_do_not_dereference() {
    let mut code = vec![0x48, 0x8b, 0x05];
    code.extend_from_slice(&((DATA as i64 - ENTRY as i64 - 7) as i32).to_le_bytes());
    code.push(0xc3);
    assert_eq!(
        footprint(&code, [u64::MAX; 16]).unwrap().accesses(),
        [access(BIAS + DATA, 8, MemoryAccessKind::Read)]
    );
    for code in [&[0x48, 0x8d, 0x00, 0xc3][..], &[0x0f, 0x1f, 0x00, 0xc3][..]] {
        assert!(
            footprint(code, [u64::MAX; 16])
                .unwrap()
                .accesses()
                .is_empty()
        );
    }
}

#[test]
fn unmodeled_bit_strings_control_records_and_invalid_full_spans_are_refused() {
    assert_eq!(
        footprint(&[0x48, 0x0f, 0xa3, 0x08, 0xc3], [0; 16]),
        Err(MemoryAccessError::UnsupportedBitString)
    );
    assert_eq!(
        footprint(&[0xc3], [0; 16]),
        Err(MemoryAccessError::NotEmitted)
    );
    for address in [u64::MAX, (1_u64 << 47) - 4] {
        let mut registers = [0; 16];
        registers[0] = address;
        assert_eq!(
            footprint(&[0x48, 0x8b, 0x00, 0xc3], registers),
            Err(MemoryAccessError::InvalidRange)
        );
    }
    let mut registers = [0; 16];
    registers[12] = u64::MAX - 7;
    registers[1] = 2;
    assert_eq!(
        footprint(&[0x49, 0x8b, 0x44, 0xcc, 0xf8, 0xc3], registers)
            .unwrap()
            .accesses(),
        [access(0, 8, MemoryAccessKind::Read)]
    );
}
