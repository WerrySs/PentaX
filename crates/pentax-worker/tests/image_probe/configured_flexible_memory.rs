// SPDX-License-Identifier: GPL-2.0-only
//! Original configured-capacity consumers; no host-memory or availability claim.

use super::*;

const IMPORTS: [&str; 3] = [
    "n1-v6FgU7MQ", // KernelConfiguredFlexibleMemorySize(size_t*).
    "IWIBBdTHit4", // KernelMapFlexibleMemory(output,len,prot,flags).
    "cQke9UuBQOk", // KernelMunmap(address,len).
];
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const SAVED: u64 = 0x6a95_713c_d842_b0ef;
const PAGE: u64 = 0x4000;

fn image(code: &[u8]) -> Input {
    // Exact canonical module1.1/library1, three GLOBAL FUNC/JUMP_SLOT imports.
    scoped_system_image(code, &IMPORTS, "libkernel", "libkernel", 1)
}

fn fill(p: &mut Program) {
    p.reg(0, CANARY);
    for at in [0, 8, 16] {
        p.save(at);
    }
}

fn unused_arguments(p: &mut Program) {
    for register in [6, 2, 1, 8, 9, 11] {
        p.reg(register, u64::MAX);
    }
}

fn caller_context(p: &mut Program) {
    // Only the actual caller RSP and saved nonvolatile R12 survive by contract;
    // volatile scratch registers are reloaded before the post-call consumer.
    p.code.extend([0x48, 0x89, 0xe0]); // MOV RAX,RSP.
    p.load(10, 40);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]); // MOV RAX,R12.
    p.expect(SAVED);
}

fn query(p: &mut Program, offset: u8, first: u64, second: u64) {
    fill(p);
    p.ptr(7, offset);
    unused_arguments(p);
    p.call(0);
    p.expect(0);
    p.word(0, CANARY);
    p.word(8, first);
    p.word(16, second);
    caller_context(p);
}

fn consumer(configured: u64, unaligned_first: u64) -> (Vec<u8>, u64) {
    let mut p = Program::new();
    p.stack(0x89, 4, 40);
    p.stack(0x89, 12, 48);
    p.reg(12, SAVED);
    // Every output byte begins nonzero: a four-byte write would leave the high
    // half incorrect. The value is configured capacity, not free/backed bytes.
    query(&mut p, 8, configured, CANARY);

    p.set(64, 0);
    p.ptr(7, 64);
    p.reg(6, PAGE);
    p.reg(2, 3); // Real CPU read/write mapping.
    p.reg(1, 0);
    for register in [8, 9] {
        p.reg(register, u64::MAX);
    }
    p.call(1);
    p.expect(0);
    caller_context(&mut p);
    p.load(7, 64);
    p.reg(0, CANARY);
    p.code.extend([0x48, 0x89, 0x07, 0x48, 0x8b, 0x07]);
    p.expect(CANARY); // Actual guest write/read of the returned mapping.

    // The eight-byte unaligned output crosses the two seeded qwords. Exactly
    // its two high bytes replace the low two bytes of the following canary.
    query(&mut p, 10, unaligned_first, 0xa5a5_a5a5_a5a5_0000);
    p.load(7, 64);
    p.reg(6, PAGE);
    p.call(2);
    p.expect(0);
    caller_context(&mut p);
    query(&mut p, 8, configured, CANARY);

    fill(&mut p);
    p.reg(7, 0);
    unused_arguments(&mut p);
    p.call(0);
    p.expect(0x8002_0016); // Normal zero-extended SCE int32 EINVAL.
    for at in [0, 8, 16] {
        p.word(at, CANARY);
    }
    caller_context(&mut p);
    p.load(12, 48);
    p.finish(None)
}

#[test]
fn native_configured_flexible_capacity_is_eight_bytes_and_stable_across_real_map_unmap() {
    for (runtime, configured, unaligned) in [
        (false, 0x0400_0000, 0x0000_0400_0000_a5a5), // Compact:64MiB.
        (true, 0x4000_0000, 0x0000_4000_0000_a5a5),  // Runtime:1GiB.
    ] {
        let (code, calls) = consumer(configured, unaligned);
        assert_eq!(calls, 6); // Four getters, one map and one unmap.
        let input = image(&code);
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
        assert_eq!(fields["service_stops"], "6", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
    }
    println!(
        "configured_flexible_memory_native_available=true configured_flexible_memory_native_programs=2 configured_flexible_memory_getter_calls=8"
    );
}

#[test]
fn native_configured_flexible_capacity_keeps_nonnull_backing_faults_typed() {
    for (readonly, expected) in [(false, "guest-unmapped"), (true, "guest-write-denied")] {
        let mut p = Program::new();
        if readonly {
            p.path(7, 0x800); // Actual initialized RX source, not an output.
        } else {
            p.reg(7, 1); // Nonnull address with no backing in this image.
        }
        unused_arguments(&mut p);
        p.call(0);
        p.expect(0); // Never reached on the actual typed guest failure.
        let (code, calls) = p.finish(None);
        assert_eq!(calls, 1);
        let input = image(&code);
        let (status, fields, text) = probe(&input);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], expected, "{text}");
        assert_eq!(
            fields["public_function"], "KernelConfiguredFlexibleMemorySize",
            "{text}"
        );
        assert_eq!(fields["service_stops"], "1", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
    }
    println!("configured_flexible_memory_native_refusal_programs=2");
}
