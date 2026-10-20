// SPDX-License-Identifier: GPL-2.0-only

use super::*;

fn trace_image(code: &[u8], prefixed: bool) -> Input {
    let original = image(code, &["NWtTN10cJzE"]);
    let mut bytes = original.bytes.clone();
    let mut names = if prefixed {
        b"\0libSceLibcInternal\0libSceLibcInternalExt\0".to_vec()
    } else {
        b"\0LibcInternal\0LibcInternalExt\0".to_vec()
    };
    let library_offset = if prefixed { 20_u64 } else { 14_u64 };
    put(&mut bytes, 0x618, &(names.len() as u32).to_le_bytes());
    names.extend_from_slice(b"NWtTN10cJzE#D#C\0");
    put(&mut bytes, 0x400, &names);
    put(&mut bytes, 0x218, &(names.len() as u64).to_le_bytes());
    put(
        &mut bytes,
        0x268,
        &(library_offset | (1 << 32) | (3 << 48)).to_le_bytes(),
    );
    create_input(bytes)
}

#[test]
fn native_heap_trace_info_exposes_mutable_persistent_guest_storage_and_checks_record_size() {
    for (invalid, prefixed) in [(false, false), (false, true), (true, true)] {
        let mut p = Program::new();
        p.set(16, if invalid { 31 } else { 32 });
        p.reg(0, 0x1122_3344_5566_7788);
        p.save(24);
        p.ptr(7, 16);
        p.reg(6, u64::MAX);
        p.reg(2, u64::MAX);
        p.call(0);
        p.word(16, 32);
        p.word(24, 0x1122_3344_5566_7788);
        p.load(11, 32); // Real mask pointer.
        p.code.extend([0x49, 0x8b, 0x03]);
        p.expect(0);
        p.code.extend([0x49, 0xc7, 0x03, 17, 0, 0, 0]);
        p.load(11, 40); // Real table pointer, including its final entry.
        p.code.extend([0x49, 0x8b, 0x83, 0xf8, 1, 0, 0]);
        p.expect(0);
        p.code
            .extend([0x49, 0xc7, 0x83, 0xf8, 1, 0, 0, 29, 0, 0, 0]);
        p.ptr(7, 16);
        p.call(0);
        p.load(11, 32);
        p.code.extend([0x49, 0x8b, 0x03]);
        p.expect(17);
        p.load(11, 40);
        p.code.extend([0x49, 0x8b, 0x83, 0xf8, 1, 0, 0]);
        p.expect(29);
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&trace_image(&code, prefixed));
        assert_eq!(fields["unavailable_functions"], "0");
        if invalid {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["service_stops"], "1");
            assert!(!fields.contains_key("value"));
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["service_stops"], "2");
            assert_eq!(fields["value"], "42");
        }
    }
}
