// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_rwlock_lifecycle_retains_reader_and_writer_state_and_guest_prefix() {
    let mut p = Program::new();
    p.set(0, 0);
    p.ptr(7, 0);
    p.reg(6, u64::MAX); // Pinned source ignores the attribute argument.
    p.reg(2, 0);
    p.call(0);
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x83, 0x8b, 0xfc, 0, 0, 0, 0x42]);
    for slot in [1, 2] {
        // Writer can also take a recursive reader.
        p.ptr(7, 0);
        p.call(slot);
        p.expect(0);
    }
    p.ptr(7, 0);
    p.call(3);
    p.expect(0); // Release reader first.
    p.ptr(7, 0);
    p.call(4);
    p.expect(I32_BUSY);
    p.ptr(7, 0);
    p.call(1);
    p.expect(0x8002_000b); // Writer still held.
    p.ptr(7, 0);
    p.call(3);
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x8b, 0x83, 0xfc, 0, 0, 0]);
    p.expect(0x42);
    p.ptr(7, 0);
    p.call(4);
    p.expect(0);
    p.word(0, 0);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "6ULAa0fq4jA",
            "mqdNorrB+gI",
            "Ox9i0c7L5w0",
            "+L98PIbGttk",
            "BB+kb08Tl9A",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
}

