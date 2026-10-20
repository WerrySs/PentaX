// SPDX-License-Identifier: GPL-2.0-only

//! Original owned-memory/metadata tests. No test calls `step`, enters the
//! assembly bridge or executes native cache/guest code. Segment cases call the
//! checked Rust memory semantics directly, without changing host FS/GS.

#[path = "tests/arithmetic_vectors.rs"]
mod arithmetic_vectors;
#[path = "tests/atomics.rs"]
mod atomics;
#[path = "tests/bit_counts.rs"]
mod bit_counts;
#[path = "tests/blend_vectors.rs"]
mod blend_vectors;
#[path = "tests/broadcast_vectors.rs"]
mod broadcast_vectors;
#[path = "tests/cache_hints.rs"]
mod cache_hints;
#[path = "tests/comparison_vectors.rs"]
mod comparison_vectors;
#[path = "tests/element_permutations.rs"]
mod element_permutations;
#[path = "tests/extract_vectors.rs"]
mod extract_vectors;
#[path = "tests/float_arithmetic.rs"]
mod float_arithmetic;
#[path = "tests/float_comparisons.rs"]
mod float_comparisons;
#[path = "tests/qword_comparisons.rs"]
mod qword_comparisons;

#[path = "tests/cpu_identification.rs"]
mod cpu_identification_tests;
#[path = "tests/float_integer_convert.rs"]
mod float_integer_convert;
#[path = "tests/floating_bit_tests.rs"]
mod floating_bit_tests;
#[path = "tests/fused_vectors.rs"]
mod fused_vectors;
#[path = "tests/half_vectors.rs"]
mod half_vectors;
#[path = "tests/immediate_permutations.rs"]
mod immediate_permutations;
#[path = "tests/insert_vectors.rs"]
mod insert_vectors;
#[path = "tests/integer_convert.rs"]
mod integer_convert;
#[path = "tests/lane_extract.rs"]
mod lane_extract;
#[path = "tests/logical_vectors.rs"]
mod logical_vectors;
#[path = "tests/mask_comparisons.rs"]
mod mask_comparisons_tests;
#[path = "tests/minimum_position.rs"]
mod minimum_position;
#[path = "tests/minmax_vectors.rs"]
mod minmax_vectors;
#[path = "tests/multiply_vectors.rs"]
mod multiply_vectors;
#[path = "tests/mxcsr_registers.rs"]
mod mxcsr_registers;
#[path = "tests/native_cache.rs"]
mod native_cache;
#[path = "tests/packed_convert.rs"]
mod packed_convert_tests;
#[path = "tests/packed_vectors.rs"]
mod packed_vectors;
#[path = "tests/permute_vectors.rs"]
mod permute_vectors;
#[path = "tests/precision_convert.rs"]
mod precision_convert;
#[path = "tests/scalar_elements.rs"]
mod scalar_elements;
#[path = "tests/scalar_vectors.rs"]
mod scalar_vectors;
#[path = "tests/segments_integer.rs"]
mod segments_integer;
#[path = "tests/shift_vectors.rs"]
mod shift_vectors;
#[path = "tests/shuffle_vectors.rs"]
mod shuffle_vectors;
#[path = "tests/streaming_loads.rs"]
mod streaming_loads;
#[path = "tests/strings.rs"]
mod strings;
#[path = "tests/thread_exit.rs"]
mod thread_exit_tests;
#[path = "tests/threads.rs"]
mod threads;
#[path = "tests/timestamp_counter.rs"]
mod timestamp_counter_tests;
#[path = "tests/unpack_vectors.rs"]
mod unpack_vectors;
#[path = "tests/vector_masks.rs"]
mod vector_masks;
#[path = "tests/vectors.rs"]
mod vectors;
#[path = "tests/widen_vectors.rs"]
mod widen_vectors;
#[path = "tests/x87_control.rs"]
mod x87_controls;

use super::super::{
    DATA_OFF, ENTRY, PAGE, STACK_BYTES, STACK_OFF, STOP, build_elf, fresh_guest_space,
    original_image, stage_guest,
};
use super::*;
use crate::native_memory::{NativeAddressSpace, NativePermissions};
use pentax_codegen::{
    BlockLimits, TranslationPolicy, declare_materialized_source, plan_materialized_block,
};
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn with_source<T>(
    code: Option<&[u8]>,
    test: impl FnOnce(
        &MaterializedSource<'_, '_>,
        &mut NativeAddressSpace,
        NativeStackFrame,
        NativeCpuState,
    ) -> T,
) -> T {
    with_reserved_source(code, super::super::ARENA_BYTES, test)
}

