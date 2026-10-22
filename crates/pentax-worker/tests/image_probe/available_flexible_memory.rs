// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers of real remaining capacity and mapping splits.
use super::*;

const PAGE: u64 = 0x4000;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn query(p: &mut Program, available: u64) {
    p.reg(0, CANARY);
    for offset in [0, 8, 16] {
        p.save(offset);
    }
    p.ptr(7, 9); // Unaligned full-width output between two untouched canaries.
    for register in [6, 2, 1, 8, 9, 11] {
        p.reg(register, u64::MAX);
    }
    p.call(0);
    p.expect(0);
    p.word(0, CANARY);
    p.word(8, (available << 8) | 0xa5);
    p.word(16, CANARY & !0xff);
}

fn consumer(capacity: u64) -> (Vec<u8>, u64) {
    let mut p = Program::new();
    query(&mut p, capacity);
    p.set(64, 0);
    p.ptr(7, 64);
    p.reg(6, 2 * PAGE);
    p.reg(2, 3);
    p.reg(1, 0);
    p.call(1);
    p.expect(0);
    p.load(7, 64);
    p.reg(0, CANARY);
    p.code.extend([0x48, 0x89, 0x07, 0x48, 0x8b, 0x07]);
    p.expect(CANARY); // Consume the actual returned guest mapping.
    p.load(7, 64);
    p.reg(6, PAGE);
    p.reg(2, 1);
    p.call(2); // Protection splits must retain flexible-pool membership.
    p.expect(0);
    query(&mut p, capacity - 2 * PAGE);
    p.load(7, 64);
    p.reg(6, PAGE);
    p.call(3);
    p.expect(0);
    query(&mut p, capacity - PAGE);
    p.load(7, 64);
    p.code.extend([0x48, 0x81, 0xc7, 0, 0x40, 0, 0]); // ADD RDI,PAGE.
    p.reg(6, PAGE);
    p.call(3);
    p.expect(0);
    query(&mut p, capacity);
    p.finish(None)
}

#[test]
fn native_available_pool_changes_on_real_map_and_subrange_unmap_after_protection_split() {
    for (runtime, capacity) in [(false, 0x0400_0000), (true, 0x4000_0000)] {
        let (code, calls) = consumer(capacity);
        assert_eq!(calls, 8);
        let input = scoped_system_image(
            &code,
            &["aNz11fnnzi4", "IWIBBdTHit4", "vSMAm3cxYTY", "cQke9UuBQOk"],
            "libkernel",
            "libkernel",
            1,
        );
        let (status, fields, text) = if runtime {
            probe_arguments(&[
                "--image".into(),
                input.path.as_os_str().to_owned(),
                "--memory-profile".into(),
                "runtime".into(),
            ])
        } else {
            probe(&input)
        };
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["code"], "outer-return", "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], "8", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
    }
}
