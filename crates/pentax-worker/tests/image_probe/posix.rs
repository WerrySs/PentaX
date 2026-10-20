// SPDX-License-Identifier: GPL-2.0-only

//! Original native calls through separately scoped libkernel/Posix imports.

use super::*;

#[test]
fn native_apr_resolution_observes_real_sizes_stable_ids_partial_errors_and_errno() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.set_errno(37);
    p.path(0, NAME);
    p.save(16);
    p.save(24);
    p.set(88, 85);
    p.ptr(7, 16);
    p.reg(6, 2);
    p.ptr(2, 48);
    p.ptr(1, 64);
    p.ptr(8, 88);
    p.call(1);
    p.expect(0);
    p.errno(37);
    p.word(48, 0x0000_0001_0000_0001);
    p.word(64, 8);
    p.word(72, 8);
    p.word(88, 85);

    p.path(0, MISSING);
    p.save(24);
    p.ptr(7, 16);
    p.reg(6, 2);
    p.ptr(2, 48);
    p.ptr(1, 64);
    p.ptr(8, 88);
    p.call(1);
    p.expect(u64::from(u32::MAX));
    p.errno(2);
    p.word(48, 0xffff_ffff_0000_0001);
    p.word(64, 8);
    p.word(72, 0);
    p.word(88, 1);

    p.ptr(7, 16);
    p.reg(6, 1);
    p.reg(2, 0);
    p.ptr(1, 64);
    p.reg(8, 0);
    p.call(1);
    p.expect(0);
    p.errno(2);
    p.word(64, 8);
    let (code, calls) = p.finish(None);
    let main = image(&code, &["9BcDykPmo1I", "gEpBkcwxUjw"]);
    let data = create_input(vec![42; 8]);
    let mut binding = OsString::from(format!("{PATH}="));
    binding.push(&data.path);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-file".into(),
        binding,
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

impl Program {
    pub(super) fn errno(&mut self, expected: u32) {
        self.load(11, 0); // Retained guest-owned errno pointer, never a host pointer.
        self.code.extend([0x41, 0x8b, 0x03]); // mov eax,[r11]
        self.expect(u64::from(expected));
    }
    pub(super) fn set_errno(&mut self, value: u32) {
        self.load(11, 0);
        self.code.extend([0x41, 0xc7, 0x03]); // mov dword [r11],imm32
        self.code.extend_from_slice(&value.to_le_bytes());
    }
}

pub(super) fn scoped_image(code: &[u8], imports: &[(&str, bool)], posix_version: u16) -> Input {
    scoped_image_with_library(code, imports, posix_version, "Posix")
}

