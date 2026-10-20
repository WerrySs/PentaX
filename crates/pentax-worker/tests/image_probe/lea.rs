// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_lea_preserves_flags_and_uses_effective_width_without_segment_or_memory_access() {
    let mut p = Program::new();
    for (instruction, expected) in [
        (&[0x67, 0x8d, 0x44, 0x8b, 0x20][..], 0x34),
        (&[0x64, 0x67, 0x48, 0x8d, 0x44, 0x8b, 0x20], 0x34),
        (
            &[0x65, 0x66, 0x67, 0x8d, 0x44, 0x8b, 0x20],
            0x8877_6655_4433_0034,
        ),
        (&[0x64, 0x48, 0x8d, 0x44, 0x8b, 0x20], 0xa_0000_0034),
        (&[0x65, 0x8d, 0x44, 0x8b, 0x20], 0x34),
        (&[0x66, 0x8d, 0x44, 0x8b, 0x20], 0x8877_6655_4433_0034),
    ] {
        p.reg(0, 0x8877_6655_4433_2211);
        p.reg(3, 0x1_ffff_fff0);
        p.reg(1, 0x2_0000_0009);
        p.code.extend([0x45, 0x31, 0xd2, 0xf9]); // XOR R10D,R10D; STC: ZF/PF/CF set, SF/OF clear.
        p.code.extend_from_slice(instruction);
        for condition in [0x83, 0x85, 0x80, 0x88, 0x8b] {
            // JNC/JNZ/JO/JS/JNP must not take.
            p.code.extend([0x0f, condition]);
            p.branches.push(p.code.len());
            p.code.extend([0; 4]);
        }
        p.expect(expected);
    }
    rip(&mut p.code, 0x800, &[0x48, 0x8d, 0x1d], NAME);
    rip(&mut p.code, 0x800, &[0x67, 0x48, 0x8d, 0x05], NAME);
    p.code.extend([0x89, 0xdb, 0x48, 0x39, 0xd8]); // EIP result is zero-extended low32 guest target.
    checked(&mut p.code, &mut p.branches);
    let (code, _) = p.finish(None);
    let input = image(&code, &[]);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
}
