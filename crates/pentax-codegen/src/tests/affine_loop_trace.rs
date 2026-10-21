// SPDX-License-Identifier: GPL-2.0-only
//! Original finite traces: only a trusted forward branch leaves ordinary bodies.
use super::*;

fn policy() -> TranslationPolicy {
    TranslationPolicy::x64_sse2()
        .with_independent_memory()
        .with_affine_loop_trace()
}
fn looping(body: &[u8], condition: u8) -> Vec<u8> {
    let mut code = body.to_vec();
    code.extend([0x70 | condition, (-(body.len() as i8 + 2)) as u8]);
    code
}
fn inspect(
    code: &[u8],
    selected: TranslationPolicy,
    check: impl FnOnce(MaterializedBlockPlan<'_, '_, '_>),
) {
    let bytes = image(code);
    let source = link(&[(&bytes, BIAS)]);
    let limits = BlockLimits::default();
    let declaration = declare_materialized_source(&source, &[], &limits).unwrap();
    let plan = plan_materialized_block(
        &declaration,
        0,
        BIAS + ENTRY as u64,
        code,
        placement(),
        &selected,
        &limits,
    )
    .unwrap();
    check(plan);
}
fn attach(plan: &mut MaterializedBlockPlan<'_, '_, '_>, limits: &BlockLimits) -> u64 {
    let allowance = plan.affine_loop_trace_work_bound(limits).unwrap();
    let spent = plan.prepare_affine_loop_trace(limits, allowance, limits.max_allocation_bytes);
    assert!(spent > 0 && spent <= allowance);
    spent
}

#[test]
fn affine_trace_all_conditions_are_forward_and_both_rip_reads_keep_original_targets() {
    // MOV EAX,[RIP+data]; MOV ECX,[RDI]; ADD RDI,4; CMP RDI,RDX.
    let mut body = vec![0x8b, 0x05];
    let target = BIAS + DATA;
    body.extend(((target - (BIAS + ENTRY as u64 + 6)) as i32).to_le_bytes());
    body.extend([0x8b, 0x0f, 0x48, 0x83, 0xc7, 4, 0x48, 0x39, 0xd7]);
    for condition in 0..16 {
        let code = looping(&body, condition);
        inspect(&code, policy(), |mut plan| {
            let usage = plan.usage();
            let admission = (
                plan.native_admission().emitted_instructions(),
                plan.native_admission().feature_visits(),
                plan.native_admission().dispatch_work_units(),
                plan.native_admission().is_memory_free(),
            );
            let original = plan.bytes().to_vec();
            let spent = attach(&mut plan, &BlockLimits::default());
            assert_eq!(plan.usage(), usage);
            assert_eq!(
                (
                    plan.native_admission().emitted_instructions(),
                    plan.native_admission().feature_visits(),
                    plan.native_admission().dispatch_work_units(),
                    plan.native_admission().is_memory_free()
                ),
                admission
            );
            assert_eq!(plan.bytes(), original);
            assert!(plan.retained_allocation_bytes() > usage.allocation_bytes);
            let trace = plan.affine_loop_trace().unwrap();
            assert!(std::ptr::eq(trace.block(), &plan));
            assert_eq!(trace.preparation_work(), spent);
            assert_eq!(trace.placement(), plan.placement());
            assert!(trace.bytes().len() as u64 <= trace.placement().capacity - 16);
            let mut decode = Decoder::with_ip(64, trace.bytes(), CACHE, DecoderOptions::NONE);
            let first = (0..4).map(|_| decode.decode()).collect::<Vec<_>>();
            let branch = decode.decode();
            assert_eq!(branch.flow_control(), FlowControl::ConditionalBranch);
            let mut inverted = *plan.exit().instruction().unwrap();
            inverted.negate_condition_code();
            assert_eq!(branch.condition_code(), inverted.condition_code());
            assert_eq!(
                branch.near_branch_target(),
                CACHE + trace.first_exit_offset() as u64
            );
            assert!(branch.near_branch_target() > branch.next_ip());
            assert_eq!(decode.ip(), CACHE + trace.second_iteration_offset() as u64);
            let second = (0..4).map(|_| decode.decode()).collect::<Vec<_>>();
            for (left, right) in first.iter().zip(&second) {
                assert_eq!(left.code(), right.code());
                assert_eq!(left.op0_register(), right.op0_register());
            }
            assert_eq!(first[0].ip_rel_memory_address(), target);
            assert_eq!(second[0].ip_rel_memory_address(), target);
            // A byte-copy of B1 would address a different location in B2.
            assert_ne!(
                first[0]
                    .memory_displacement64()
                    .wrapping_sub(first[0].next_ip()),
                second[0]
                    .memory_displacement64()
                    .wrapping_sub(second[0].next_ip())
            );
            assert_eq!(decode.ip(), CACHE + trace.second_exit_offset() as u64);
            for offset in [trace.first_exit_offset(), trace.second_exit_offset()] {
                assert_eq!(
                    &trace.bytes()[offset..offset + 6],
                    &[0x0f, 0x0b, 0x90, 0x90, 0x90, 0x90]
                );
            }
            // Ordinary source/body output never gains the trusted branch.
            let mut ordinary = Decoder::with_ip(64, plan.bytes(), CACHE, DecoderOptions::NONE);
            for _ in 0..4 {
                assert_eq!(ordinary.decode().flow_control(), FlowControl::Next);
            }
            assert_eq!(ordinary.decode().code(), Code::Ud2);
        });
    }
}

#[test]
fn affine_trace_induction_is_exact_nonzero_checked_and_zeroextends_32bit_updates() {
    // All updates target the original address register after its last access.
    let updates: &[(&[u8], usize, i64)] = &[
        (&[0x48, 0xff, 0xc7], 8, 1),
        (&[0xff, 0xc7], 4, 1),
        (&[0x48, 0x83, 0xc7, 4], 8, 4),
        (&[0x48, 0x83, 0xc7, 0xfc], 8, -4),
        (&[0x83, 0xc7, 4], 4, 4),
        (&[0x83, 0xc7, 0xfc], 4, -4),
        (&[0x48, 0x81, 0xc7, 0x00, 0x01, 0, 0], 8, 256),
        (&[0x81, 0xc7, 0x00, 0x01, 0, 0], 4, 256),
    ];
    for &(update, width, delta) in updates {
        let mut body = vec![0x8b, 0x07];
        body.extend(update);
        let code = looping(&body, 5);
        inspect(&code, policy(), |mut plan| {
            attach(&mut plan, &BlockLimits::default());
            let trace = plan.affine_loop_trace().unwrap();
            let mut input = [0x8877_6655_4433_2211; 16];
            input[7] = if width == 4 {
                0xaaaa_bbbb_0000_1000
            } else {
                0x1000
            };
            let output = trace.second_iteration_gpr(&input).unwrap();
            let mut expected = input;
            expected[7] = (0x1000_i64 + delta) as u64;
            assert_eq!(output, expected);
            input[7] = if delta < 0 {
                0
            } else if width == 4 {
                u32::MAX as u64
            } else {
                u64::MAX
            };
            assert!(trace.second_iteration_gpr(&input).is_none());
        });
    }
}

#[test]
fn affine_trace_declines_unsafe_or_stable_shapes_without_changing_ordinary_plan() {
    let bodies: &[&[u8]] = &[
        &[0x8b, 0x07, 0x48, 0x83, 0xc7, 0], // Stable poll, not a real induction.
        &[0x8b, 0x07, 0x48, 0x83, 0xef, 4], // SUB is outside the closed induction set.
        &[0x8b, 0x07, 0x40, 0xfe, 0xc7],    // Partial-byte DIL update.
        &[0x8b, 0x07, 0x48, 0xff, 0xc7, 0x48, 0xff, 0xc7], // Two EA updates.
        &[0x8b, 0x07, 0x48, 0x0f, 0x45, 0xfa], // Conditional EA write.
        &[0x8b, 0x07, 0x48, 0x87, 0xfa],    // XCHG EA write.
        &[0x8b, 0x04, 0x24, 0x48, 0xff, 0xc4], // Stack address/update.
        &[0x8b, 0x07, 0x48, 0x89, 0xe0, 0x48, 0xff, 0xc7], // Reads RSP.
        &[0xf0, 0xff, 0x07, 0x48, 0xff, 0xc7], // Atomic memory.
        &[0x66, 0x0f, 0xe7, 0x07, 0x48, 0xff, 0xc7], // Streaming store.
        &[0x8b, 0x07, 0xf7, 0xf3, 0x48, 0xff, 0xc7], // Potentially trapping DIV.
        &[0x48, 0xff, 0xc7, 0x8b, 0x07],    // EA update precedes memory; ordinary split.
        &[0x8b, 0x07, 0x48, 0xff, 0xc7, 0x8b, 0x0e], // Update precedes another memory.
    ];
    for body in bodies {
        let code = looping(body, 5);
        inspect(&code, policy(), |mut plan| {
            let usage = plan.usage();
            let original = plan.bytes().to_vec();
            if let Some(allowance) = plan.affine_loop_trace_work_bound(&BlockLimits::default()) {
                let work =
                    plan.prepare_affine_loop_trace(&BlockLimits::default(), allowance, u64::MAX);
                assert!(work > 0 && work <= allowance);
                assert_eq!(
                    plan.prepare_affine_loop_trace(&BlockLimits::default(), allowance, u64::MAX),
                    0
                );
            }
            assert!(plan.affine_loop_trace().is_none());
            assert_eq!(plan.usage(), usage);
            assert_eq!(plan.bytes(), original);
        });
    }
    let code = looping(&[0x8b, 0x07, 0x48, 0xff, 0xc7], 5);
    inspect(
        &code,
        TranslationPolicy::x64_sse2().with_independent_memory(),
        |plan| {
            assert!(
                plan.affine_loop_trace_work_bound(&BlockLimits::default())
                    .is_none()
            );
            assert!(plan.affine_loop_trace().is_none());
        },
    );
}

#[test]
fn affine_trace_optional_reservation_caps_and_attempts_preserve_ordinary_usage() {
    let code = looping(&[0x8b, 0x07, 0x48, 0xff, 0xc7], 5);
    inspect(&code, policy(), |mut plan| {
        let limits = BlockLimits::default();
        let usage = plan.usage();
        let bound = plan.affine_loop_trace_work_bound(&limits).unwrap();
        assert_eq!(
            plan.prepare_affine_loop_trace(&limits, bound - 1, u64::MAX),
            0
        );
        assert_eq!(plan.affine_loop_trace_work_bound(&limits), Some(bound));
        // Before allocation the fixed optional metadata cannot fit. A genuine
        // attempt is charged and remembered, and the original block survives.
        let spent = plan.prepare_affine_loop_trace(&limits, bound, usage.allocation_bytes);
        assert!(spent > 0 && spent <= bound);
        assert!(plan.affine_loop_trace().is_none());
        assert!(plan.affine_loop_trace_work_bound(&limits).is_none());
        assert_eq!(plan.prepare_affine_loop_trace(&limits, bound, u64::MAX), 0);
        assert_eq!(plan.usage(), usage);
        assert_eq!(plan.retained_allocation_bytes(), usage.allocation_bytes);
    });
    inspect(&code, policy(), |plan| {
        let limits = BlockLimits {
            max_instructions: 5,
            ..BlockLimits::default()
        };
        assert!(plan.affine_loop_trace_work_bound(&limits).is_none());
        let limits = BlockLimits {
            max_output_bytes: 20,
            ..BlockLimits::default()
        };
        assert!(plan.affine_loop_trace_work_bound(&limits).is_none());
        let limits = BlockLimits {
            max_work_units: plan.usage().work_units,
            ..BlockLimits::default()
        };
        assert!(plan.affine_loop_trace_work_bound(&limits).is_none());
    });
}
