// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers: real caller-owned RW/NX arena and copied bytes.
use super::json_primitives::Program;
use super::*;

const REGULAR: [&[u8; 11]; 10] = [
    b"-hn1tcVHq5Q",
    b"W6SiVSiCDtI",
    b"OJjm-QOIHlI",
    b"Vla-Z+eXlxo",
    b"LYo3GhIlB38",
    b"gigoVHZvVPE",
    b"iF1iQHzxBJU",
    b"p6lrRW8-MLY",
    b"mfHdJTIvhuo",
    b"k04jLXu3+Ic",
];
const PAF: [&[u8; 11]; 10] = [
    b"6hdfGRKHefs",
    b"qB5nGjWa-bk",
    b"QuZzFJD5Hrw",
    b"9mMuuhXMwqQ",
    b"-lZdT34nAAE",
    b"u32UXVridxQ",
    b"PKJcFUfhKtw",
    b"4SvlEtd0j40",
    b"mO8NB8whKy8",
    b"OmG3YPCBLJs",
];
const PAYLOAD: u64 = 0x0102_03fe_fd80_7f42;
const HEADER: u64 = 0xaabb_ccdd_0102_0028;

fn imm(p: &mut Program, register: usize, value: u64) {
    let (prefix, opcode) = [
        (0x48, 0xbf),
        (0x48, 0xbe),
        (0x48, 0xba),
        (0x48, 0xb9),
        (0x49, 0xb8),
        (0x49, 0xb9),
    ][register];
    p.code.extend([prefix, opcode]);
    p.code.extend(value.to_le_bytes());
}
fn load(p: &mut Program, register: usize, slot: u8) {
    let (prefix, opcode) = [
        (0x48, 0x7c),
        (0x48, 0x74),
        (0x48, 0x54),
        (0x48, 0x4c),
        (0x4c, 0x44),
        (0x4c, 0x4c),
    ][register];
    p.code.extend([prefix, 0x8b, opcode, 0x24, slot]);
}
fn save(p: &mut Program, slot: u8) {
    p.code.extend([0x48, 0x89, 0x44, 0x24, slot]);
}
fn base_result(p: &mut Program, offset: u8) {
    load(p, 2, 0);
    if offset != 0 {
        p.code.extend([0x48, 0x83, 0xc2, offset]);
    }
    p.code.extend([0x48, 0x39, 0xd0]);
    p.checked();
}
fn aligned(p: &mut Program, alignment: u64) {
    p.code.extend([0x48, 0x85, 0xc0]); // non-null
    // Convert the nonzero check to the existing equality-check failure path.
    p.code
        .extend([0x0f, 0x95, 0xc2, 0x0f, 0xb6, 0xd2, 0x83, 0xfa, 1]);
    p.checked();
    save(p, 40);
    p.code.extend([0x48, 0x25]);
    p.code.extend(((alignment - 1) as u32).to_le_bytes());
    p.result(0);
    p.code.extend([0x48, 0x8b, 0x44, 0x24, 40]);
}
fn payload(p: &mut Program, slot: u8, expected: u64) {
    load(p, 2, slot);
    p.code.extend([0x48, 0x8b, 0x02]);
    p.result(expected);
}
fn free(p: &mut Program, nids: &[&'static [u8; 11]; 10], slot: u8) {
    load(p, 0, 0);
    load(p, 1, slot);
    imm(p, 2, u64::MAX); // ignored PAF owner
    p.call_nid(nids[3]);
}
fn stats(p: &mut Program, nid: &'static [u8; 11]) {
    p.store(64, HEADER);
    load(p, 0, 0);
    p.source(64);
    p.call_nid(nid);
    p.result(0);
    p.word(64, HEADER); // version/reserved and oversized-independent header
    p.word(72, 4096);
    p.word(80, 4096);
    p.word(96, 0);
}

fn lifecycle(paf: bool) {
    let nids = if paf { &PAF } else { &REGULAR };
    let mut p = Program::new();
    // Keep the normal builder's fixed epilogue; restore the extra 7936 below.
    p.code[4..8].copy_from_slice(&8192_u32.to_le_bytes());
    // RSI = aligned caller-owned stack pool; no HLE-created host allocation.
    p.code.extend([0x48, 0x8d, 0xb4, 0x24]);
    p.code.extend(543_u32.to_le_bytes());
    p.code.extend([0x48, 0x83, 0xe6, 0xe0, 0x48, 0x89, 0xf0]);
    save(&mut p, 0);
    imm(&mut p, 2, PAYLOAD);
    p.code.extend([0x48, 0x89, 0x16]);
    imm(&mut p, 0, u64::MAX); // source ignores name without dereferencing it
    imm(&mut p, 2, 4096);
    imm(&mut p, 3, 0xfeed_cafe_0000_0001); // unsigned flags use low 32 bits
    imm(&mut p, 4, 0);
    p.call_nid(nids[0]);
    base_result(&mut p, 0);
    payload(&mut p, 0, PAYLOAD); // Create wrote no guest header

    load(&mut p, 0, 0);
    imm(&mut p, 1, 64);
    imm(&mut p, 2, u64::MAX);
    p.call_nid(nids[2]);
    base_result(&mut p, 64);
    save(&mut p, 8);
    load(&mut p, 2, 8);
    imm(&mut p, 3, PAYLOAD);
    p.code.extend([0x48, 0x89, 0x0a]);
    load(&mut p, 0, 0);
    load(&mut p, 1, 8);
    imm(&mut p, 2, 96);
    imm(&mut p, 3, u64::MAX);
    p.call_nid(nids[5]);
    base_result(&mut p, 64);
    // Block adjacent growth, then force a genuine move and byte copy.
    load(&mut p, 0, 0);
    imm(&mut p, 1, 32);
    imm(&mut p, 2, u64::MAX);
    p.call_nid(nids[2]);
    save(&mut p, 16);
    load(&mut p, 0, 0);
    load(&mut p, 1, 8);
    imm(&mut p, 2, 160);
    imm(&mut p, 3, u64::MAX);
    p.call_nid(nids[5]);
    save(&mut p, 8);
    payload(&mut p, 8, PAYLOAD);
    load(&mut p, 0, 0);
    imm(&mut p, 1, 2);
    imm(&mut p, 2, 8);
    imm(&mut p, 3, u64::MAX);
    p.call_nid(nids[4]);
    base_result(&mut p, 64);
    save(&mut p, 24);
    payload(&mut p, 24, 0);

    if !paf {
        load(&mut p, 0, 0);
        p.source(32);
        imm(&mut p, 2, 64);
        imm(&mut p, 3, 17);
        p.call_nid(b"qWESlyXMI3E");
        p.result(0);
        load(&mut p, 0, 32);
        p.call_nid(b"fEoW6BJsPt4");
        p.result(32);
        free(&mut p, nids, 32);
        load(&mut p, 0, 0);
        imm(&mut p, 1, 64);
        imm(&mut p, 2, 3);
        p.call_nid(b"ljkqMcC4-mk");
        aligned(&mut p, 64);
        save(&mut p, 32);
        free(&mut p, nids, 32); // source accepts non-multiple size
    }
    load(&mut p, 0, 0);
    imm(&mut p, 1, 128);
    imm(&mut p, 2, 9);
    imm(&mut p, 3, u64::MAX);
    p.call_nid(nids[6]);
    aligned(&mut p, 128);
    save(&mut p, 32);
    free(&mut p, nids, 32);
    load(&mut p, 0, 0);
    load(&mut p, 1, 8);
    imm(&mut p, 2, 256);
    imm(&mut p, 3, 200);
    imm(&mut p, 4, u64::MAX);
    p.call_nid(nids[7]);
    aligned(&mut p, 256);
    save(&mut p, 8);
    payload(&mut p, 8, PAYLOAD);
    for slot in [8, 16, 24] {
        free(&mut p, nids, slot);
    }
    stats(&mut p, nids[8]);
    stats(&mut p, nids[9]);
    load(&mut p, 0, 0);
    p.call_nid(nids[1]);
    p.result(0);
    payload(&mut p, 0, PAYLOAD); // Destroy neither unmaps nor clears backing
    p.code.extend([0x48, 0x81, 0xc4]);
    p.code.extend(7936_u32.to_le_bytes());
    let input = p.finish(b"libSceLibcInternal", b"libSceLibcInternal");
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(
        fields["kernel_handles_live"], fields["kernel_handles_accounted"],
        "{text}"
    );
    assert!(
        !fields["kernel_handles_by_type"].contains("libc_heaps:"),
        "{text}"
    );
}

#[test]
fn native_mspace_lifecycle_uses_caller_arena_and_copies_real_guest_bytes() {
    lifecycle(false);
}

#[test]
fn native_paf_mspace_lifecycle_preserves_fourth_and_fifth_sysv_arguments() {
    lifecycle(true);
}
