// SPDX-License-Identifier: GPL-2.0-only

//! Original linked guest checks subrange effects on the actual native owner.
use super::*;

const PAGE: u32 = 0x4000;

#[test]
fn native_protection_refusals_report_closed_causes_without_success_values() {
    for (address, length, expected) in [
        (0, 1, "protection-untracked-range"),
        (u64::MAX, 1, "protection-range-overflow"),
    ] {
        let mut p = Program::new();
        p.reg(7, address);
        p.reg(6, length);
        p.reg(2, 1);
        p.call(0);
        let (code, _) = p.finish(None);
        let original = image(&code, &["vSMAm3cxYTY"]);
        let (status, fields, text) = probe(&original);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["public_function"], "KernelMprotect");
        assert_eq!(fields["runtime_failure"], expected);
        assert!(!fields.contains_key("value"));
    }
    let mut p = Program::new();
    p.call(0); // Actual owned main-thread prefix, never a supplied address.
    p.code.extend([0x48, 0x89, 0xc7]);
    p.reg(6, 1);
    p.reg(2, 1);
    p.call(1);
    let (code, _) = p.finish(None);
    let original = image(&code, &["EotR8a3ASf4", "vSMAm3cxYTY"]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["public_function"], "KernelMprotect");
    assert_eq!(fields["runtime_failure"], "protection-runtime-owned");
    assert!(!fields.contains_key("value"));
}

fn image_with_data(code: &[u8]) -> Input {
    let original = image(code, &["vSMAm3cxYTY", "WFcfL2lzido"]);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x4010, 0);
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x4000_u64),
        (192, 0x4000),
        (208, 16),
        (216, 0x8000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &value.to_le_bytes());
    }
    put(&mut bytes, 0x4000, &42_u64.to_le_bytes());
    create_input(bytes)
}

#[test]
fn native_loaded_image_protection_preserves_data_neighbors_and_raw_query_words() {
    let mut p = Program::new();
    p.path(7, 0x4001);
    p.reg(6, 1);
    p.reg(2, 0x8000_0001);
    p.call(0);
    p.expect(0);
    p.path(7, 0x4000);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(42);
    p.path(7, 0x8000);
    p.code.extend([0x48, 0xc7, 0x07, 33, 0, 0, 0]);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(33);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(48);
    p.path(7, 0x4000);
    p.ptr(6, 32);
    p.ptr(2, 40);
    p.ptr(1, 48);
    p.call(1);
    p.expect(0);
    for (slot, target) in [(32, 0x4000), (40, 0x8000)] {
        p.path(7, target);
        p.load(0, slot);
        p.code.extend([0x48, 0x39, 0xf8]);
        checked(&mut p.code, &mut p.branches);
    }
    p.word(48, 0xa5a5_a5a5_8000_0001);
    for protection in [0, 2] {
        p.path(7, 0x4000);
        p.reg(6, 0x4000);
        p.reg(2, protection);
        p.call(0);
        p.expect(0);
    }
    p.path(7, 0x4000);
    p.code.extend([0x48, 0xc7, 0x07, 37, 0, 0, 0]);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(37);
    let (code, calls) = p.finish(None);
    let original = image_with_data(&code);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_loaded_image_read_only_protection_denies_actual_cpu_stores() {
    let mut p = Program::new();
    p.path(7, 0x4000);
    p.reg(6, 0x4000);
    p.reg(2, 1);
    p.call(0);
    p.expect(0);
    p.path(7, 0x4000);
    p.code.extend([0x48, 0xc7, 0x07, 99, 0, 0, 0]);
    let (code, _) = p.finish(None);
    let original = image_with_data(&code);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["runtime_failure"], "memory-permission");
    assert_eq!(fields["dispatch_cpu_access"], "read");
    assert_eq!(fields["dispatch_origin"], "primary-image");
    assert!(!fields.contains_key("value"));
}

