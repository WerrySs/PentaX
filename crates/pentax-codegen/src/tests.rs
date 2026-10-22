// SPDX-License-Identifier: GPL-2.0-only

mod affine_loop_trace;
mod align_right;
mod approximations;
mod bit_strings;
mod bmi;
mod cpu_identification;
mod element_permutations;
mod fixture;
mod immediate_permutations;
mod lane_extract;
mod lea;
mod materialized;
mod memory;
mod minimum_position;
mod multiply_add_words;
mod native_float_min_max;
mod native_fma_registers;
mod native_packed_float;
mod native_packed_float_memory;
mod native_scalar_float;
mod native_scalar_fma;
mod native_streaming_stores;
mod native_string_index;
mod native_vector_broadcasts;
mod native_vector_moves;
mod native_vector_register_bits;
mod native_ymm;
mod packed_convert;
mod pause;
mod qword_comparisons;
mod register_loop_trace;
mod rounding;
mod saturating_packs;
mod source_window;
mod timestamp_counter;
mod vector_masks;
mod vex_comparisons;
use super::*;
use fixture::{BIAS, DATA, ENTRY, PAGE, image, link, with_relocation};
use iced_x86::{Decoder, DecoderOptions, FlowControl, OpKind};

const CACHE: u64 = 0x1000_0000;
fn placement() -> CachePlacement {
    CachePlacement {
        address: CACHE,
        capacity: PAGE,
    }
}
fn make<'p, 'i>(plan: &'p pentax_linker::SymbolicLinkPlan<'i>) -> BlockPlan<'p, 'i> {
    plan_block(
        plan,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap()
}
fn ordinary_output(plan: &BlockPlan<'_, '_>) -> Vec<iced_x86::Instruction> {
    let mut decoder = Decoder::with_ip(64, plan.bytes(), CACHE, DecoderOptions::NONE);
    let mut result = Vec::new();
    while decoder.can_decode() {
        let instruction = decoder.decode();
        assert!(!instruction.is_invalid());
        if instruction.code() == Code::Ud2 {
            assert_eq!(instruction.ip(), plan.exit().cache_ip());
            assert!(!decoder.can_decode());
        } else {
            assert_eq!(instruction.flow_control(), FlowControl::Next);
            result.push(instruction);
        }
    }
    result
}

#[test]
fn memory_fences_keep_their_hardware_instructions_in_native_blocks() {
    let code = [0x0f, 0xae, 0xf0, 0x0f, 0xae, 0xe8, 0x0f, 0xae, 0xf8, 0xc3];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert!(matches!(plan.exit().kind(), ExitKind::Return { adjust: 0 }));
    assert_eq!(&plan.bytes()[..9], &code[..9]);
    let emitted = ordinary_output(&plan);
    assert_eq!(
        emitted
            .iter()
            .map(Instruction::mnemonic)
            .collect::<Vec<_>>(),
        [Mnemonic::Mfence, Mnemonic::Lfence, Mnemonic::Sfence]
    );
    assert!(
        emitted
            .iter()
            .all(|instruction| instruction.op_count() == 0)
    );
}

#[test]
fn broad_original_integer_sib_stack_and_sse_forms_reencode_before_return_exit() {
    let code = [
        0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x8d, 0x44, 0xcb, 0x10, 0x48, 0x0f,
        0xaf, 0xc1, 0x48, 0x0f, 0xba, 0xe0, 5, 0x48, 0x0f, 0x45, 0xd0, 0x0f, 0x95, 0xc0, 0x66,
        0x0f, 0xef, 0xc0, 0xf2, 0x0f, 0x58, 0xc1, 0x48, 0x83, 0xc4, 0x20, 0x5d, 0xc3,
    ];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert!(matches!(plan.exit().kind(), ExitKind::Return { adjust: 0 }));
    let emitted = ordinary_output(&plan);
    assert_eq!(emitted.len(), 12);
    assert!(emitted.iter().any(|i| i.mnemonic() == Mnemonic::Lea));
    assert!(emitted.iter().any(|i| i.mnemonic() == Mnemonic::Pxor));
    assert!(emitted.iter().any(|i| i.mnemonic() == Mnemonic::Addsd));
    assert_eq!(plan.instructions().len(), 13);
    assert_eq!(plan.instructions().last().unwrap().emitted_len(), 0);
    assert!(std::ptr::eq(plan.source_plan(), &source));
    assert_eq!(plan.module(), source.modules()[0].id());
    assert_eq!(plan.original_bytes(), code);
    assert_eq!(plan.relocated_source_bytes(), code);
}

#[test]
fn direct_branch_call_and_conditional_are_typed_guest_targets_never_emitted() {
    for (code, expected) in [
        (
            vec![0xe8, 0x20, 0, 0, 0],
            ExitKind::Call {
                target: ControlTarget::Direct(BIAS + ENTRY as u64 + 5 + 0x20),
                return_pc: BIAS + ENTRY as u64 + 5,
            },
        ),
        (
            vec![0xeb, 0xfe],
            ExitKind::Jump {
                target: ControlTarget::Direct(BIAS + ENTRY as u64),
            },
        ),
        (
            vec![0x75, 0x7f],
            ExitKind::Conditional {
                condition: ConditionCode::ne,
                target: BIAS + ENTRY as u64 + 2 + 0x7f,
                fallthrough: BIAS + ENTRY as u64 + 2,
            },
        ),
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        assert_eq!(plan.exit().kind(), expected);
        assert_eq!(plan.instructions().len(), 1);
        assert_eq!(plan.exit().instruction().unwrap().len(), code.len());
    }
}

#[test]
fn indirect_register_and_sib_targets_retain_original_source_operands() {
    let bytes = image(&[0x41, 0xff, 0xd3]);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert_eq!(
        plan.exit().kind(),
        ExitKind::Call {
            target: ControlTarget::Register(Register::R11),
            return_pc: BIAS + ENTRY as u64 + 3
        }
    );
    let bytes = image(&[0x41, 0xff, 0x64, 0xcc, 0x18]);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    let ExitKind::Jump {
        target: ControlTarget::Memory(operand),
    } = plan.exit().kind()
    else {
        panic!("expected indirect memory jump")
    };
    assert_eq!(
        (
            operand.base(),
            operand.index(),
            operand.scale(),
            operand.displacement()
        ),
        (Register::R12, Register::RCX, 8, 24)
    );
    assert_eq!(operand.ip_relative_address(), None);
    assert_eq!(plan.bytes(), [0x0f, 0x0b]);
}

#[test]
fn rip_relative_indirect_call_is_a_guest_pointer_location_not_a_cache_reference() {
    let displacement = (DATA as i64 - ENTRY as i64 - 6) as i32;
    let mut code = vec![0xff, 0x15];
    code.extend_from_slice(&displacement.to_le_bytes());
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    let ExitKind::Call {
        target: ControlTarget::Memory(operand),
        return_pc,
    } = plan.exit().kind()
    else {
        panic!("expected memory call")
    };
    assert_eq!(operand.ip_relative_address(), Some(BIAS + DATA));
    assert_eq!(return_pc, BIAS + ENTRY as u64 + 6);
    assert_eq!(
        plan.instructions()[0].rip_relative_target(),
        Some(BIAS + DATA)
    );
}

#[test]
fn return_immediate_syscall_interrupt_and_guest_ud2_always_end_at_owned_marker() {
    for (code, kind) in [
        (vec![0xc2, 0x20, 0], ExitKind::Return { adjust: 32 }),
        (vec![0x0f, 0x05], ExitKind::Syscall),
        (
            vec![0xcc],
            ExitKind::Interrupt {
                mnemonic: Mnemonic::Int3,
            },
        ),
        (
            vec![0x0f, 0x0b],
            ExitKind::GuestException { code: Code::Ud2 },
        ),
    ] {
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
        assert_eq!(plan.exit().kind(), kind);
        assert_eq!(plan.exit().guest_pc(), BIAS + ENTRY as u64);
        assert_eq!(
            plan.exit().next_guest_pc(),
            BIAS + ENTRY as u64 + code.len() as u64
        );
    }
}

#[test]
fn rip_load_store_and_lea_keep_exact_original_guest_addresses_at_new_cache_ips() {
    for opcode in [0x8b, 0x89, 0x8d] {
        let mut code = vec![0x48, opcode, 5];
        code.extend_from_slice(&((DATA as i64 - ENTRY as i64 - 7) as i32).to_le_bytes());
        code.push(0xc3);
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        let emitted = ordinary_output(&plan);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].ip_rel_memory_address(), BIAS + DATA);
        assert_eq!(
            plan.instructions()[0].rip_relative_target(),
            Some(BIAS + DATA)
        );
        assert_eq!(plan.instructions()[0].guest_pc(), BIAS + ENTRY as u64);
        assert_eq!(
            (
                plan.instructions()[0].source_offset(),
                plan.instructions()[0].source_len()
            ),
            (0, 7)
        );
    }
}

