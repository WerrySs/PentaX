// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded register loops; trace bytes never authorize native entry.
use super::*;

fn looping(body: &[u8], condition: u8) -> Vec<u8> {
    let mut code = body.to_vec();
    code.extend([0x70 | condition, (-(body.len() as i8 + 2)) as u8]);
    code
}
fn inspect(
    code: &[u8],
    policy: TranslationPolicy,
    check: impl FnOnce(MaterializedBlockPlan<'_, '_, '_>),
) {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let limits = BlockLimits::default();
    let declaration = declare_materialized_source(&source, &[], &limits).unwrap();
    check(
        plan_materialized_block(
            &declaration,
            0,
            BIAS + ENTRY as u64,
            code,
            placement(),
            &policy,
            &limits,
        )
        .unwrap(),
    );
}
fn policy() -> TranslationPolicy {
    TranslationPolicy::x64_sse2()
        .with_affine_loop_trace()
        .with_register_loop_trace()
}

#[test]
fn register_trace_all_sixteen_conditions_have_only_forward_internal_control_and_two_exits() {
    // Partial-register aliases, carry dependency and a destructive SSE operand.
    let body = [0x00, 0xc4, 0x11, 0xc3, 0x0f, 0x58, 0xc0]; // ADD AH,AL; ADC EBX,EAX; ADDPS XMM0,XMM0.
    for condition in 0..16 {
        let code = looping(&body, condition);
        inspect(&code, policy(), |mut plan| {
            let original = plan.bytes().to_vec();
            let usage = plan.usage();
            let limits = BlockLimits::default();
            let reserved = plan.register_loop_trace_work_bound(&limits).unwrap();
            let spent =
                plan.prepare_register_loop_trace(&limits, reserved, limits.max_allocation_bytes);
            assert!(spent > 0 && spent <= reserved);
            assert!(plan.affine_loop_trace().is_none());
            assert!(plan.affine_loop_trace_work_bound(&limits).is_none());
            assert_eq!(plan.bytes(), original);
            assert_eq!(plan.usage(), usage);
            assert!(plan.native_admission().is_memory_free());
            let trace = plan.register_loop_trace().unwrap();
            assert!(std::ptr::eq(trace.block(), &plan));
            assert_eq!(trace.preparation_work(), spent);
            assert_eq!(trace.placement(), placement());
            let mut decoder = Decoder::with_ip(64, trace.bytes(), CACHE, DecoderOptions::NONE);
            let first: Vec<_> = (0..3).map(|_| decoder.decode()).collect();
            let branch = decoder.decode();
            let mut inverse = *plan.exit().instruction().unwrap();
            inverse.negate_condition_code();
            assert_eq!(branch.condition_code(), inverse.condition_code());
            assert_eq!(
                branch.near_branch_target(),
                CACHE + trace.first_exit_offset() as u64
            );
            assert!(branch.near_branch_target() > branch.next_ip());
            assert_eq!(decoder.ip(), CACHE + trace.second_iteration_offset() as u64);
            for left in first {
                let right = decoder.decode();
                assert_eq!(left.code(), right.code());
                assert_eq!(left.op0_register(), right.op0_register());
                assert_eq!(left.op1_register(), right.op1_register());
                assert_eq!(right.flow_control(), FlowControl::Next);
            }
            assert_eq!(decoder.ip(), CACHE + trace.second_exit_offset() as u64);
            for offset in [trace.first_exit_offset(), trace.second_exit_offset()] {
                assert_eq!(
                    &trace.bytes()[offset..offset + 6],
                    &[0x0f, 0x0b, 0x90, 0x90, 0x90, 0x90]
                );
            }
            assert!(trace.bytes().len() as u64 + 16 <= trace.placement().capacity);
            assert!(plan.retained_allocation_bytes() > usage.allocation_bytes);
        });
    }
}

#[test]
fn register_trace_is_separate_opt_in_and_refuses_memory_stack_traps_and_nonself_exits() {
    let bodies: &[&[u8]] = &[
        &[0x8b, 0x07, 0xff, 0xc9],       // Actual read.
        &[0x89, 0x07, 0xff, 0xc9],       // Actual write.
        &[0x48, 0x89, 0xe0, 0xff, 0xc9], // RSP input.
        &[0x48, 0xff, 0xc4, 0xff, 0xc9], // RSP output.
        &[0xf7, 0xf3, 0xff, 0xc9],       // Integer DIV can trap.
        &[0xf0, 0xff, 0x07, 0xff, 0xc9], // Atomic memory.
    ];
    for body in bodies {
        inspect(&looping(body, 5), policy(), |mut plan| {
            let usage = plan.usage();
            let bytes = plan.bytes().to_vec();
            let limits = BlockLimits::default();
            if let Some(bound) = plan.register_loop_trace_work_bound(&limits) {
                assert!(
                    plan.prepare_register_loop_trace(&limits, bound, limits.max_allocation_bytes)
                        <= bound
                );
            }
            assert!(plan.register_loop_trace().is_none());
            assert_eq!(plan.usage(), usage);
            assert_eq!(plan.bytes(), bytes);
        });
    }
    for code in [looping(&[0xff, 0xc9], 5), vec![0xff, 0xc9, 0x75, 0, 0xc3]] {
        let selected = if code.len() == 4 {
            TranslationPolicy::x64_sse2().with_affine_loop_trace()
        } else {
            policy()
        };
        inspect(&code, selected, |plan| {
            assert!(
                plan.register_loop_trace_work_bound(&BlockLimits::default())
                    .is_none()
            );
        });
    }
}

#[test]
fn register_trace_optional_caps_preserve_ordinary_usage_and_do_not_repeat_attempts() {
    let code = looping(&[0xff, 0xc9], 5);
    inspect(&code, policy(), |mut plan| {
        let limits = BlockLimits::default();
        let original = plan.usage();
        let reserved = plan.register_loop_trace_work_bound(&limits).unwrap();
        assert_eq!(
            plan.prepare_register_loop_trace(&limits, reserved - 1, u64::MAX),
            0
        );
        assert_eq!(plan.register_loop_trace_work_bound(&limits), Some(reserved));
        let spent = plan.prepare_register_loop_trace(&limits, reserved, original.allocation_bytes);
        assert!(spent > 0 && spent <= reserved);
        assert!(plan.register_loop_trace().is_none());
        assert!(plan.register_loop_trace_work_bound(&limits).is_none());
        assert_eq!(
            plan.prepare_register_loop_trace(&limits, reserved, u64::MAX),
            0
        );
        assert_eq!(plan.usage(), original);
        assert_eq!(plan.retained_allocation_bytes(), original.allocation_bytes);
    });
    inspect(&code, policy(), |plan| {
        for limits in [
            BlockLimits {
                max_instructions: 3,
                ..Default::default()
            },
            BlockLimits {
                max_output_bytes: 20,
                ..Default::default()
            },
            BlockLimits {
                max_work_units: plan.usage().work_units,
                ..Default::default()
            },
        ] {
            assert!(plan.register_loop_trace_work_bound(&limits).is_none());
        }
    });
}
