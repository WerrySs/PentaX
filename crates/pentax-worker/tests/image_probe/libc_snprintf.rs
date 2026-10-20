// SPDX-License-Identifier: GPL-2.0-only
//! Original native calls: real PRE-SYSCALL GP, XMM and guest-stack arguments.
use super::json_primitives::Program;
use super::*;

fn store(p: &mut Program, offset: u32, value: u64) {
    p.code.extend([0x48, 0xb8]);
    p.code.extend(value.to_le_bytes());
    p.code.extend([0x48, 0x89, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
}

fn text(p: &mut Program, offset: u32, bytes: &[u8]) {
    for (index, chunk) in bytes.chunks(8).enumerate() {
        let mut word = [0; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        store(p, offset + index as u32 * 8, u64::from_le_bytes(word));
    }
}

fn pointer(p: &mut Program, register: usize, offset: u32) {
    p.code
        .extend([0x48, 0x8d, [0xbc, 0xb4, 0x94][register], 0x24]);
    p.code.extend(offset.to_le_bytes());
}

fn arguments(p: &mut Program, a: u64, b: u64, c: u64) {
    for (prefix, opcode, value) in [(0x48, 0xb9, a), (0x49, 0xb8, b), (0x49, 0xb9, c)] {
        p.code.extend([prefix, opcode]);
        p.code.extend(value.to_le_bytes());
    }
}

fn invoke(p: &mut Program, format: u32) {
    for offset in (160..256).step_by(8) {
        store(p, offset, 0xa7a7_a7a7_a7a7_a7a7);
    }
    pointer(p, 0, 160);
    p.argument(96);
    pointer(p, 2, format);
    // Caller AL advertises the full eight saved vector registers. The marker
    // overwrites it; the source profile eagerly snapshots XMM independently.
    p.code.extend([0xb8, 8, 0, 0, 0]);
    p.call_nid(b"eLdDw6l0-bU");
}

fn output(p: &mut Program, expected: &[u8]) {
    for (index, byte) in expected.iter().copied().chain([0, 0xa7]).enumerate() {
        p.code.extend([0x0f, 0xb6, 0x84, 0x24]);
        p.code.extend((160 + index as u32).to_le_bytes());
        p.result(u64::from(byte));
    }
}

fn passed(p: Program, module: &[u8], library: &[u8]) {
    let input = p.finish(module, library);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_snprintf_preserves_gp_xmm_and_one_shared_overflow_cursor() {
    let mut p = Program::new();
    text(
        &mut p,
        64,
        b"%u/%u/%u/%u/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%.0f/%u\0",
    );
    store(&mut p, 0, 40);
    store(&mut p, 8, 9_f64.to_bits());
    store(&mut p, 16, 50);
    for index in 0..8 {
        store(&mut p, 32, ((index + 1) as f64).to_bits());
        store(&mut p, 40, 0xface_cafe_0000_0000 | u64::from(index));
        p.code
            .extend([0xf3, 0x0f, 0x6f, 0x44 | (index << 3), 0x24, 32]);
    }
    arguments(&mut p, 10, 20, 30);
    invoke(&mut p, 64);
    let expected = b"10/20/30/40/1/2/3/4/5/6/7/8/9/50";
    p.result(expected.len() as u64);
    output(&mut p, expected);
    passed(p, b"LibcInternal", b"LibcInternal");
}

#[test]
fn native_snprintf_uses_guest_utf16_and_preserves_embedded_nul_copy_contract() {
    let mut p = Program::new();
    let wide = "hé😀"
        .encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    text(&mut p, 64, &wide);
    text(&mut p, 96, b"%ls|%#p|%cTAIL\0");
    // LEA RCX,[rsp+64] supplies the first variadic guest pointer.
    p.code.extend([0x48, 0x8d, 0x8c, 0x24]);
    p.code.extend(64_u32.to_le_bytes());
    p.code.extend([0x49, 0xb8]);
    p.code.extend(0xab_u64.to_le_bytes());
    p.code.extend([0x49, 0xb9]);
    p.code.extend(0_u64.to_le_bytes());
    invoke(&mut p, 96);
    let prefix = "hé😀|0X000000000000AB|".as_bytes();
    p.result(prefix.len() as u64 + 5);
    output(&mut p, prefix);
    passed(p, b"libSceLibcInternal", b"libSceLibcInternal");
}
