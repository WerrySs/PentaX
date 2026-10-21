// SPDX-License-Identifier: GPL-2.0-only
//! Actual original temporal loads/stores and independent live-memory refusals.

use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

#[derive(Clone, Copy)]
struct Form {
    opcode: u8,
    prefix: u8,
    size: usize,
    write: bool,
    aligned: bool,
}
const fn form(opcode: u8, prefix: u8, size: usize, write: bool, aligned: bool) -> Form {
    Form {
        opcode,
        prefix,
        size,
        write,
        aligned,
    }
}
const FORMS: [Form; 28] = [
    form(0x6f, 1, 16, false, true),
    form(0x7f, 1, 16, true, true),
    form(0x6f, 1, 32, false, true),
    form(0x7f, 1, 32, true, true),
    form(0x6f, 2, 16, false, false),
    form(0x7f, 2, 16, true, false),
    form(0x6f, 2, 32, false, false),
    form(0x7f, 2, 32, true, false),
    form(0x28, 0, 16, false, true),
    form(0x29, 0, 16, true, true),
    form(0x28, 0, 32, false, true),
    form(0x29, 0, 32, true, true),
    form(0x10, 0, 16, false, false),
    form(0x11, 0, 16, true, false),
    form(0x10, 0, 32, false, false),
    form(0x11, 0, 32, true, false),
    form(0x28, 1, 16, false, true),
    form(0x29, 1, 16, true, true),
    form(0x28, 1, 32, false, true),
    form(0x29, 1, 32, true, true),
    form(0x10, 1, 16, false, false),
    form(0x11, 1, 16, true, false),
    form(0x10, 1, 32, false, false),
    form(0x11, 1, 32, true, false),
    form(0x10, 2, 4, false, false),
    form(0x11, 2, 4, true, false),
    form(0x10, 3, 8, false, false),
    form(0x11, 3, 8, true, false),
];
const CONTROLS: [u32; 4] = [0x1f80, 0x3fa5, 0xdfbf, 0xffbf];
const PAYLOAD: [u8; 32] = [
    0x72, 0x00, 0xa0, 0x7f, 0x00, 0x00, 0xf0, 0x7f, 0x72, 0x00, 0xc0, 0xff, 0x00, 0x00, 0xf8, 0xff,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe,
];

