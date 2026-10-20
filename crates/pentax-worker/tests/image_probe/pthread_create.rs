// SPDX-License-Identifier: GPL-2.0-only

//! Actual guest create, independent identity/errno and scheduled child execution.

use super::*;

#[test]
fn native_pthread_create_runs_on_caller_stack_without_clearing_or_guarding_it() {
    let mut p = Program::new();
    p.set(8, 0);
    p.ptr(7, 8);
    p.reg(6, 0xc000);
    p.reg(2, 3);
    p.reg(1, 0);
    p.call(0); // Fully mapped RW allocation, including both stack neighbors.
    p.expect(0);
    p.set(16, 0);
    p.ptr(7, 16);
    p.call(1);
    p.expect(0);
    p.ptr(7, 16);
    p.load(6, 8);
    p.code.extend([0x48, 0x83, 0xc6, 17]);
    p.reg(2, 0x4000 + 31);
    p.call(2); // Deliberately non-page-aligned byte bounds.
    p.expect(0);
    p.ptr(7, 16);
    p.reg(6, u64::MAX);
    p.call(3); // Supplied-stack guard size is ignored, including huge values.
    p.expect(0);
    for offset in [0_u32, 32, 0x4030] {
        p.load(11, 8);
        p.code.extend([0x49, 0xc7, 0x83]);
        p.code.extend(offset.to_le_bytes());
        p.code.extend(91_u32.to_le_bytes());
    }
    p.set(32, 0);
    p.ptr(7, 0);
    p.ptr(6, 16);
    p.path(2, 0xc00);
    p.ptr(1, 32);
    p.reg(8, 0);
    p.call(4);
    p.expect(0);
    p.load(0, 32);
    p.code.extend([0x48, 0x83, 0xf8, 42, 0x75, 0xf5]);
    for offset in [0_u32, 32, 0x4030] {
        p.load(11, 8);
        p.code.extend([0x49, 0x8b, 0x83]);
        p.code.extend(offset.to_le_bytes());
        p.expect(91);
    }
    let (mut code, calls) = p.finish(None);
    assert!(code.len() <= 0x400);
    code.resize(0x400, 0xcc);
    code.extend([0x48, 0xc7, 0x07, 42, 0, 0, 0, 0xb8, 73, 0, 0, 0, 0xc3]);
    let main = image(
        &code,
        &[
            "IWIBBdTHit4",
            "nsYoNRywwNg",
            "Bvn74vj6oLo",
            "El+cQ20DynU",
            "6UgtwV+0zb4",
        ],
    );
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_pthread_create_executes_child_and_restores_parent_context() {
    // Self, errno, thread ID, create.
    let imports = ["aI+OeCz8xrQ", "9BcDykPmo1I", "EI-5-jlq2dE", "6UgtwV+0zb4"];
    let mut p = Program::new();
    p.call(0);
    p.save(8);
    p.call(1);
    p.save(16);
    p.code.extend([0xc7, 0x00, 37, 0, 0, 0]);
    p.call(2);
    p.expect(1);
    p.set(0, 0);
    p.set(32, 0);
    p.ptr(7, 0);
    p.reg(6, 0);
    p.path(2, 0xb00);
    p.ptr(1, 32);
    p.reg(8, 0);
    p.call(3);
    p.expect(0);
    p.set(32, 1); // Release the child after the scheduler resumes this parent.
    p.load(0, 32);
    p.code.extend([0x48, 0x83, 0xf8, 42, 0x75, 0xf5]); // wait for child result
    p.word(32, 42);
    p.word(48, 2);
    p.load(0, 0);
    p.load(10, 40);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(0);
    p.load(10, 8);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.call(2);
    p.expect(1);
    p.call(1);
    p.load(10, 16);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x8b, 0x00]);
    p.expect(37);
    p.load(0, 56);
    p.code.extend([0x8b, 0x00]);
    p.expect(19);
    let (mut code, calls) = p.finish(None);
    assert!(code.len() <= 0x300);
    code.resize(0x300, 0xcc);
    let mut child = vec![0x53, 0x48, 0x89, 0xfb]; // preserve RBX; RBX=argument
    child.extend([0x48, 0x83, 0x3b, 1, 0x75, 0xfa]); // wait for parent release
    for (slot, offset) in [(0, 8), (2, 16), (1, 24)] {
        rip(&mut child, 0xb00, &[0xff, 0x15], 0xf00 + slot * 8);
        child.extend([0x48, 0x89, 0x43, offset]);
    }
    child.extend([0xc7, 0x00, 19, 0, 0, 0]); // child's errno
    child.extend([0x48, 0xc7, 0x03, 42, 0, 0, 0]); // shared result
    child.extend([0xb8, 73, 0, 0, 0, 0x5b, 0xc3]);
    code.extend(child);
    let main = image(&code, &imports);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], (calls + 3).to_string());
    assert_eq!(fields["phase"], "main-entry");
    for (library, nid, ignored_name) in [
        ("Posix", "OxhIB8LB-PQ", true),
        ("libScePosix", "Jmi+9w9u0E4", false),
    ] {
        let mut variant = code.clone();
        if ignored_name {
            // The four-argument function must not inspect an unspecified R8.
            let at = variant
                .windows(10)
                .position(|bytes| bytes == [0x49, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap();
            variant[at + 2..at + 10].copy_from_slice(&u64::MAX.to_le_bytes());
        }
        let main = posix::scoped_image_with_library(
            &variant,
            &[
                (imports[0], false),
                (imports[1], false),
                (imports[2], false),
                (nid, true),
            ],
            1,
            library,
        );
        let (status, fields, text) = probe(&main);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], (calls + 3).to_string());
    }
}

#[test]
fn native_pthread_create_rejects_null_output_without_starting_child() {
    let mut p = Program::new();
    for register in [7, 6, 2, 1, 8] {
        p.reg(register, 0);
    }
    p.call(0);
    p.expect(I32_INVALID);
    let (code, _) = p.finish(None);
    let main = image(&code, &["6UgtwV+0zb4"]);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    for nid in ["OxhIB8LB-PQ", "Jmi+9w9u0E4"] {
        let mut p = Program::new();
        p.reg(7, 0);
        p.call(0);
        p.expect(22);
        let (code, _) = p.finish(None);
        let main = posix::scoped_image_with_library(&code, &[(nid, true)], 1, "libScePosix");
        let (status, fields, text) = probe(&main);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
    }
}
