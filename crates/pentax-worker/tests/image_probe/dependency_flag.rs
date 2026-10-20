// SPDX-License-Identifier: GPL-2.0-or-later

//! Actual native loads/stores to the source-supported canonical uint32 object.
use super::*;

#[test]
fn native_dependency_flag_is_initially_one_then_preserves_guest_writes_in_its_own_process() {
    let mut code = Vec::new();
    rip(&mut code, 0x100, &[0x48, 0x8b, 0x15], 0xf00);
    code.extend([0x83, 0x3a, 1]);
    let mut branches = Vec::new();
    checked(&mut code, &mut branches);
    code.extend([0xc7, 2, 42, 0, 0, 0, 0x8b, 2, 0xc3]);
    finish_checks(&mut code, branches);
    for scope in [b"LibcInternal".as_slice(), b"libSceLibcInternal".as_slice()] {
        let mut image = module_bytes(&code, true, None);
        let mut names = vec![0];
        names.extend_from_slice(scope);
        names.push(0);
        let library = names.len();
        names.extend_from_slice(scope);
        names.push(0);
        let qualified = names.len();
        names.extend_from_slice(b"ZT4ODD2Ts9o#D#C\0");
        put(&mut image, 0x400, &names);
        put(&mut image, 0x218, &(names.len() as u64).to_le_bytes());
        put(&mut image, 0x618, &(qualified as u32).to_le_bytes());
        image[0x61c] = 0x11;
        put(&mut image, 0x688, &((1_u64 << 32) | 6).to_le_bytes());
        put(
            &mut image,
            0x258,
            &(1_u64 | (1 << 32) | (1 << 40) | (2 << 48)).to_le_bytes(),
        );
        put(
            &mut image,
            0x268,
            &(library as u64 | (1 << 32) | (3 << 48)).to_le_bytes(),
        );
        let image = create_input(image);
        for _ in 0..2 {
            let (status, fields, text) = probe(&image);
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["unavailable_functions"], "0");
            assert_eq!(fields["service_stops"], "0");
        }
    }
}
