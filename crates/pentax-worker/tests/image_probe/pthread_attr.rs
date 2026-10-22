// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_thread_attributes_roundtrip_full_stack_values_and_signed_priority() {
    let mut p = Program::new();
    p.set(0, 0);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    p.load(11, 0);
    p.code.extend([0x41, 0x83, 0x4b, 0x38, 0x42]); // Write the owned reserved prefix.
    p.ptr(7, 0);
    p.ptr(6, 16);
    p.ptr(2, 24);
    p.call(1);
    p.expect(0);
    p.word(16, 0);
    p.word(24, 0x100000);
    p.ptr(7, 0);
    p.reg(6, 0x2345_6789_abcd);
    p.reg(2, 0x1_0000_4000);
    p.call(2);
    p.expect(0);
    p.ptr(7, 0);
    p.ptr(6, 16);
    p.ptr(2, 24);
    p.call(1);
    p.expect(0);
    p.word(16, 0x2345_6789_abcd);
    p.word(24, 0x1_0000_4000);
    p.reg(0, (-731_i64) as u64);
    p.save(32);
    p.ptr(7, 0);
    p.ptr(6, 32);
    p.call(3);
    p.expect(0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(32);
    p.ptr(7, 0);
    p.ptr(6, 32);
    p.call(4);
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0000 | u64::from((-731_i32) as u32));
    p.load(11, 0);
    p.code.extend([0x41, 0x8b, 0x43, 0x38]);
    p.expect(0x42);
    p.ptr(7, 0);
    p.call(5);
    p.expect(0);
    p.word(0, 0);
    let (code, calls) = p.finish(None);
    let source = image(
        &code,
        &[
            "nsYoNRywwNg",
            "-quPa4SEJUw",
            "Bvn74vj6oLo",
            "DzES9hQF4f4",
            "FXPWHNk8Of0",
            "62KCwEMmzcM",
        ],
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
}

fn replacement_image(code: &[u8], posix: bool) -> Input {
    let (library, nids) = if posix {
        (
            "Posix",
            [
                "wtkt-teR1so",
                "0qOtCR-ZHck",
                "2Q0z6rnBrTE",
                "zHchY8ft5pk",
                "dQHWEsJtoE4",
                "HF7lK46xzjY",
            ],
        )
    } else {
        (
            "libkernel",
            [
                "nsYoNRywwNg",
                "-fA+7ZlGDQs",
                "UTXzJbWhhTE",
                "62KCwEMmzcM",
                "F8bUHwAG284",
                "smWEktiyyG0",
            ],
        )
    };
    scoped_system_image(code, &nids, "libkernel", library, 1)
}

#[test]
fn native_kernel_and_exact_posix_attribute_init_overwrite_slots_and_preserve_old_owners() {
    for posix in [false, true] {
        let mut p = Program::new();
        p.reg(0, u64::MAX); // An output slot is not an input token.
        p.save(0);
        p.ptr(7, 0);
        p.call(0);
        p.expect(0);
        p.load(0, 0);
        p.save(8); // Keep a separately usable alias to the old attribute.
        p.ptr(7, 0);
        p.reg(6, 0x8000);
        p.call(2);
        p.expect(0);
        p.load(11, 8);
        p.code.extend([0x41, 0x83, 0x4b, 0x38, 0x42]);
        p.ptr(7, 0); // Overwrite an actual live attribute pointer.
        p.call(0);
        p.expect(0);
        for (slot, expected) in [(0, 0x100000), (8, 0x8000)] {
            p.ptr(7, slot);
            p.ptr(6, 32);
            p.call(1);
            p.expect(0);
            p.word(32, expected);
        }
        p.load(11, 8);
        p.code.extend([0x41, 0x8b, 0x43, 0x38]);
        p.expect(0x42); // Old real backing survives the destination overwrite.
        p.set(16, 0);
        p.ptr(7, 16);
        p.call(4);
        p.expect(0);
        p.load(0, 16);
        p.save(24);
        p.ptr(7, 16); // A live object of another kind is also just old output.
        p.call(0);
        p.expect(0);
        p.ptr(7, 16);
        p.ptr(6, 32);
        p.call(1);
        p.expect(0);
        p.word(32, 0x100000);
        p.ptr(7, 24);
        p.call(5); // The copied mutex-attribute alias still owns its object.
        p.expect(0);
        for slot in [0, 8, 16] {
            p.ptr(7, slot);
            p.call(3);
            p.expect(0);
            p.word(slot, 0);
        }
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 12);
        let source = replacement_image(&code, posix);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["code"], "outer-return", "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
    }
}

#[test]
fn native_attribute_init_output_refusal_keeps_existing_owner_and_typed_fault() {
    for posix in [false, true] {
        let mut p = Program::new();
        p.reg(0, u64::MAX);
        p.save(0);
        p.ptr(7, 0);
        p.call(0);
        p.expect(0);
        p.ptr(7, 0);
        p.reg(6, 0x8000);
        p.call(2);
        p.expect(0);
        p.path(7, 0x800); // The actual initialized RX code page refuses output.
        p.call(0);
        p.expect(0); // Unreachable after the typed transactional failure.
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 3);
        let source = replacement_image(&code, posix);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], "guest-write-denied", "{text}");
        assert_eq!(
            fields["public_function"],
            if posix {
                "pthread_attr_init"
            } else {
                "PthreadAttrInit"
            },
            "{text}"
        );
        assert_eq!(fields["service_stops"], "3", "{text}");
        assert_eq!(fields["kernel_handles_live"], "1", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "1", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
}
