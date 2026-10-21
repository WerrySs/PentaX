// SPDX-License-Identifier: GPL-2.0-only
//! Original publication guards; none of these checks executes guest code.

use super::*;
use pentax_codegen::CpuidFeature;

fn requested<'view, 'plan, 'image>(
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
    let features = pentax_codegen::CpuFeatures::x64_sse2()
        .with(CpuidFeature::AVX)
        .with(CpuidFeature::AVX2);
    plan_materialized_block(
        source,
        0,
        pc,
        &bytes,
        placement,
        &TranslationPolicy::new(features).with_native_ymm_registers(),
        &BlockLimits::default(),
    )
    .unwrap()
}

#[test]
fn requested_native_vectors_refuse_legacy_bridge_and_unadmitted_avx2_before_publication() {
    for avx2 in [false, true] {
        let code = if avx2 {
            vec![0xc5, 0xfd, 0x6b, 0xc1, 0xc3] // VPACKSSDW YMM0,YMM0,YMM1.
        } else {
            vec![0xc5, 0xfe, 0x6f, 0xc1, 0xc3] // VMOVDQU YMM0,YMM1.
        };
        with_source(Some(&code), |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let admitted_mode = engine.machine.vector_mode;
            let planned = requested(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(planned.instructions()[0].emitted_len() > 0);
            engine.machine.vector_mode = VectorMode::Sse2;
            assert!(engine.native_ymm_features().is_none());
            assert_eq!(
                engine.publish(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::InvalidState)
            );
            assert_eq!(*engine.state(), state);
            assert_eq!(engine.usage().native_entries, 0);
            assert_eq!(engine.usage().cache_publications, 0);
            assert!(!engine.machine.cache.executable);
            if avx2 {
                // Original test metadata only; never enter this synthetic mode.
                // AVX1 cannot satisfy a decoded AVX2 requirement.
                engine.machine.vector_mode = VectorMode::Avx { avx2: true };
                assert!(
                    engine
                        .machine
                        .vector_mode
                        .supports_admission(planned.native_admission())
                );
                assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
                assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
                let before = engine.usage();
                engine.machine.vector_mode = VectorMode::Avx { avx2: false };
                assert!(
                    !engine
                        .machine
                        .vector_mode
                        .supports_admission(planned.native_admission())
                );
                assert_eq!(
                    engine.publish(&arena.pause().unwrap(), &planned),
                    Err(NativeDispatchError::InvalidState)
                );
                assert_eq!(engine.usage(), before);
                assert_eq!(*engine.state(), state);
                let diagnostic = pentax_codegen::plan_block(
                    source.source_plan(),
                    0,
                    state.guest_pc,
                    engine.placement(),
                    &planned.policy(),
                    &BlockLimits::default(),
                )
                .unwrap();
                // The fast mask miss must preserve the diagnostic's original
                // first unsupported decoded-feature label, before source work.
                assert!(matches!(
                    super::super::super::publish_block_with_mode(
                        &mut engine.machine.cache,
                        arena,
                        &diagnostic,
                        state.guest_pc,
                        VectorMode::Avx { avx2: false },
                    ),
                    Err(PlatformError::UnsupportedHostFeature {
                        feature: "unsupported-feature"
                    })
                ));
            }
            if !avx2 && admitted_mode.features().is_some() {
                engine.machine.vector_mode = admitted_mode;
                engine.machine.limits.max_work_units =
                    engine.usage().work_units + planned.instructions().len() as u64;
                // SAFETY: Original register-only source and this actual owned
                // stack satisfy the normal preparation obligations. The real
                // admitted host mode is restored. Transfer-budget preflight
                // must refuse before publication and any native execution.
                let result = unsafe { engine.enter_block(&mut arena.pause().unwrap(), &planned) };
                assert_eq!(
                    result,
                    Err(NativeDispatchError::Budget("vector context transfer"))
                );
                assert_eq!(engine.usage().native_entries, 0);
                assert_eq!(engine.usage().cache_publications, 0);
                assert_eq!(*engine.state(), state);
            }
        });
    }
}

#[test]
fn cache_tail_matches_retained_bridge_mode_and_reattachment_cannot_change_it() {
    with_source(
        Some(&[0xb8, 42, 0, 0, 0, 0xc3]),
        |source, arena, stack, state| {
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let mode = engine.machine.vector_mode;
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
            let cell = engine.machine.cache.len - TAIL_CELL;
            let mut bytes = [0; 8];
            engine.machine.cache.read(cell, &mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), mode.trailer());
            let other = if matches!(mode, VectorMode::Sse2) {
                VectorMode::Avx { avx2: false }
            } else {
                VectorMode::Sse2
            };
            engine.machine.cache.arm_writable().unwrap();
            engine
                .machine
                .cache
                .write(cell, &other.trailer().to_le_bytes())
                .unwrap();
            engine.machine.cache.seal_executable().unwrap();
            assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
            assert_eq!(engine.usage().cache_publications, 2);
            assert_eq!(engine.usage().cache_reuses, 0);
            engine.machine.cache.read(cell, &mut bytes).unwrap();
            assert_eq!(u64::from_le_bytes(bytes), mode.trailer());
            let mut parked = Some(engine.into_machine());
            let engine =
                NativeBlockStepper::from_machine(source, &mut arena.pause().unwrap(), &mut parked)
                    .unwrap();
            assert!(parked.is_none());
            assert_eq!(engine.machine.vector_mode, mode);
            assert_eq!(engine.usage().cache_publications, 2);
            assert_eq!(*engine.state(), state);
        },
    );
}
