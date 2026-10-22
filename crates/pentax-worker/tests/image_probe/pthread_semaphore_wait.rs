// SPDX-License-Identifier: GPL-2.0-only
//! Original real pthread park/post/resume programs, without host timing waits.
use super::*;

const CHILD: i32 = 0x1100;
fn imports(direct: bool) -> [(&'static str, bool); 8] {
    let sem = if direct {
        [
            "GEnUkDZoUwY",
            "C36iRE0F5sE",
            "aishVAiFaYM",
            "H2a+IN9TP0E",
            "Vwc+L05e6oE",
        ]
    } else {
        [
            "pDuPEf3m4fI",
            "YCV5dGGBcCo",
            "IKP8typ0QUk",
            "WBWzsRifCEA",
            "cDW233RAwWo",
        ]
    };
    [
        (sem[0], !direct),
        (sem[1], !direct),
        (sem[2], !direct),
        (sem[3], !direct),
        (sem[4], !direct),
        ("6UgtwV+0zb4", false),
        ("onNY9Byn-W8", false),
        ("9BcDykPmo1I", false),
    ]
}
fn input(main: &[u8], child: &[u8], direct: bool, library: &str) -> Input {
    let imports = imports(direct);
    let original = if direct {
        image(&[], &imports.map(|(nid, _)| nid))
    } else {
        posix::scoped_image_with_library(&[], &imports, 1, library)
    };
    let mut bytes = original.bytes.clone();
    bytes.resize(0x1800, 0);
    put(&mut bytes, 0x800, main);
    put(&mut bytes, CHILD as usize, child);
    put(&mut bytes, 96, &0x1800_u64.to_le_bytes());
    create_input(bytes)
}
fn child_call(p: &mut Program, slot: i32) {
    rip(&mut p.code, CHILD, &[0xff, 0x15], 0xf00 + slot * 8);
    p.calls += 1;
}
fn vector(p: &mut Program, value: u64, check: bool) {
    if check {
        p.code.extend([0x66, 0x48, 0x0f, 0x7e, 0xc0]);
        p.expect(value);
    } else {
        p.reg(0, value);
        p.code.extend([0x66, 0x48, 0x0f, 0x6e, 0xc0]);
    }
}
fn context(p: &mut Program, value: u64, check: bool) {
    if check {
        p.reg(10, value);
        p.code.extend([0x4d, 0x39, 0xd5]); // CMP R13,R10
        checked(&mut p.code, &mut p.branches);
    } else {
        p.reg(13, value);
    }
    vector(p, value * 0x0001_0001_0001_0001, check);
    if !check {
        p.reg(0, value);
    }
    p.code
        .extend([0x64, 0x48, if check { 0x8b } else { 0x89 }, 4, 0x25]);
    p.code.extend(0x28_u32.to_le_bytes());
    if check {
        p.expect(value);
    }
}

#[test]
fn native_indefinite_sem_wait_resumes_only_after_post_with_thread_errno_tls_and_context_intact() {
    for (direct, library) in [
        (false, "Posix"),
        (false, "libScePosix"),
        (true, "libkernel"),
    ] {
        let flags = if direct { 0_u64 } else { 0x1234_5678_u64 };
        let mut main = Program::new();
        main.call(7);
        main.save(0);
        main.set_errno(37);
        main.ptr(7, 32);
        main.reg(6, flags);
        main.reg(2, 0);
        main.reg(1, 0);
        main.call(0);
        main.expect(0);
        main.set(48, 0);
        context(&mut main, 0x1111, false);
        main.ptr(7, 8);
        main.reg(6, 0);
        main.path(2, CHILD);
        main.ptr(1, 32);
        main.reg(8, 0);
        main.call(5);
        main.expect(0);
        // Child has actually parked inside sem_wait; its next guest write
        // remains unexecuted while the parent observes the waiter prefix.
        main.word(48, 0);
        main.word(32, 0x0000_0001_0000_09fa);
        main.word(40, flags << 32);
        main.errno(37);
        context(&mut main, 0x1111, true);
        main.load(0, 56);
        main.load(10, 0);
        main.code.extend([0x4c, 0x39, 0xd0, 0x0f, 0x84, 0, 0, 0, 0]);
        main.branches.push(main.code.len() - 4);
        main.ptr(7, 32);
        main.call(3);
        main.expect(if direct {
            0x8002_0023
        } else {
            u64::from(u32::MAX)
        });
        main.errno(if direct { 37 } else { 35 });
        main.word(32, 0x0000_0001_0000_09fa);
        main.ptr(7, 32);
        main.call(2);
        main.expect(0);
        main.word(32, 0x0000_0001_0000_09fa);
        main.word(40, (flags << 32) | 1);
        main.load(7, 8);
        main.ptr(6, 16);
        main.call(6);
        main.expect(0);
        main.word(16, 42);
        main.word(48, 3);
        main.word(32, 0x09fa);
        main.word(40, flags << 32);
        main.errno(if direct { 37 } else { 35 });
        context(&mut main, 0x1111, true);
        main.ptr(7, 32);
        main.call(4);
        main.expect(0);
        let (main, calls) = main.finish(None);
        let mut child = Program::new();
        child.code.extend([0x48, 0x89, 0xfb]); // RBX is the actual parent's semaphore.
        child_call(&mut child, 7);
        child.save(0);
        child.set_errno(19);
        child.code.extend([0x48, 0x89, 0x43, 24]);
        context(&mut child, 0x2222, false);
        child.code.extend([0x48, 0x89, 0xdf]);
        for register in [6, 2, 1, 8, 9] {
            child.reg(register, u64::MAX);
        }
        child_call(&mut child, 1);
        child.expect(0);
        child.errno(19);
        context(&mut child, 0x2222, true);
        child.code.extend([0x48, 0xc7, 0x43, 16, 3, 0, 0, 0]);
        let (child, child_calls) = child.finish(None);
        let (status, fields, text) = probe(&input(&main, &child, direct, library));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(
            fields["service_stops"],
            (calls + child_calls).to_string(),
            "{text}"
        );
        assert_eq!(fields["scheduling_threads"], "2", "{text}");
        assert_eq!(fields["scheduling_returned"], "1", "{text}");
        assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        assert!(
            fields["native_instructions"].parse::<u64>().unwrap() > 0,
            "{text}"
        );
    }
    println!(
        "pthread_semaphore_wait_native_available=true pthread_semaphore_wait_native_programs=3 pthread_semaphore_wait_native_returns=3"
    );
}

#[test]
fn native_indefinite_sem_wait_without_any_notifier_keeps_the_existing_deadlock_stop() {
    for (direct, library) in [
        (false, "Posix"),
        (false, "libScePosix"),
        (true, "libkernel"),
    ] {
        let mut p = Program::new();
        p.ptr(7, 32);
        p.reg(6, 0);
        p.reg(2, 0);
        p.reg(1, 0);
        p.call(0);
        p.expect(0);
        p.ptr(7, 32);
        p.call(1);
        p.expect(0); // Must remain pending, never reach success.
        let (main, calls) = p.finish(None);
        let (status, fields, text) = probe(&input(&main, &[], direct, library));
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "thread-deadlock", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["scheduling_pending_waits"], "1", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
    println!(
        "pthread_semaphore_deadlock_native_available=true pthread_semaphore_deadlock_native_programs=3 pthread_semaphore_deadlock_native_pending=3"
    );
}
