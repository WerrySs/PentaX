// SPDX-License-Identifier: GPL-2.0-only

//! Original native imports exercise the offline service through all six GPRs.
use super::json_primitives::Program;
use super::*;

const NIDS: [&[u8; 11]; 19] = [
    b"+o9816YQhqQ",
    b"Xweb+naPZ8Y",
    b"WV1GwM32NgY",
    b"sk54bi6FtYM",
    b"9X9+cneTGUU",
    b"3EI-OSJ65Xc",
    b"lQOCF84lvzw",
    b"OOY9+ObfKec",
    b"zpiPsH7dbFQ",
    b"vvzWO-DvG1s",
    b"egOOvrnF6mI",
    b"HwP3aM+c85c",
    b"hksbskNToEA",
    b"fIATVMo4Y1w",
    b"QafxeZM3WK4",
    b"MsaFhR+lPE4",
    b"fY3QqeNkF8k",
    b"3Tt9zL3tkoc",
    b"bEvXpcEk200",
];

fn arguments(p: &mut Program, values: &[u64]) {
    for (index, value) in values.iter().enumerate() {
        p.code.extend([
            if index < 4 { 0x48 } else { 0x49 },
            [0xbf, 0xbe, 0xba, 0xb9, 0xb8, 0xb9][index],
        ]);
        p.code.extend(value.to_le_bytes());
    }
}

fn pointer(p: &mut Program, index: usize, offset: u8) {
    p.code.extend([
        if index < 4 { 0x48 } else { 0x4c },
        0x8d,
        [0x7c, 0x74, 0x54, 0x4c, 0x44, 0x4c][index],
        0x24,
        offset,
    ]);
}

fn call(p: &mut Program, index: usize, result: u64) {
    p.call_nid(NIDS[index]);
    p.result(result);
}

