// SPDX-License-Identifier: GPL-2.0-only
// Original end-to-end programs for the Rust port. No upstream or guest fixtures.
mod buffer_analysis;
mod capacities;
mod fixtures;
mod images;
mod interpolation;
mod logical_wave64;
mod matrix;
mod shared_memory;
mod storage_barrier;
use super::*;
use pentax_shader::ShaderStage;

fn common() -> CommonDescription {
    let mut common = CommonDescription::new(WaveSize::Wave32);
    common.subgroup = Some(SubgroupProfile { size: 32 });
    common
}
fn profile(stage: ShaderStage) -> Profile {
    let desc = match stage {
        ShaderStage::Vertex => StageDescription::Vertex(VertexDescription {
            common: common(),
            start_instance_sgpr: None,
        }),
        ShaderStage::Pixel => {
            let mut pixel = PixelDescription::new(common());
            pixel.inputs.push(PixelInput {
                location: 0,
                flat: false,
            });
            pixel.perspective_center_vgpr = Some(6);
            StageDescription::Pixel(pixel)
        }
        ShaderStage::Compute => {
            let mut common = common();
            common.buffers.push(BufferDescription {
                user_sgpr: 0,
                stride: 0,
                swizzle: false,
                index_stride: 0,
                access: BufferAccess::WriteOnly,
            });
            StageDescription::Compute(ComputeDescription {
                common,
                workgroup_size: [8, 1, 1],
                thread_ids: 1,
                group_id: [false; 3],
                workgroup_register: 16,
                tg_size: false,
                float_mode: 0xc0,
                shared_memory_bytes: 0,
            })
        }
        _ => unreachable!("original fixture stage"),
    };
    Profile::new(desc).unwrap()
}

fn original(name: &str, stage: ShaderStage, words: &[u32]) -> CompiledShader {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, stage, &limits).unwrap();
    let module = compile(&program, &profile(stage), &limits).unwrap();
    assert_eq!(module.spirv()[0], 0x0723_0203);
    assert_eq!(module.manifest().stage(), stage);
    assert!(module.spirv().len() > 32);
    validate(name, &module);
    module
}