fn address(p: &mut Program, offset: u32) {
    p.load(7, 8);
    p.code.extend([0x48, 0x81, 0xc7]); // ADD RDI, imm32
    p.code.extend(offset.to_le_bytes());
}
fn relative_word(p: &mut Program, slot: u8, offset: u32) {
    address(p, offset);
    p.load(0, slot);
    p.code.extend([0x48, 0x39, 0xf8]); // CMP RAX, RDI
    checked(&mut p.code, &mut p.branches);
}
fn query(p: &mut Program, offset: u32, result: u64) {
    address(p, offset);
    p.ptr(6, 24);
    p.ptr(2, 32);
    p.ptr(1, 40);
    p.call(3);
    p.expect(result);
}
fn map(p: &mut Program, slot: u8, length: u32) {
    p.ptr(7, slot);
    p.reg(6, u64::from(length));
    p.reg(2, 3);
    p.reg(1, 0);
    p.call(0);
    p.expect(0);
}
fn protect(p: &mut Program, offset: u32, length: u32, protection: u64) {
    address(p, offset);
    p.reg(6, u64::from(length));
    p.reg(2, protection);
    p.call(1);
    p.expect(0);
}
fn unmap(p: &mut Program, offset: u32, length: u32, result: u64) {
    address(p, offset);
    p.reg(6, u64::from(length));
    p.call(2);
    p.expect(result);
}
fn write(p: &mut Program, offset: u32, value: u32) {
    address(p, offset);
    p.code.extend([0x48, 0xc7, 0x07]); // MOV qword [RDI], imm32
    p.code.extend(value.to_le_bytes());
}
fn read(p: &mut Program, offset: u32, value: u64) {
    address(p, offset);
    p.code.extend([0x48, 0x8b, 0x07]); // MOV RAX, [RDI]
    p.expect(value);
}

#[test]
fn guest_memory_range_protection_unmap_and_reuse_preserve_neighbors_and_source_results() {
    let mut p = Program::new();
    p.set(8, 0);
    p.set(40, 0); // Query writes exactly four bytes of protection.
    map(&mut p, 8, 3 * PAGE);
    write(&mut p, 0, 33);
    write(&mut p, 2 * PAGE, 42);
    protect(&mut p, PAGE + 1, 1, 1);
    query(&mut p, PAGE + 1, 0);
    relative_word(&mut p, 24, PAGE);
    relative_word(&mut p, 32, 2 * PAGE);
    p.word(40, 1);
    write(&mut p, 0, 34);
    read(&mut p, 2 * PAGE, 42);
    protect(&mut p, 0, 3 * PAGE, 3);
    query(&mut p, PAGE, 0);
    relative_word(&mut p, 24, 0);
    relative_word(&mut p, 32, 3 * PAGE);
    p.word(40, 3);
    unmap(&mut p, PAGE, PAGE, 0);
    query(&mut p, PAGE, 0x8002_000d);
    // A late hole refuses the whole request, leaving both neighbors readable.
    unmap(&mut p, 0, 3 * PAGE, 0x8002_000d);
    read(&mut p, 0, 34);
    read(&mut p, 2 * PAGE, 42);
    address(&mut p, PAGE);
    p.stack(0x89, 7, 16);
    map(&mut p, 16, PAGE);
    relative_word(&mut p, 16, PAGE);
    read(&mut p, PAGE, 0);
    write(&mut p, PAGE, 99);
    // This now spans three records with two distinct allocation identities.
    unmap(&mut p, 0, 3 * PAGE, 0);
    map(&mut p, 8, 3 * PAGE);
    read(&mut p, 0, 0);
    read(&mut p, PAGE, 0);
    read(&mut p, 2 * PAGE, 0);
    let (code, calls) = p.finish(None);
    let original = image(
        &code,
        &["IWIBBdTHit4", "vSMAm3cxYTY", "cQke9UuBQOk", "WFcfL2lzido"],
    );
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