fn passed(p: Program, module: &[u8], library: &[u8]) {
    let image = p.finish(module, library);
    let (status, fields, text) = probe(&image);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_np_webapi2_context_stats_receipts_and_termination_keep_source_widths() {
    let mut p = Program::new();
    arguments(&mut p, &[u64::MAX, 0x4001]);
    call(&mut p, 0, 1);
    for offset in [16, 24, 32, 40] {
        p.store(offset, u64::MAX);
    }
    arguments(&mut p, &[1]);
    pointer(&mut p, 1, 16);
    call(&mut p, 1, 0);
    p.word(16, 0x8000);
    p.word(24, 0);
    p.word(32, 0);
    p.dword(40, 0);
    p.dword(44, u32::MAX as u64);

    arguments(&mut p, &[1, u64::MAX]);
    call(&mut p, 3, 1001);
    arguments(&mut p, &[1]);
    call(&mut p, 2, 1);
    arguments(
        &mut p,
        &[1, 1, 0, 0xface_cafe_0000_0042, u64::MAX, u64::MAX],
    );
    call(&mut p, 15, 1);
    arguments(&mut p, &[1001, 1, 0, u64::MAX]);
    call(&mut p, 16, 1);
    arguments(&mut p, &[1001, 0]);
    call(&mut p, 14, 0);
    arguments(&mut p, &[1, 1]);
    call(&mut p, 13, 0);
    arguments(&mut p, &[1001]);
    call(&mut p, 4, 0);
    arguments(&mut p, &[1]);
    call(&mut p, 18, 0);
    arguments(&mut p, &[1]);
    call(&mut p, 18, 0x8055_3404);
    arguments(&mut p, &[0]);
    call(&mut p, 18, 0x8055_3403);
    arguments(&mut p, &[0, 0]);
    call(&mut p, 1, 0x8055_3402);

    // A zero-sized pool is accepted and termination does not recycle IDs.
    arguments(&mut p, &[u64::MAX, 0]);
    call(&mut p, 0, 2);
    arguments(&mut p, &[2]);
    pointer(&mut p, 1, 16);
    call(&mut p, 1, 0);
    p.word(16, 0);
    arguments(&mut p, &[2]);
    call(&mut p, 18, 0);
    call(&mut p, 17, 0);
    passed(p, b"NpWebApi2", b"NpWebApi2");
}

#[test]
fn native_np_webapi2_six_argument_requests_headers_and_offline_outputs() {
    let mut p = Program::new();
    p.store(96, u64::from_le_bytes(*b"group\0\0\0"));
    p.store(104, u64::from_le_bytes(*b"/v1\0\0\0\0\0"));
    p.store(112, u64::from_le_bytes(*b"GET\0\0\0\0\0"));
    arguments(&mut p, &[u64::MAX, 0, 0, 0, u64::MAX, 0]);
    pointer(&mut p, 1, 96);
    pointer(&mut p, 2, 104);
    pointer(&mut p, 3, 112);
    pointer(&mut p, 5, 0);
    call(&mut p, 5, 0);
    p.word(0, 1000);
    arguments(&mut p, &[1000, u64::MAX, u64::MAX]);
    call(&mut p, 10, 0);

    p.store(112, u64::from_le_bytes(*b"Content-"));
    p.store(120, u64::from_le_bytes(*b"Type\0\0\0\0"));
    arguments(&mut p, &[1000]);
    pointer(&mut p, 1, 112);
    pointer(&mut p, 2, 8);
    call(&mut p, 11, 0);
    p.word(8, 32);
    arguments(&mut p, &[1000, 0, 0, 8]);
    pointer(&mut p, 1, 112);
    pointer(&mut p, 2, 64);
    call(&mut p, 12, 0);
    p.word(64, u64::from_le_bytes(*b"applica\0"));
    p.store(112, u64::from_le_bytes(*b"x\0\0\0\0\0\0\0"));
    p.store(64, 0x5a5a_5a5a_5a5a_5a5a);
    arguments(&mut p, &[1000, 0, 0, 8]);
    pointer(&mut p, 1, 112);
    pointer(&mut p, 2, 64);
    call(&mut p, 12, 0);
    p.word(64, 0x5a5a_5a5a_5a5a_5a00);

    p.store(16, 0xaabb_ccdd_ffff_ffff);
    // error_object points at a separate byte in the same actual native stack.
    p.code
        .extend([0x48, 0x8d, 0x44, 0x24, 64, 0x48, 0x89, 0x44, 0x24, 24]);
    p.store(32, 8);
    p.store(40, u64::MAX);
    p.store(64, u64::MAX);
    arguments(&mut p, &[1000, u64::MAX, u64::MAX]);
    pointer(&mut p, 3, 16);
    call(&mut p, 6, 0x8055_3407);
    p.word(16, 0xaabb_ccdd_0000_0000);
    p.code.extend([0x48, 0x8b, 0x44, 0x24, 24]);
    p.pointer(64);
    p.word(32, 8);
    p.word(40, 0);
    p.word(64, 0xffff_ffff_ffff_ff00);
    arguments(&mut p, &[1000, u64::MAX, u64::MAX]);
    call(&mut p, 7, 0);
    arguments(&mut p, &[1000]);
    call(&mut p, 8, 0);
    arguments(&mut p, &[1000]);
    call(&mut p, 9, 0);
    arguments(&mut p, &[1000]);
    call(&mut p, 8, 0x8055_3406);
    arguments(&mut p, &[1000]);
    call(&mut p, 9, 0);

    // Null output neither consumes an ID nor reads the three invalid strings.
    arguments(&mut p, &[0, u64::MAX, u64::MAX, u64::MAX, u64::MAX, 0]);
    call(&mut p, 5, 0);
    arguments(&mut p, &[0, 0, 0, 0, 0, 0]);
    pointer(&mut p, 5, 0);
    call(&mut p, 5, 0);
    p.word(0, 1001);
    arguments(&mut p, &[1000, u64::MAX, u64::MAX, u64::MAX]);
    call(&mut p, 6, 0x8055_3406);
    arguments(&mut p, &[1001]);
    call(&mut p, 9, 0);
    passed(p, b"libSceNpWebApi2", b"libSceNpWebApi2");
}
