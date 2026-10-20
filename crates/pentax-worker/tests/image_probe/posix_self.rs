// SPDX-License-Identifier: GPL-2.0-only
//! Original selected-thread POSIX identity consumers; TLS is checked separately.

use super::*;

const CHILD: i32 = 0x1100;
const CANARY: u64 = 0x8877_6655_4433_2211;
const CHILD_CANARY: u64 = 0x1020_3040_5060_7080;

fn equal_saved(p: &mut Program, offset: u8) {
    p.load(10, offset);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
}

fn different_saved(p: &mut Program, offset: u8) {
    p.load(10, offset);
    p.code.extend([0x4c, 0x39, 0xd0, 0x0f, 0x84, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}

fn child() -> Vec<u8> {
    let mut code = vec![0x53, 0x48, 0x83, 0xec, 32, 0x48, 0x89, 0xfb];
    let mut branches = Vec::new();
    // RDI points to the parent's admitted shared stack window. No host address
    // or equivalence between the pthread prefix and FS:0 is assumed.
    code.extend([0x48, 0x89, 0x64, 0x24, 16]);
    code.extend([0xc7, 0x04, 0x24, 0xc0, 0x1f, 0, 0]);
    code.extend([0x0f, 0xae, 0x14, 0x24]);
    code.extend([0x49, 0xb9]);
    code.extend(CHILD_CANARY.to_le_bytes());
    let wait = code.len();
    code.extend([0x48, 0x83, 0x3b, 1, 0x0f, 0x85]);
    code.extend((wait as i32 - (code.len() as i32 + 4)).to_le_bytes());
    for (slot, offset) in [(0, 8), (1, 16), (2, 24), (4, 40)] {
        rip(&mut code, CHILD, &[0xff, 0x15], 0xf00 + slot * 8);
        code.extend([0x48, 0x89, 0x43, offset]);
        if slot == 1 {
            code.extend([0x48, 0x3b, 0x43, 8]);
            checked(&mut code, &mut branches);
        }
    }
    code.extend([0xc7, 0x00, 19, 0, 0, 0]); // Actual child errno cell.
    code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    code.extend([0x48, 0x89, 0x43, 32]);
    code.extend([0x48, 0x8b, 0x44, 0x24, 16, 0x48, 0x39, 0xe0]);
    checked(&mut code, &mut branches);
    code.extend([0x49, 0xba]);
    code.extend(CHILD_CANARY.to_le_bytes());
    code.extend([0x4d, 0x39, 0xd1]);
    checked(&mut code, &mut branches);
    code.extend([0x0f, 0xae, 0x1c, 0x24, 0x81, 0x3c, 0x24, 0xc0, 0x1f, 0, 0]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0xc7, 0x03, 42, 0, 0, 0]);
    code.extend([0xb8, 73, 0, 0, 0, 0x48, 0x83, 0xc4, 32, 0x5b, 0xc3]);
    finish_checks(&mut code, branches);
    code
}

#[test]
fn native_posix_self_uses_each_actual_selected_thread_and_keeps_tls_distinct() {
    let imports = [
        ("aI+OeCz8xrQ", false), // Canonical selected-thread self.
        ("EotR8a3ASf4", true),  // The exact appended POSIX binding.
        ("EI-5-jlq2dE", false),
        ("6UgtwV+0zb4", false),
        ("9BcDykPmo1I", false),
    ];
    let mut p = Program::new();
    p.stack(0x89, 4, 120);
    p.reg(0, CANARY);
    p.save(80);
    p.reg(0, 0x1f84);
    p.save(112);
    p.code.extend([0x0f, 0xae, 0x54, 0x24, 112]);
    p.reg(9, CANARY);
    p.call(0);
    p.save(8);
    p.code.extend([0x48, 0x85, 0xc0, 0x0f, 0x84, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
    p.call(1);
    equal_saved(&mut p, 8);
    p.call(2);
    p.expect(1);
    p.call(4);
    p.save(0);
    p.set_errno(37);
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    p.save(16);
    p.set(32, 0);
    p.ptr(7, 24);
    p.reg(6, 0);
    p.path(2, CHILD);
    p.ptr(1, 32);
    p.reg(8, 0);
    p.call(3);
    p.expect(0);
    p.set(32, 1);
    let wait = p.code.len();
    p.load(0, 32);
    p.code.extend([0x48, 0x83, 0xf8, 42, 0x0f, 0x85]);
    p.code
        .extend((wait as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
    p.word(56, 2);
    p.word(80, CANARY);
    p.load(0, 40);
    different_saved(&mut p, 8);
    equal_saved(&mut p, 24); // Create's guest handle, not the TLS pointer.
    equal_saved(&mut p, 48);
    p.load(0, 64);
    different_saved(&mut p, 16);
    p.load(0, 72);
    p.code.extend([0x8b, 0x00]);
    p.expect(19);
    p.call(1);
    equal_saved(&mut p, 8);
    p.call(0);
    equal_saved(&mut p, 8);
    p.call(4);
    equal_saved(&mut p, 0);
    p.errno(37);
    p.call(2);
    p.expect(1);
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    equal_saved(&mut p, 16);
    p.code.extend([0x4c, 0x89, 0xc8]);
    p.expect(CANARY);
    p.code.extend([0x48, 0x89, 0xe0]);
    equal_saved(&mut p, 120);
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, 112]);
    p.word(112, 0x1f84);
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 9);
    let child = child();
    assert!(child.len() < 0x700);
    for library in ["Posix", "libScePosix"] {
        let original = posix::scoped_image_with_library(&code, &imports, 1, library);
        let mut bytes = original.bytes.clone();
        bytes.resize(0x1800, 0);
        put(&mut bytes, 96, &0x1800_u64.to_le_bytes());
        put(&mut bytes, 0x800, &code); // Restore any helper path-label overlap.
        put(&mut bytes, CHILD as usize, &child);
        let input = create_input(bytes);
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], (calls + 4).to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        assert!(
            fields["primary_native_instructions"]
                .parse::<u64>()
                .unwrap()
                > 0,
            "{text}"
        );
    }
}