#[test]
fn rip_data_too_far_from_cache_is_explicitly_rejected_without_rewriting_semantics() {
    let bytes = image(&[0x48, 0x8b, 5, 0x10, 0, 0, 0, 0xc3]);
    let before = bytes.clone();
    let source = link(&[(&bytes, BIAS)]);
    assert!(
        matches!(plan_block(&source,0,BIAS+ENTRY as u64,CachePlacement{address:0x7000_0000_0000,capacity:PAGE},&TranslationPolicy::x64_sse2(),&BlockLimits::default()),Err(CodegenError::RipRelativeOutOfRange{target,..}) if target==BIAS+ENTRY as u64+7+0x10)
    );
    assert_eq!(bytes, before);
}

#[test]
fn unported_system_segment_state_and_repeat_forms_are_not_native_output() {
    let cases: &[&[u8]] = &[
        &[0x64, 0x48, 0x8b, 3],
        &[0x65, 0x48, 0x8b, 3],
        &[0xf3, 0xa4],
        &[0xf2, 0xf0, 0x48, 0x01, 8], // HLE hints remain unsupported.
        &[0x0f, 0xa2],
        &[0x0f, 0x31],
        &[0xf3, 0x48, 0x0f, 0xae, 0xd0],
        &[0x0f, 0x01, 0xef],
        &[0x0f, 0xae, 0x28],
        &[0x0f, 0xae, 0x18],
        &[0xc5, 0xf8, 0x77],
        &[0xd9, 0xe8],
        &[0x0f, 0x6f, 0xc0],
        &[0x9d],
        &[0xf4],
        &[0x0f, 0x34],
        &[0x0f, 0x01, 0xd5],
        &[0x0f, 0x01, 0xd6],
        &[0xe2, 0xfe],
        &[0x66, 0xc3],
    ];
    let generous = TranslationPolicy::new(
        CpuidFeature::values().fold(CpuFeatures::empty(), CpuFeatures::with),
    );
    for code in cases {
        let bytes = image(code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &generous,
            &BlockLimits::default(),
        )
        .unwrap();
        assert!(
            matches!(plan.exit().kind(), ExitKind::Mediated { .. }),
            "{code:x?}: {:?}",
            plan.exit()
        );
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
    }
}

