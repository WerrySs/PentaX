// SPDX-License-Identifier: GPL-2.0-only

//! Original x64 code observes a shared relocated kernel object across modules.
use super::*;

#[test]
fn native_entry_observes_null_optional_weak_imports_without_callable_substitutes() {
    let mut p = Program::new();
    for slot in [0xf00, 0xf08] {
        rip(&mut p.code, 0x800, &[0x48, 0x8b, 0x05], slot);
        p.expect(0);
    }
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 0);
    let original = image(&code, &["AAAAAAAAAAA", "BBBBBBBBBBA"]);
    let mut bytes = original.bytes.clone();
    for (index, info) in [0x22, 0x21].into_iter().enumerate() {
        bytes[0x600 + (index + 1) * 24 + 4] = info;
        put(
            &mut bytes,
            0x708 + index * 24,
            &(((index as u64 + 1) << 32) | 6).to_le_bytes(),
        );
    }
    let main = create_input(bytes);
    let (status, fields, text) = probe_images(&[("--image", &main)]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
    assert_eq!(fields["unavailable_functions"], "0");
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

fn guard_input(expected: u64, replacement: u64, initializer: bool) -> Input {
    let mut p = Program::new();
    rip(&mut p.code, 0x800, &[0x48, 0x8b, 0x05], 0xf10);
    p.code.extend([0x48, 0x8b, 0x00, 0x48, 0x8b, 0x00]);
    p.expect(u64::from_le_bytes(*b"PentaX\0\0"));
    rip(&mut p.code, 0x800, &[0x48, 0x8b, 0x05], 0xf00);
    p.code.extend([0x48, 0x8b, 0x00]); // Read the actual imported object's storage.
    p.expect(expected);
    rip(&mut p.code, 0x800, &[0x48, 0x8b, 0x0d], 0xf08);
    p.reg(0, replacement);
    p.code.extend([0x48, 0x89, 0x01]); // Write through an independent import.
    rip(&mut p.code, 0x800, &[0x48, 0x8b, 0x05], 0xf00);
    p.code.extend([0x48, 0x8b, 0x00]);
    p.expect(replacement);
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 0);
    let original = image(&code, &["f7uOxY9mM1U", "f7uOxY9mM1U", "djxxOmW6-aw"]);
    let mut bytes = original.bytes.clone();
    for index in 0..3 {
        bytes[0x600 + (index + 1) * 24 + 4] = 0x11; // GLOBAL OBJECT.
        put(
            &mut bytes,
            0x708 + index * 24,
            &(((index as u64 + 1) << 32) | 6).to_le_bytes(),
        );
    }
    if initializer {
        for offset in [152, 160] {
            put(&mut bytes, offset, &(12_u64 * 16).to_le_bytes());
        }
        put(&mut bytes, 0x2a0, &12_i64.to_le_bytes());
        put(&mut bytes, 0x2a8, &0x800_u64.to_le_bytes());
    }
    create_input(bytes)
}

#[test]
fn native_preload_and_primary_share_the_real_kernel_guard_and_new_owners_reset_it() {
    let initial = 0xdead_beef_0000_0007;
    let preload = guard_input(initial, 73, true);
    let primary = guard_input(73, 99, false);
    let (status, fields, text) =
        probe_images(&[("--image", &primary), ("--preload-libc", &preload)]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "0");
    assert_eq!(fields["unavailable_functions"], "0");
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
    let fresh = guard_input(initial, 17, false);
    let (status, fields, text) = probe_images(&[("--image", &fresh)]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "0");
}