fn with_reserved_source<T>(
    code: Option<&[u8]>,
    reserved_bytes: u64,
    test: impl FnOnce(
        &MaterializedSource<'_, '_>,
        &mut NativeAddressSpace,
        NativeStackFrame,
        NativeCpuState,
    ) -> T,
) -> T {
    let arena = NativeAddressSpace::reserve_any(Default::default(), reserved_bytes).unwrap();
    let bias = arena.reserved_ranges().next().unwrap().address;
    let image = code.map_or_else(original_image, build_elf);
    let inputs = [ModuleInput::new(open_executable(&image).unwrap(), bias)];
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let source = declare_materialized_source(&plan, &[], &BlockLimits::default()).unwrap();
    let (mut arena, rsp) = stage_guest(&plan, arena).unwrap();
    let stack = NativeStackFrame {
        stack: NativeRange {
            address: bias + STACK_OFF,
            size: STACK_BYTES,
        },
        guards: Some([
            NativeRange {
                address: bias + STACK_OFF - PAGE,
                size: PAGE,
            },
            NativeRange {
                address: bias + STACK_OFF + STACK_BYTES,
                size: PAGE,
            },
        ]),
        return_slot: rsp,
        return_marker: STOP,
    };
    test(
        &source,
        &mut arena,
        stack,
        NativeCpuState::new(plan.primary_entry(), rsp),
    )
}

#[test]
fn near_cache_search_handles_large_owned_reservations_with_an_explicit_finite_bound() {
    with_reserved_source(None, 32 * 1024 * 1024, |source, arena, stack, state| {
        let small = NativeDispatchLimits {
            cache_search_granules: 256,
            ..Default::default()
        };
        assert!(matches!(
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &small),
            Err(NativeDispatchError::Budget("near cache search"))
        ));
        let engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let base = source.source_plan().modules()[0].load_bias();
        assert!(engine.placement().address >= base + 32 * 1024 * 1024);
        assert!(engine.placement().address < base + 257 * 1024 * 1024);
        assert!(!engine.machine.cache.executable);
    });
}

#[test]
fn modeled_segment_loads_preserve_flags_partial_registers_and_address_semantics() {
    for (code, initial, base_offset, expected) in [
        (
            vec![0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0xc3],
            0,
            0,
            100,
        ),
        (
            vec![0x64, 0x8b, 0x04, 0x25, 8, 0, 0, 0, 0xc3],
            u64::MAX,
            0,
            0xccdd_ee11,
        ),
        (
            vec![0x64, 0x8a, 0x24, 0x25, 8, 0, 0, 0, 0xc3],
            0x1122_3344_5566_7788,
            0,
            0x1122_3344_5566_1188,
        ),
        (
            vec![0x64, 0x66, 0x8b, 0x04, 0x25, 8, 0, 0, 0, 0xc3],
            0x1122_3344_5566_7788,
            0,
            0x1122_3344_5566_ee11,
        ),
        (
            vec![0x64, 0x48, 0x8b, 0x04, 0x25, 0xf8, 0xff, 0xff, 0xff, 0xc3],
            0,
            8,
            100,
        ),
        (
            vec![0x64, 0x67, 0x48, 0x8b, 0x00, 0xc3],
            0xffff_ffff_0000_0008,
            0,
            0xaabb_bbcc_ccdd_ee11,
        ),
        (
            vec![0x64, 0x0f, 0xbe, 0x04, 0x25, 11, 0, 0, 0, 0xc3],
            u64::MAX,
            0,
            0xffff_ffcc,
        ),
        (
            vec![0x64, 0x48, 0x0f, 0xbe, 0x04, 0x25, 11, 0, 0, 0, 0xc3],
            0,
            0,
            0xffff_ffff_ffff_ffcc,
        ),
        (
            vec![0x64, 0x0f, 0xb6, 0x04, 0x25, 11, 0, 0, 0, 0xc3],
            u64::MAX,
            0,
            0xcc,
        ),
        (
            vec![0x64, 0x48, 0x63, 0x04, 0x25, 8, 0, 0, 0, 0xc3],
            0,
            0,
            0xffff_ffff_ccdd_ee11,
        ),
    ] {
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.fs_base = Some(data + base_offset);
            state.gpr[RAX] = initial;
            state.rflags |= 1 | (1 << 10);
            arena
                .pause()
                .unwrap()
                .write(data + 8, &0xaabb_bbcc_ccdd_ee11_u64.to_le_bytes())
                .unwrap();
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(matches!(
                planned.exit().kind(),
                ExitKind::Mediated {
                    reason: pentax_codegen::UnsupportedReason::SegmentPrefix
                }
            ));
            engine.machine.phase = Phase::Terminal;
            assert!(matches!(
                engine.emulate_segment(&mut arena.pause().unwrap(), &planned),
                Ok(NativeBlockStop::Resume { .. })
            ));
            assert_eq!(engine.machine.state.gpr[RAX], expected);
            assert_eq!(engine.machine.state.rflags, state.rflags);
            assert_eq!(engine.machine.state.fs_base, state.fs_base);
            assert_eq!(
                engine.machine.state.guest_pc,
                planned.exit().next_guest_pc()
            );
            assert_eq!(engine.machine.usage.segment_operations, 1);
            assert!(!engine.machine.cache.executable);
        });
    }
}

