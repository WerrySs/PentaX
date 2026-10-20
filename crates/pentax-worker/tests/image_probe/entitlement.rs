// SPDX-License-Identifier: GPL-2.0-only

//! Original native calls over the explicit local FULL/empty-addon profile.
//! These checks do not establish PSN, purchase or donor-title entitlement state.

use super::*;

const PARAMETER: u64 = 0x817d_0002;
const NO_ENTITLEMENT: u64 = 0x817d_0007;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const IMPORTS: [&str; 6] = [
    "jO8DM8oyego", // Initialize(init, boot).
    "lPDO62PpJIA", // GetSkuFlag(sku).
    "TFyU+KFBv54", // GetAddcontEntitlementInfoList(label, list, count, hit).
    "xddD23+8TfQ", // GetAddcontEntitlementInfo(label, name, info).
    "5LiMEPuW0DQ", // GetEntitlementKey(label, name, key).
    "9BcDykPmo1I", // Independent libkernel POSIX errno cell.
];

fn entitlement_image(code: &[u8]) -> Input {
    const STRINGS: usize = 0x1400;
    const SYMBOLS: usize = 0x1800;
    const RELOCATIONS: usize = 0x2000;
    assert!(0x800 + code.len() <= 0xf00);
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x2400, 0);
    bytes[0x200..0x800].fill(0);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    let file_size = bytes.len() as u64;
    put(&mut bytes, 96, &file_size.to_le_bytes());
    put(&mut bytes, 0x800, code);
    let mut names = vec![0];
    let mut scopes = Vec::new();
    for name in ["NpEntitlementAccess", "libkernel"] {
        let module = names.len() as u64;
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        let library = names.len() as u64;
        names.extend_from_slice(name.as_bytes());
        names.push(0);
        scopes.push((module, library));
    }
    for (index, nid) in IMPORTS.iter().enumerate() {
        let scope = usize::from(index == 5);
        let module = char::from(b'C' + scope as u8 * 2);
        let library = char::from(b'D' + scope as u8 * 2);
        let symbol = SYMBOLS + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12; // Undefined GLOBAL FUNC.
        names.extend_from_slice(format!("{nid}#{library}#{module}\0").as_bytes());
        put(
            &mut bytes,
            RELOCATIONS + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            RELOCATIONS + index * 24 + 8,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(STRINGS + names.len() <= SYMBOLS);
    assert!(SYMBOLS + (IMPORTS.len() + 1) * 24 <= RELOCATIONS);
    assert!(RELOCATIONS + IMPORTS.len() * 24 <= bytes.len());
    put(&mut bytes, STRINGS, &names);
    let mut dynamic = vec![
        (5_i64, STRINGS as u64),
        (10, names.len() as u64),
        (6, SYMBOLS as u64),
        (39, (IMPORTS.len() as u64 + 1) * 24),
        (11, 24),
    ];
    for (index, (module, library)) in scopes.into_iter().enumerate() {
        let id = 2 + index as u64 * 2;
        dynamic.push((0x6100_000f, module | (1 << 32) | (1 << 40) | (id << 48)));
        dynamic.push((0x6100_0015, library | (1 << 32) | ((id + 1) << 48)));
    }
    dynamic.extend([
        (7, RELOCATIONS as u64),
        (8, IMPORTS.len() as u64 * 24),
        (9, 24),
        (0, 0),
    ]);
    assert!(0x200 + dynamic.len() * 16 < 0x800);
    for offset in [152, 160] {
        put(
            &mut bytes,
            offset,
            &((dynamic.len() * 16) as u64).to_le_bytes(),
        );
    }
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}

fn fill(p: &mut Program, offsets: &[u8]) {
    p.reg(0, CANARY);
    for offset in offsets {
        p.save(*offset);
    }
}

fn zero_word(p: &mut Program, offset: u8) {
    p.load(0, offset);
    p.code.extend([0x48, 0x85, 0xc0]);
    checked(&mut p.code, &mut p.branches);
}

fn check(code: &[u8], calls: u64) {
    let input = entitlement_image(code);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["phase"], "main-entry");
    assert_eq!(
        fields["kernel_handles_live"],
        fields["kernel_handles_accounted"]
    );
}

