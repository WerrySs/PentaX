// SPDX-License-Identifier: GPL-2.0-only
//! Original direct-descriptor programs; no game or upstream shader bytes.
use super::*;

fn stage_profile(stage: ShaderStage) -> Profile {
    let mut common = CommonDescription::new(WaveSize::Wave32);
    let desc = if stage == ShaderStage::Vertex {
        StageDescription::Vertex(VertexDescription {
            common,
            start_instance_sgpr: None,
        })
    } else {
        common.push_data_start_dword = 16;
        StageDescription::Pixel(PixelDescription::new(common))
    };
    Profile::new(desc).unwrap()
}

fn source(stage: ShaderStage, base: u32, scalar: bool) -> Vec<u32> {
    let mut words = if scalar {
        let mut words = vec![
            fixtures::scalar_move(40, 128),
            0xf400_0000 | (0x0a << 18) | (32 << 6) | (base / 2),
            40 << 25,
        ];
        for lane in 0..4 {
            words.push(fixtures::vector_unary(1, lane, 32 + lane));
        }
        words
    } else {
        vec![0xe000_0000 | (0x0e << 18), (128 << 24) | ((base / 4) << 16)]
    };
    words.extend(fixtures::export(
        if stage == ShaderStage::Vertex { 12 } else { 0 },
        true,
        [0, 1, 2, 3],
    ));
    words.push(fixtures::END);
    words
}

fn description(base: u8) -> BufferDescription {
    BufferDescription {
        user_sgpr: base,
        stride: 0,
        swizzle: false,
        index_stride: 0,
        access: BufferAccess::ReadOnly,
    }
}

#[test]
fn staged_live_buffers_preserve_exact_stage_binding_and_original_compilation() {
    let limits = Limits::default();
    for stage in [ShaderStage::Vertex, ShaderStage::Pixel] {
        for base in if stage == ShaderStage::Vertex {
            [4, 8, 12]
        } else {
            [0, 4, 12]
        } {
            for scalar in [false, true] {
                let program =
                    ShaderProgram::decode(&source(stage, base, scalar), stage, &limits).unwrap();
                let analysis = analyze(&program, stage_profile(stage), &limits).unwrap();
                assert_eq!(
                    analysis.buffers(),
                    [BufferRequirement {
                        user_sgpr: base as u8,
                        read: true,
                        written: false,
                        scalar,
                        coherent: false,
                        max_byte_extent: 16,
                        first_use_pc: if scalar { 4 } else { 0 },
                    }]
                );
                let module = analysis.finish(&[description(base as u8)]).unwrap();
                let direct_profile = stage_profile(stage)
                    .with_buffers(vec![description(base as u8)])
                    .unwrap();
                let direct = compile(&program, &direct_profile, &limits).unwrap();
                assert_eq!(module.spirv(), direct.spirv());
                assert_eq!(module.manifest().buffers(), direct.manifest().buffers());
                assert_eq!(
                    module.manifest().buffer_binding(),
                    Some(if stage == ShaderStage::Pixel { 57 } else { 0 })
                );
                assert_eq!(
                    module.manifest().push_data().unwrap().memory_offset_count,
                    1
                );
                validate(&format!("staged-buffer-{stage:?}-{base}-{scalar}"), &module);
            }
        }
    }
}

#[test]
fn staged_discovery_ignores_dead_loads_and_tracks_only_live_unmodified_copies() {
    let limits = Limits::default();
    let mut dead = vec![0xe000_0000 | (0x0e << 18), (128 << 24) | (8 << 8)];
    dead.extend(fixtures::vertex());
    let program = ShaderProgram::decode(&dead, ShaderStage::Vertex, &limits).unwrap();
    let analysis = analyze(&program, stage_profile(ShaderStage::Vertex), &limits).unwrap();
    assert!(analysis.buffers().is_empty());
    let compiled = analysis.finish(&[]).unwrap();
    assert!(compiled.manifest().buffers().is_empty());
    assert_eq!(
        compiled.spirv(),
        compile(&program, &stage_profile(ShaderStage::Vertex), &limits)
            .unwrap()
            .spirv()
    );

    // The V# is copied to another quad, but SSA retains its true user base4.
    let mut copy: Vec<u32> = (0..4)
        .map(|lane| fixtures::scalar_move(16 + lane, 4 + lane))
        .collect();
    let mut code = source(ShaderStage::Vertex, 16, false);
    copy.append(&mut code);
    let program = ShaderProgram::decode(&copy, ShaderStage::Vertex, &limits).unwrap();
    let analysis = analyze(&program, stage_profile(ShaderStage::Vertex), &limits).unwrap();
    assert_eq!(analysis.buffers()[0].user_sgpr, 4);
    assert_eq!(
        analysis
            .finish(&[description(4)])
            .unwrap()
            .manifest()
            .buffers()[0]
            .user_sgpr,
        4
    );
}