#[test]
fn modeled_segment_store_sign_extends_immediate_and_requires_valid_continuation() {
    for with_return in [true, false] {
        let mut code = vec![
            0x64, 0x48, 0xc7, 0x04, 0x25, 0x28, 0, 0, 0, 0xff, 0xff, 0xff, 0xff,
        ];
        if with_return {
            code.push(0xc3);
        }
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.fs_base = Some(data);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            let result = engine.emulate_segment(&mut arena.pause().unwrap(), &planned);
            let mut actual = [0; 8];
            arena
                .pause()
                .unwrap()
                .read(data + 0x28, &mut actual)
                .unwrap();
            if with_return {
                assert!(result.is_ok());
                assert_eq!(u64::from_le_bytes(actual), u64::MAX);
                assert_eq!(engine.machine.state.rflags, state.rflags);
            } else {
                assert!(matches!(result, Err(NativeDispatchError::InvalidTarget(_))));
                assert_eq!(actual, [0; 8]);
                assert_eq!(engine.machine.state, state);
                assert!(engine.machine.phase == Phase::Terminal);
            }
        });
    }
}

#[test]
fn modeled_segment_refusals_keep_state_and_output_unchanged() {
    for (code, configured, cross_page) in [
        (
            vec![0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0xc3],
            false,
            false,
        ),
        (
            vec![0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0xc3],
            true,
            true,
        ),
        (
            vec![0x64, 0x48, 0x8b, 0x24, 0x25, 0, 0, 0, 0, 0xc3],
            true,
            false,
        ), // RSP load must remain on guest stack.
        (vec![0xf0, 0x64, 0x48, 0x01, 0x00, 0xc3], true, false), // LOCK needs atomic semantics.
    ] {
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.fs_base = configured.then_some(if cross_page { data + PAGE - 4 } else { data });
            state.gpr[RAX] = 0;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            assert!(
                engine
                    .emulate_segment(&mut arena.pause().unwrap(), &planned)
                    .is_err()
            );
            assert_eq!(engine.machine.state, state);
            assert!(engine.machine.phase == Phase::Terminal);
            assert_eq!(engine.machine.usage.segment_operations, 0);
            let mut value = [0; 8];
            arena.pause().unwrap().read(data, &mut value).unwrap();
            assert_eq!(u64::from_le_bytes(value), 100);
        });
    }
}

#[test]
fn modeled_segment_work_budget_precedes_memory_effects() {
    with_source(
        Some(&[0x64, 0x48, 0x89, 0x04, 0x25, 0, 0, 0, 0, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.fs_base = Some(data);
            state.gpr[RAX] = 0xffff;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.limits.max_work_units = engine.machine.usage.work_units;
            let before = engine.machine.usage;
            assert_eq!(
                engine.emulate_segment(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Budget("segment work"))
            );
            assert_eq!(engine.machine.usage, before);
            let mut value = [0; 8];
            arena.pause().unwrap().read(data, &mut value).unwrap();
            assert_eq!(u64::from_le_bytes(value), 100);
        },
    );
}

#[test]
fn checked_memory_rejects_foreign_and_partial_cross_page_reads_without_publication() {
    with_source(
        Some(&[0xf3, 0x0f, 0x6f, 0x00, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let foreign = [0xa5_u8; 16];
            state.gpr[RAX] = foreign.as_ptr() as u64;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(matches!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Memory(_))
            ));
            assert_eq!(foreign, [0xa5; 16]);
            engine.machine.state.gpr[RAX] = data + PAGE - 8;
            assert!(matches!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Memory(_))
            ));
            arena
                .map_at(data + PAGE, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            engine
                .preflight_ordinary_memory(&arena.pause().unwrap(), &planned)
                .unwrap();
            assert!(!engine.machine.cache.executable);
            assert_eq!(engine.machine.state.gpr[RAX], data + PAGE - 8);
        },
    );
}

