// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn lea_address_only_forms_have_no_accesses_and_keep_destination_dependencies() {
    for code in [
        &[0x67, 0x8d, 0x44, 0x8b, 0x20][..],
        &[0x64, 0x67, 0x48, 0x8d, 0x44, 0x8b, 0x20],
        &[0x65, 0x66, 0x67, 0x8d, 0x44, 0x8b, 0x20],
        &[0x64, 0x48, 0x8d, 0x44, 0x8b, 0x20],
        &[0x65, 0x8d, 0x44, 0x8b, 0x20],
        &[0x66, 0x8d, 0x44, 0x8b, 0x20],
    ] {
        let mut input = code.to_vec();
        input.push(0xc3);
        let bytes = image(&input);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        let emitted = ordinary_output(&plan);
        assert_eq!(emitted.len(), 1);
        assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
        let record = &plan.instructions()[0];
        assert_eq!(emitted[0].code(), record.instruction().code());
        assert_eq!(emitted[0].memory_base(), record.instruction().memory_base());
        assert_eq!(
            emitted[0].memory_index(),
            record.instruction().memory_index()
        );
        let accesses = MemoryAccessAnalyzer::default()
            .analyze(record, &[u64::MAX; 16])
            .unwrap();
        assert!(accesses.accesses().is_empty());
        assert_eq!(accesses.register_dependencies().address, 0);
        assert_eq!(accesses.register_dependencies().written, 1);
    }
}

#[test]
fn lea_eip_relative_reencoding_wraps_at_32_bits_even_above_four_gibibytes() {
    let mut code = vec![0x67, 0x48, 0x8d, 0x05];
    code.extend_from_slice(
        &0xffff_fff0_u32
            .wrapping_sub((BIAS + ENTRY as u64 + 8) as u32)
            .to_le_bytes(),
    );
    code.push(0xc3);
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let cache = CachePlacement {
        address: 0x100_8000_0000,
        capacity: PAGE,
    };
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        cache,
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let emitted = Decoder::with_ip(64, plan.bytes(), cache.address, DecoderOptions::NONE).decode();
    assert_eq!(emitted.mnemonic(), Mnemonic::Lea);
    assert_eq!(emitted.memory_base(), Register::EIP);
    assert_eq!(emitted.ip_rel_memory_address(), 0xffff_fff0);
    assert_eq!(
        plan.instructions()[0].rip_relative_target(),
        Some(0xffff_fff0)
    );
    assert!(
        MemoryAccessAnalyzer::default()
            .analyze(&plan.instructions()[0], &[u64::MAX; 16])
            .unwrap()
            .accesses()
            .is_empty()
    );
}

#[test]
fn lea_destination_still_splits_before_a_dependent_real_memory_access() {
    let bytes = image(&[0x67, 0x48, 0x8d, 0x01, 0x8b, 0x10, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let policy = TranslationPolicy::x64_sse2().with_independent_memory();
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &policy,
        &BlockLimits::default(),
    )
    .unwrap();
    assert_eq!(ordinary_output(&plan).len(), 1);
    assert_eq!(
        plan.exit().kind(),
        ExitKind::Fallthrough {
            target: BIAS + ENTRY as u64 + 4
        }
    );
}

#[test]
fn lea_distant_and_noncanonical_ip_constants_use_width_matched_immediates() {
    for prefix in [&[0x66][..], &[][..], &[0x48][..]] {
        for (bias, displacement) in [
            (BIAS, 0x10_i32),
            (BIAS, -0x20_0000),
            (0x7fff_ffff_0000, 0x1_0000),
        ] {
            let mut code = prefix.to_vec();
            code.extend([0x8d, 0x05]);
            code.extend(displacement.to_le_bytes());
            code.push(0xc3);
            let bytes = image(&code);
            let source = link(&[(&bytes, bias)]);
            let cache = CachePlacement {
                address: 0x7000_0000_0000,
                capacity: PAGE,
            };
            let plan = plan_block(
                &source,
                0,
                bias + ENTRY as u64,
                cache,
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            )
            .unwrap();
            let original = plan.instructions()[0].instruction();
            let target = original.ip_rel_memory_address();
            let emitted =
                Decoder::with_ip(64, plan.bytes(), cache.address, DecoderOptions::NONE).decode();
            assert_eq!(original.mnemonic(), Mnemonic::Lea);
            assert_eq!(emitted.mnemonic(), Mnemonic::Mov);
            assert_eq!(emitted.op0_register(), original.op0_register());
            match emitted.op0_register().size() {
                2 => assert_eq!(emitted.immediate16(), target as u16),
                4 => assert_eq!(emitted.immediate32(), target as u32),
                8 => assert_eq!(emitted.immediate64(), target),
                _ => panic!("unexpected original LEA destination"),
            }
            assert_eq!(plan.instructions()[0].rip_relative_target(), Some(target));
            assert!(
                MemoryAccessAnalyzer::default()
                    .analyze(&plan.instructions()[0], &[u64::MAX; 16])
                    .unwrap()
                    .accesses()
                    .is_empty()
            );
        }
    }
}

#[test]
fn lea_normalized_prefix_boundary_retries_constant_with_charged_work_and_clean_scratch() {
    let bytes = image(&[0x64, 0x65, 0x48, 0x8d, 0x05, 0, 0, 0, 0, 0xc3]);
    let bias = 0x4_0000_0000;
    let pc = bias + ENTRY as u64;
    let source = link(&[(&bytes, bias)]);
    let cache = CachePlacement {
        address: pc - i32::MAX as u64,
        capacity: PAGE,
    };
    let plan = plan_block(
        &source,
        0,
        pc,
        cache,
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let emitted = Decoder::with_ip(64, plan.bytes(), cache.address, DecoderOptions::NONE).decode();
    assert_eq!(emitted.code(), Code::Mov_r64_imm64);
    assert_eq!(emitted.immediate64(), pc + 9);
    assert_eq!(plan.instructions()[0].source_len(), 9);
    assert_eq!(plan.instructions()[0].emitted_len(), emitted.len());
    let limits = BlockLimits {
        max_work_units: plan.usage().work_units - 1,
        ..BlockLimits::default()
    };
    assert!(matches!(
        plan_block(
            &source,
            0,
            pc,
            cache,
            &TranslationPolicy::x64_sse2(),
            &limits
        ),
        Err(CodegenError::Budget {
            kind: Budget::Work,
            ..
        })
    ));
}
