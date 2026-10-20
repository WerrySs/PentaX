// SPDX-License-Identifier: GPL-2.0-only

//! Original native registration calls retain opaque words without invoking them.
use super::*;

#[test]
fn native_cxa_registration_accepts_null_and_opaque_words_in_all_reviewed_scopes() {
    let mut code = vec![0x55];
    let mut branches = Vec::new();
    for words in [
        [0_u64, 0, u64::MAX],
        [u64::MAX, 0xfeed_face_1234_5678, u64::MAX],
        [u64::MAX, 0xfeed_face_1234_5678, u64::MAX],
    ] {
        for (opcode, value) in [
            ([0x48, 0xbf], words[0]),
            ([0x48, 0xbe], words[1]),
            ([0x48, 0xba], words[2]),
        ] {
            code.extend(opcode);
            code.extend(value.to_le_bytes());
        }
        rip(&mut code, 0x100, &[0xff, 0x15], 0xf00);
        code.extend([0x48, 0x85, 0xc0]);
        checked(&mut code, &mut branches);
    }
    code.extend([0xb8, 42, 0, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut code, branches);
    assert!(code.len() < 0x100);
    let canonical = b"LibcInternal".as_slice();
    let sce = b"libSceLibcInternal".as_slice();
    for (module, library) in [
        (canonical, canonical),
        (sce, sce),
        (canonical, sce),
        (sce, canonical),
    ] {
        let mut image = module_bytes(&code, true, None);
        let mut names = vec![0];
        names.extend_from_slice(module);
        names.push(0);
        let library_name = names.len();
        names.extend_from_slice(library);
        names.push(0);
        let qualified = names.len();
        names.extend_from_slice(b"tsvEmnenz48#D#C\0");
        put(&mut image, 0x400, &names);
        put(&mut image, 0x218, &(names.len() as u64).to_le_bytes());
        put(&mut image, 0x618, &(qualified as u32).to_le_bytes());
        put(
            &mut image,
            0x258,
            &(1_u64 | (1 << 32) | (1 << 40) | (2 << 48)).to_le_bytes(),
        );
        put(
            &mut image,
            0x268,
            &(library_name as u64 | (1 << 32) | (3 << 48)).to_le_bytes(),
        );
        let image = create_input(image);
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["service_stops"], "3", "{text}");
    }
}