fn rwlock_quantum(p: &mut Program) {
    p.reg(12, 3000);
    let begin = p.code.len();
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]);
    p.code
        .extend((begin as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
}
fn rwlock_wait_word(p: &mut Program, offset: u8) {
    let begin = p.code.len();
    p.load(0, offset);
    p.code.extend([0x48, 0x83, 0xf8, 1, 0x0f, 0x85]);
    p.code
        .extend((begin as i32 - (p.code.len() as i32 + 4)).to_le_bytes());
}
fn rwlock_child(entry: i32, writer: bool) -> (Vec<u8>, u64) {
    // Preserve RBX and retain the actual main-stack shared argument in it.
    let mut code = vec![0x53, 0x48, 0x89, 0xfb];
    let mut branches = Vec::new();
    let mut calls = 0;
    let mut call = |code: &mut Vec<u8>, slot| {
        code.extend([0x48, 0x89, 0xdf]); // RDI=&shared.rwlock.
        rip(code, entry, &[0xff, 0x15], 0xf00 + slot * 8);
        calls += 1;
    };
    let expect = |code: &mut Vec<u8>, branches: &mut Vec<usize>, value: u64| {
        code.extend([0x49, 0xba]);
        code.extend(value.to_le_bytes());
        code.extend([0x4c, 0x39, 0xd0]);
        checked(code, branches);
    };
    if writer {
        code.extend([0x31, 0xf6]); // Relative zero timeout, genuinely contended.
        call(&mut code, 5);
        expect(&mut code, &mut branches, 0x8002_003c);
        code.extend([0x48, 0xc7, 0x43, 24, 1, 0, 0, 0]);
        call(&mut code, 2); // Must park until BOTH actual readers release.
        expect(&mut code, &mut branches, 0);
        call(&mut code, 3);
        expect(&mut code, &mut branches, 0);
        code.extend([0x48, 0xc7, 0x43, 32, 1, 0, 0, 0]);
    } else {
        call(&mut code, 1); // A distinct concurrent reader.
        expect(&mut code, &mut branches, 0);
        code.extend([0x48, 0xc7, 0x43, 8, 1, 0, 0, 0]);
        let begin = code.len();
        code.extend([0x48, 0x83, 0x7b, 16, 1, 0x0f, 0x85]);
        code.extend((begin as i32 - (code.len() as i32 + 4)).to_le_bytes());
        call(&mut code, 3);
        expect(&mut code, &mut branches, 0);
        code.extend([0x48, 0xc7, 0x43, 40, 1, 0, 0, 0]);
    }
    code.extend([0x31, 0xc0, 0x5b, 0xc3]);
    finish_checks(&mut code, branches);
    assert!(code.len() < 0x200);
    (code, calls)
}

#[test]
fn native_rwlock_concurrent_readers_writer_wait_and_scoped_aliases_complete_real_threads() {
    for positive_write in [false, true] {
        let mut p = Program::new();
        for offset in (0..64).step_by(8) {
            p.set(offset, 0);
        }
        p.ptr(7, 0);
        p.call(1); // Lazy main reader.
        p.expect(0);
        p.ptr(7, 0);
        p.reg(6, 0);
        p.call(7); // Available timed-read zero succeeds, recursively.
        p.expect(0);
        p.ptr(7, 0);
        p.call(3);
        p.expect(0);
        p.ptr(7, 0);
        p.call(2);
        p.expect(if positive_write { 11 } else { 0x8002_000b });
        for (entry, slot, flag) in [(0x1100, 48, 8), (0x1400, 56, 24)] {
            p.ptr(7, slot);
            p.reg(6, 0);
            p.path(2, entry);
            p.ptr(1, 0);
            p.reg(8, 0);
            p.call(0);
            p.expect(0);
            rwlock_wait_word(&mut p, flag);
        }
        rwlock_quantum(&mut p); // Give the writer its real contended syscall.
        p.ptr(7, 0);
        p.call(4); // Existing reader may recurse behind the waiting writer.
        p.expect(0);
        for _ in 0..2 {
            p.ptr(7, 0);
            p.call(3);
            p.expect(0);
        }
        p.ptr(7, 0);
        p.call(4); // Now a NEW reader: queued-writer preference is real.
        p.expect(I32_BUSY); // Posix-scope try alias retains kernel error domain.
        rwlock_quantum(&mut p);
        p.word(32, 0); // Remaining child reader still prevents writer acquisition.
        p.set(16, 1);
        rwlock_wait_word(&mut p, 40);
        rwlock_wait_word(&mut p, 32);
        rwlock_quantum(&mut p); // Retire both real child returns after their final stores.
        p.ptr(7, 0);
        p.call(6);
        p.expect(0);
        p.word(0, 0);
        let (main, main_calls) = p.finish(None);
        let (reader, reader_calls) = rwlock_child(0x1100, false);
        let (writer, writer_calls) = rwlock_child(0x1400, true);
        let imports = [
            ("6UgtwV+0zb4", false),
            (
                if positive_write {
                    "iGjsr1WAtI0"
                } else {
                    "Ox9i0c7L5w0"
                },
                false,
            ),
            ("sIlRvQqsN2Y", positive_write),
            (
                if positive_write {
                    "EgmLo6EWgso"
                } else {
                    "+L98PIbGttk"
                },
                false,
            ),
            ("SFxTMOfuCkE", true),
            ("adh--6nIqTk", false),
            ("BB+kb08Tl9A", false),
            ("iPtZRWICjrM", false),
        ];
        assert_eq!(imports.len(), 8);
        let original = posix::scoped_image(&main, &imports, 1);
        let mut bytes = original.bytes.clone();
        bytes.resize(0x1800, 0);
        put(&mut bytes, 96, &0x1800_u64.to_le_bytes());
        put(&mut bytes, 0x800, &main); // Restore code over unused helper path labels.
        put(&mut bytes, 0x1100, &reader);
        put(&mut bytes, 0x1400, &writer);
        let source = create_input(bytes);
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            source.path.clone().into(),
            "--execution-profile".into(),
            "startup".into(),
        ]);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["scheduling_threads"], "3");
        assert_eq!(fields["scheduling_returned"], "2");
        assert_eq!(
            fields["service_stops"],
            (main_calls + reader_calls + writer_calls).to_string()
        );
        assert_eq!(fields["unavailable_functions"], "0");
        assert_eq!(
            fields["kernel_handles_live"],
            fields["kernel_handles_accounted"]
        );
    }
    println!(
        "native_rwlock_wait_available=true native_rwlock_wait_programs=2 native_rwlock_wait_threads=3"
    );
}
