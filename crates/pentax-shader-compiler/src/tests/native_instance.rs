// SPDX-License-Identifier: GPL-2.0-only
//! Original live-v8 compiler contracts; no capability flags are overwritten.
use super::*;
fn description() -> VertexDescription {
    VertexDescription {
        common: CommonDescription::new(WaveSize::Wave32),
        start_instance_sgpr: None,
    }
}
fn vertex(buffer: bool) -> Vec<u32> {
    let mut words = if buffer {
        // Actual16B descriptor4 load fills position0..3; v8 changes x.
        vec![
            0xe000_0000 | (0x0e << 18),
            (128 << 24) | (1 << 16),
            fixtures::vector_unary(6, 4, 264),
            fixtures::vector_binary(3, 0, 260, 0),
        ]
    } else {
        vec![
            fixtures::vector_unary(6, 0, 264),
            fixtures::vector_unary(1, 1, 128),
            fixtures::vector_unary(1, 2, 128),
            fixtures::vector_unary(1, 3, 242),
        ]
    };
    words.extend(fixtures::export(12, true, [0, 1, 2, 3]));
    words.push(fixtures::END);
    words
}
#[test]
fn native_instance_specialization_removes_only_the_real_base_instance_dependency_through_buffer_finish()
 {
    let limits = Limits::default();
    for buffer in [false, true] {
        let program = ShaderProgram::decode(&vertex(buffer), ShaderStage::Vertex, &limits).unwrap();
        let mut description = description();
        if buffer {
            description.common.user_sgpr_count = 8;
        }
        let generic = analyze(
            &program,
            Profile::new(StageDescription::Vertex(description.clone())).unwrap(),
            &limits,
        )
        .unwrap();
        let native = analyze_native_indirect_vertex(&program, description, &limits).unwrap();
        assert_eq!(generic.buffers(), native.buffers());
        let buffers = if buffer {
            vec![BufferDescription {
                user_sgpr: 4,
                stride: 0,
                swizzle: false,
                index_stride: 0,
                access: BufferAccess::ReadOnly,
            }]
        } else {
            vec![]
        };
        let generic = generic.finish(&buffers).unwrap();
        let native = native.finish(&buffers).unwrap();
        // The original checker independently decodes all actual OpCapability
        // and execution-mode words and compares every feature to the manifest.
        assert_manifest_requirements(&generic);
        assert_manifest_requirements(&native);
        assert_eq!(
            generic.manifest().vertex_instance_contract(),
            VertexInstanceContract::Dynamic
        );
        assert_eq!(
            native.manifest().vertex_instance_contract(),
            VertexInstanceContract::NativeIndirectFirstInstanceZero
        );
        assert!(
            generic
                .manifest()
                .inputs()
                .contains(&InputInterface::BaseInstance)
        );
        assert!(
            generic
                .manifest()
                .inputs()
                .contains(&InputInterface::InstanceIndex)
        );
        assert!(generic.manifest().required_features().draw_parameters());
        assert!(
            !native
                .manifest()
                .inputs()
                .contains(&InputInterface::BaseInstance)
        );
        assert!(
            native
                .manifest()
                .inputs()
                .contains(&InputInterface::InstanceIndex)
        );
        assert!(!native.manifest().required_features().draw_parameters());
        assert_eq!(native.manifest().buffers().len(), usize::from(buffer));
        assert!(native.manifest().required_features().shader());
        validate(
            if buffer {
                "native-v8-buffer"
            } else {
                "native-v8"
            },
            &native,
        );
    }
}
#[test]
fn native_instance_context_rejects_overridden_sgpr_and_nonvertex_without_changing_generic_profiles()
{
    let limits = Limits::default();
    let program = ShaderProgram::decode(&vertex(false), ShaderStage::Vertex, &limits).unwrap();
    let mut override_description = description();
    override_description.start_instance_sgpr = Some(10);
    assert!(matches!(
        analyze_native_indirect_vertex(&program, override_description.clone(), &limits),
        Err(CompileError::Profile(
            ProfileError::NativeIndirectInstanceContract
        ))
    ));
    // The same valid override retains the original generic API contract.
    let module = compile(
        &program,
        &Profile::new(StageDescription::Vertex(override_description)).unwrap(),
        &limits,
    )
    .unwrap();
    assert_eq!(
        module.manifest().vertex_instance_contract(),
        VertexInstanceContract::Dynamic
    );
    assert!(module.manifest().required_features().draw_parameters());
    let pixel = ShaderProgram::decode(&fixtures::fragment(), ShaderStage::Pixel, &limits).unwrap();
    assert!(matches!(
        analyze_native_indirect_vertex(&pixel, description(), &limits),
        Err(CompileError::Profile(ProfileError::StageMismatch))
    ));
}