fn validate(name: &str, module: &CompiledShader) {
    assert_manifest_requirements(module);
    if let Some(tool) = std::env::var_os("PENTAX_SPIRV_VAL") {
        let directory = std::env::var_os("PENTAX_SHADER_OUTPUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("pentax-original-shaders-{}", std::process::id()))
            });
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("{name}.spv"));
        let bytes: Vec<_> = module
            .spirv()
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        std::fs::write(&path, bytes).unwrap();
        let result = std::process::Command::new(tool)
            .args(["--target-env", "vulkan1.1"])
            .arg(&path)
            .output()
            .expect("run explicitly supplied official validator");
        assert!(
            result.status.success(),
            "official validator rejected {name}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

fn assert_manifest_requirements(module: &CompiledShader) {
    let requirements = module.manifest().required_features();
    assert_eq!(requirements.spirv_version(), module.spirv()[1]);
    let mut capabilities = Vec::new();
    let mut preserve32 = false;
    let mut float_controls = false;
    let mut at = 5;
    while at < module.spirv().len() {
        let header = module.spirv()[at];
        let count = (header >> 16) as usize;
        assert!(count > 0);
        let operands = &module.spirv()[at + 1..at + count];
        match header as u16 {
            17 => capabilities.push(operands[0]),
            16 if operands[1] == 4461 => {
                assert_eq!(operands[2..], [32]);
                preserve32 = true;
            }
            10 => {
                let bytes: Vec<u8> = operands
                    .iter()
                    .flat_map(|word| word.to_le_bytes())
                    .take_while(|&byte| byte != 0)
                    .collect();
                assert_eq!(bytes, b"SPV_KHR_float_controls");
                float_controls = true;
            }
            _ => {}
        }
        at += count;
    }
    assert_eq!(requirements.shader(), capabilities.contains(&1));
    assert_eq!(
        requirements.signed_zero_inf_nan_preserve_capability(),
        capabilities.contains(&4466)
    );
    assert_eq!(
        requirements.float32_signed_zero_inf_nan_preserve(),
        preserve32
    );
    assert_eq!(requirements.float_controls_extension(), float_controls);
    assert_eq!(requirements.shader_int64(), capabilities.contains(&11));
    assert_eq!(requirements.draw_parameters(), capabilities.contains(&4427));
    assert_eq!(requirements.subgroup_basic(), capabilities.contains(&61));
    assert_eq!(requirements.subgroup_ballot(), capabilities.contains(&64));
    assert_eq!(
        requirements.subgroup_arithmetic(),
        capabilities.contains(&63)
    );
    assert_eq!(requirements.image_query(), capabilities.contains(&50));
    assert!(
        capabilities
            .iter()
            .all(|capability| [1, 4466, 11, 4427, 61, 63, 64, 50].contains(capability))
    );
}

#[test]
fn original_vertex_math_has_real_position_and_parameter_interfaces() {
    let module = original("vertex-math", ShaderStage::Vertex, &fixtures::vertex());
    assert!(
        module
            .manifest()
            .inputs()
            .contains(&InputInterface::VertexIndex)
    );
    assert!(
        module
            .manifest()
            .outputs()
            .contains(&OutputInterface::Position)
    );
    assert!(module.manifest().outputs().iter().any(|o| matches!(
        o,
        OutputInterface::Parameter {
            index: 0,
            location: 0
        }
    )));
}
#[test]
fn original_fragment_math_links_an_input_to_color_output() {
    let module = original("fragment-math", ShaderStage::Pixel, &fixtures::fragment());
    assert!(module.manifest().inputs().iter().any(|i| matches!(
        i,
        InputInterface::Parameter {
            input: 0,
            location: 0,
            flat: false
        }
    )));
    assert!(
        module
            .manifest()
            .outputs()
            .contains(&OutputInterface::Color {
                export: 0,
                location: 0,
                uint: false
            })
    );
}
#[test]
fn original_compute_write_retains_the_source_buffer_binding_contract() {
    let module = original("compute-write", ShaderStage::Compute, &fixtures::compute());
    assert_eq!(module.manifest().buffer_binding(), Some(0));
    assert_eq!(module.manifest().workgroup_size(), Some([8, 1, 1]));
    assert_eq!(module.manifest().buffers().len(), 1);
    assert!(module.manifest().buffers()[0].written);
    assert!(!module.manifest().buffers()[0].read);
}

#[test]
fn system_scalar_masks_initialize_only_private_metadata_and_preserve_undefined_policy() {
    let StageDescription::Compute(mut desc) = profile(ShaderStage::Compute).description().clone()
    else {
        unreachable!()
    };
    desc.common.user_sgpr_count = 4;
    desc.thread_ids = 0;
    desc.group_id = [true, true, false];
    desc.workgroup_register = 4;
    let profile = Profile::new(StageDescription::Compute(desc)).unwrap();
    let limits = Limits::default();
    let mut words = vec![
        fixtures::vector_unary(1, 1, 135),
        fixtures::vector_unary(1, 2, 128),
        0xbefe_0404, // S_MOV_B64 EXEC, s[4:5]: defined WorkgroupId values.
    ];
    words.extend(fixtures::store(1, 2));
    words.push(fixtures::END);
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    let module = compile(&program, &profile, &limits).unwrap();
    assert_eq!(module.manifest().undefined_register_reads(), 0);
    assert!(
        module
            .manifest()
            .inputs()
            .contains(&InputInterface::WorkgroupId)
    );
    validate("system-scalar-mask", &module);

    // Initializing cache metadata must not invent initial guest SGPR values.
    words[2] = 0xbefe_0406; // s[6:7] has no guest definition.
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    assert!(matches!(
        compile(&program, &profile, &limits),
        Err(CompileError::UndefinedRegister {
            register: Register::Sgpr(6),
            ..
        })
    ));

    // A dynamic join chooses a cached EXEC mask or an ordinary defined pair.
    // The cache's unused false-tag path must not introduce a live OpUndef.
    let mut joined = vec![
        fixtures::vector_unary(1, 1, 135),
        fixtures::vector_unary(1, 2, 128),
        0xbf06_8004, // S_CMP_EQ_U32 s4, 0.
        0xbf84_0002, // S_CBRANCH_SCC0 to the ordinary pair.
        0xbe86_047e, // S_MOV_B64 s[6:7], EXEC.
        0xbf82_0002, // S_BRANCH to the join.
        fixtures::scalar_move(6, 193),
        fixtures::scalar_move(7, 128),
        0xbefe_0406, // S_MOV_B64 EXEC, s[6:7].
    ];
    joined.extend(fixtures::store(1, 2));
    joined.push(fixtures::END);
    let program = ShaderProgram::decode(&joined, ShaderStage::Compute, &limits).unwrap();
    let module = compile(&program, &profile, &limits).unwrap();
    assert_eq!(module.manifest().undefined_register_reads(), 0);
    validate("joined-scalar-mask", &module);
}

#[test]
fn original_scalar_loop_is_structured_and_preserves_a_live_carried_value() {
    let module = original(
        "scalar-loop",
        ShaderStage::Compute,
        &fixtures::scalar_loop(),
    );
    let mut at = 5;
    let mut loops = 0;
    let mut phis = 0;
    while at < module.spirv().len() {
        let word = module.spirv()[at];
        assert!(word >> 16 != 0);
        loops += usize::from(word as u16 == 246);
        phis += usize::from(word as u16 == 245);
        at += (word >> 16) as usize;
    }
    assert!(loops > 0);
    assert!(phis > 0);
}
#[test]
fn branch_into_a_literal_is_rejected_before_lowering() {
    let words = [
        0xbf82_0001,
        fixtures::vector_unary(1, 0, 255),
        0x3f80_0000,
        fixtures::END,
    ];
    assert!(matches!(
        ShaderProgram::decode(&words, ShaderStage::Compute, &Limits::default()),
        Err(CompileError::Program(ProgramError {
            kind: ProgramErrorKind::BranchIntoInstruction { .. },
            ..
        }))
    ));
}
#[test]
fn hostile_workgroup_dimensions_are_a_profile_error() {
    let StageDescription::Compute(mut desc) = profile(ShaderStage::Compute).description().clone()
    else {
        unreachable!()
    };
    desc.workgroup_size = [u32::MAX; 3];
    assert!(matches!(
        Profile::new(StageDescription::Compute(desc)),
        Err(ProfileError::WorkgroupSize)
    ));
}
#[test]
fn output_budget_refuses_a_module_including_its_header() {
    let words = fixtures::vertex();
    let program = ShaderProgram::decode(&words, ShaderStage::Vertex, &Limits::default()).unwrap();
    let limits = Limits {
        max_spirv_words: 4,
        ..Limits::default()
    };
    assert!(matches!(
        compile(&program, &profile(ShaderStage::Vertex), &limits),
        Err(CompileError::LimitExceeded {
            kind: LimitKind::SpirvWords,
            ..
        })
    ));
}

#[test]
fn unported_pixel_valid_mask_is_an_explicit_refusal() {
    let mut words = fixtures::fragment();
    let export_word = words.len() - 3;
    words[export_word] |= 1 << 12;
    let program = ShaderProgram::decode(&words, ShaderStage::Pixel, &Limits::default()).unwrap();
    assert!(matches!(
        compile(&program, &profile(ShaderStage::Pixel), &Limits::default()),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::PixelValidMask,
            ..
        }))
    ));
}