#[test]
fn checked_rmw_requires_both_live_permissions_and_preserves_actual_output() {
    with_source(
        Some(&[0x01, 0x08, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[RAX] = data;
            state.gpr[RCX] = 0xffff;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            for permissions in [
                NativePermissions::new(false, true, false),
                NativePermissions::READ,
            ] {
                arena.protect(data, PAGE, permissions).unwrap();
                assert!(matches!(
                    engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                    Err(NativeDispatchError::Memory(
                        NativeMemoryError::PermissionDenied(_)
                    ))
                ));
            }
            arena
                .protect(data, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            engine
                .preflight_ordinary_memory(&arena.pause().unwrap(), &planned)
                .unwrap();
            let mut value = [0; 8];
            arena.pause().unwrap().read(data, &mut value).unwrap();
            assert_eq!(u64::from_le_bytes(value), 100);
            assert_eq!(engine.machine.state, state);
            assert!(!engine.machine.cache.executable);
        },
    );
}

#[test]
fn checked_store_refuses_code_and_stale_multi_instruction_addresses() {
    for code in [
        &[0x48, 0x89, 0x00, 0xc3][..],
        &[0x48, 0x8b, 0x00, 0x48, 0x89, 0x08, 0xc3][..],
    ] {
        with_source(Some(code), |source, arena, stack, mut state| {
            state.gpr[RAX] = state.guest_pc;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            let rejected = engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned);
            if code.len() == 4 {
                assert!(matches!(
                    rejected,
                    Err(NativeDispatchError::Memory(
                        NativeMemoryError::PermissionDenied(_)
                    ))
                ));
            } else {
                assert_eq!(rejected, Err(NativeDispatchError::MemoryIsolationRequired));
            }
            let mut live = vec![0; code.len()];
            arena
                .pause()
                .unwrap()
                .fetch_source(state.guest_pc, &mut live)
                .unwrap();
            assert_eq!(live, code);
            assert_eq!(engine.machine.state, state);
            assert!(!engine.machine.cache.executable);
        });
    }
}

#[test]
fn checked_memory_analysis_is_bounded_before_visiting_instruction_metadata() {
    with_source(
        Some(&[0x48, 0x8b, 0x00, 0xc3]),
        |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.limits.max_work_units = engine.machine.usage.work_units;
            let before = engine.usage();
            assert_eq!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Budget("memory analysis"))
            );
            assert_eq!(engine.usage(), before);
            assert!(!engine.machine.cache.executable);
        },
    );
}

#[test]
fn independent_memory_admission_checks_every_access_and_rejects_stale_addresses() {
    // MOV EAX,[RDI]; MOV [RSI],EAX; RET. Both addresses are independent.
    with_source(
        Some(&[0x8b, 0x07, 0x89, 0x06, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[7] = data;
            state.gpr[6] = data + 8;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine
                .preflight_ordinary_memory(&arena.pause().unwrap(), &planned)
                .unwrap();
            // A later write to read-only code must reject the entire native entry.
            engine.machine.state.gpr[6] = state.guest_pc;
            assert!(matches!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Memory(
                    NativeMemoryError::PermissionDenied(_)
                ))
            ));
            let mut bytes = [0; 8];
            arena.pause().unwrap().read(data, &mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), 100);
            assert_eq!(engine.machine.usage.native_entries, 0);
        },
    );
    for code in [
        &[0xb0, 1, 0x8b, 0x08, 0xc3][..],             // AL changes RAX.
        &[0xb4, 1, 0x8b, 0x08, 0xc3][..],             // AH changes RAX.
        &[0xb8, 1, 0, 0, 0, 0x8b, 0x08, 0xc3][..],    // EAX zero-extends.
        &[0x0f, 0x44, 0xc7, 0x8b, 0x08, 0xc3][..],    // Conditional RAX write.
        &[0xf7, 0xe7, 0x8b, 0x0a, 0xc3][..],          // MUL implicitly writes RDX.
        &[0x50, 0x8b, 0x04, 0x24, 0xc3][..],          // PUSH changes RSP.
        &[0x83, 0xc1, 1, 0x8b, 0x04, 0x8f, 0xc3][..], // Changed index.
    ] {
        with_source(Some(code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.gpr[0] = data;
            state.gpr[1] = 0;
            state.gpr[2] = data;
            state.gpr[7] = data;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            // Deliberately use the unrestricted pure plan, not the split policy.
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert_eq!(
                engine.preflight_ordinary_memory(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::MemoryIsolationRequired)
            );
            assert_eq!(engine.machine.state, state);
            assert!(!engine.machine.cache.executable);
        });
    }
}

fn block<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    memory: &PausedNativeMemory<'_>,
    pc: u64,
    placement: CachePlacement,
) -> MaterializedBlockPlan<'view, 'plan, 'image> {
    let module = &source.source_plan().modules()[0];
    let segment = &module.runtime_plan().segments[0];
    let end = module.load_bias() + segment.vaddr + segment.file_bytes.len() as u64;
    let mut bytes = vec![0; (end - pc) as usize];
    memory.fetch_source(pc, &mut bytes).unwrap();
    plan_materialized_block(
        source,
        0,
        pc,
        &bytes,
        placement,
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap()
}

#[test]
fn state_profile_refuses_control_flags_and_unmasked_simd_exceptions() {
    let original = NativeCpuState::new(0x1000, 0x8000);
    validate_state(&original).unwrap();
    for bit in [8, 12, 14, 16, 17, 18, 21, 63] {
        let mut changed = original;
        changed.rflags |= 1 << bit;
        assert_eq!(
            validate_state(&changed),
            Err(NativeDispatchError::InvalidState)
        );
    }
    for bit in 7..13 {
        let mut changed = original;
        changed.mxcsr &= !(1 << bit);
        assert_eq!(
            validate_state(&changed),
            Err(NativeDispatchError::InvalidState)
        );
    }
    let mut changed = original;
    changed.mxcsr |= 1 << 31;
    assert_eq!(
        validate_state(&changed),
        Err(NativeDispatchError::InvalidState)
    );
}