#[test]
fn staged_discovery_refuses_dynamic_descriptors_and_preserves_access_errors() {
    let limits = Limits::default();
    let mut dynamic = vec![fixtures::scalar_move(4, 128)];
    dynamic.extend(source(ShaderStage::Vertex, 4, false));
    let program = ShaderProgram::decode(&dynamic, ShaderStage::Vertex, &limits).unwrap();
    assert!(matches!(
        analyze(&program, stage_profile(ShaderStage::Vertex), &limits),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::DynamicDescriptor,
            ..
        }))
    ));

    let mut words = vec![
        fixtures::vector_unary(1, 0, 128),
        fixtures::vector_unary(1, 1, 128),
    ];
    let mut store = fixtures::store(0, 1);
    store[1] |= 1 << 16;
    words.extend(store);
    words.extend(fixtures::vertex());
    let program = ShaderProgram::decode(&words, ShaderStage::Vertex, &limits).unwrap();
    let analysis = analyze(&program, stage_profile(ShaderStage::Vertex), &limits).unwrap();
    assert!(analysis.buffers()[0].written);
    assert!(matches!(
        analysis.finish(&[description(4)]),
        Err(CompileError::Resource(ResourceError {
            kind: ResourceErrorKind::AccessMismatch { user_sgpr: 4 },
            ..
        }))
    ));
    // The current vertex ABI replaces s2/s3 with system constants. A raw V#
    // at quad0 therefore is not an unmodified user descriptor.
    let program = ShaderProgram::decode(
        &source(ShaderStage::Vertex, 0, false),
        ShaderStage::Vertex,
        &limits,
    )
    .unwrap();
    assert!(matches!(
        analyze(&program, stage_profile(ShaderStage::Vertex), &limits),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::DynamicDescriptor,
            ..
        }))
    ));
}

#[test]
fn staged_specialization_keeps_missing_unused_and_finite_budget_contracts() {
    let limits = Limits::default();
    let program = ShaderProgram::decode(
        &source(ShaderStage::Vertex, 4, false),
        ShaderStage::Vertex,
        &limits,
    )
    .unwrap();
    assert!(matches!(
        analyze(&program, stage_profile(ShaderStage::Vertex), &limits)
            .unwrap()
            .finish(&[]),
        Err(CompileError::Resource(ResourceError {
            kind: ResourceErrorKind::MissingBuffer { user_sgpr: 4 },
            ..
        }))
    ));
    assert!(matches!(
        analyze(&program, stage_profile(ShaderStage::Vertex), &limits)
            .unwrap()
            .finish(&[description(4), description(8)]),
        Err(CompileError::Resource(ResourceError {
            kind: ResourceErrorKind::UnusedBuffer { user_sgpr: 8 },
            ..
        }))
    ));
    let predeclared = stage_profile(ShaderStage::Vertex)
        .with_buffers(vec![description(4)])
        .unwrap();
    assert!(matches!(
        analyze(&program, predeclared, &limits),
        Err(CompileError::Profile(ProfileError::PredeclaredBuffers))
    ));
    for reduced in [
        Limits {
            max_work: 1,
            ..limits
        },
        Limits {
            max_bytes: 1,
            ..limits
        },
    ] {
        assert!(matches!(
            analyze(&program, stage_profile(ShaderStage::Vertex), &reduced),
            Err(CompileError::LimitExceeded { .. })
        ));
    }
    let used = analyze(&program, stage_profile(ShaderStage::Vertex), &limits)
        .unwrap()
        .work_units();
    // The continuation does not reset already spent work or capacity.
    let analysis = analyze(
        &program,
        stage_profile(ShaderStage::Vertex),
        &Limits {
            max_work: used,
            ..limits
        },
    )
    .unwrap();
    assert!(matches!(
        analysis.finish(&[description(4)]),
        Err(CompileError::LimitExceeded {
            kind: LimitKind::Work,
            ..
        })
    ));
}