#[test]
fn conditional_end_target_keeps_the_non_taken_store_path() {
    // CMP_EQ_U32(0,1) is false: the branch to one-past-END is not taken.
    let mut words = vec![
        0xbf06_8180,
        0xbf85_0005,
        fixtures::vector_unary(1, 1, 137),
        fixtures::vector_unary(1, 2, 128),
    ];
    words.extend(fixtures::store(1, 2));
    words.push(fixtures::END);
    let _module = original("conditional-end", ShaderStage::Compute, &words);
}

#[test]
fn identity_chain_walks_consume_work_on_every_visit() {
    use crate::budget::Budget;
    use crate::ir::{Ir, Op, Type, Value};
    let mut ir = Ir::new();
    let mut construction = Budget::new(&Limits::default());
    let mut value = Value::U32(73);
    for _ in 0..32 {
        value = Value::Inst(
            ir.push(&mut construction, Op::Identity, Type::U32, 0, 0, &[value])
                .unwrap(),
        );
    }
    let limits = Limits {
        max_work: 31,
        ..Limits::default()
    };
    let short = Budget::new(&limits);
    assert!(matches!(
        ir.resolve(&short, value),
        Err(CompileError::LimitExceeded {
            kind: LimitKind::Work,
            ..
        })
    ));
    assert_eq!(short.work(), 31);
    let exact = Budget::new(&Limits {
        max_work: 32,
        ..limits
    });
    assert_eq!(ir.resolve(&exact, value).unwrap(), Value::U32(73));
    assert_eq!(exact.work(), 32);
    // Repeated lookups of the same owner-bound chain still consume work.
    assert!(matches!(
        ir.resolve(&exact, value),
        Err(CompileError::LimitExceeded {
            kind: LimitKind::Work,
            ..
        })
    ));
}

