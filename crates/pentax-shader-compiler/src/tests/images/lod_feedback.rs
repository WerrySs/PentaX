// SPDX-License-Identifier: GPL-2.0-only
// Original shaders and structural checks; no host counter model or GPU success
// is substituted for executing the generated shader in the backend fixture.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn feedback_profile(enabled: bool, subgroup: bool) -> Profile {
    let mut common = CommonDescription::new(WaveSize::Wave32);
    common.lod_feedback = enabled;
    if subgroup {
        common.subgroup = Some(SubgroupProfile { size: 32 });
    }
    Profile::new(StageDescription::Pixel(PixelDescription::new(common))).unwrap()
}

fn counter() -> LodCounterDescription {
    LodCounterDescription {
        user_sgpr: 0,
        counter_id: 7,
        base_level: 1,
        min_lod_u4_8: 512,
    }
}

fn compile_feedback(
    words: &[u32],
    enabled: bool,
    subgroup: bool,
    counters: &[LodCounterDescription],
) -> Result<CompiledShader, CompileError> {
    let program = ShaderProgram::decode(words, ShaderStage::Pixel, &Limits::default())?;
    let analysis = analyze(
        &program,
        feedback_profile(enabled, subgroup),
        &Limits::default(),
    )?;
    let min_lod = counters.first().map_or(0, |description| {
        description
            .min_lod_u4_8
            .saturating_sub(u16::from(description.base_level) * 256)
    });
    analysis.finish_with_lod_feedback(
        &[],
        &[ImageDescription {
            user_sgpr: 0,
            min_lod_u4_8: min_lod,
        }],
        &[SamplerDescription { user_sgpr: 8 }],
        counters,
    )
}

fn dead_sample(op: u32, exec: Option<u32>) -> Vec<u32> {
    let original = source(op, 15);
    let mut words = original[..5].to_vec();
    for reg in 4..8 {
        words.push(fixtures::vector_unary(1, reg, 128));
    }
    if let Some(exec) = exec {
        words.push(exec);
    }
    words.extend(image(op, 15, 0, 8, 4));
    for reg in 4..8 {
        words.push(fixtures::vector_unary(1, reg, 128));
    }
    words.extend(fixtures::export(0, true, [4, 5, 6, 7]));
    words.push(fixtures::END);
    words
}

fn constants(module: &CompiledShader) -> BTreeMap<u32, u32> {
    ops(module, 43)
        .into_iter()
        .filter(|record| record.len() == 3)
        .map(|record| (record[1], record[2]))
        .collect()
}

type BlockInstructions = BTreeMap<u32, Vec<(u16, Vec<u32>)>>;

fn blocks(module: &CompiledShader) -> BlockInstructions {
    let mut result = BTreeMap::new();
    let mut block = None;
    let mut at = 5;
    while at < module.spirv().len() {
        let header = module.spirv()[at];
        let count = (header >> 16) as usize;
        let opcode = header as u16;
        let operands = &module.spirv()[at + 1..at + count];
        if opcode == 248 {
            block = Some(operands[0]);
        }
        if let Some(block) = block {
            result
                .entry(block)
                .or_insert_with(Vec::new)
                .push((opcode, operands.to_vec()));
        }
        at += count;
    }
    result
}