fn policy() -> TranslationPolicy {
    TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers()
        .with_native_vector_moves()
}
fn encoding(form: Form, vector: u8, legacy: bool) -> Vec<u8> {
    let mut code = Vec::new();
    if legacy {
        if form.prefix == 1 {
            code.push(0x66);
        }
        if vector >= 8 {
            code.push(0x44);
        }
        code.push(0x0f);
    } else {
        code.extend_from_slice(&[
            0xc4,
            if vector >= 8 { 0x61 } else { 0xe1 },
            0x78 | u8::from(form.size == 32) << 2 | form.prefix,
        ]);
    }
    code.extend_from_slice(&[form.opcode, ((vector & 7) << 3) | 3, 0xc3]);
    code
}
fn input(mut state: NativeCpuState, address: u64, control: u32) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[3] = address;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control;
    for register in 0..16 {
        for word in 0..4 {
            let value = [
                0x7ff0_0000_7fa0_0031_u64,
                0x8000_0000_ffc0_0011,
                0xfff8_0000_0000_0011,
                0x0123_4567_89ab_cdef,
            ][word]
                ^ ((register as u64) << 16);
            let bank = if word < 2 {
                &mut state.xmm
            } else {
                &mut state.ymm_upper
            };
            let at = register * 16 + word % 2 * 8;
            bank[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
    }
    state
}
fn expected(mut state: NativeCpuState, form: Form, vector: usize, legacy: bool) -> NativeCpuState {
    if !form.write {
        let at = vector * 16;
        state.xmm[at..at + 16].fill(0);
        state.xmm[at..at + form.size.min(16)].copy_from_slice(&PAYLOAD[..form.size.min(16)]);
        if !legacy {
            state.ymm_upper[at..at + 16].fill(0);
            if form.size == 32 {
                state.ymm_upper[at..at + 16].copy_from_slice(&PAYLOAD[16..]);
            }
        }
    }
    state.guest_pc = STOP;
    state.gpr[RSP] += 8;
    state
}

#[test]
fn native_vector_memory_all_28_forms_copy_raw_bits_and_preserve_complete_state() {
    if VectorMode::detect().features().is_none() {
        println!(
            "native_vector_memory_available=false native_vector_memory_cases=0 native_vector_memory_mode=sse2"
        );
        return;
    }
    let controls = CONTROLS
        .iter()
        .filter(|control| mxcsr_control::valid(**control))
        .count() as u64;
    let mut cases = 0_u64;
    for form in FORMS {
        for vector in [0_u8, 13, 15] {
            let code = encoding(form, vector, false);
            with_source(Some(&code), |source, arena, stack, state| {
                let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let first = page + if form.aligned { 64 } else { 65 };
                let initial = input(state, first, CONTROLS[0]);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    initial,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                // Actual AVX was independently admitted above. AVX2 is never
                // required for these moves, including integer YMM DQA/DQU.
                engine.machine.vector_mode = VectorMode::Avx {
                    avx2: false,
                    fma: false,
                };
                let plan = plan_materialized_block(
                    source,
                    0,
                    state.guest_pc,
                    &code,
                    engine.placement(),
                    &policy(),
                    &BlockLimits::default(),
                )
                .unwrap();
                assert!(plan.instructions()[0].emitted_len() > 0);
                assert!(plan.emits_vector_memory_move());
                assert!(!plan.emits_non_temporal_store());
                assert_eq!(
                    plan.instructions()[0].instruction().memory_size().size(),
                    form.size
                );
                let mut completed = 0;
                for address in [first, page + PAGE - form.size as u64] {
                    for control in CONTROLS {
                        if !mxcsr_control::valid(control) {
                            continue;
                        }
                        let current = input(state, address, control);
                        if completed != 0 {
                            engine
                                .begin_call(&mut arena.pause().unwrap(), current)
                                .unwrap();
                        } else {
                            assert_eq!(*engine.state(), current);
                        }
                        arena
                            .protect(page, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                        arena
                            .pause()
                            .unwrap()
                            .write(address - 16, &[0xa5; 16])
                            .unwrap();
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &PAYLOAD[..form.size])
                            .unwrap();
                        if address == first {
                            arena
                                .pause()
                                .unwrap()
                                .write(address + form.size as u64, &[0x3c; 16])
                                .unwrap();
                        }
                        let permissions = NativePermissions::new(!form.write, form.write, false);
                        arena.protect(page, PAGE, permissions).unwrap();
                        let wanted = expected(current, form, usize::from(vector), false);
                        assert_eq!(
                            // SAFETY: Original initialized source/stack; every
                            // current exact read/write span is independently checked.
                            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }
                                .unwrap(),
                            NativeBlockStop::Returned { value: 42 }
                        );
                        assert_eq!(*engine.state(), wanted);
                        arena
                            .protect(page, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                        let mut bytes = [0; 48];
                        arena
                            .pause()
                            .unwrap()
                            .read(address - 16, &mut bytes[..form.size + 16])
                            .unwrap();
                        assert_eq!(&bytes[..16], &[0xa5; 16]);
                        if form.write {
                            let at = usize::from(vector) * 16;
                            assert_eq!(
                                &bytes[16..16 + form.size.min(16)],
                                &current.xmm[at..at + form.size.min(16)]
                            );
                            if form.size == 32 {
                                assert_eq!(&bytes[32..48], &current.ymm_upper[at..at + 16]);
                            }
                        } else {
                            assert_eq!(&bytes[16..16 + form.size], &PAYLOAD[..form.size]);
                        }
                        if address == first {
                            let mut tail = [0; 16];
                            arena
                                .pause()
                                .unwrap()
                                .read(address + form.size as u64, &mut tail)
                                .unwrap();
                            assert_eq!(tail, [0x3c; 16]);
                        }
                        completed += 1;
                        cases += 1;
                        assert_eq!(engine.usage().native_entries, completed);
                        assert_eq!(engine.usage().native_instructions, completed);
                        assert_eq!(engine.usage().memory_access_checks, completed);
                        assert_eq!(
                            engine.usage().memory_bytes_checked,
                            completed * form.size as u64
                        );
                        assert_eq!(engine.usage().vector_operations, 0);
                    }
                }
                // A sealed native body never authorizes its later operand.
                // Re-arm the same call, revoke exactly its required right,
                // and prove that no new body or destination effect occurs.
                let current = input(state, first, CONTROLS[0]);
                engine
                    .begin_call(&mut arena.pause().unwrap(), current)
                    .unwrap();
                let mut before_bytes = [0; 32];
                arena
                    .pause()
                    .unwrap()
                    .read(first, &mut before_bytes[..form.size])
                    .unwrap();
                arena
                    .protect(
                        page,
                        PAGE,
                        NativePermissions::new(form.write, !form.write, false),
                    )
                    .unwrap();
                assert!(matches!(
                    // SAFETY: Same original owner; the deliberate denied right
                    // must be detected before any native execution/cache reuse.
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) },
                    Err(NativeDispatchError::Memory(
                        NativeMemoryError::PermissionDenied(_)
                    ))
                ));
                assert_eq!(*engine.state(), current);
                assert_eq!(engine.usage().native_entries, completed);
                arena
                    .protect(page, PAGE, NativePermissions::READ_WRITE)
                    .unwrap();
                let mut after_bytes = [0; 32];
                arena
                    .pause()
                    .unwrap()
                    .read(first, &mut after_bytes[..form.size])
                    .unwrap();
                assert_eq!(after_bytes, before_bytes);
                assert_eq!(engine.usage().cache_publications, 1);
                assert_eq!(engine.usage().cache_reuses, completed - 1);
            });
        }
    }
    assert_eq!(cases, 28 * 3 * 2 * controls);
    println!(
        "native_vector_memory_available=true native_vector_memory_cases={cases} native_vector_memory_mode=avx"
    );
}

