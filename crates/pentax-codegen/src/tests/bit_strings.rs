// SPDX-License-Identifier: GPL-2.0-only
//! Original selected-element addresses and independent-block dependency proofs.
use super::*;

fn analyze(code: &[u8], gpr: [u64; 16]) -> Result<InstructionMemoryAccesses, MemoryAccessError> {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let block = make(&source);
    MemoryAccessAnalyzer::default().analyze(&block.instructions()[0], &gpr)
}
fn encoded(size: u64, opcode: u8, extension: u8, immediate: Option<u8>) -> Vec<u8> {
    let mut code = Vec::new();
    if size == 2 {
        code.push(0x66);
    } else if size == 8 {
        code.push(0x48);
    }
    code.extend([
        0x0f,
        opcode,
        if immediate.is_some() {
            (extension << 3) | 7
        } else {
            0x0f
        },
    ]);
    if let Some(value) = immediate {
        code.push(value);
    }
    code.push(0xc3);
    code
}

#[test]
fn all_bit_string_widths_select_signed_elements_and_mask_immediates_without_extra_accesses() {
    let cases = [
        (
            2,
            [
                (-17i64, 0x7ffc),
                (-16, 0x7ffe),
                (-1, 0x7ffe),
                (15, 0x8000),
                (16, 0x8002),
            ],
        ),
        (
            4,
            [
                (-33, 0x7ff8),
                (-32, 0x7ffc),
                (-1, 0x7ffc),
                (31, 0x8000),
                (32, 0x8004),
            ],
        ),
        (
            8,
            [
                (-65, 0x7ff0),
                (-64, 0x7ff8),
                (-1, 0x7ff8),
                (63, 0x8000),
                (64, 0x8008),
            ],
        ),
    ];
    for (size, offsets) in cases {
        for (opcode, extension, kind) in [
            (0xa3, 4, MemoryAccessKind::Read),
            (0xab, 5, MemoryAccessKind::ReadWrite),
            (0xb3, 6, MemoryAccessKind::ReadWrite),
            (0xbb, 7, MemoryAccessKind::ReadWrite),
        ] {
            let mut gpr = [0; 16];
            gpr[7] = 0x8000;
            for (index, address) in offsets {
                // Narrow index forms ignore unrelated high GPR bits.
                gpr[1] = match size {
                    2 => 0x1234_5678_9abc_0000 | u64::from(index as u16),
                    4 => 0x1234_5678_0000_0000 | u64::from(index as u32),
                    _ => index as u64,
                };
                let result = analyze(&encoded(size, opcode, extension, None), gpr).unwrap();
                assert_eq!(
                    result.accesses(),
                    [DeclaredMemoryAccess {
                        address,
                        size,
                        kind
                    }]
                );
                assert_eq!(result.register_dependencies().address, (1 << 7) | (1 << 1));
            }
            for imm in [0, 0x7f, 0xff] {
                let result = analyze(&encoded(size, 0xba, extension, Some(imm)), gpr).unwrap();
                assert_eq!(
                    result.accesses(),
                    [DeclaredMemoryAccess {
                        address: 0x8000,
                        size,
                        kind
                    }]
                );
                assert_eq!(result.register_dependencies().address, 1 << 7);
            }
        }
    }
}

#[test]
fn bit_string_rip_sib_full_spans_and_prior_partial_index_writes_keep_original_authority() {
    let mut gpr = [0; 16];
    gpr[12] = 0x9000;
    gpr[13] = 2;
    gpr[9] = u64::MAX;
    let result = analyze(&[0x4f, 0x0f, 0xab, 0x4c, 0xac, 8, 0xc3], gpr).unwrap();
    assert_eq!(
        result.accesses(),
        [DeclaredMemoryAccess {
            address: 0x9008,
            size: 8,
            kind: MemoryAccessKind::ReadWrite
        }]
    );
    assert_eq!(
        result.register_dependencies().address,
        (1 << 12) | (1 << 13) | (1 << 9)
    );
    let mut rip = vec![0x48, 0x0f, 0xa3, 0x0d];
    rip.extend_from_slice(&((DATA as i64 - ENTRY as i64 - 8) as i32).to_le_bytes());
    rip.push(0xc3);
    gpr[1] = 129;
    assert_eq!(
        analyze(&rip, gpr).unwrap().accesses(),
        [DeclaredMemoryAccess {
            address: BIAS + DATA + 16,
            size: 8,
            kind: MemoryAccessKind::Read
        }]
    );
    let mut immediate = vec![0x48, 0x0f, 0xba, 0x25];
    immediate.extend_from_slice(&((DATA as i64 - ENTRY as i64 - 9) as i32).to_le_bytes());
    immediate.extend([0xff, 0xc3]);
    assert_eq!(
        analyze(&immediate, gpr).unwrap().accesses(),
        [DeclaredMemoryAccess {
            address: BIAS + DATA,
            size: 8,
            kind: MemoryAccessKind::Read
        }]
    );
    for (base, index) in [
        (0, -1i64),
        ((1u64 << 47) - 8, 64),
        ((1u64 << 47) - 4, 0),
        (0x8000, i64::MIN),
        (0x8000, i64::MAX),
    ] {
        gpr[7] = base;
        gpr[1] = index as u64;
        assert_eq!(
            analyze(&encoded(8, 0xa3, 4, None), gpr),
            Err(MemoryAccessError::InvalidRange)
        );
    }
    for prefix in [
        &[0xb1, 0xff][..],
        &[0xb5, 0xff][..],
        &[0xb9, 64, 0, 0, 0][..],
        &[0x48, 0x0f, 0x44, 0xc8][..],
    ] {
        let mut code = prefix.to_vec();
        code.extend(encoded(8, 0xab, 5, None));
        let bytes = image(&code);
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
        assert_eq!(ordinary_output(&plan).len(), 1);
        assert_eq!(
            plan.exit().guest_pc(),
            BIAS + ENTRY as u64 + prefix.len() as u64
        );
    }
    for code in [
        vec![0xf0, 0x48, 0x0f, 0xab, 0x0f],
        vec![0xf3, 0x48, 0x0f, 0xab, 0x0f],
        vec![0x64, 0x48, 0x0f, 0xab, 0x0f],
        vec![0x67, 0x48, 0x0f, 0xab, 0x0f],
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(ordinary_output(&plan).len(), 0);
        assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
    }
}