fn assert_atomic_guards(module: &CompiledShader) {
    let blocks = blocks(module);
    let labels: BTreeSet<_> = blocks.keys().copied().collect();
    let first = *blocks.keys().min().unwrap();
    let entry = blocks
        .iter()
        .find(|(_, records)| records.iter().any(|(op, _)| *op == 249))
        .map_or(first, |(label, _)| *label);
    let mut predecessors: BTreeMap<u32, BTreeSet<u32>> = labels
        .iter()
        .map(|&label| (label, BTreeSet::new()))
        .collect();
    for (&label, records) in &blocks {
        for (op, operands) in records {
            let targets = match *op {
                249 => &operands[..1],
                250 => &operands[1..3],
                _ => &[],
            };
            for &target in targets {
                predecessors.get_mut(&target).unwrap().insert(label);
            }
        }
    }
    // The function entry has no predecessor. This does not depend on id numbering.
    let entry = predecessors
        .iter()
        .find(|(_, incoming)| incoming.is_empty())
        .map_or(entry, |(&label, _)| label);
    let mut dominators: BTreeMap<_, _> = labels
        .iter()
        .map(|&label| {
            (
                label,
                if label == entry {
                    BTreeSet::from([entry])
                } else {
                    labels.clone()
                },
            )
        })
        .collect();
    for _ in 0..labels.len() {
        let old = dominators.clone();
        for &label in labels.iter().filter(|&&label| label != entry) {
            let mut current = labels.clone();
            for predecessor in &predecessors[&label] {
                current = current.intersection(&old[predecessor]).copied().collect();
            }
            current.insert(label);
            dominators.insert(label, current);
        }
        if old == dominators {
            break;
        }
    }
    let helper_not = ops(module, 168)[0][1];
    let eligible = ops(module, 167)
        .into_iter()
        .find(|record| record[3] == helper_not)
        .unwrap()[1];
    let elected = ops(module, 333)[0][1];
    let branches = ops(module, 250);
    let eligible_body = branches
        .iter()
        .find(|record| record[0] == eligible)
        .unwrap()[1];
    let elected_body = branches.iter().find(|record| record[0] == elected).unwrap()[1];
    for (&block, records) in &blocks {
        if records.iter().any(|(op, _)| matches!(*op, 234 | 237)) {
            assert!(dominators[&block].contains(&eligible_body));
            assert!(dominators[&block].contains(&elected_body));
        }
    }
}

#[test]
fn feedback_retains_dead_color_samples_and_records_actual_subgroup_atomic_operations() {
    let words = dead_sample(0x27, None);
    let program = ShaderProgram::decode(&words, ShaderStage::Pixel, &Limits::default()).unwrap();
    let old = analyze(&program, feedback_profile(false, false), &Limits::default()).unwrap();
    assert!(old.images().is_empty());
    let old = old.finish(&[]).unwrap();
    assert!(ops(&old, 88).is_empty());
    let module = compile_feedback(&words, true, true, &[counter()]).unwrap();
    assert_eq!(ops(&module, 88).len(), 1);
    assert_eq!(ops(&module, 354).len(), 1); // actual subgroup UMin
    assert_eq!(ops(&module, 357).len(), 1); // actual subgroup UMax
    assert_eq!(ops(&module, 333).len(), 1); // one eligible elected writer
    assert_eq!(ops(&module, 237).len(), 1); // actual device AtomicUMin
    assert_eq!(ops(&module, 234).len(), 1); // actual device AtomicIAdd
    let clamp = ops(&module, 12)
        .into_iter()
        .find(|record| record[3] == 43)
        .unwrap();
    let floor = ops(&module, 12)
        .into_iter()
        .find(|record| record[3] == 8)
        .unwrap();
    assert_eq!(floor[4], clamp[1]);
    // A negative unclamped LOD with threshold zero must not look like a clamp.
    assert_eq!(ops(&module, 184)[0][2], clamp[1]);
    assert_eq!(module.manifest().lod_feedback_binding(), Some(90));
    assert_eq!(module.manifest().required_subgroup_size(), Some(32));
    assert!(module.manifest().required_features().subgroup_basic());
    assert!(module.manifest().required_features().subgroup_arithmetic());
    assert!(
        module
            .manifest()
            .inputs()
            .contains(&InputInterface::HelperInvocation)
    );
    assert_eq!(
        module.manifest().lod_counters(),
        [LodCounterElement {
            user_sgpr: 0,
            profile_index: 0,
            counter_id: 7,
            base_level: 1,
            min_lod_u4_8: 512,
            first_use_pc: 36,
        }]
    );
    let constants = constants(&module);
    let binding = ops(&module, 71)
        .into_iter()
        .find(|record| record[1..] == [33, 90])
        .unwrap()[0];
    let indices: Vec<_> = ops(&module, 65)
        .into_iter()
        .filter(|record| record[2] == binding)
        .map(|record| constants[&record[4]])
        .collect();
    assert_eq!(indices, [7, 264]);
    for atomic in [ops(&module, 237)[0].clone(), ops(&module, 234)[0].clone()] {
        assert_eq!(constants[&atomic[3]], 1); // Device scope
        assert_eq!(constants[&atomic[4]], 0); // relaxed atomics, real command barriers outside
    }
    assert_eq!(constants[&ops(&module, 234)[0][5]], 1);
    assert!(
        ops(&module, 28)
            .iter()
            .any(|record| constants.get(&record[2]) == Some(&514))
    );
    assert_atomic_guards(&module);
    assert!(module.retained_host_bytes() <= module.charged_bytes());
    validate("original-lod-feedback-dead-sample", &module);
}

