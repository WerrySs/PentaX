// SPDX-License-Identifier: GPL-2.0-only

//! Original native C++ constructor calls observe guest words and terminal slots.
use super::*;

fn consumer(code: &[u8], module: &[u8], library: &[u8]) -> Input {
    scoped_consumer(code, module, library, b"-hJRce8wn1U")
}

pub(super) fn scoped_consumer(code: &[u8], module: &[u8], library: &[u8], nid: &[u8; 11]) -> Input {
    let mut image = module_bytes(code, true, None);
    let mut names = vec![0];
    names.extend_from_slice(module);
    names.push(0);
    let library_name = names.len();
    names.extend_from_slice(library);
    names.push(0);
    let qualified = names.len();
    names.extend_from_slice(nid);
    names.extend_from_slice(b"#D#C\0");
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
    create_input(image)
}

fn code(virtual_call: bool) -> Vec<u8> {
    let mut code = vec![0x55, 0x48, 0x83, 0xec, 0x20];
    let mut branches = Vec::new();
    code.extend([0xc7, 0x04, 0x24, 42, 0, 0, 0]);
    code.extend([0xc7, 0x44, 0x24, 16, 42, 0, 0, 0]);
    code.extend([0x48, 0x8d, 0x7c, 0x24, 8]);
    rip(&mut code, 0x100, &[0xff, 0x15], 0xf00);
    code.extend([0x48, 0x8b, 0x44, 0x24, 8]);
    code.extend([0x48, 0x85, 0xc0, 0x0f, 0x84, 0, 0, 0, 0]);
    branches.push(code.len() - 4);
    for comparison in [
        &[0x83, 0x3c, 0x24, 42][..],
        &[0x83, 0x7c, 0x24, 16, 42][..],
        &[0x48, 0x83, 0x78, 0xf0, 0][..],
        &[0x48, 0x83, 0x78, 0xf8, 0][..],
    ] {
        code.extend_from_slice(comparison);
        checked(&mut code, &mut branches);
    }
    code.extend([0x48, 0x8b, 0x10]);
    for offset in [8, 16, 24, 32, 40] {
        code.extend([0x48, 0x39, 0x50, offset]);
        checked(&mut code, &mut branches);
    }
    if virtual_call {
        code.extend([0xff, 0x10]);
    }
    code.extend([0xb8, 42, 0, 0, 0, 0x48, 0x83, 0xc4, 0x20, 0x5d, 0xc3]);
    finish_checks(&mut code, branches);
    assert!(code.len() < 0x100);
    code
}

#[test]
fn native_constructor_initializes_guest_table_under_each_exact_scope() {
    let canonical = b"Json2".as_slice();
    let sce = b"libSceJson2".as_slice();
    for (module, library) in [
        (canonical, canonical),
        (sce, sce),
        (canonical, sce),
        (sce, canonical),
        (b"libSceJson".as_slice(), sce),
    ] {
        let image = consumer(&code(false), module, library);
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["service_stops"], "1", "{text}");
    }
}

#[test]
fn native_indirect_virtual_call_stops_with_explicit_unavailable_category() {
    let image = consumer(&code(true), b"libSceJson", b"libSceJson2");
    let (status, fields, text) = probe(&image);
    assert_ne!(status, 0, "{text}");
    assert_eq!(
        fields["code"], "json-allocator-virtual-unavailable",
        "{text}"
    );
    assert!(!fields.contains_key("value"), "{text}");
}