#[test]
fn vector_memory_full_spans_rights_owners_and_modes_refuse_before_native_entry() {
    // Both directions at every admitted width; DQA supplies aligned128/256,
    // UPS supplies arbitrary packed addresses, SS/SD exact4/8-byte operands.
    for form in [
        FORMS[0], FORMS[3], FORMS[14], FORMS[13], FORMS[24], FORMS[25], FORMS[26], FORMS[27],
    ] {
        for manual in [false, true] {
            for case in 0..12 {
                let code = encoding(form, 13, false);
                with_source(Some(&code), |source, arena, stack, state| {
                    let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let address = match case {
                        2 => page + PAGE,
                        3 => page + PAGE - form.size as u64 + 1,
                        10 => u64::MAX - form.size as u64 + 1,
                        _ => page + 64,
                    };
                    let current = input(state, address, CONTROLS[0]);
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        current,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    let plan = plan_materialized_block(
                        source,
                        0,
                        state.guest_pc,
                        &code,
                        engine.placement(),
                        &policy(),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    assert!(plan.emits_vector_memory_move());
                    assert!(plan.instructions()[0].emitted_len() > 0);
                    let mut bytes = [0; 128];
                    arena.pause().unwrap().read(page, &mut bytes).unwrap();
                    let wanted = bytes;
                    match case {
                        0 => arena
                            .protect(
                                page,
                                PAGE,
                                NativePermissions::new(form.write, !form.write, false),
                            )
                            .unwrap(),
                        1 => arena
                            .protect(page, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        5 => arena
                            .protect(
                                state.guest_pc & !(PAGE - 1),
                                PAGE,
                                NativePermissions::READ_WRITE,
                            )
                            .unwrap(),
                        6 => engine.machine.vector_mode = VectorMode::Sse2,
                        7 => engine.machine.state.rflags |= 1 << 18,
                        8 => engine.machine.state.mxcsr &= !0x80,
                        9 => engine.machine.limits.max_work_units = 0,
                        11 => engine.machine.usage.native_instructions = u64::MAX,
                        _ => {}
                    }
                    let before = *engine.state();
                    let mut foreign = if case == 4 {
                        Some(NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap())
                    } else {
                        None
                    };
                    let result = {
                        let mut paused = match foreign.as_mut() {
                            Some(owner) => owner.pause().unwrap(),
                            None => arena.pause().unwrap(),
                        };
                        // SAFETY: Original source; every deliberately bad
                        // operand/mode/state must refuse before native entry.
                        unsafe {
                            if manual {
                                engine.step(&mut paused, &plan)
                            } else {
                                engine.step_checked(&mut paused, &plan)
                            }
                        }
                    };
                    assert!(
                        result.is_err(),
                        "width={} write={} case={case} manual={manual}",
                        form.size,
                        form.write
                    );
                    match case {
                        0 | 1 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::Memory(
                                NativeMemoryError::PermissionDenied(_)
                            ))
                        )),
                        2 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::Memory(NativeMemoryError::Unmapped(_)))
                        )),
                        3 if form.aligned => assert!(matches!(
                            result,
                            Err(NativeDispatchError::MemoryFootprint {
                                reason: MemoryAccessError::UnalignedVector,
                                ..
                            })
                        )),
                        3 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::Memory(NativeMemoryError::Unmapped(_)))
                        )),
                        4 => assert_eq!(result, Err(NativeDispatchError::WrongOwner)),
                        10 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::MemoryFootprint {
                                reason: MemoryAccessError::InvalidRange,
                                ..
                            })
                        )),
                        _ => {}
                    }
                    assert_eq!(*engine.state(), before);
                    assert_eq!(engine.usage().native_entries, 0);
                    assert_eq!(engine.usage().cache_publications, 0);
                    arena
                        .protect(page, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    arena.pause().unwrap().read(page, &mut bytes).unwrap();
                    assert_eq!(bytes, wanted);
                });
            }
        }
    }
}