#[test]
fn feedback_queries_real_implicit_lod_and_excludes_helpers_and_guest_exec_inactive_lanes() {
    for exec in [None, Some(0xbefe_0480)] {
        // real S_MOV_B64 EXEC,0
        let module = compile_feedback(&dead_sample(0x20, exec), true, true, &[counter()]).unwrap();
        assert_eq!(ops(&module, 105).len(), 1);
        assert!(module.manifest().required_features().image_query());
        assert!(
            ops(&module, 71)
                .iter()
                .any(|record| record[1..] == [11, 23])
        );
        let query = ops(&module, 105)[0][1];
        assert!(
            ops(&module, 81)
                .iter()
                .any(|record| record[2..] == [query, 1])
        );
        assert_eq!(ops(&module, 168).len(), 1); // excludes actual helper invocation
        if exec.is_some() {
            let false_value = ops(&module, 42)[0][1];
            let helper_not = ops(&module, 168)[0][1];
            let eligible = ops(&module, 167)
                .into_iter()
                .find(|record| record[3] == helper_not)
                .unwrap();
            assert_eq!(eligible[2], false_value);
        }
        assert_atomic_guards(&module);
        validate("original-lod-feedback-exec", &module);
    }
    let module = compile_feedback(&source(0x24, 15), true, false, &[]).unwrap();
    assert!(module.manifest().lod_feedback_binding().is_none());
    assert!(ops(&module, 237).is_empty() && ops(&module, 234).is_empty());
    assert!(!module.manifest().required_features().subgroup_arithmetic());
    // A genuine query by itself is not an actual sample and never increments a counter.
    let module = compile_feedback(&source(0x60, 3), true, false, &[counter()]).unwrap();
    assert!(module.manifest().lod_feedback_binding().is_none());
    assert!(module.manifest().lod_counters().is_empty());
}

