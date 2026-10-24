// SPDX-License-Identifier: GPL-2.0-only
// Original end-to-end lowering matrix: every accepted generated program must
// produce a module accepted by the explicitly supplied official validator.
use super::*;
use pentax_shader::{Family, OperandKind};

#[test]
fn accepted_scalar_vector_and_alias_programs_validate() {
    let mut description = common();
    description.user_sgpr_count = 32;
    description.buffers.push(BufferDescription {
        user_sgpr: 20,
        stride: 0,
        swizzle: false,
        index_stride: 0,
        access: BufferAccess::WriteOnly,
    });
    let profile = Profile::new(StageDescription::Compute(ComputeDescription {
        dispatch_thread_dimensions: false,
        common: description,
        workgroup_size: [8, 1, 1],
        thread_ids: 1,
        group_id: [false; 3],
        workgroup_register: 32,
        tg_size: false,
        float_mode: 0xc0,
        shared_memory_bytes: 0,
    }))
    .unwrap();
    let limits = Limits::default();
    let mut compiled = 0;
    let mut unique = Vec::new();
    for (family, count) in [
        (Family::Sop1, 256),
        (Family::Sop2, 96),
        (Family::Vop1, 256),
        (Family::Vop2, 64),
        (Family::Vop3, 1024),
    ] {
        for op in 0..count {
            let mut candidate = match family {
                Family::Sop1 => vec![0xbe80_0000 | (10 << 16) | (op << 8) | 11],
                Family::Sop2 => vec![0x8000_0000 | (op << 23) | (10 << 16) | (12 << 8) | 11],
                Family::Vop1 => vec![fixtures::vector_unary(op, 4, 256)],
                Family::Vop2 => vec![fixtures::vector_binary(op, 4, 256, 0)],
                Family::Vop3 => vec![0xd400_0000 | (op << 16) | 4, 256 | (256 << 9) | (256 << 18)],
                _ => unreachable!(),
            };
            if let Err(pentax_shader::DecodeError::Truncated { required_words, .. }) =
                pentax_shader::decode(&candidate, ShaderStage::Compute, &limits.decode)
            {
                // Literal words requested by the frozen decoder, not guessed lengths.
                candidate.resize(required_words, 0);
            }
            let Ok(decoded) =
                pentax_shader::decode(&candidate, ShaderStage::Compute, &limits.decode)
            else {
                continue;
            };
            let instruction = decoded.instruction(0).unwrap();
            if instruction.family() != family {
                continue;
            }
            let destination = instruction.destination();
            let source = match destination.kind() {
                OperandKind::Sgpr => u32::from(destination.register()),
                OperandKind::Vgpr => 256 + u32::from(destination.register()),
                OperandKind::VccLo => 106,
                OperandKind::VccHi => 107,
                // Mask/side-effect instructions need their own dedicated fixtures.
                _ => continue,
            };
            let mut words = vec![fixtures::vector_unary(1, 4, 128)];
            words.extend_from_slice(&candidate);
            words.push(fixtures::vector_unary(1, 4, source));
            let mut store = fixtures::store(4, 0);
            store[1] |= 5 << 16; // V# begins in user SGPR20, away from ALU destinations.
            words.extend(store);
            words.push(fixtures::END);
            let Ok(program) = ShaderProgram::decode(&words, ShaderStage::Compute, &limits) else {
                continue;
            };
            match compile(&program, &profile, &limits) {
                Ok(module) => {
                    assert_eq!(module.manifest().buffers().len(), 1);
                    validate(&format!("matrix-{family:?}-{op:03x}"), &module);
                    compiled += 1;
                    if !unique.contains(&instruction.opcode()) {
                        unique.push(instruction.opcode())
                    }
                }
                Err(
                    CompileError::Unsupported(_)
                    | CompileError::UndefinedRegister { .. }
                    | CompileError::Cfg(_),
                ) => {}
                Err(error) => panic!("original {family:?}/{op:#x} failed: {error}"),
            }
        }
    }
    assert!(
        compiled >= 100,
        "loss of substantive pipeline coverage: {compiled}"
    );
    assert!(
        unique.len() >= 60,
        "loss of distinct operation coverage: {}",
        unique.len()
    );
    println!(
        "Original lowering matrix: {compiled} accepted programs / {} distinct operations",
        unique.len()
    );
}
