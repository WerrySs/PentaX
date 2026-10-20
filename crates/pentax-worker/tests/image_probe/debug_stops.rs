// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_unsupported_stop_exposes_closed_encoding_and_policy_without_operand_payloads() {
    let mut p = Program::new();
    p.reg(0, 0x7359_1268_a4fe_c091);
    p.code.extend([0x67, 0x48, 0x8b, 0x00]); // A real address32 load remains refused.
    let (code, _) = p.finish(None);
    let original = image(&code, &[]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction");
    assert_eq!(fields["stopped_form"], "Mov_r64_rm64");
    assert_eq!(fields["stopped_reason"], "AddressSize");
    assert!(!text.contains("7359") && !text.contains("a4fe"));
    assert!(!fields.contains_key("guest_pc") && !fields.contains_key("address"));
    assert!(!fields.contains_key("value"));
}

#[test]
fn native_permission_stop_reports_actual_rights_and_source_role_without_addresses() {
    let original = input(
        &[
            0x48, 0x8d, 0x05, 0x19, 0, 0, 0, // LEA RAX, original source offset0x120.
            0xc5, 0xf9, 0x7f, 0x00, // VMOVDQA [RAX], XMM0, an RX source page.
            0xc3,
        ],
        false,
    );
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["runtime_failure"], "memory-permission");
    assert_eq!(fields["planned_form"], "VEX_Vmovdqa_xmmm128_xmm");
    assert_eq!(fields["planned_memory"], "destination");
    assert_eq!(fields["dispatch_cpu_access"], "read-execute");
    assert_eq!(fields["dispatch_gpu_access"], "none");
    assert_eq!(fields["dispatch_origin"], "primary-image");
    assert_eq!(fields["dispatch_body_ran"], "1");
    assert!(!fields.contains_key("value"));
    assert!(!fields.contains_key("address"));
}

#[test]
fn native_dispatch_failure_exposes_closed_class_and_planned_exit_without_raw_payloads() {
    let mut p = Program::new();
    p.code.extend([0xc5, 0xf8, 0x28, 0x44, 0x24, 1]); // Original misaligned VMOVAPS.
    let (code, _) = p.finish(None);
    let original = image(&code, &[]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "native-dispatch");
    assert_eq!(fields["runtime_failure"], "vector-alignment");
    assert_eq!(fields["planned_exit"], "Vmovaps");
    assert!(!fields.contains_key("value"));
    assert!(!fields.contains_key("guest_pc"));
}

#[test]
fn debug_stop_reports_actual_fixed_memory_refusal_without_reflecting_addresses() {
    let mut p = Program::new();
    p.set(0, 0);
    p.ptr(7, 0);
    p.reg(6, 16384);
    p.reg(2, 3);
    p.reg(1, 0);
    p.call(0);
    p.expect(0);
    // A fixed request one GiB beyond this mapping exceeds the compact fixture
    // window. Exercise a genuine kernel refusal, then stop in original code.
    p.load(0, 0);
    p.code.extend([0x48, 0x05, 0, 0, 0, 0x40]);
    p.save(0);
    p.ptr(7, 0);
    p.reg(6, 16384);
    p.reg(2, 3);
    p.reg(1, 0x10);
    p.call(0);
    p.expect(0x8002_000c);
    p.code.extend([0xcd, 0x41]);
    let (code, _) = p.finish(None);
    let original = image(&code, &["IWIBBdTHit4"]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["instruction"], "Int");
    assert_eq!(fields["trap_kind"], "debug-break");
    assert_eq!(fields["trap_origin"], "primary");
    assert_eq!(fields["memory_calls"], "2");
    assert_eq!(fields["memory_errors"], "1");
    assert_eq!(fields["memory_last_operation"], "KernelMapFlexibleMemory");
    assert_eq!(
        fields["memory_last_error_operation"],
        "KernelMapFlexibleMemory"
    );
    assert_eq!(fields["memory_last_bytes"], "16384");
    assert_eq!(fields["memory_last_fixed"], "1");
    assert_eq!(fields["memory_last_error"], "out-of-memory");
    assert_eq!(fields["memory_map_refusal"], "virtual-window");
    assert!(fields["memory_mapped_bytes"].parse::<u64>().unwrap() >= 16384);
    assert!(fields["memory_handles"].parse::<usize>().unwrap() > 0);
    assert!(!fields.contains_key("value"));
    assert!(!fields.contains_key("memory_last_address"));
}

#[test]
fn native_debug_interrupt_reports_closed_hints_without_executing_or_reflecting_output() {
    for (interrupt, expected, with_output) in [
        (0x41, "debug-break", true),
        (0x22, "software-interrupt", false),
    ] {
        let message = b"ASSERT FAILED: untrusted-text-239 memory buffer\n\tguest_pc=private-value";
        let mut p = Program::new();
        if with_output {
            p.reg(7, 1);
            p.path(6, NAME);
            p.reg(2, message.len() as u64);
            p.call(0);
            p.expect(message.len() as u64);
        }
        p.code.extend([0xcd, interrupt]);
        let (code, _) = p.finish(None);
        let mut source = image(&code, &["4wSze92BhLI"]);
        put(&mut source.bytes, NAME as usize, message);
        std::fs::write(&source.path, &source.bytes).unwrap();
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "unsupported-instruction");
        assert_eq!(fields["instruction"], "Int");
        assert_eq!(fields["trap_kind"], expected);
        assert_eq!(fields["trap_origin"], "primary");
        assert_eq!(
            fields["console_hints"],
            if with_output {
                "assertion,failure,memory,buffer"
            } else {
                "none"
            }
        );
        assert!(!text.contains("untrusted-text-239") && !text.contains("private-value"));
        assert!(!fields.contains_key("value"));
    }
}