#[test]
fn features_are_declared_gates_not_a_bypass_for_semantic_families() {
    let bytes = image(&[0x66, 0x0f, 0xef, 0xc0, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::new(CpuFeatures::empty()),
        &BlockLimits::default(),
    )
    .unwrap();
    assert_eq!(
        plan.exit().kind(),
        ExitKind::Mediated {
            reason: UnsupportedReason::MissingFeature(CpuidFeature::SSE2)
        }
    );
    assert_eq!(plan.bytes(), [0x0f, 0x0b]);
}

#[test]
fn truncation_and_invalid_decoding_never_skip_or_publish_partial_blocks() {
    for code in [&[0x90, 0x0f][..], &[0xb8, 1, 2][..]] {
        let bytes = image(code);
        let source = link(&[(&bytes, BIAS)]);
        assert!(matches!(
            plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default()
            ),
            Err(CodegenError::TruncatedInstruction { .. })
        ));
    }
    let bytes = image(&[0xf0, 0x90, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let source = link(&[(&bytes, BIAS)]);
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::InvalidInstruction { .. })
    ));
}

#[test]
fn partial_window_policy_never_skips_invalid_or_truncated_first_instructions() {
    let policy = TranslationPolicy::x64_sse2().with_partial_window_splits();
    for prefix in [false, true] {
        let mut code = Vec::new();
        if prefix {
            code.push(0x90);
        }
        code.extend_from_slice(&[0xf0, 0x90]); // Invalid LOCK NOP, not a partial fetch.
        let bytes = image(&code);
        let source = link(&[(&bytes, BIAS)]);
        assert!(matches!(
            plan_block(
                &source,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &policy,
                &BlockLimits::default()
            ),
            Err(CodegenError::InvalidInstruction { .. })
        ));
    }
    let bytes = image(&[0x90, 0x0f]);
    let source = link(&[(&bytes, BIAS)]);
    let pc = BIAS + ENTRY as u64;
    let block = plan_block(
        &source,
        0,
        pc,
        placement(),
        &policy,
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(matches!(block.exit().kind(), ExitKind::Fallthrough { target } if target == pc + 1));
    assert_eq!(block.instructions().len(), 1);
    assert!(matches!(
        plan_block(&source, 0, pc + 1, placement(), &policy, &BlockLimits::default()),
        Err(CodegenError::TruncatedInstruction { guest_pc }) if guest_pc == pc + 1
    ));
}

#[test]
fn cache_placement_cannot_alias_any_modules_mapped_page_padding() {
    let a = image(&[0xc3]);
    let b = image(&[0xc3]);
    let source = link(&[(&a, BIAS), (&b, 0x20_0000)]);
    for (address, module) in [
        (BIAS + 0x3000, 0),
        (BIAS + DATA, 0),
        (0x20_0000 + 0x3000, 1),
    ] {
        assert!(
            matches!(plan_block(&source,0,BIAS+ENTRY as u64,CachePlacement{address,capacity:16},&TranslationPolicy::x64_sse2(),&BlockLimits::default()),Err(CodegenError::CacheOverlap{module:id}) if id.index()==module)
        );
    }
}

#[test]
fn source_must_be_initialized_executable_and_addresses_checked() {
    let bytes = image(&[0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    for pc in [BIAS + DATA, BIAS + 0x1000, 0x50_0000] {
        assert!(matches!(
            plan_block(
                &source,
                0,
                pc,
                placement(),
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default()
            ),
            Err(CodegenError::MissingInitializedExecutableSource { .. })
        ));
    }
    assert!(matches!(
        plan_block(
            &source,
            8,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::MissingModule { index: 8 })
    ));
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            CachePlacement {
                address: u64::MAX - 1,
                capacity: PAGE
            },
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::InvalidAddress)
    ));
}