#[test]
fn six_legacy_aligned_moves_and_both_publication_profile_gates_remain_checked() {
    for form in [FORMS[0], FORMS[1], FORMS[8], FORMS[9], FORMS[16], FORMS[17]] {
        let code = encoding(form, 13, true);
        with_source(Some(&code), |source, arena, stack, state| {
            let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let initial = input(state, page + 64, CONTROLS[0]);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            let plan = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            )
            .unwrap();
            assert!(plan.emits_vector_memory_move());
            assert!(!plan.emits_non_temporal_store());
            arena
                .pause()
                .unwrap()
                .write(page + 64, &PAYLOAD[..16])
                .unwrap();
            assert_eq!(
                // SAFETY: Actual original SSE2 aligned load/store; even manual
                // stepping now independently validates this six-code family's span.
                unsafe { engine.step(&mut arena.pause().unwrap(), &plan) }.unwrap(),
                NativeBlockStop::Returned { value: 42 }
            );
            assert_eq!(*engine.state(), expected(initial, form, 13, true));
            let mut observed = [0; 16];
            arena
                .pause()
                .unwrap()
                .read(page + 64, &mut observed)
                .unwrap();
            assert_eq!(
                &observed[..],
                if form.write {
                    &initial.xmm[208..224]
                } else {
                    &PAYLOAD[..16]
                }
            );
            let bad = input(state, page + 65, CONTROLS[0]);
            engine.begin_call(&mut arena.pause().unwrap(), bad).unwrap();
            assert!(matches!(
                // SAFETY: Deliberately misaligned original legacy operand, refused
                // by the independent footprint analysis before native execution.
                unsafe { engine.step(&mut arena.pause().unwrap(), &plan) },
                Err(NativeDispatchError::MemoryFootprint {
                    reason: MemoryAccessError::UnalignedVector,
                    ..
                })
            ));
            assert_eq!(*engine.state(), bad);
            assert_eq!(engine.usage().native_entries, 1);
            assert_eq!(engine.usage().cache_publications, 1);

            // The flag is a request, not an authority. Missing full-YMM and
            // forcing the SSE2-only mode each fail independently in BOTH
            // publication entrypoints, even for otherwise native legacy code.
            for requested in [
                TranslationPolicy::x64_sse2().with_native_vector_moves(),
                policy(),
            ] {
                engine.machine.vector_mode = VectorMode::Sse2;
                let prepared = plan_materialized_block(
                    source,
                    0,
                    state.guest_pc,
                    &code,
                    engine.placement(),
                    &requested,
                    &BlockLimits::default(),
                )
                .unwrap();
                assert_eq!(
                    engine.publish(&arena.pause().unwrap(), &prepared),
                    Err(NativeDispatchError::InvalidState)
                );
                let diagnostic = pentax_codegen::plan_block(
                    source.source_plan(),
                    0,
                    state.guest_pc,
                    engine.placement(),
                    &requested,
                    &BlockLimits::default(),
                )
                .unwrap();
                assert!(matches!(
                    super::super::super::publish_block_with_mode(
                        &mut engine.machine.cache,
                        arena,
                        &diagnostic,
                        state.guest_pc,
                        VectorMode::Sse2,
                    ),
                    Err(PlatformError::UnsupportedHostFeature {
                        feature: "native YMM context"
                    })
                ));
            }
            assert_eq!(engine.usage().native_entries, 1);
            assert_eq!(engine.usage().cache_publications, 1);
        });
    }
}
