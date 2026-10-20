// SPDX-License-Identifier: GPL-2.0-only

//! Actual original guest file calls through retained read-only data handles.

use super::*;

const GUEST_FILE: &str = "/app0/assets/pentax-original.bin";

pub(super) fn consumer(flags: u32, invalid_output: bool) -> Input {
    let mut code = vec![
        0x55, 0x48, 0x89, 0xe5, 0x53, 0x48, 0x81, 0xec, 0x08, 1, 0, 0,
    ];
    let mut branches = Vec::new();
    rip(&mut code, 0x800, &[0x48, 0x8d, 0x3d], 0xd00);
    code.push(0xbe);
    code.extend_from_slice(&flags.to_le_bytes());
    code.extend([0x31, 0xd2]);
    call(&mut code, 0);
    code.extend([0x83, 0xf8, 3]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x89, 0xc3]);
    // The first ordinary read advances the logical cursor by one.
    read(&mut code, 1, 1, None, invalid_output);
    code.extend([0x83, 0xf8, 1]);
    checked(&mut code, &mut branches);
    code.extend([0x80, 0x7c, 0x24, 32, 37]);
    checked(&mut code, &mut branches);
    // Positional read observes the next byte but leaves the ordinary cursor.
    read(&mut code, 2, 1, Some(1), false);
    code.extend([0x83, 0xf8, 1]);
    checked(&mut code, &mut branches);
    code.extend([0x80, 0x7c, 0x24, 32, 42]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x89, 0xdf, 0x31, 0xf6, 0xba, 1, 0, 0, 0]);
    call(&mut code, 3); // SEEK_CUR must still be one after pread.
    code.extend([0x83, 0xf8, 1]);
    checked(&mut code, &mut branches);
    read(&mut code, 1, 1, None, false);
    code.extend([0x83, 0xf8, 1]);
    checked(&mut code, &mut branches);
    code.extend([0x80, 0x7c, 0x24, 32, 42]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x89, 0xdf, 0x48, 0x8d, 0x74, 0x24, 64]);
    call(&mut code, 4);
    code.extend([0x85, 0xc0]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x83, 0xbc, 0x24, 136, 0, 0, 0, 3]); // fstat size at64+72.
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x89, 0xdf]);
    call(&mut code, 5);
    code.extend([0x85, 0xc0]);
    checked(&mut code, &mut branches);
    code.extend([
        0x0f, 0xb6, 0x44, 0x24, 32, 0x48, 0x81, 0xc4, 0x08, 1, 0, 0, 0x5b, 0x5d, 0xc3,
    ]);
    finish_checks(&mut code, branches);
    let mut bytes = module_bytes(&[], true, None);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    put(&mut bytes, 0x800, &code);
    put(&mut bytes, 0xd00, format!("{GUEST_FILE}\0").as_bytes());
    let mut names = b"\0libkernel\0libkernel\0".to_vec();
    bytes[0x600..0x7f0].fill(0);
    let nids = [
        "1G3lF1Gg1k8",
        "Cg4srZ6TKbU",
        "+r3rMFwItV4",
        "oib76F-12fk",
        "kBwCPsYX-m4",
        "UK2Tl2DWUns",
    ];
    for (index, nid) in nids.into_iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        names.extend_from_slice(format!("{nid}#D#C\0").as_bytes());
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
    for (index, (tag, value)) in [
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, 7 * 24),
        (11, 24),
        (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
        (0x6100_0015, 11 | (1 << 32) | (3 << 48)),
        (7, 0x700),
        (8, 6 * 24),
        (9, 24),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}
fn call(code: &mut Vec<u8>, index: u64) {
    rip(code, 0x800, &[0xff, 0x15], 0xf00 + index as i32 * 8);
}
fn read(code: &mut Vec<u8>, index: u64, bytes: u32, offset: Option<u32>, invalid: bool) {
    code.extend([0x48, 0x89, 0xdf]);
    if invalid {
        code.extend([0xbe, 1, 0, 0, 0]);
    } else {
        code.extend([0x48, 0x8d, 0x74, 0x24, 32]);
    }
    code.push(0xba);
    code.extend_from_slice(&bytes.to_le_bytes());
    if let Some(offset) = offset {
        code.push(0xb9);
        code.extend_from_slice(&offset.to_le_bytes());
    }
    call(code, index);
}
fn arguments(main: &Input, data: &Input, guest: &str) -> Vec<OsString> {
    let mut binding = OsString::from(guest);
    binding.push("=");
    binding.push(&data.path);
    vec![
        "--image".into(),
        main.path.clone().into(),
        "--data-file".into(),
        binding,
    ]
}

#[test]
fn actual_guest_reads_explicit_data_and_pread_preserves_its_independent_cursor() {
    let main = consumer(0, false);
    let data = create_input(vec![37, 42, 99]);
    let (code, fields, text) = probe_arguments(&arguments(&main, &data, GUEST_FILE));
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "7");
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn unhandled_file_errno_reaches_guest_checks_and_bad_backing_remains_a_typed_stop() {
    let data = create_input(vec![37, 42, 99]);
    for (flags, invalid_output, guest, stops) in [
        (0, false, "/app0/other/pentax-original.bin", "1"),
        (1, false, GUEST_FILE, "1"),
        (0, true, GUEST_FILE, "2"),
    ] {
        let main = consumer(flags, invalid_output);
        let (code, fields, text) = probe_arguments(&arguments(&main, &data, guest));
        assert_eq!(code, 2, "{text}");
        assert_eq!(
            fields["code"],
            if invalid_output {
                "runtime-service"
            } else {
                "unsupported-instruction"
            }
        );
        if !invalid_output {
            assert_eq!(fields["instruction"], "Ud2");
        }
        assert_eq!(fields["service_stops"], stops);
        assert!(!fields.contains_key("value"));
    }
}

#[test]
fn data_bindings_are_separate_from_preloads_and_reject_traversal_duplicates_and_cap_limits() {
    let main = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let data = create_input(vec![37, 42, 99]);
    let library = module_input(&[0xb8, 7, 0, 0, 0, 0xc3], false, Some(0x100));
    let mut args = arguments(&main, &data, GUEST_FILE);
    args.extend(["--preload-libc".into(), library.path.clone().into()]);
    let (code, fields, text) = probe_arguments(&args);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    for guest in [
        "/app0/../escape",
        "/app0/./file",
        "/app0//file",
        "C:/file",
        "/app0",
        "/app0/file\\other",
    ] {
        let (code, fields, text) = probe_arguments(&arguments(&main, &data, guest));
        assert_eq!(code, 2, "{text}");
        assert_eq!(fields["code"], "data-file-binding");
        assert_eq!(fields["block_attempts"], "0");
    }
    let mut args = arguments(&main, &data, GUEST_FILE);
    args.extend_from_slice(&arguments(&main, &data, GUEST_FILE)[2..]);
    let (_, fields, _) = probe_arguments(&args);
    assert_eq!(fields["code"], "duplicate-data-file");
    let mut args: Vec<OsString> = vec!["--image".into(), main.path.clone().into()];
    for _ in 0..257 {
        args.extend_from_slice(&arguments(&main, &data, GUEST_FILE)[2..]);
    }
    let (_, fields, _) = probe_arguments(&args);
    assert_eq!(fields["code"], "image-arguments");
    assert_eq!(fields["block_attempts"], "0");
}