#[test]
fn constructor_binds_actual_owner_and_requires_owned_guards_and_return_marker() {
    with_source(None, |source, arena, stack, state| {
        let identity = arena.owner_id();
        {
            let mut paused = arena.pause().unwrap();
            let engine =
                NativeBlockStepper::new(source, &mut paused, state, stack, &Default::default())
                    .unwrap();
            assert_eq!(engine.owner_id(), identity);
            assert_eq!(engine.state(), &state);
            assert!(
                engine.placement().capacity < NativeDispatchLimits::default().cache_bytes as u64
            );
            assert_eq!(engine.usage().blocks, 0);
            let mut wrong = stack;
            wrong.return_marker = 42;
            assert!(matches!(
                NativeBlockStepper::new(source, &mut paused, state, wrong, &Default::default()),
                Err(NativeDispatchError::InvalidStack)
            ));
        }
        arena
            .map_at(
                stack.guards.unwrap()[0].address,
                PAGE,
                NativePermissions::READ,
            )
            .unwrap();
        assert!(matches!(
            NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default()
            ),
            Err(NativeDispatchError::Memory(NativeMemoryError::Overlap))
        ));
    });
}

#[test]
fn constructor_charges_metadata_before_cache_and_refuses_executable_stack() {
    with_source(None, |source, arena, stack, state| {
        let limits = NativeDispatchLimits {
            max_work_units: 0,
            ..Default::default()
        };
        assert!(matches!(
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits),
            Err(NativeDispatchError::Budget("work"))
        ));
        arena
            .protect(
                stack.stack.address,
                stack.stack.size,
                NativePermissions::new(true, true, true),
            )
            .unwrap();
        assert!(matches!(
            NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default()
            ),
            Err(NativeDispatchError::InvalidStack)
        ));
    });
}

#[test]
fn publication_checks_live_bytes_permissions_and_owned_native_tail_without_execution() {
    with_source(None, |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        {
            let paused = arena.pause().unwrap();
            engine.preflight_block(&paused, &planned).unwrap();
            engine.publish(&paused, &planned).unwrap();
            assert!(engine.machine.cache.executable);
            assert_eq!(
                engine.usage().emitted_bytes,
                planned.bytes().len() as u64 + 12
            );
        }
        let code_page = source.source_plan().modules()[0].load_bias();
        arena
            .protect(code_page, PAGE, NativePermissions::new(true, true, true))
            .unwrap();
        assert_eq!(
            engine.publish(&arena.pause().unwrap(), &planned),
            Err(NativeDispatchError::WritableSource)
        );
        arena
            .protect(code_page, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        arena
            .pause()
            .unwrap()
            .write(state.guest_pc, &[0x90])
            .unwrap();
        arena
            .protect(code_page, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        assert_eq!(
            engine.publish(&arena.pause().unwrap(), &planned),
            Err(NativeDispatchError::SourceMismatch)
        );
    });
}

#[test]
fn admission_rejects_detached_declarations_replaced_owner_and_budget_before_execution() {
    with_source(None, |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let other = declare_materialized_source(source.source_plan(), &[], &BlockLimits::default())
            .unwrap();
        let detached = block(
            &other,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        assert_eq!(
            engine.preflight_block(&arena.pause().unwrap(), &detached),
            Err(NativeDispatchError::WrongSource)
        );
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let mut replacement = fresh_guest_space().unwrap();
        assert_eq!(
            engine.preflight_block(&replacement.pause().unwrap(), &planned),
            Err(NativeDispatchError::WrongOwner)
        );
        engine.machine.limits.max_emitted_bytes = planned.bytes().len() as u64 + 11;
        assert_eq!(
            engine.preflight_block(&arena.pause().unwrap(), &planned),
            Err(NativeDispatchError::Budget("emitted bytes"))
        );
        assert!(!engine.machine.cache.executable);
    });
}

#[test]
fn pre_syscall_token_preserves_original_rcx_all_six_args_and_full_id_and_rejects_replay() {
    with_source(
        Some(&[0x0f, 0x05, 0xc3]),
        |source, arena, stack, mut state| {
            state.gpr[RAX] = 0x10000002a;
            for (register, value) in [
                (RDI, 11),
                (RSI, 22),
                (RDX, 33),
                (RCX, 44),
                (8, 55),
                (9, 66),
                (10, 77),
            ] {
                state.gpr[register] = value;
            }
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            let NativeBlockStop::PreSyscall(token) = engine
                .resolve(&mut arena.pause().unwrap(), &planned)
                .unwrap()
            else {
                panic!("expected pre-syscall metadata");
            };
            assert_eq!(token.owner_id(), arena.owner_id());
            assert_eq!(token.service_id(), 0x10000002a);
            assert_eq!(token.args(), [11, 22, 33, 44, 55, 66]);
            assert_eq!(token.stack_pointer(), state.gpr[RSP]);
            assert_eq!(token.syscall_pc(), state.guest_pc);
            assert_eq!(token.next_pc(), state.guest_pc + 2);
            let mut other = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                other.complete_syscall(token, 7),
                Err(NativeDispatchError::WrongPhase)
            );
            engine.complete_syscall(token, 123).unwrap();
            assert_eq!(engine.machine.state.gpr[RAX], 123);
            assert_eq!(engine.machine.state.gpr[RCX], token.next_pc());
            assert_eq!(engine.machine.state.gpr[11], token.rflags());
            assert_eq!(
                engine.complete_syscall(token, 8),
                Err(NativeDispatchError::WrongPhase)
            );
        },
    );
}

