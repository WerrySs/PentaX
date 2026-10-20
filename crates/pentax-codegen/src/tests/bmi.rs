// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn prefixed_alignment_nops_have_no_effective_address_or_memory_footprint() {
    for segment in [0x2e, 0x36, 0x3e, 0x26, 0x64, 0x65] {
        for address32 in [false, true] {
            let mut code = vec![segment, 0x66];
            if address32 {
                code.push(0x67);
            }
            code.extend([0x0f, 0x1f, 0x84, 0x00, 0xff, 0xff, 0xff, 0x7f, 0xc3]);
            let bytes = image(&code);
            let source = link(&[(&bytes, BIAS)]);
            let plan = make(&source);
            assert_eq!(ordinary_output(&plan)[0].mnemonic(), Mnemonic::Nop);
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
fn bmi_vex_gprs_require_declared_features_and_preserve_memory_isolation() {
    // Original encodings: destination RAX (MULX also RCX), source RDX.
    for (third, opcode, modrm, immediate, mnemonic, feature) in [
        (0xf0, 0xf2, 0xc2, false, Mnemonic::Andn, CpuidFeature::BMI1),
        (0xf0, 0xf7, 0xc2, false, Mnemonic::Bextr, CpuidFeature::BMI1),
        (0xf8, 0xf3, 0xda, false, Mnemonic::Blsi, CpuidFeature::BMI1),
        (
            0xf8,
            0xf3,
            0xd2,
            false,
            Mnemonic::Blsmsk,
            CpuidFeature::BMI1,
        ),
        (0xf8, 0xf3, 0xca, false, Mnemonic::Blsr, CpuidFeature::BMI1),
        (0xf0, 0xf5, 0xc2, false, Mnemonic::Bzhi, CpuidFeature::BMI2),
        (0xf3, 0xf6, 0xc2, false, Mnemonic::Mulx, CpuidFeature::BMI2),
        (0xf3, 0xf5, 0xc2, false, Mnemonic::Pdep, CpuidFeature::BMI2),
        (0xf2, 0xf5, 0xc2, false, Mnemonic::Pext, CpuidFeature::BMI2),
        (0xfb, 0xf0, 0xc2, true, Mnemonic::Rorx, CpuidFeature::BMI2),
        (0xf2, 0xf7, 0xc2, false, Mnemonic::Sarx, CpuidFeature::BMI2),
        (0xf1, 0xf7, 0xc2, false, Mnemonic::Shlx, CpuidFeature::BMI2),
        (0xf3, 0xf7, 0xc2, false, Mnemonic::Shrx, CpuidFeature::BMI2),
    ] {
        for wide in [false, true] {
            for memory in [false, true] {
                let mut code = vec![
                    0xc4,
                    if immediate { 0xe3 } else { 0xe2 },
                    if wide { third } else { third & 0x7f },
                    opcode,
                    if memory { modrm & 0x3f } else { modrm },
                ];
                if immediate {
                    code.push(7);
                }
                code.push(0xc3);
                let bytes = image(&code);
                let source = link(&[(&bytes, BIAS)]);
                let refused = make(&source);
                assert_eq!(refused.exit().instruction().unwrap().mnemonic(), mnemonic);
                assert_eq!(
                    refused.exit().kind(),
                    ExitKind::Mediated {
                        reason: UnsupportedReason::MissingFeature(feature),
                    }
                );
                let plan = plan_block(
                    &source,
                    0,
                    BIAS + ENTRY as u64,
                    placement(),
                    &TranslationPolicy::new(CpuFeatures::x64_sse2().with(feature))
                        .with_isolated_memory(),
                    &BlockLimits::default(),
                )
                .unwrap();
                let output = ordinary_output(&plan);
                assert_eq!(output.len(), 1);
                assert_eq!(output[0].mnemonic(), mnemonic);
                assert_eq!(output[0].op0_register().size(), if wide { 8 } else { 4 });
                let mut registers = [0; 16];
                registers[2] = 0x8000;
                let footprint = MemoryAccessAnalyzer::default()
                    .analyze(&plan.instructions()[0], &registers)
                    .unwrap();
                if memory {
                    assert!(matches!(plan.exit().kind(), ExitKind::Fallthrough { .. }));
                    assert_eq!(
                        footprint.accesses(),
                        [DeclaredMemoryAccess {
                            address: 0x8000,
                            size: if wide { 8 } else { 4 },
                            kind: MemoryAccessKind::Read,
                        }]
                    );
                } else {
                    assert!(matches!(plan.exit().kind(), ExitKind::Return { .. }));
                    assert!(footprint.accesses().is_empty());
                }
            }
        }
    }
}

#[test]
fn bmi_rip_operands_relocate_without_admitting_vectors_or_segment_and_address_overrides() {
    let policy = TranslationPolicy::new(
        CpuidFeature::values().fold(CpuFeatures::empty(), CpuFeatures::with),
    );
    let mut code = vec![0xc4, 0xe2, 0xf0, 0xf2, 0x05];
    code.extend(((DATA as i64 - ENTRY as i64 - 9) as i32).to_le_bytes());
    code.push(0xc3);
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &policy,
        &BlockLimits::default(),
    )
    .unwrap();
    assert_eq!(
        ordinary_output(&plan)[0].ip_rel_memory_address(),
        BIAS + DATA
    );
    for rejected in [
        &[0xc5, 0xf8, 0x77][..],               // VZEROUPPER
        &[0xc5, 0xf9, 0xef, 0xc0],             // VPXOR
        &[0x64, 0xc4, 0xe2, 0xf0, 0xf2, 0x02], // FS memory
        &[0x67, 0xc4, 0xe2, 0xf0, 0xf2, 0x02], // 32-bit address
    ] {
        let bytes = image(rejected);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &policy,
            &BlockLimits::default(),
        )
        .unwrap();
        assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
        assert!(ordinary_output(&plan).is_empty());
    }
}