#[test]
fn stage_limits_refuse_the_whole_compilation() {
    let words = fixtures::scalar_loop();
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &Limits::default()).unwrap();
    for (kind, limits) in [
        (
            LimitKind::Work,
            Limits {
                max_work: 1,
                ..Limits::default()
            },
        ),
        (
            LimitKind::Bytes,
            Limits {
                max_bytes: 0,
                ..Limits::default()
            },
        ),
        (
            LimitKind::Blocks,
            Limits {
                max_blocks: 1,
                ..Limits::default()
            },
        ),
        (
            LimitKind::Edges,
            Limits {
                max_edges: 0,
                ..Limits::default()
            },
        ),
        (
            LimitKind::IrInstructions,
            Limits {
                max_ir_instructions: 1,
                ..Limits::default()
            },
        ),
        (
            LimitKind::IrOperands,
            Limits {
                max_ir_operands: 1,
                ..Limits::default()
            },
        ),
        (
            LimitKind::SsaSlots,
            Limits {
                max_ssa_slots: 1,
                ..Limits::default()
            },
        ),
        (
            LimitKind::SpirvIds,
            Limits {
                max_spirv_ids: 1,
                ..Limits::default()
            },
        ),
    ] {
        assert!(
            matches!(compile(&program, &profile(ShaderStage::Compute), &limits),
            Err(CompileError::LimitExceeded { kind: found, .. }) if found == kind),
            "{kind:?}"
        );
    }
}

#[test]
fn original_raw_and_scalar_buffer_load_shapes_retain_live_data() {
    let StageDescription::Compute(mut desc) = profile(ShaderStage::Compute).description().clone()
    else {
        unreachable!()
    };
    desc.common.buffers[0].access = BufferAccess::ReadWrite;
    let profile = Profile::new(StageDescription::Compute(desc)).unwrap();
    let limits = Limits::default();
    // Original copies through all raw dword vector shapes. A store consumes
    // every loaded component so neither translation nor liveness can erase it.
    for (count, load, store) in [
        (1, 0x0c, 0x1c),
        (2, 0x0d, 0x1d),
        (3, 0x0f, 0x1f),
        (4, 0x0e, 0x1e),
    ] {
        let words = [
            fixtures::vector_unary(1, 2, 128),
            0xe000_0000 | (load << 18) | (1 << 12),
            (128 << 24) | (4 << 8) | 2,
            0xe000_0000 | (store << 18) | (1 << 12) | 32,
            (128 << 24) | (4 << 8) | 2,
            fixtures::END,
        ];
        let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
        let module = compile(&program, &profile, &limits).unwrap();
        let buffer = module.manifest().buffers()[0];
        assert!(buffer.read && buffer.written && !buffer.scalar);
        assert_eq!(buffer.max_byte_extent, 32 + count * 4);
        validate(&format!("raw-buffer-copy-{count}"), &module);
    }
    for (count, opcode) in [(1, 0x08), (2, 0x09), (4, 0x0a), (8, 0x0b), (16, 0x0c)] {
        let mut words = vec![
            fixtures::scalar_move(12, 128),
            fixtures::vector_unary(1, 2, 128),
            0xf400_0000 | (opcode << 18) | (32 << 6),
            (12 << 25) | 8,
        ];
        for component in 0..count {
            words.push(fixtures::vector_unary(1, 4, 32 + component));
            let mut store = fixtures::store(4, 2);
            store[0] |= 128 + component * 4;
            words.extend(store);
        }
        words.push(fixtures::END);
        let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
        let module = compile(&program, &profile, &limits).unwrap();
        let buffer = module.manifest().buffers()[0];
        assert!(buffer.read && buffer.written && buffer.scalar);
        assert_eq!(buffer.max_byte_extent, 128 + count * 4);
        validate(&format!("scalar-buffer-copy-{count}"), &module);
    }
}

mod native_instance;
