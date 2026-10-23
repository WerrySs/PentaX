// SPDX-License-Identifier: GPL-2.0-only
// Original bounded shader programs, not extracted guest or upstream fixtures.
mod lod_feedback;

use super::*;

fn pixel() -> Profile {
    Profile::new(StageDescription::Pixel(PixelDescription::new(
        CommonDescription::new(WaveSize::Wave32),
    )))
    .unwrap()
}

fn image(op: u32, mask: u32, texture: u32, sampler: u32, dst: u32) -> [u32; 2] {
    [
        0xf000_0000 | (op << 18) | (1 << 3) | (mask << 8),
        (dst << 8) | ((texture / 4) << 16) | ((sampler / 4) << 21),
    ]
}

fn source(op: u32, mask: u32) -> Vec<u32> {
    let mut words = vec![
        fixtures::vector_unary(1, 0, 255),
        0x3e80_0000, // .25
        fixtures::vector_unary(1, 1, 255),
        0x3f40_0000,                       // .75
        fixtures::vector_unary(1, 2, 242), // explicit LOD 1
    ];
    words.extend(image(op, mask, 0, 8, 4));
    let n = mask.count_ones();
    words.extend(fixtures::export(
        0,
        true,
        [
            4,
            4 + (n > 1) as u32,
            4 + (n > 2) as u32 * 2,
            4 + (n > 3) as u32 * 3,
        ],
    ));
    words.push(fixtures::END);
    words
}

fn ops(module: &CompiledShader, op: u16) -> Vec<Vec<u32>> {
    let mut records = Vec::new();
    let mut at = 5;
    while at < module.spirv().len() {
        let header = module.spirv()[at];
        let count = (header >> 16) as usize;
        assert!(count > 0 && at + count <= module.spirv().len());
        if header as u16 == op {
            records.push(module.spirv()[at + 1..at + count].to_vec());
        }
        at += count;
    }
    records
}

fn staged(words: &[u32]) -> Result<CompiledShader, CompileError> {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, ShaderStage::Pixel, &limits)?;
    let analysis = analyze(&program, pixel(), &limits)?;
    analysis.finish_with_images(
        &[],
        &[ImageDescription {
            user_sgpr: 0,
            min_lod_u4_8: 0,
        }],
        &[SamplerDescription { user_sgpr: 8 }],
    )
}

#[test]
fn original_textured_pixel_shader_emits_real_samples_and_separate_exact_bindings() {
    for (op, opcode) in [(0x20, 87), (0x24, 88), (0x27, 88)] {
        for mask in [1, 5, 15] {
            let words = source(op, mask);
            let program =
                ShaderProgram::decode(&words, ShaderStage::Pixel, &Limits::default()).unwrap();
            let analysis = analyze(&program, pixel(), &Limits::default()).unwrap();
            assert!(analysis.buffers().is_empty());
            assert_eq!(
                analysis.images(),
                [ImageRequirement {
                    user_sgpr: 0,
                    first_use_pc: 20
                }]
            );
            assert_eq!(
                analysis.samplers(),
                [SamplerRequirement {
                    user_sgpr: 8,
                    first_use_pc: 20
                }]
            );
            let module = analysis
                .finish_with_images(
                    &[],
                    &[ImageDescription {
                        user_sgpr: 0,
                        min_lod_u4_8: 0,
                    }],
                    &[SamplerDescription { user_sgpr: 8 }],
                )
                .unwrap();
            assert_eq!(
                module.manifest().images(),
                [ImageElement {
                    user_sgpr: 0,
                    min_lod_u4_8: 0,
                    profile_index: 0,
                    binding: 58,
                    first_use_pc: 20,
                }]
            );
            assert_eq!(
                module.manifest().samplers(),
                [SamplerElement {
                    user_sgpr: 8,
                    image_user_sgpr: 0,
                    profile_index: 0,
                    binding: 74,
                    first_use_pc: 20,
                }]
            );
            assert_eq!(ops(&module, opcode).len(), 1);
            assert_eq!(ops(&module, 86).len(), 1); // genuinely combines loaded image/sampler
            assert!(module.manifest().buffer_binding().is_none());
            assert!(!module.manifest().required_features().image_query());
            let sample = ops(&module, opcode)[0][1];
            let extracts: Vec<_> = ops(&module, 81)
                .iter()
                .filter(|record| record[2] == sample)
                .map(|record| record[3])
                .collect();
            assert_eq!(
                extracts,
                (0..4)
                    .filter(|bit| mask & (1 << bit) != 0)
                    .collect::<Vec<_>>()
            );
            if op != 0x20 {
                assert_eq!(ops(&module, 88)[0][4], 2);
            }
            let direct = compile(
                &program,
                &pixel()
                    .with_images(
                        vec![ImageDescription {
                            user_sgpr: 0,
                            min_lod_u4_8: 0,
                        }],
                        vec![SamplerDescription { user_sgpr: 8 }],
                    )
                    .unwrap(),
                &Limits::default(),
            )
            .unwrap();
            assert_eq!(module.spirv(), direct.spirv());
            assert!(module.retained_host_bytes() <= module.charged_bytes());
            validate(&format!("original-image-{op}-{mask}"), &module);
        }
    }
    // An in-place image result must still sample the original v0/v1 coordinates.
    let mut words = source(0x20, 15);
    words[6] &= !(0xff << 8);
    words[8] = 0x0302_0100;
    let module = staged(&words).unwrap();
    assert_eq!(ops(&module, 87).len(), 1);
    validate("original-image-coordinate-alias", &module);
}

