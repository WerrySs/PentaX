// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_integer_counts_execute_every_width_and_real_flag_consumers() {
    let mut p = Program::new();
    for (opcode, input) in [(0xbd, 0), (0xbc, 1), (0xb8, 0xaa)] {
        for size in [2, 4, 8] {
            let count = match opcode {
                0xbd => size * 8,
                0xbc => 0,
                _ => 4,
            };
            p.reg(9, 0xdead_beef_abcd_0000);
            p.reg(0, input);
            if size == 4 {
                p.save(64);
                p.ptr(0, 64);
            }
            if size == 8 {
                p.reg(11, 0);
                p.reg(8, 0);
            }
            if size == 2 {
                p.code.push(0x66);
            }
            p.code.extend([
                0xf3,
                if size == 8 { 0x4c } else { 0x44 },
                0x0f,
                opcode,
                if size == 4 { 0x08 } else { 0xc8 },
            ]);
            if size == 8 {
                p.code.extend([0x41, 0x0f, 0x92, 0xc3]); // SETC R11B.
                p.code.extend([0x41, 0x0f, 0x94, 0xc0]); // SETZ R8B.
            }
            p.code.extend([0x4c, 0x89, 0xc8]); // MOV RAX, R9.
            p.expect(if size == 2 {
                0xdead_beef_abcd_0000 | count
            } else {
                count
            });
            if size == 8 {
                p.code.extend([0x4c, 0x89, 0xd8]);
                p.expect(u64::from(opcode == 0xbd));
                p.code.extend([0x4c, 0x89, 0xc0]);
                p.expect(u64::from(opcode == 0xbc));
            }
        }
    }
    let (code, _) = p.finish(None);
    let original = image(&code, &[]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    // LZCNT is modeled by this worker profile. Supported TZCNT/POPCNT forms
    // remain in the native body; absent host features use the Rust fallback.
    let modeled_families = 1
        + u64::from(!std::is_x86_feature_detected!("bmi1"))
        + u64::from(!std::is_x86_feature_detected!("popcnt"));
    assert_eq!(fields["integer_ops"], (3 * modeled_families).to_string());
    assert_eq!(fields["integer_bytes"], (4 * modeled_families).to_string());
    assert_eq!(fields["vector_ops"], "0");
}