#[test]
fn per_image_sampler_pairs_preserve_shared_source_identity_and_separate_minimum_lod() {
    let mut words = source(0x24, 1)[..5].to_vec();
    words.extend(image(0x24, 1, 0, 16, 4));
    words.extend(image(0x24, 1, 8, 16, 5));
    words.extend(fixtures::export(0, true, [4, 5, 4, 5]));
    words.push(fixtures::END);
    let mut common = CommonDescription::new(WaveSize::Wave32);
    common.user_sgpr_count = 20;
    let profile = Profile::new(StageDescription::Pixel(PixelDescription::new(common))).unwrap();
    let program = ShaderProgram::decode(&words, ShaderStage::Pixel, &Limits::default()).unwrap();
    let analysis = analyze(&program, profile, &Limits::default()).unwrap();
    assert_eq!(analysis.samplers().len(), 1);
    assert_eq!(analysis.samplers()[0].user_sgpr, 16);
    let module = analysis
        .finish_with_images(
            &[],
            &[
                ImageDescription {
                    user_sgpr: 0,
                    min_lod_u4_8: 256,
                },
                ImageDescription {
                    user_sgpr: 8,
                    min_lod_u4_8: 512,
                },
            ],
            &[SamplerDescription { user_sgpr: 16 }],
        )
        .unwrap();
    assert_eq!(module.manifest().samplers().len(), 2);
    for (entry, image, binding) in [
        (&module.manifest().samplers()[0], 0, 74),
        (&module.manifest().samplers()[1], 8, 75),
    ] {
        assert_eq!(entry.user_sgpr, 16);
        assert_eq!(entry.image_user_sgpr, image);
        assert_eq!(entry.profile_index, 0);
        assert_eq!(entry.binding, binding);
    }
    assert_eq!(module.manifest().images()[0].min_lod_u4_8, 256);
    assert_eq!(module.manifest().images()[1].min_lod_u4_8, 512);
    assert_eq!(ops(&module, 86).len(), 2);
    assert_ne!(ops(&module, 86)[0][3], ops(&module, 86)[1][3]);
    validate("original-image-shared-sampler-pairs", &module);
    // Two genuine textures may contribute to the same hardware counter ID.
    let mut common = CommonDescription::new(WaveSize::Wave32);
    common.user_sgpr_count = 20;
    common.lod_feedback = true;
    common.subgroup = Some(SubgroupProfile { size: 32 });
    let profile = Profile::new(StageDescription::Pixel(PixelDescription::new(common))).unwrap();
    let analysis = analyze(&program, profile, &Limits::default()).unwrap();
    let module = analysis
        .finish_with_lod_feedback(
            &[],
            &[
                ImageDescription {
                    user_sgpr: 0,
                    min_lod_u4_8: 256,
                },
                ImageDescription {
                    user_sgpr: 8,
                    min_lod_u4_8: 512,
                },
            ],
            &[SamplerDescription { user_sgpr: 16 }],
            &[
                LodCounterDescription {
                    user_sgpr: 0,
                    counter_id: 7,
                    base_level: 0,
                    min_lod_u4_8: 256,
                },
                LodCounterDescription {
                    user_sgpr: 8,
                    counter_id: 7,
                    base_level: 0,
                    min_lod_u4_8: 512,
                },
            ],
        )
        .unwrap();
    assert_eq!(module.manifest().lod_counters().len(), 2);
    assert!(
        module
            .manifest()
            .lod_counters()
            .iter()
            .all(|counter| counter.counter_id == 7)
    );
    assert_eq!(ops(&module, 237).len(), 2);
    validate("original-image-shared-counter-id", &module);
}

#[test]
fn feedback_cannot_be_enabled_after_dce_or_without_exact_subgroup_and_descriptor_metadata() {
    let source = source(0x24, 15);
    assert!(matches!(
        compile_feedback(&source, false, true, &[counter()]),
        Err(CompileError::Profile(ProfileError::LodFeedbackProfile))
    ));
    assert!(matches!(
        compile_feedback(&source, true, false, &[counter()]),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::RequiresExactSubgroup,
            ..
        }))
    ));
    for bad in [
        LodCounterDescription {
            base_level: 15,
            ..counter()
        },
        LodCounterDescription {
            min_lod_u4_8: 4096,
            ..counter()
        },
        LodCounterDescription {
            user_sgpr: 4,
            ..counter()
        },
    ] {
        assert!(matches!(
            compile_feedback(&source, true, true, &[bad]),
            Err(CompileError::Profile(ProfileError::LodFeedbackProfile))
        ));
    }
    assert!(matches!(
        compile_feedback(&source, true, true, &[counter(), counter()]),
        Err(CompileError::Profile(ProfileError::LodFeedbackProfile))
    ));
    let program = ShaderProgram::decode(&source, ShaderStage::Pixel, &Limits::default()).unwrap();
    let analysis = analyze(&program, feedback_profile(true, true), &Limits::default()).unwrap();
    assert!(matches!(
        analysis.finish_with_lod_feedback(
            &[],
            &[ImageDescription {
                user_sgpr: 0,
                min_lod_u4_8: 0
            }],
            &[SamplerDescription { user_sgpr: 8 }],
            &[counter()]
        ),
        Err(CompileError::Profile(ProfileError::LodFeedbackProfile))
    ));
}
