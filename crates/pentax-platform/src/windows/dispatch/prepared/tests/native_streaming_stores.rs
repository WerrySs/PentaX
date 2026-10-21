// SPDX-License-Identifier: GPL-2.0-only
//! Actual original non-temporal writes, ordered before Rust observations.

use super::*;
use pentax_codegen::CpuidFeature;

fn encoding(form: usize, source: u8) -> Vec<u8> {
    let mut bytes = Vec::new();
    if form < 3 {
        if form != 1 {
            bytes.push(0x66);
        }
        if source >= 8 {
            bytes.push(0x44);
        }
        bytes.extend_from_slice(&[0x0f, if form == 0 { 0xe7 } else { 0x2b }]);
    } else {
        bytes.extend_from_slice(&[
            0xc4,
            if source >= 8 { 0x61 } else { 0xe1 },
            if form == 4 { 0x7d } else { 0x79 },
            0xe7,
        ]);
    }
    bytes.push(((source & 7) << 3) | 3); // [RBX], source XMM/YMM.
    bytes.push(0xc3);
    bytes
}

fn policy(form: usize) -> TranslationPolicy {
    if form < 3 {
        TranslationPolicy::x64_sse2()
    } else {
        TranslationPolicy::new(pentax_codegen::CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
            .with_native_ymm_registers()
            .with_native_streaming_stores()
    }
}

fn input(mut state: NativeCpuState, address: u64, control: u32) -> NativeCpuState {
    for (index, register) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *register = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[3] = address;
    state.mxcsr = control;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    for register in 0..16 {
        for word in 0..4 {
            // Actual raw signaling/quiet NaNs, signed zero and unrelated tags;
            // these stores must neither quiet bits nor raise FP exceptions.
            let value = [
                0x7ff0_0000_0000_0031_u64,
                0x7fa0_0072_ffc0_0011,
                0x8000_0000_0000_0000,
                0xfedc_ba98_7654_3210,
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

#[test]
fn native_streaming_stores_retire_exact_bytes_and_preserve_complete_source_context() {
    let features = VectorMode::detect().features();
    let mut cases = 0_u64;
    let mut controls = 0_u64;
    for control in [0x1f80, 0x3fa5, 0xdfbf, 0xffbf] {
        controls += u64::from(mxcsr_control::valid(control));
    }
    for form in 0..5 {
        if form >= 3 && features.is_none() {
            continue;
        }
        for source_register in [0_u8, 13, 15] {
            let code = encoding(form, source_register);
            with_source(Some(&code), |source, arena, stack, state| {
                let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let size = if form == 4 { 32 } else { 16 };
                let initial = input(state, page + 64, 0x1f80);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    initial,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                if features.is_some() {
                    // This profile needs AVX, not AVX2. Restrict the private
                    // test mode only after actual AVX+OS/XCR0 admission.
                    engine.machine.vector_mode = VectorMode::Avx { avx2: false };
                }
                let planned = plan_materialized_block(
                    source,
                    0,
                    state.guest_pc,
                    &code,
                    engine.placement(),
                    &policy(form),
                    &BlockLimits::default(),
                )
                .unwrap();
                assert!(planned.emits_non_temporal_store());
                assert_eq!(
                    planned.instructions()[0].instruction().memory_size().size(),
                    size
                );
                assert!(planned.instructions()[0].emitted_len() > 0);
                let mut completed = 0;
                for address in [page + 64, page + PAGE - size as u64] {
                    for control in [0x1f80, 0x3fa5, 0xdfbf, 0xffbf] {
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
                            .write(address, &[0x5a; 32][..size])
                            .unwrap();
                        if address == page + 64 {
                            arena
                                .pause()
                                .unwrap()
                                .write(address + size as u64, &[0x3c; 16])
                                .unwrap();
                        }
                        // The source CPU need not have data-read permission
                        // on a write-only destination. Restore it only to inspect.
                        arena
                            .protect(page, PAGE, NativePermissions::new(false, true, false))
                            .unwrap();
                        let mut wanted = current;
                        wanted.guest_pc = STOP;
                        wanted.gpr[RSP] += 8;
                        assert_eq!(
                            // SAFETY: Exact original source, owned stack and
                            // checked destination; actual AVX admission above.
                            unsafe { engine.step_checked(&mut arena.pause().unwrap(), &planned) }
                                .unwrap(),
                            NativeBlockStop::Returned { value: 42 }
                        );
                        assert_eq!(*engine.state(), wanted);
                        arena
                            .protect(page, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                        let mut observed = [0; 48];
                        arena
                            .pause()
                            .unwrap()
                            .read(address - 16, &mut observed[..size + 16])
                            .unwrap();
                        assert_eq!(&observed[..16], &[0xa5; 16]);
                        let at = usize::from(source_register) * 16;
                        assert_eq!(&observed[16..32], &current.xmm[at..at + 16]);
                        if size == 32 {
                            assert_eq!(&observed[32..48], &current.ymm_upper[at..at + 16]);
                        }
                        if address == page + 64 {
                            let mut tail = [0; 16];
                            arena
                                .pause()
                                .unwrap()
                                .read(address + size as u64, &mut tail)
                                .unwrap();
                            assert_eq!(tail, [0x3c; 16]);
                        }
                        completed += 1;
                        cases += 1;
                        assert_eq!(engine.usage().native_entries, completed);
                        assert_eq!(engine.usage().native_instructions, completed);
                        assert_eq!(engine.usage().memory_access_checks, completed);
                        assert_eq!(engine.usage().memory_bytes_checked, completed * size as u64);
                        assert_eq!(engine.usage().vector_operations, 0);
                    }
                }
                assert_eq!(engine.usage().cache_publications, 1);
                assert_eq!(engine.usage().cache_reuses, completed - 1);
            });
        }
    }
    assert_eq!(
        cases,
        (if features.is_some() { 5 } else { 3 }) * 3 * 2 * controls
    );
    println!(
        "native_streaming_stores_available={} native_streaming_stores_cases={} native_streaming_stores_mode={}",
        features.is_some(),
        cases,
        if features.is_some() { "avx" } else { "sse2" }
    );
}

#[test]
fn streaming_store_guards_refuse_before_entry_in_checked_and_manual_paths() {
    for form in [0, 1, 2, 3, 4] {
        for manual in [false, true] {
            for case in 0..12 {
                let code = encoding(form, 13);
                with_source(Some(&code), |source, arena, stack, state| {
                    let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let size = if form == 4 { 32 } else { 16 };
                    let address = match case {
                        0 => page + 65,              // In-page misalignment.
                        1 => page + PAGE - size / 2, // Cross-page misalignment.
                        3 => page + PAGE,            // Aligned but uncommitted whole element.
                        8 => u64::MAX - size + 1,    // Aligned overflowing extent.
                        _ => page + 64,
                    };
                    let current = input(state, address, 0x1f80);
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        current,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    let planned = plan_materialized_block(
                        source,
                        0,
                        state.guest_pc,
                        &code,
                        engine.placement(),
                        &policy(form),
                        &BlockLimits::default(),
                    )
                    .unwrap();
                    assert!(planned.emits_non_temporal_store());
                    let mut bytes = [0; 96];
                    arena.pause().unwrap().read(page, &mut bytes).unwrap();
                    let wanted = bytes;
                    match case {
                        2 => arena.protect(page, PAGE, NativePermissions::READ).unwrap(),
                        4 => engine.machine.state.rflags |= 1 << 18,
                        5 => engine.machine.state.mxcsr &= !0x80,
                        6 => engine.machine.limits.max_work_units = 0,
                        7 => arena
                            .protect(
                                state.guest_pc & !(PAGE - 1),
                                PAGE,
                                NativePermissions::READ_WRITE,
                            )
                            .unwrap(),
                        9 => engine.machine.usage.native_instructions = u64::MAX,
                        10 if form >= 3 => engine.machine.vector_mode = VectorMode::Sse2,
                        10 => arena
                            .protect(page, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        _ => {}
                    }
                    let before = *engine.state();
                    let mut foreign = if case == 11 {
                        Some(NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap())
                    } else {
                        None
                    };
                    let result = {
                        let mut paused = match foreign.as_mut() {
                            Some(owner) => owner.pause().unwrap(),
                            None => arena.pause().unwrap(),
                        };
                        // SAFETY: Original fixture deliberately refuses before
                        // entry; manual entry still checks NT spans.
                        unsafe {
                            if manual {
                                engine.step(&mut paused, &planned)
                            } else {
                                engine.step_checked(&mut paused, &planned)
                            }
                        }
                    };
                    assert!(result.is_err(), "form={form} case={case} manual={manual}");
                    match case {
                        0 | 1 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::MemoryFootprint {
                                reason: MemoryAccessError::UnalignedNonTemporal,
                                ..
                            })
                        )),
                        2 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::Memory(
                                NativeMemoryError::PermissionDenied(_)
                            ))
                        )),
                        3 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::Memory(NativeMemoryError::Unmapped(_)))
                        )),
                        8 => assert!(matches!(
                            result,
                            Err(NativeDispatchError::MemoryFootprint {
                                reason: MemoryAccessError::InvalidRange,
                                ..
                            })
                        )),
                        11 => assert_eq!(result, Err(NativeDispatchError::WrongOwner)),
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
