// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_cache_hints_resume_all_four_forms_without_target_access_or_flag_changes() {
    let mut code = vec![0x49, 0xb9]; // MOV R9, noncanonical hint target.
    code.extend(0x0000_8000_0000_0000_u64.to_le_bytes());
    code.extend([0x31, 0xc0, 0xf9]); // XOR EAX,EAX; STC: ZF=CF=1.
    for group in 0..4 {
        for prefix in [vec![], vec![0x64], vec![0x65, 0x67]] {
            code.extend(prefix);
            code.extend([0x41, 0x0f, 0x18, 0x01 | (group << 3)]);
        }
    }
    let mut failures = Vec::new();
    for condition in [0x83, 0x85] {
        // JNC/JNZ must both remain untaken.
        code.extend([0x0f, condition]);
        failures.push(code.len());
        code.extend(0_u32.to_le_bytes());
    }
    code.extend([0xb8, 42, 0, 0, 0, 0xc3]);
    let failure = code.len();
    code.extend([0xb8, 0, 0, 0, 0, 0xc3]);
    for offset in failures {
        let relative = i32::try_from(failure - (offset + 4)).unwrap();
        code[offset..offset + 4].copy_from_slice(&relative.to_le_bytes());
    }
    let original = input(&code, false);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
    assert_eq!(fields["vector_ops"], "0");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
}