#[test]
fn source_relative_writes_are_applied_before_decode_without_mutating_original_image() {
    let bytes = with_relocation(8, ENTRY as u64 + 2, 0x3456);
    let before = bytes.clone();
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert_eq!(
        plan.instructions()[0].instruction().op1_kind(),
        OpKind::Immediate64
    );
    assert_eq!(
        plan.instructions()[0].instruction().immediate64(),
        BIAS + 0x3456
    );
    assert_eq!(&plan.original_bytes()[2..10], &[0; 8]);
    assert_eq!(
        &plan.relocated_source_bytes()[2..10],
        &(BIAS + 0x3456).to_le_bytes()
    );
    assert_eq!(bytes, before);
}

#[test]
fn any_remaining_deferred_record_is_refused_instead_of_using_original_unpatched_code() {
    let bytes = with_relocation(42, DATA, 0);
    let source = link(&[(&bytes, BIAS)]);
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::DeferredRelocations { count: 1 })
    ));
}

#[test]
fn owned_exit_attestation_ignores_incidental_ud2_and_syscall_bytes_in_immediates() {
    let code = [0xb8, 0x0f, 0x0b, 0x0f, 0x05, 0xc3];
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert_eq!(ordinary_output(&plan).len(), 1);
    assert!(!plan.attest_exit(CACHE + 1, &[0x0f, 0x0b]));
    assert!(plan.attest_exit(CACHE + 5, &[0x0f, 0x0b]));
    assert!(!plan.attest_exit(CACHE + 5, &[0x0f, 0x05]));
    assert!(plan.attest_emission(CACHE, plan.bytes()));
    assert!(!plan.attest_emission(CACHE + 1, plan.bytes()));
    let mut changed = plan.bytes().to_vec();
    changed[0] ^= 1;
    assert!(!plan.attest_emission(CACHE, &changed));
}