pub(super) fn scoped_image_with_library(
    code: &[u8],
    imports: &[(&str, bool)],
    posix_version: u16,
    library: &str,
) -> Input {
    // Nine imports keep symbols below0x700 and relocations below0x800.
    assert!(imports.len() <= 9);
    let mut bytes = module_bytes(&[], true, None);
    bytes[0x600..0x800].fill(0);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    put(&mut bytes, 0x800, code);
    put(&mut bytes, NAME as usize, format!("{PATH}\0").as_bytes());
    put(
        &mut bytes,
        MISSING as usize,
        b"/app0/assets/missing-original.bin\0",
    );
    let mut names = b"\0libkernel\0libkernel\0".to_vec();
    names.extend_from_slice(library.as_bytes());
    names.push(0);
    for (index, (nid, posix)) in imports.iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        // Consumer-local IDs deliberately differ from the provider's IDs.
        let library = if *posix { 'F' } else { 'D' };
        names.extend_from_slice(format!("{nid}#{library}#C\0").as_bytes());
        put(
            &mut bytes,
            0x700 + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x708 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    put(&mut bytes, 0x400, &names);
    let entries = [
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, (imports.len() as u64 + 1) * 24),
        (11, 24),
        (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
        (0x6100_0015, 11 | (1 << 32) | (3 << 48)),
        (
            0x6100_0015,
            21 | (u64::from(posix_version) << 32) | (5 << 48),
        ),
        (7, 0x700),
        (8, imports.len() as u64 * 24),
        (9, 24),
        (0, 0),
    ];
    for offset in [120 + 32, 120 + 40] {
        put(
            &mut bytes,
            offset,
            &((entries.len() * 16) as u64).to_le_bytes(),
        );
    }
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}

#[test]
fn native_posix_libraries_share_stable_errno_and_continue_to_actual_file_data() {
    // Slots: errno, open, read(libkernel), pread, close, stat(Posix),
    // getpagesize(Posix), read(Posix).
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.errno(0);
    p.set_errno(37);
    p.call(6);
    p.expect(0x4000);
    p.errno(37);
    p.path(7, MISSING);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(1);
    p.expect(0xffff_ffff);
    p.errno(2);
    p.path(7, NAME);
    p.call(1);
    p.expect(3);
    p.save(8);
    p.errno(2);
    p.set(32, 85);
    p.load(7, 8);
    p.ptr(6, 32);
    p.reg(2, 8);
    p.reg(1, u64::MAX);
    p.call(3);
    p.expect(u64::MAX);
    p.errno(22);
    p.word(32, 85);
    p.load(7, 8);
    p.reg(6, 0);
    p.call(2);
    p.expect(u64::MAX);
    p.errno(14);
    p.set(64, 85);
    p.path(7, MISSING);
    p.ptr(6, 64);
    p.call(5);
    p.expect(0xffff_ffff);
    p.errno(2);
    p.word(64, 85);
    p.load(7, 8);
    p.ptr(6, 32);
    p.reg(2, 8);
    p.call(7);
    p.expect(8);
    p.errno(2);
    p.word(32, 42);
    p.load(7, 8);
    p.call(4);
    p.expect(0);
    p.errno(2);
    p.call(0);
    p.load(10, 0);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    let (code, calls) = p.finish(Some(32));
    let main = scoped_image(
        &code,
        &[
            ("9BcDykPmo1I", false),
            ("6c3rCVE-fTU", false),
            ("DRuBt2pvICk", false),
            ("ezv-RSBNKqI", false),
            ("NNtFaKJbPt0", false),
            ("E6ao34wPw+U", true),
            ("k+AXqu2-eBc", true),
            ("AqBioC2vF3I", true),
        ],
        1,
    );
    let data = create_input(42_u64.to_le_bytes().to_vec());
    let mut binding = OsString::from(format!("{PATH}="));
    binding.push(&data.path);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-file".into(),
        binding,
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn native_posix_fstat_promotes_minus_one_and_time_wrappers_preserve_outputs() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.set(64, 85);
    p.reg(7, 0);
    p.ptr(6, 64);
    p.call(1); // Source fstat returns int64 although KernelFstat returns int.
    p.expect(u64::MAX);
    p.errno(1);
    p.word(64, 85);
    p.reg(7, 0);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(2);
    p.expect(u64::MAX);
    p.errno(9);
    p.reg(7, u64::MAX);
    p.call(3);
    p.expect(0xffff_ffff);
    p.errno(9);
    p.reg(7, 0);
    p.call(3); // Source POSIX close accepts stdio without closing host handles.
    p.expect(0);
    p.errno(9);
    for slot in [4, 5, 6] {
        p.reg(7, 3); // Source-invalid clock, distinct from unsupported CPU clocks.
        p.ptr(6, 64);
        p.call(slot);
        p.expect(0xffff_ffff);
        p.errno(22);
        p.word(64, 85);
    }
    p.set(32, 0);
    p.reg(0, u64::MAX);
    p.save(24); // Negative seconds in a complete 16-byte request.
    p.ptr(7, 24);
    p.reg(6, 0);
    p.call(7);
    p.expect(0xffff_ffff);
    p.errno(22);
    let (code, calls) = p.finish(None);
    let main = scoped_image(
        &code,
        &[
            ("9BcDykPmo1I", false),
            ("mqQMh1zPPT8", false),
            ("Oy6IpwgtYOk", false),
            ("NNtFaKJbPt0", false),
            ("lLMT9vJAck0", false),
            ("lLMT9vJAck0", true),
            ("smIj7eqzZE8", true),
            ("yS8U2TGCe1A", true),
        ],
        1,
    );
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_posix_scope_and_version_mismatches_have_no_nid_only_fallback() {
    for (nid, posix, version) in [
        ("9BcDykPmo1I", true, 1),  // errno exists only in libkernel.
        ("E6ao34wPw+U", false, 1), // stat exists only in Posix.
        ("ezv-RSBNKqI", true, 1),  // pread's wrapper export is in libkernel.
        ("k+AXqu2-eBc", true, 2),  // Known NID, unsupported library version.
    ] {
        let mut p = Program::new();
        p.call(0);
        let (code, _) = p.finish(None);
        let main = scoped_image(&code, &[(nid, posix)], version);
        let (status, fields, text) = probe(&main);
        assert_eq!(status, 2, "{text}");
        if version == 1 {
            assert_eq!(fields["code"], "unavailable-function", "{text}");
            assert_eq!(fields["unavailable_functions"], "1");
            assert_eq!(fields["phase"], "main-entry");
            assert_eq!(fields["service_stops"], "1");
        } else {
            assert_eq!(fields["code"], "import-version-mismatch", "{text}");
            assert_eq!(fields["phase"], "admission");
            assert_eq!(fields["native_instructions"], "0");
            assert_eq!(fields["service_stops"], "0");
        }
        assert!(!fields.contains_key("value"));
    }
}

#[test]
fn native_posix_unavailable_clock_remains_a_stop_without_a_fake_errno_result() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.set_errno(37);
    p.reg(7, 14);
    p.ptr(6, 32);
    p.call(1);
    let (code, _) = p.finish(None);
    let main = scoped_image(&code, &[("9BcDykPmo1I", false), ("lLMT9vJAck0", true)], 1);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service");
    assert_eq!(fields["service_stops"], "2");
    assert!(!fields.contains_key("value"));
}
