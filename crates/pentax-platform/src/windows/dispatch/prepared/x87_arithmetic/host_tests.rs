// SPDX-License-Identifier: GPL-2.0-only
//! Original capability refusals and complete defined host state restoration.
use super::*;
#[test]
fn x87_arithmetic_host_gate_requires_real_vendor_fxsr_and_amd_error_pointer_restoration() {
    for vendor in 0..3 {
        for maximum in [0, 1] {
            for fpu in [false, true] {
                for fxsr in [false, true] {
                    for extended in [0x8000_0000, 0x8000_0008] {
                        for pointers in [false, true] {
                            let actual = available_with(|leaf| match leaf {
                                0 => match vendor {
                                    1 => [maximum, 0x756e6547, 0x6c65746e, 0x49656e69],
                                    2 => [maximum, 0x68747541, 0x444d4163, 0x69746e65],
                                    _ => [maximum, 0, 0, 0],
                                },
                                1 => [0, 0, 0, u32::from(fpu) | (u32::from(fxsr) << 24)],
                                0x8000_0000 => [extended, 0, 0, 0],
                                0x8000_0008 => [0, u32::from(pointers) << 2, 0, 0],
                                _ => panic!("Unexpected capability leaf"),
                            });
                            assert_eq!(
                                actual,
                                maximum >= 1
                                    && fpu
                                    && fxsr
                                    && (vendor == 1
                                        || vendor == 2 && extended >= 0x8000_0008 && pointers)
                            );
                        }
                    }
                }
            }
        }
    }
}
#[test]
fn x87_arithmetic_closed_transaction_restores_nonzero_top_full_and_empty_host_values_and_environment()
 {
    if !available() {
        return;
    }
    #[repr(C, align(16))]
    struct Fixture {
        outer: [u8; 512],
        before: [u8; 512],
        after: [u8; 512],
        seed: [[u8; 16]; 8],
        control: [u8; 16],
        mxcsr: [u8; 16],
    }
    let mut fixture = Fixture {
        outer: [0; 512],
        before: [0; 512],
        after: [0; 512],
        seed: [[0; 16]; 8],
        control: [0; 16],
        mxcsr: [0; 16],
    };
    for (i, value) in fixture.seed.iter_mut().enumerate() {
        value[..8].copy_from_slice(&(0x8000_0000_0000_0001_u64 + i as u64).to_le_bytes());
        value[8..10].copy_from_slice(&(0x3ff0_u16 + i as u16).to_le_bytes());
    }
    fixture.control[..2].copy_from_slice(&0x0b7f_u16.to_le_bytes());
    fixture.mxcsr[..4].copy_from_slice(&0x1fa5_u32.to_le_bytes());
    let left = [0, 0, 0, 0, 0, 0, 0, 0x80, 0xff, 0x3f];
    let right = [0, 0, 0, 0, 0, 0, 0, 0x80, 0, 0x40];
    let control = 0x037f_u16;
    macro_rules! check {
        ($op:literal) => {{
            let mut original = Image([0; 512]);
            let mut post = Image([0; 512]);
            transaction!(
                $op,
                original.0.as_mut_ptr(),
                post.0.as_mut_ptr(),
                left.as_ptr(),
                right.as_ptr(),
                &control as *const u16,
                [
                    "fxsave64 [{fixture}]",
                    "fninit",
                    "fldz",
                    "fldz",
                    "fdivp st(1), st(0)",
                    "fstp st(0)",
                    "fld tbyte ptr [{fixture}+1536]",
                    "fld tbyte ptr [{fixture}+1552]",
                    "fld tbyte ptr [{fixture}+1568]",
                    "fld tbyte ptr [{fixture}+1584]",
                    "fld tbyte ptr [{fixture}+1600]",
                    "fld tbyte ptr [{fixture}+1616]",
                    "fld tbyte ptr [{fixture}+1632]",
                    "fld tbyte ptr [{fixture}+1648]",
                    "ffree st(2)",
                    "ffree st(5)",
                    "fincstp",
                    "fldcw [{fixture}+1664]",
                    "ldmxcsr [{fixture}+1680]",
                    "fxsave64 [{fixture}+512]"
                ],
                ["fxsave64 [{fixture}+1024]", "fxrstor64 [{fixture}]"],
                [fixture = &mut fixture as *mut Fixture]
            );
            // Restore precedes every assertion. Never print host pointers,
            // environment, register payloads or even an assertion byte diff.
            let before_status = u16::from_le_bytes([fixture.before[2], fixture.before[3]]);
            assert!(
                before_status & 0x3801 == 0x0801,
                "Host fixture did not establish TOP1 and masked invalid"
            );
            assert!(
                fixture.before[4] == 0xdb,
                "Host fixture did not retain two empty payload slots"
            );
            assert!(
                fixture.before[..5] == fixture.after[..5],
                "Host x87 control, status and tags changed"
            );
            assert!(
                (u16::from_le_bytes([fixture.before[6], fixture.before[7]])
                    ^ u16::from_le_bytes([fixture.after[6], fixture.after[7]]))
                    & 0x7ff
                    == 0,
                "Defined host x87 opcode changed"
            );
            assert!(
                fixture.before[8..28] == fixture.after[8..28],
                "Host x87 pointers or MXCSR changed"
            );
            for slot in 0..8 {
                let at = 32 + 16 * slot;
                assert!(
                    fixture.before[at..at + 10] == fixture.after[at..at + 10],
                    "Host x87 register payload changed"
                );
            }
            assert!(
                fixture.before[160..416] == fixture.after[160..416],
                "Host XMM registers changed"
            );
        }};
    }
    check!("fadd st(0), st(1)");
    check!("fsub st(0), st(1)");
    check!("fmul st(0), st(1)");
    check!("fdiv st(0), st(1)");
}
