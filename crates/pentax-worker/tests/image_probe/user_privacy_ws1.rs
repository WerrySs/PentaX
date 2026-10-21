// SPDX-License-Identifier: GPL-2.0-only
//! Original two-argument privacy import consumers, without user initialization.

use super::*;

const NID: &str = "D-CzAxQL0XI";
const MODULE: &str = "libSceUserService";
const LIBRARY: &str = "libSceUserServicePlatformPrivacyWs1";
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const SAVED: u64 = 0x6a95_713c_d842_b0ef;
const NULL_OUTPUT: u64 = 0x8096_0005;

fn image(code: &[u8]) -> Input {
    // One undefined GLOBAL FUNC and its real JUMP_SLOT use this exact special
    // module1.1/library1 pair. No general UserService alias is substituted.
    scoped_system_image(code, &[NID], MODULE, LIBRARY, 1)
}

fn fill(p: &mut Program) {
    p.reg(0, CANARY);
    for at in [0, 8, 16] {
        p.save(at);
    }
}

fn unused_arguments(p: &mut Program) {
    // The source has two arguments. Poison the other SysV argument registers;
    // RCX/R11 remain caller-clobbered and are never relied on after the call.
    for register in [2, 1, 8, 9, 11] {
        p.reg(register, u64::MAX);
    }
}

fn caller_context(p: &mut Program) {
    // Compare the actual post-call RSP and nonvolatile R12. Reload volatile
    // scratch registers before use rather than assuming a stronger public ABI.
    p.code.extend([0x48, 0x89, 0xe0]); // MOV RAX,RSP.
    p.load(10, 24);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]); // MOV RAX,R12.
    p.expect(SAVED);
}

#[test]
fn native_privacy_ws1_ignores_user_ids_and_writes_only_four_bytes_before_normal_return() {
    let mut p = Program::new();
    p.stack(0x89, 4, 24); // Save this real caller RSP inside its own frame.
    p.stack(0x89, 12, 32);
    p.reg(12, SAVED);
    // ID1 and signed -1 need neither login nor initialization. The third ID
    // also tests ignored upper32 bits, with an unaligned but valid int output.
    for (user, output, expected) in [
        (1, 8, 0xa5a5_a5a5_0000_0000),
        (u64::MAX, 8, 0xa5a5_a5a5_0000_0000),
        (0xa1b2_c3d4_0000_0001, 10, 0xa5a5_0000_0000_a5a5),
    ] {
        fill(&mut p);
        p.reg(7, user);
        p.ptr(6, output);
        unused_arguments(&mut p);
        p.call(0);
        p.expect(0);
        p.word(0, CANARY);
        p.word(8, expected);
        p.word(16, CANARY);
        caller_context(&mut p);
    }
    fill(&mut p);
    p.reg(7, u64::MAX);
    p.reg(6, 0);
    unused_arguments(&mut p);
    p.call(0);
    // Exact zero-extended SCE int32 domain, not a sign-extended host errno.
    p.expect(NULL_OUTPUT);
    for at in [0, 8, 16] {
        p.word(at, CANARY);
    }
    caller_context(&mut p);
    p.load(12, 32); // Preserve this program's own incoming nonvolatile register.
    let (code, calls) = p.finish(None);
    assert_eq!(calls, 4);
    let input = image(&code);
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["code"], "outer-return", "{text}");
    assert_eq!(fields["service_stops"], "4", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["phase"], "main-entry", "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
    println!(
        "user_privacy_ws1_native_available=true user_privacy_ws1_native_programs=1 user_privacy_ws1_native_calls=4"
    );
}

#[test]
fn native_privacy_ws1_nonnull_unmapped_and_readonly_outputs_keep_typed_guest_failures() {
    for (readonly, expected) in [(false, "guest-unmapped"), (true, "guest-write-denied")] {
        let mut p = Program::new();
        p.reg(7, 1);
        if readonly {
            // This actual initialized RX instruction span belongs to the same
            // image owner. An output cannot gain data-write authority from it.
            p.path(6, 0x800);
        } else {
            p.reg(6, 1); // Nonnull, with no mapped guest backing in this image.
        }
        unused_arguments(&mut p);
        p.call(0);
        // Reaching this consumer would fabricate a normal service result.
        p.expect(0);
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 1);
        let input = image(&code);
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], expected, "{text}");
        assert_eq!(
            fields["public_function"], "sceUserServiceGetPlatformPrivacyWs1",
            "{text}"
        );
        assert_eq!(fields["service_stops"], "1", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
    println!("user_privacy_ws1_native_refusal_programs=2");
}