#[test]
fn original_pixel_lod_query_exports_both_genuine_components_and_declares_capability() {
    let module = staged(&source(0x60, 3)).unwrap();
    assert_eq!(ops(&module, 105).len(), 1);
    assert!(ops(&module, 87).is_empty());
    assert!(ops(&module, 88).is_empty());
    assert!(module.manifest().required_features().image_query());
    assert!(ops(&module, 17).contains(&vec![50]));
    let result = ops(&module, 105)[0][1];
    let channels: Vec<_> = ops(&module, 81)
        .iter()
        .filter(|record| record[2] == result)
        .map(|record| record[3])
        .collect();
    assert_eq!(channels, [0, 1]);
    validate("original-image-query-lod", &module);
}

#[test]
fn image_descriptor_provenance_and_exact_specialization_refuse_missing_unused_and_mutated_data() {
    let words = source(0x20, 15);
    let program = ShaderProgram::decode(&words, ShaderStage::Pixel, &Limits::default()).unwrap();
    assert!(matches!(
        analyze(&program, pixel(), &Limits::default())
            .unwrap()
            .finish(&[]),
        Err(CompileError::Resource(ResourceError {
            kind: ResourceErrorKind::MissingImage { user_sgpr: 0 },
            ..
        }))
    ));
    assert!(matches!(
        analyze(&program, pixel(), &Limits::default())
            .unwrap()
            .finish_with_images(
                &[],
                &[ImageDescription {
                    user_sgpr: 0,
                    min_lod_u4_8: 0
                }],
                &[]
            ),
        Err(CompileError::Resource(ResourceError {
            kind: ResourceErrorKind::MissingSampler { user_sgpr: 8 },
            ..
        }))
    ));
    assert!(matches!(
        analyze(&program, pixel(), &Limits::default())
            .unwrap()
            .finish_with_images(
                &[],
                &[
                    ImageDescription {
                        user_sgpr: 0,
                        min_lod_u4_8: 0
                    },
                    ImageDescription {
                        user_sgpr: 4,
                        min_lod_u4_8: 0
                    }
                ],
                &[SamplerDescription { user_sgpr: 8 }]
            ),
        Err(CompileError::Resource(ResourceError {
            kind: ResourceErrorKind::UnusedImage { user_sgpr: 4 },
            ..
        }))
    ));
    let mut mutation = vec![fixtures::scalar_move(3, 128)];
    mutation.extend(words.clone());
    let program = ShaderProgram::decode(&mutation, ShaderStage::Pixel, &Limits::default()).unwrap();
    assert!(matches!(
        analyze(&program, pixel(), &Limits::default()),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::DynamicDescriptor,
            ..
        }))
    ));
    // Copies retain their original user-data provenance, including all eight image words.
    let mut copied: Vec<_> = (0..8).map(|i| fixtures::scalar_move(16 + i, i)).collect();
    let mut relocated = words;
    relocated[6] |= 4 << 16;
    copied.extend(relocated);
    let module = staged(&copied).unwrap();
    assert_eq!(module.manifest().images()[0].user_sgpr, 0);
    // Dead samples confer no image or sampler requirement and no descriptor binding.
    let mut dead = source(0x20, 15);
    dead.truncate(7);
    for reg in 4..8 {
        dead.push(fixtures::vector_unary(1, reg, 128));
    }
    dead.extend(fixtures::export(0, true, [4, 5, 6, 7]));
    dead.push(fixtures::END);
    let program = ShaderProgram::decode(&dead, ShaderStage::Pixel, &Limits::default()).unwrap();
    let analysis = analyze(&program, pixel(), &Limits::default()).unwrap();
    assert!(analysis.images().is_empty() && analysis.samplers().is_empty());
    assert!(analysis.finish(&[]).unwrap().manifest().images().is_empty());
}

#[test]
fn image_modes_profiles_and_compiler_limits_fail_without_approximate_sampling() {
    for op in [0x21, 0x22, 0x25, 0x28, 0x30] {
        assert!(matches!(
            staged(&source(op, 15)),
            Err(CompileError::Unsupported(Unsupported {
                reason: UnsupportedReason::ImageMode,
                ..
            }))
        ));
    }
    for bit in [15, 16, 17] {
        let mut words = source(0x20, 15);
        words[5] |= 1 << bit;
        assert!(matches!(
            staged(&words),
            Err(CompileError::Unsupported(Unsupported {
                reason: UnsupportedReason::ImageMode,
                ..
            }))
        ));
    }
    let mut words = source(0x20, 15);
    words[5] &= !(7 << 3); // 1D rather than the admitted 2D profile
    assert!(matches!(
        staged(&words),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::ImageMode,
            ..
        }))
    ));
    let mut profile = pixel().description().clone();
    let StageDescription::Pixel(pixel_desc) = &mut profile else {
        unreachable!()
    };
    pixel_desc.common.images.push(ImageDescription {
        user_sgpr: 12,
        min_lod_u4_8: 0,
    }); // requires s12..s19
    assert!(matches!(
        Profile::new(profile),
        Err(ProfileError::ResourceRegister)
    ));
    let limits = Limits {
        max_spirv_words: 16,
        ..Limits::default()
    };
    let program =
        ShaderProgram::decode(&source(0x20, 15), ShaderStage::Pixel, &Limits::default()).unwrap();
    assert!(matches!(
        analyze(&program, pixel(), &limits)
            .unwrap()
            .finish_with_images(
                &[],
                &[ImageDescription {
                    user_sgpr: 0,
                    min_lod_u4_8: 0
                }],
                &[SamplerDescription { user_sgpr: 8 }]
            ),
        Err(CompileError::LimitExceeded { .. })
    ));
}