#[test]
fn pre_syscall_vector_snapshot_preserves_all_raw_xmm_arguments_and_rejects_altered_token_replay() {
    with_source(
        Some(&[0x0f, 0x05, 0xc3]),
        |source, arena, stack, mut state| {
            for (register, bytes) in state.xmm.chunks_exact_mut(16).enumerate() {
                for (byte, value) in bytes.iter_mut().enumerate() {
                    *value = (register as u8)
                        .wrapping_mul(17)
                        .wrapping_add(byte as u8)
                        .wrapping_add(1);
                }
            }
            state.gpr[RAX] = 0x1234_5678_9abc_def0;
            state.gpr[RDI] = 0xfedc_ba98_7654_3210;
            let expected: [[u8; 16]; 8] = std::array::from_fn(|register| {
                std::array::from_fn(|byte| (register * 17 + byte + 1) as u8)
            });
            assert!(
                expected
                    .iter()
                    .all(|bytes| bytes.iter().all(|&byte| byte != 0))
            );
            assert!(
                expected
                    .windows(2)
                    .all(|registers| registers[0] != registers[1])
            );
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            let NativeBlockStop::PreSyscall(token) = engine
                .resolve(&mut arena.pause().unwrap(), &planned)
                .unwrap()
            else {
                panic!("expected original PRE-SYSCALL vector snapshot");
            };
            assert_eq!(token.vector_args(), expected);
            assert_eq!(token.service_id(), state.gpr[RAX]);
            assert_eq!(token.args()[0], state.gpr[RDI]);
            assert_eq!(token.owner_id(), arena.owner_id());
            let mut detached = token.vector_args();
            detached[0].fill(0);
            assert_ne!(detached, token.vector_args());
            let mut altered = token;
            altered.vector_args[7][15] ^= 0x80;
            assert_eq!(
                engine.complete_syscall(altered, 17),
                Err(NativeDispatchError::WrongPhase)
            );
            assert_eq!(*engine.state(), state);
            assert_eq!(token.vector_args(), expected);
            engine.complete_syscall(token, 42).unwrap();
            assert_eq!(engine.state().xmm, state.xmm); // Includes untouched XMM8..15.
            assert_eq!(token.vector_args(), expected);
            assert_eq!(
                engine.complete_syscall(token, 99),
                Err(NativeDispatchError::WrongPhase)
            );
            assert_eq!(engine.state().gpr[RAX], 42);
            assert_eq!(engine.state().xmm, state.xmm);
        },
    );
}

#[test]
fn return_exit_boundary_refuses_native_bodies_other_exits_and_adjusted_returns_before_admission() {
    for code in [
        [0x0f, 0x05].as_slice(),
        [0xb8, 1, 0, 0, 0, 0xc3].as_slice(),
        [0xc2, 8, 0].as_slice(),
    ] {
        with_source(Some(code), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(matches!(
                engine.step_return_exit(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::EmissionMismatch)
            ));
            assert_eq!(*engine.state(), state);
            assert!(matches!(engine.machine.phase, Phase::Ready));
            assert_eq!(engine.usage().blocks, 0);
            assert_eq!(engine.usage().native_entries, 0);
        });
    }
}