#[test]
fn instruction_and_output_splits_resume_at_first_unemitted_guest_instruction() {
    let bytes = image(&[0xb8, 1, 0, 0, 0, 0xb9, 2, 0, 0, 0, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let mut limits = BlockLimits {
        max_instructions: 1,
        ..BlockLimits::default()
    };
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::x64_sse2(),
        &limits,
    )
    .unwrap();
    assert_eq!(
        plan.exit().kind(),
        ExitKind::Fallthrough {
            target: BIAS + ENTRY as u64 + 5
        }
    );
    assert_eq!(plan.instructions().len(), 1);
    assert!(plan.exit().instruction().is_none());
    limits.max_instructions = 256;
    limits.max_output_bytes = 7;
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::x64_sse2(),
        &limits,
    )
    .unwrap();
    assert_eq!(
        plan.exit().kind(),
        ExitKind::Fallthrough {
            target: BIAS + ENTRY as u64 + 5
        }
    );
    assert_eq!(plan.usage().decoded_instructions, 2);
    limits.max_output_bytes = 6;
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &limits
        ),
        Err(CodegenError::Budget {
            kind: Budget::OutputBytes,
            ..
        })
    ));
}

#[test]
fn allocation_work_and_aggregate_limits_are_precharged_and_inclusive() {
    let bytes = image(&[0x90, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let ordinary = make(&source);
    let usage = ordinary.usage();
    let admission = ordinary.native_admission();
    let construction_work = std::mem::size_of::<NativeAdmissionSummary>() as u64
        + 32 * ordinary.instructions().len() as u64
        + admission.feature_visits();
    assert_eq!(
        usage.work_units,
        admission.dispatch_work_units() + construction_work
    );
    assert!(usage.allocation_bytes >= std::mem::size_of::<NativeAdmissionSummary>() as u64);
    let mut exact = BlockLimits {
        max_allocation_bytes: usage.allocation_bytes,
        max_work_units: usage.work_units,
        ..BlockLimits::default()
    };
    assert!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &exact
        )
        .is_ok()
    );
    exact.max_allocation_bytes -= 1;
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &exact
        ),
        Err(CodegenError::Budget {
            kind: Budget::AllocationBytes,
            ..
        })
    ));
    exact.max_allocation_bytes = usage.allocation_bytes;
    exact.max_work_units -= 1;
    assert!(matches!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &exact
        ),
        Err(CodegenError::Budget {
            kind: Budget::Work,
            ..
        })
    ));
    for (kind, limits) in [
        (
            Budget::Modules,
            BlockLimits {
                max_modules: 0,
                ..BlockLimits::default()
            },
        ),
        (
            Budget::Segments,
            BlockLimits {
                max_segments: 1,
                ..BlockLimits::default()
            },
        ),
        (
            Budget::SourceBytes,
            BlockLimits {
                max_source_bytes: 0,
                ..BlockLimits::default()
            },
        ),
        (
            Budget::Instructions,
            BlockLimits {
                max_instructions: 0,
                ..BlockLimits::default()
            },
        ),
        (
            Budget::OutputBytes,
            BlockLimits {
                max_output_bytes: 1,
                ..BlockLimits::default()
            },
        ),
    ] {
        assert!(
            matches!(plan_block(&source,0,BIAS+ENTRY as u64,placement(),&TranslationPolicy::x64_sse2(),&limits),Err(CodegenError::Budget{kind:actual,..}) if kind==actual)
        );
    }
}

#[test]
fn isolated_short_blocks_do_not_reserve_for_every_fetched_byte() {
    let mut code = vec![0x48, 0x8b, 0x00]; // MOV RAX,[RAX], then unread-ahead NOPs.
    code.resize(64, 0x90);
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let limits = BlockLimits {
        max_source_bytes: 64,
        max_allocation_bytes: 2048,
        max_work_units: 4096,
        ..Default::default()
    };
    let plan = plan_block(
        &source,
        0,
        BIAS + ENTRY as u64,
        placement(),
        &TranslationPolicy::x64_sse2().with_isolated_memory(),
        &limits,
    )
    .unwrap();
    assert_eq!(plan.original_bytes(), code);
    assert_eq!(plan.relocated_source_bytes(), code);
    assert_eq!(plan.instructions().len(), 1);
    assert_eq!(plan.bytes(), [0x48, 0x8b, 0x00, 0x0f, 0x0b]);
    assert_eq!(
        plan.exit().kind(),
        ExitKind::Fallthrough {
            target: BIAS + ENTRY as u64 + 3
        }
    );
    assert!(plan.usage().allocation_bytes <= 2048);
    // These bounds include complete source snapshots and preserve the next PC.
    // The previous eager 64-record reservation alone exceeded both bounds.
    assert!(plan.usage().work_units <= 4096);
}

