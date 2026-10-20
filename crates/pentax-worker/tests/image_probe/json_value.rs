// SPDX-License-Identifier: GPL-2.0-only

//! Original native Null-value construction observes all words and edge canaries.
use super::json_allocator::scoped_consumer;
use super::*;

fn code() -> Vec<u8> {
    let mut code = vec![0x55, 0x48, 0x83, 0xec, 0x40];
    let mut branches = Vec::new();
    code.extend([0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff, 0xff]);
    for offset in [0, 8, 16, 24, 32, 40] {
        code.extend([0x48, 0x89, 0x44, 0x24, offset]);
    }
    code.extend([0x48, 0x8d, 0x7c, 0x24, 9]);
    rip(&mut code, 0x100, &[0xff, 0x15], 0xf00);
    code.extend([0x48, 0x8d, 0x54, 0x24, 9, 0x48, 0x39, 0xd0]);
    checked(&mut code, &mut branches);
    for offset in [9, 17, 25, 33] {
        code.extend([0x48, 0x83, 0x7c, 0x24, offset, 0]);
        checked(&mut code, &mut branches);
    }
    for offset in [8, 41] {
        code.extend([0x80, 0x7c, 0x24, offset, 0xff]);
        checked(&mut code, &mut branches);
    }
    code.extend([0x31, 0xff]);
    rip(&mut code, 0x100, &[0xff, 0x15], 0xf00);
    code.extend([0x48, 0x85, 0xc0]);
    checked(&mut code, &mut branches);
    code.extend([0xb8, 42, 0, 0, 0, 0x48, 0x83, 0xc4, 0x40, 0x5d, 0xc3]);
    finish_checks(&mut code, branches);
    assert!(code.len() < 0x100);
    code
}

fn verify(module: &[u8], library: &[u8], nid: &[u8; 11]) {
    let image = scoped_consumer(&code(), module, library, nid);
    let (status, fields, text) = probe(&image);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["service_stops"], "2", "{text}");
}

#[test]
fn native_value_constructor_clears_32_bytes_and_returns_self_in_both_public_identities() {
    for nid in [b"qBMjqyBn3OM", b"-wa17B7TGnw"] {
        let canonical = b"Json2".as_slice();
        let sce = b"libSceJson2".as_slice();
        for (module, library) in [
            (canonical, canonical),
            (sce, sce),
            (canonical, sce),
            (sce, canonical),
        ] {
            verify(module, library, nid);
        }
    }
}

#[test]
fn native_value_constructor_admits_the_observed_distinct_module_library_identity() {
    verify(b"libSceJson", b"libSceJson2", b"qBMjqyBn3OM");
}