#[test]
fn pending_syscall_validation_refuses_foreign_stale_and_consumed_tokens_without_advancing() {
    for code in [
        [0xc3].as_slice(),
        [0xb8, 1, 0, 0, 0, 0x0f, 0x05, 0xc3].as_slice(),
    ] {
        with_source(Some(code), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(matches!(
                engine.step_syscall_exit(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::EmissionMismatch)
            ));
            assert_eq!(*engine.state(), state);
            assert!(matches!(engine.machine.phase, Phase::Ready));
            assert_eq!(engine.usage().blocks, 0);
            assert_eq!(engine.usage().native_entries, 0);
        });
    }
    with_source(Some(&[0x0f, 0x05, 0xc3]), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let NativeBlockStop::PreSyscall(token) = engine
            .step_syscall_exit(&mut arena.pause().unwrap(), &planned)
            .unwrap()
        else {
            panic!("expected original pre-syscall metadata");
        };
        assert_eq!(engine.usage().blocks, 1);
        assert_eq!(engine.usage().native_entries, 0);
        assert_eq!(engine.usage().cache_publications, 0);
        let mut other = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            other.validate_pending_syscall(token),
            Err(NativeDispatchError::WrongPhase)
        );
        assert!(matches!(other.machine.phase, Phase::Ready));
        assert_eq!(*other.state(), state);
        // Even a matching phase alone cannot substitute another executor.
        other.machine.phase = Phase::Syscall(token);
        assert_eq!(
            other.validate_pending_syscall(token),
            Err(NativeDispatchError::WrongPhase)
        );
        assert!(other.machine.phase == Phase::Syscall(token));
        let before = *engine.state();
        let mut stale = token;
        stale.sequence += 1;
        assert_eq!(
            engine.validate_pending_syscall(stale),
            Err(NativeDispatchError::WrongPhase)
        );
        let mut altered = token;
        altered.vector_args[7][15] ^= 0x80;
        assert_eq!(
            engine.validate_pending_syscall(altered),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(*engine.state(), before);
        assert!(engine.machine.phase == Phase::Syscall(token));
        engine.validate_pending_syscall(token).unwrap();
        engine.validate_pending_syscall(token).unwrap();
        assert_eq!(*engine.state(), before);
        assert!(engine.machine.phase == Phase::Syscall(token));
        engine.complete_syscall(token, 42).unwrap();
        let completed = *engine.state();
        assert_eq!(
            engine.validate_pending_syscall(token),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(*engine.state(), completed);
        assert!(matches!(engine.machine.phase, Phase::Ready));
    });
}

#[test]
fn mediated_call_is_transactional_on_failure_and_outer_return_requires_exact_slot() {
    with_source(None, |source, arena, stack, mut state| {
        let bias = source.source_plan().modules()[0].load_bias();
        state.guest_pc = bias + ENTRY as u64 + 44;
        state.gpr[11] = bias + ENTRY as u64 + 53;
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let call = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let minimum = stack.stack.address + 128;
        engine.machine.state.gpr[RSP] = minimum;
        assert_eq!(
            engine.resolve(&mut arena.pause().unwrap(), &call),
            Err(NativeDispatchError::InvalidStack)
        );
        assert_eq!(engine.machine.state.gpr[RSP], minimum);
        engine.machine.state.gpr[RSP] = state.gpr[RSP];
        assert!(matches!(
            engine.resolve(&mut arena.pause().unwrap(), &call),
            Ok(NativeBlockStop::Resume { .. })
        ));
        let mut actual = [0; 8];
        arena
            .pause()
            .unwrap()
            .read(state.gpr[RSP] - 8, &mut actual)
            .unwrap();
        assert_eq!(u64::from_le_bytes(actual), bias + ENTRY as u64 + 47);
        engine.machine.state.guest_pc = bias + ENTRY as u64 + 55;
        let ret = block(
            source,
            &arena.pause().unwrap(),
            engine.machine.state.guest_pc,
            engine.placement(),
        );
        arena
            .pause()
            .unwrap()
            .write(state.gpr[RSP] - 8, &STOP.to_le_bytes())
            .unwrap();
        assert_eq!(
            engine.resolve(&mut arena.pause().unwrap(), &ret),
            Err(NativeDispatchError::InvalidTarget(STOP))
        );
        engine.machine.state.gpr[RSP] = stack.return_slot;
        engine.machine.state.gpr[RAX] = 42;
        assert_eq!(
            engine.resolve(&mut arena.pause().unwrap(), &ret),
            Ok(NativeBlockStop::Returned { value: 42 })
        );
        assert!(
            engine.observed_return().is_none(),
            "data-only exit resolution is not execution evidence"
        );
    });
}

#[test]
fn another_call_requires_a_completed_return_and_keeps_owner_context_and_cumulative_limits() {
    with_source(Some(&[0xc3]), |source, arena, stack, mut state| {
        state.fs_base = Some(stack.stack.address);
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), state),
            Err(NativeDispatchError::WrongPhase)
        );
        engine.machine.phase = Phase::Terminal;
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), state),
            Err(NativeDispatchError::WrongPhase)
        );
        let ret = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        assert!(matches!(
            engine.resolve(&mut arena.pause().unwrap(), &ret),
            Ok(NativeBlockStop::Returned { .. })
        ));
        assert!(engine.machine.phase == Phase::Returned);
        let returned = *engine.state();
        let mut other = NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
        assert_eq!(
            engine.begin_call(&mut other.pause().unwrap(), state),
            Err(NativeDispatchError::WrongOwner)
        );
        let mut wrong = state;
        wrong.fs_base = None;
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), wrong),
            Err(NativeDispatchError::InvalidState)
        );
        wrong = state;
        wrong.gpr[RSP] += 8;
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), wrong),
            Err(NativeDispatchError::InvalidStack)
        );
        arena
            .pause()
            .unwrap()
            .write(stack.return_slot, &(stack.return_marker ^ 1).to_le_bytes())
            .unwrap();
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), state),
            Err(NativeDispatchError::InvalidStack)
        );
        assert_eq!(*engine.state(), returned);
        arena
            .pause()
            .unwrap()
            .write(stack.return_slot, &stack.return_marker.to_le_bytes())
            .unwrap();
        engine.machine.usage.blocks = 7;
        engine.machine.usage.native_instructions = 11;
        engine.machine.usage.memory_access_checks = 13;
        let work = engine.machine.usage.work_units;
        engine
            .begin_call(&mut arena.pause().unwrap(), state)
            .unwrap();
        assert_eq!(*engine.state(), state);
        assert!(engine.machine.phase == Phase::Ready);
        assert_eq!(engine.machine.usage.blocks, 7);
        assert_eq!(engine.machine.usage.native_instructions, 11);
        assert_eq!(engine.machine.usage.memory_access_checks, 13);
        assert!(engine.machine.usage.work_units > work);
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), state),
            Err(NativeDispatchError::WrongPhase)
        );
        engine.resolve(&mut arena.pause().unwrap(), &ret).unwrap();
        engine.machine.limits.max_work_units = engine.machine.usage.work_units;
        assert!(matches!(
            engine.begin_call(&mut arena.pause().unwrap(), state),
            Err(NativeDispatchError::Budget(_))
        ));
        assert!(engine.machine.phase == Phase::Returned);
    });
}

