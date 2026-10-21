// SPDX-License-Identifier: GPL-2.0-only
//! Original native caller: fused cancellation, scalar lanes and guest FTZ flags.
use super::*;

#[test]
fn native_fma_continues_scalar_register_and_memory_operations_with_exact_fusion_and_status() {
    let mut p = Program::new();
    for (slot, value) in [
        (16, 0x3fef_ffff_fc00_0000_u64),
        (24, 0x1122_3344_5566_7788),
        (32, 0x3ff0_0000_0200_0000),
        (40, 0xa5a5_a5a5_a5a5_a5a5),
        (48, 0xbff0_0000_0000_0000),
        (56, 0),
    ] {
        p.reg(0, value);
        p.save(slot);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x44, 0x24, 16]); // Full ymm0 has nonzero upper lanes.
    p.code.extend([0xc5, 0xf9, 0x10, 0x4c, 0x24, 32]);
    p.code.extend([0xc5, 0xf9, 0x10, 0x54, 0x24, 48]);
    p.code.extend([0xc4, 0xe2, 0xf1, 0xa9, 0xc2]); // VFMADD213SD xmm0,xmm1,xmm2.
    p.code.extend([0xc5, 0xfc, 0x11, 0x44, 0x24, 80]);
    for (slot, value) in [
        (80, 0xbc90_0000_0000_0000_u64),
        (88, 0x1122_3344_5566_7788),
        (96, 0),
        (104, 0),
    ] {
        p.word(slot, value);
    }

    for (slot, value) in [
        (16, 0xdead_beef_3f7f_f800_u64),
        (32, 0x3f80_0400),
        (48, 0xfeed_face_bf80_0000),
    ] {
        p.reg(0, value);
        p.save(slot);
    }
    p.code.extend([0xc5, 0xfc, 0x10, 0x5c, 0x24, 16]);
    p.code.extend([0xc5, 0xf9, 0x10, 0x64, 0x24, 32]);
    p.code.extend([0xc4, 0xe2, 0x59, 0xa9, 0x5c, 0x24, 48]); // Scalar four-byte memory operand.
    p.code.extend([0xc5, 0xfc, 0x11, 0x5c, 0x24, 80]);
    for (slot, value) in [
        (80, 0xdead_beef_b280_0000_u64),
        (88, 0x1122_3344_5566_7788),
        (96, 0),
        (104, 0),
    ] {
        p.word(slot, value);
    }

    // A tiny exact result that rounds to minimum normal still flushes with
    // source FTZ and sets underflow/precision in both execution paths.
    p.reg(0, 0x9f80);
    p.save(16);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 16]);
    p.reg(0, 0x3f7f_ffff);
    p.code.extend([0xc5, 0xf9, 0x6e, 0xd8]);
    p.reg(0, 0x0080_0000);
    p.code.extend([0xc5, 0xf9, 0x6e, 0xe0]);
    p.code.extend([0xc5, 0xd0, 0x57, 0xed]);
    p.code.extend([0xc4, 0xe2, 0x59, 0xa9, 0xdd]);
    p.code.extend([0xc5, 0xf9, 0x7e, 0xd8]);
    p.expect(0);
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["fp_status"], "48");
}