#[test]
fn growing_block_buffers_preserve_all_records_bytes_and_inclusive_budgets() {
    let mut code = Vec::new();
    for value in 0..32_u64 {
        code.extend([0x48, 0xb8]);
        code.extend((0x1234_5678_0000_0000 | value).to_le_bytes());
    }
    code.push(0xc3);
    let bytes = image(&code);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert_eq!(plan.instructions().len(), 33);
    let decoded = ordinary_output(&plan);
    assert_eq!(decoded.len(), 32);
    for (index, instruction) in decoded.iter().enumerate() {
        assert_eq!(
            instruction.immediate64(),
            0x1234_5678_0000_0000 | index as u64
        );
        assert_eq!(plan.instructions()[index].cache_offset(), index * 10);
        assert_eq!(plan.instructions()[index].source_offset(), index * 10);
    }
    assert_eq!(&plan.bytes()[..320], &code[..320]);
    assert_eq!(&plan.bytes()[320..], [0x0f, 0x0b]);
    assert!(matches!(plan.exit().kind(), ExitKind::Return { adjust: 0 }));
    let exact = BlockLimits {
        max_allocation_bytes: plan.usage().allocation_bytes,
        max_work_units: plan.usage().work_units,
        ..Default::default()
    };
    assert!(
        plan_block(
            &source,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &exact
        )
        .is_ok()
    );
    for (limits, expected) in [
        (
            BlockLimits {
                max_allocation_bytes: exact.max_allocation_bytes - 1,
                ..exact
            },
            Budget::AllocationBytes,
        ),
        (
            BlockLimits {
                max_work_units: exact.max_work_units - 1,
                ..exact
            },
            Budget::Work,
        ),
    ] {
        assert!(
            matches!(plan_block(&source, 0, BIAS + ENTRY as u64, placement(),
            &TranslationPolicy::x64_sse2(), &limits),
            Err(CodegenError::Budget { kind, .. }) if kind == expected)
        );
    }
}

#[test]
fn fallible_reservation_capacity_overflow_is_a_typed_error() {
    let mut account = Accounting {
        work: 0,
        allocated: 0,
        limits: BlockLimits {
            max_work_units: u64::MAX,
            max_allocation_bytes: u64::MAX,
            ..BlockLimits::default()
        },
    };
    assert!(matches!(
        account.buffer::<u8>(usize::MAX),
        Err(CodegenError::Allocation)
    ));
    assert_eq!(account.allocated, 0);
}

#[test]
fn ordinary_memory_forms_admit_sib_but_refuse_address_size_and_special_registers() {
    let bytes = image(&[0x4b, 0x8b, 0x44, 0x8c, 0x20, 0xc3]);
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    let output = ordinary_output(&plan);
    assert_eq!(
        (
            output[0].memory_base(),
            output[0].memory_index(),
            output[0].memory_index_scale()
        ),
        (Register::R12, Register::R9, 4)
    );
    for code in [
        &[0x67, 0x48, 0x8b, 0, 0xc3][..],
        &[0x8e, 0xd0][..],
        &[0x0f, 0x20, 0xc0][..],
    ] {
        let bytes = image(code);
        let source = link(&[(&bytes, BIAS)]);
        let plan = make(&source);
        assert!(matches!(plan.exit().kind(), ExitKind::Mediated { .. }));
        assert_eq!(plan.bytes(), [0x0f, 0x0b]);
    }
}

#[test]
fn partial_source_patch_intersections_use_the_same_retained_write_bytes() {
    // The entry starts partway through a relative write. Relocated bytes are
    // sampled from its correct byte positions, never from an offset-zero copy.
    let value = u64::from_le_bytes([0x11, 0x22, 0x90, 0x90, 0x90, 0x90, 0x90, 0xc3]);
    let bytes = with_relocation(8, ENTRY as u64 - 2, value.wrapping_sub(BIAS) as i64);
    let before = bytes.clone();
    let source = link(&[(&bytes, BIAS)]);
    let plan = make(&source);
    assert_eq!(
        &plan.relocated_source_bytes()[..6],
        &value.to_le_bytes()[2..]
    );
    assert_eq!(ordinary_output(&plan).len(), 5);
    assert_eq!(plan.exit().kind(), ExitKind::Return { adjust: 0 });
    assert_eq!(bytes, before);
}

mod signed_comparisons;

mod x87_data;

#[path = "tests/x87_arithmetic.rs"]
mod x87_arithmetic;