#[test]
fn detached_machine_reattaches_without_replacing_cache_context_or_usage_and_refuses_other_owners() {
    with_source(Some(&[0xc3]), |source, arena, stack, state| {
        let engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let address = engine.placement().address;
        let usage = engine.usage();
        let owner = engine.owner_id();
        let mut slot = Some(engine.into_machine());
        let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
        assert!(matches!(
            NativeBlockStepper::from_machine(source, &mut foreign.pause().unwrap(), &mut slot),
            Err(NativeDispatchError::WrongOwner)
        ));
        assert_eq!(slot.as_ref().unwrap().owner_id(), owner);
        let mut engine =
            NativeBlockStepper::from_machine(source, &mut arena.pause().unwrap(), &mut slot)
                .unwrap();
        assert!(slot.is_none());
        assert_eq!(engine.placement().address, address);
        assert_eq!(engine.owner_id(), owner);
        assert_eq!(*engine.state(), state);
        assert!(engine.usage().work_units > usage.work_units);
        assert_eq!(engine.usage().blocks, usage.blocks);
        assert!(engine.observed_return().is_none());
        engine.machine.phase = Phase::Terminal;
        slot = Some(engine.into_machine());
        assert!(matches!(
            NativeBlockStepper::from_machine(source, &mut arena.pause().unwrap(), &mut slot),
            Err(NativeDispatchError::WrongPhase)
        ));
        assert!(slot.is_some());
    });
}

#[test]
fn nested_call_preserves_caller_red_zone_and_data_only_return_cannot_resume_its_syscall() {
    with_source(Some(&[0x0f, 0x05, 0xc3]), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let syscall = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        let NativeBlockStop::PreSyscall(token) = engine
            .resolve(&mut arena.pause().unwrap(), &syscall)
            .unwrap()
        else {
            panic!("expected data-only syscall exit");
        };
        let red_zone = state.gpr[RSP] - 128;
        let expected = [0x55; 128];
        arena.pause().unwrap().write(red_zone, &expected).unwrap();
        let mut wrong = token;
        wrong.executor += 1;
        assert_eq!(
            engine.begin_syscall_call(
                &mut arena.pause().unwrap(),
                wrong,
                state.guest_pc + 2,
                [7, red_zone, 0]
            ),
            Err(NativeDispatchError::WrongPhase)
        );
        let nested = engine
            .begin_syscall_call(
                &mut arena.pause().unwrap(),
                token,
                state.guest_pc + 2,
                [7, red_zone, 0],
            )
            .unwrap();
        assert_eq!(
            [
                engine.state().gpr[RDI],
                engine.state().gpr[RSI],
                engine.state().gpr[RDX]
            ],
            [7, red_zone, 0]
        );
        assert_eq!(engine.state().gpr[RSP] % 16, 8);
        assert!(engine.state().gpr[RSP] + 8 <= red_zone);
        let mut actual = [0; 128];
        arena.pause().unwrap().read(red_zone, &mut actual).unwrap();
        assert_eq!(actual, expected);
        let ret = block(
            source,
            &arena.pause().unwrap(),
            engine.state().guest_pc,
            engine.placement(),
        );
        assert!(matches!(
            engine.resolve(&mut arena.pause().unwrap(), &ret),
            Ok(NativeBlockStop::Returned { .. })
        ));
        assert!(engine.observed_return().is_none());
        assert_eq!(
            engine.complete_syscall_call(&mut arena.pause().unwrap(), nested, 2),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(
            engine.continue_syscall_call(
                &mut arena.pause().unwrap(),
                nested,
                state.guest_pc + 2,
                [1, red_zone, 0],
            ),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(
            engine.begin_call(&mut arena.pause().unwrap(), state),
            Err(NativeDispatchError::WrongPhase)
        );
        assert_eq!(engine.machine.suspended.len(), 1);
    });
}