#[test]
fn native_entitlement_all_five_roles_keep_exact_outputs_original_rcx_and_errno() {
    let mut p = Program::new();
    p.call(5);
    p.save(0);
    p.set_errno(37);
    fill(
        &mut p,
        &[8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120],
    );
    p.reg(7, u64::MAX); // Nonnull InitParam is opaque, never dereferenced.
    p.ptr(6, 16);
    p.call(0);
    p.expect(0);
    for offset in [16, 24, 32, 40] {
        zero_word(&mut p, offset);
    }
    p.word(48, CANARY);
    p.errno(37);

    p.ptr(7, 56);
    p.call(1);
    p.expect(0);
    p.word(56, 0xa5a5_a5a5_0000_0003); // Only the four-byte SKU changes.
    p.word(64, CANARY);
    p.errno(37);

    p.reg(7, u64::MAX); // Service label is not an account/ownership lookup.
    p.ptr(6, 64);
    p.reg(2, 0xdead_beef_0000_0002); // Count is low32, exactly56 output bytes.
    p.ptr(1, 8); // The actual fourth argument is original RCX.
    p.reg(10, u64::MAX); // Using SYSCALL R10 instead would fault/refuse.
    p.call(2);
    p.expect(0);
    p.word(8, 0xa5a5_a5a5_0000_0000); // Empty local catalogue, four-byte hit.
    for offset in [64, 72, 80, 88, 96, 104, 112] {
        zero_word(&mut p, offset);
    }
    p.word(120, CANARY);
    p.errno(37);

    fill(&mut p, &[64, 72, 80, 88, 96, 104, 112]);
    p.reg(7, 0xfeed_beef_0000_002a);
    p.ptr(6, 16); // An original empty label in the already cleared boot buffer.
    p.ptr(2, 64);
    p.call(3);
    p.expect(NO_ENTITLEMENT);
    for offset in [64, 72, 80] {
        zero_word(&mut p, offset);
    }
    p.word(88, 0xa5a5_a5a5_0000_0000); // Exactly28 bytes, no tail clobber.
    p.word(96, CANARY);
    p.errno(37);

    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX); // Even an unreadable nonnull label is not inspected.
    p.ptr(2, 96);
    p.call(4);
    p.expect(NO_ENTITLEMENT);
    for offset in [96, 104, 112] {
        p.word(offset, CANARY);
    }
    p.errno(37);
    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX); // No access at all to a nonnull key on this fallback.
    p.call(4);
    p.expect(NO_ENTITLEMENT);
    p.errno(37);
    let (code, calls) = p.finish(None);
    check(&code, calls);
}

#[test]
fn native_entitlement_null_precedence_and_zero_capacity_do_not_touch_opaque_inputs() {
    let mut p = Program::new();
    p.call(5);
    p.save(0);
    p.set_errno(37);
    fill(&mut p, &[8, 16]);
    for (init, boot) in [(0, u64::MAX), (u64::MAX, 0)] {
        p.reg(7, init);
        p.reg(6, boot);
        p.call(0);
        p.expect(PARAMETER);
    }
    p.reg(7, 0);
    p.call(1);
    p.expect(PARAMETER);
    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX);
    p.reg(2, u64::MAX);
    p.reg(1, 0); // Null hit wins before any list access or count multiplication.
    p.call(2);
    p.expect(PARAMETER);
    p.reg(7, 0);
    p.reg(6, 0);
    p.reg(2, 1);
    p.ptr(1, 8);
    p.call(2);
    p.expect(PARAMETER);
    p.word(8, CANARY);
    for list in [0, u64::MAX] {
        fill(&mut p, &[8]);
        p.reg(7, u64::MAX);
        p.reg(6, list);
        p.reg(2, 0xdead_beef_0000_0000);
        p.ptr(1, 8);
        p.reg(10, u64::MAX);
        p.call(2);
        p.expect(0);
        p.word(8, 0xa5a5_a5a5_0000_0000);
        p.word(16, CANARY);
    }
    for slot in [3, 4] {
        for (label, output) in [(0, u64::MAX), (u64::MAX, 0)] {
            p.reg(7, u64::MAX);
            p.reg(6, label);
            p.reg(2, output);
            p.call(slot);
            p.expect(PARAMETER);
        }
    }
    p.errno(37);
    p.word(16, CANARY);
    let (code, calls) = p.finish(None);
    check(&code, calls);
}
