// SPDX-License-Identifier: GPL-2.0-only
//! Original constructors and retained aliases using only existing exact scopes.
use super::*;

const CASES: [(&str, [&str; 2], &str); 5] = [
    (
        "libkernel",
        ["F8bUHwAG284", "smWEktiyyG0"],
        "PthreadMutexattrInit",
    ),
    (
        "Posix",
        ["dQHWEsJtoE4", "HF7lK46xzjY"],
        "pthread_mutexattr_init",
    ),
    (
        "libkernel",
        ["m5-2bsNfv7s", "waPcxYiR3WA"],
        "PthreadCondattrInit",
    ),
    (
        "libkernel",
        ["mKoTx03HRWA", "dJcuQVn6-Iw"],
        "pthread_condattr_init",
    ),
    (
        "libkernel",
        ["yOfGg-I1ZII", "i2ifZ3fS2fo"],
        "PthreadRwlockattrInit",
    ),
];

#[test]
fn native_attribute_family_overwrites_arbitrary_and_live_outputs_without_retiring_copied_owners() {
    for (library, imports, _) in CASES {
        let mut p = Program::new();
        p.reg(0, u64::MAX);
        p.save(0);
        p.ptr(7, 0);
        p.call(0);
        p.expect(0);
        p.load(0, 0);
        p.save(8);
        p.load(11, 8);
        p.code.extend([0x41, 0x83, 0x4b, 0x38, 0x42]);
        p.ptr(7, 0);
        p.call(0);
        p.expect(0);
        p.load(11, 0);
        p.code.extend([0x41, 0x8b, 0x43, 0x38]);
        p.expect(0); // A fresh, zeroed owned prefix replaces the output.
        p.load(11, 8);
        p.code.extend([0x41, 0x8b, 0x43, 0x38]);
        p.expect(0x42); // Actual old backing remains independently usable.
        for slot in [0, 8] {
            p.ptr(7, slot);
            p.call(1);
            p.expect(0);
            p.word(slot, 0);
        }
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 4);
        let source = scoped_system_image(&code, &imports, "libkernel", library, 1);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["code"], "outer-return", "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], "4", "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
    }
}

#[test]
fn native_attribute_family_output_faults_keep_existing_owner_and_refuse_new_publication() {
    // The corresponding POSIX success cases above use the same transaction.
    for (library, imports, public_name) in [CASES[0], CASES[2], CASES[4]] {
        let mut p = Program::new();
        p.reg(0, u64::MAX);
        p.save(0);
        p.ptr(7, 0);
        p.call(0);
        p.expect(0);
        p.path(7, 0x800); // Real initialized RX bytes refuse the output store.
        p.call(0);
        p.expect(0);
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 2);
        let source = scoped_system_image(&code, &imports, "libkernel", library, 1);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], "guest-write-denied", "{text}");
        assert_eq!(fields["public_function"], public_name, "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        assert_eq!(fields["kernel_handles_live"], "1", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "1", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}
