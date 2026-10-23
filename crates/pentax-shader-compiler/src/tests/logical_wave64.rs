// SPDX-License-Identifier: GPL-2.0-only
// Original guest shaders and SPIR-V data-flow consumers; no hardware oracle.
use super::*;
use std::collections::BTreeMap;

fn description(wave: WaveSize, subgroup: bool, size: [u32; 3]) -> ComputeDescription {
    let StageDescription::Compute(mut cs) = profile(ShaderStage::Compute).description().clone()
    else {
        unreachable!()
    };
    cs.common.wave_size = wave;
    cs.common.subgroup = subgroup.then_some(SubgroupProfile { size: wave.lanes() });
    cs.common.user_sgpr_count = 6;
    cs.workgroup_size = size;
    cs
}

fn masked_words(branch: bool, data: u32) -> Vec<u32> {
    let mut words = vec![
        fixtures::vector_unary(1, 1, data),
        fixtures::vector_binary(0x1a, 2, 130, 0),
        0xbefe_0404, // S_MOV_B64 EXEC, s[4:5].
    ];
    if branch {
        words.push(0xbf88_0002); // S_CBRANCH_EXECZ past the two-word store.
    }
    words.extend(fixtures::store(1, 2));
    words.push(fixtures::END);
    words
}

fn compiled(words: &[u32], cs: ComputeDescription) -> CompiledShader {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(cs)).unwrap();
    let module = compile(&program, &profile, &limits).unwrap();
    validate("logical-wave64", &module);
    module
}

fn instructions(words: &[u32]) -> Vec<(u16, &[u32])> {
    let mut result = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0 && at + count <= words.len());
        result.push((words[at] as u16, &words[at + 1..at + count]));
        at += count;
    }
    result
}

fn builtins(module: &CompiledShader, builtin: u32) -> Vec<u32> {
    instructions(module.spirv())
        .iter()
        .filter_map(|(op, a)| match (*op, *a) {
            (71, [variable, 11, kind]) if *kind == builtin => Some(*variable),
            _ => None,
        })
        .collect()
}

fn portable(module: &CompiledShader) {
    let manifest = module.manifest();
    assert_eq!(manifest.required_subgroup_size(), None);
    assert!(!manifest.required_features().subgroup_basic());
    assert!(!manifest.required_features().subgroup_ballot());
    assert_eq!(builtins(module, 29).len(), 1); // LocalInvocationIndex.
    assert!(builtins(module, 41).is_empty()); // SubgroupLocalInvocationId.
    assert_eq!(
        manifest
            .inputs()
            .iter()
            .filter(|&&i| i == InputInterface::LocalInvocationIndex)
            .count(),
        1
    );
    assert!(
        !manifest
            .inputs()
            .contains(&InputInterface::SubgroupLocalInvocationId)
    );
}

// Evaluate only the emitted scalar integer SSA slice rooted at the actual
// LocalInvocationIndex input. Unknown memory/control flow stays unknown; this
// does not execute a shader or stand in for the separate physical GPU test.
fn integer_slice(module: &CompiledShader, local: u32) -> (Vec<u32>, Vec<u32>) {
    let input = builtins(module, 29)[0];
    let mut values = BTreeMap::new();
    let mut lanes = Vec::new();
    let mut stored = Vec::new();
    for (op, a) in instructions(module.spirv()) {
        if op == 62 {
            if let Some(&value) = values.get(&a[1]) {
                stored.push(value);
            }
            continue;
        }
        if a.len() < 2 {
            continue;
        }
        let value = match op {
            43 if a.len() == 3 => Some(a[2]), // OpConstant, 32-bit fixture values.
            61 if a[2] == input => Some(local),
            124 => values.get(&a[2]).copied(), // OpBitcast of known scalar bits.
            128 | 130 | 174 | 176 | 194 | 196 | 199 if a.len() == 4 => values
                .get(&a[2])
                .zip(values.get(&a[3]))
                .map(|(&x, &y)| match op {
                    128 => x.wrapping_add(y),
                    130 => x.wrapping_sub(y),
                    174 => u32::from(x >= y),
                    176 => u32::from(x < y),
                    194 => x.checked_shr(y).expect("defined fixture shift"),
                    196 => x.checked_shl(y).expect("defined fixture shift"),
                    199 => x & y,
                    _ => unreachable!(),
                }),
            169 => values
                .get(&a[2])
                .and_then(|&condition| values.get(&a[if condition != 0 { 3 } else { 4 }]).copied()),
            205 => values.get(&a[2]).map(|x| x.count_ones()),
            _ => None,
        };
        if let Some(value) = value {
            // The actual emitted lane definition must load the input and AND63.
            if op == 199 && a.len() == 4 && values.get(&a[3]) == Some(&63) {
                lanes.push(value);
            }
            values.insert(a[1], value);
        }
    }
    (lanes, stored)
}

#[test]
fn dynamic_exec64_uses_logical_lane_without_native_subgroup() {
    let words = masked_words(false, 135);
    for subgroup in [false, true] {
        for size in [[64, 1, 1], [128, 1, 1], [8, 4, 4]] {
            let module = compiled(&words, description(WaveSize::Wave64, subgroup, size));
            portable(&module);
            assert!(
                instructions(module.spirv())
                    .iter()
                    .any(|(op, a)| { *op == 16 && a[1] == 17 && a[2..] == size })
            );
            // Independent literal linear-index/guest-lane pairs include both
            // halves and the next logical wave; workgroup IDs do not enter it.
            for (index, expected) in [(0, 0), (31, 31), (32, 32), (63, 63), (64, 0), (127, 63)] {
                if index >= size.into_iter().product() {
                    continue;
                }
                let (lanes, _) = integer_slice(&module, index);
                assert!(!lanes.is_empty());
                assert!(lanes.iter().all(|&lane| lane == expected));
            }
        }
    }
}

#[test]
fn mbcnt64_consumes_linear_index_across_halves_and_multidimensional_groups() {
    let mut words = vec![
        fixtures::vector_unary(1, 3, 128),
        fixtures::vector_binary(0x23, 1, 193, 3), // MBCNT_LO -1, 0.
        fixtures::vector_binary(0x24, 1, 193, 1), // MBCNT_HI -1, low count.
        fixtures::vector_binary(0x1a, 2, 130, 0),
    ];
    words.extend(fixtures::store(1, 2));
    words.push(fixtures::END);
    let module = compiled(&words, description(WaveSize::Wave64, false, [8, 4, 4]));
    portable(&module);
    // X-major coordinates and independent expected guest lane numbers. The
    // evaluated output depends only on local inputs, not a global/workgroup ID.
    for (xyz, expected) in [
        ([7, 3, 0], 31),
        ([0, 0, 1], 32),
        ([7, 3, 1], 63),
        ([0, 0, 2], 0),
        ([7, 3, 3], 63),
    ] {
        let local = xyz[0] + 8 * (xyz[1] + 4 * xyz[2]);
        let (_, stored) = integer_slice(&module, local);
        assert_eq!(stored, [expected]);
    }
}

#[test]
fn existing_local_index_input_is_shared_with_logical_lane_and_tg_size() {
    let mut cs = description(WaveSize::Wave64, false, [128, 1, 1]);
    cs.tg_size = true;
    // Read the actual TG_SIZE SGPR as output as well as the EXEC mask input.
    let module = compiled(&masked_words(false, 16), cs);
    portable(&module);
    assert_eq!(builtins(&module, 29).len(), 1);
}

// collect_info declares LocalInvocationIndex for thread_ids > 0 even when
// it is unused. Prove the actual mask bit and (for Wave64) mask-half selection
// consume native subgroup lane SSA, rather than rejecting a valid interface.
fn native_mask_lane(module: &CompiledShader, wave64: bool) {
    let subgroup = builtins(module, 41);
    let local = builtins(module, 29);
    assert_eq!(subgroup.len(), 1);
    assert_eq!(local.len(), 1);
    let ops = instructions(module.spirv());
    let loads: Vec<_> = ops
        .iter()
        .filter_map(|(op, a)| match *op {
            61 if a[2] == subgroup[0] => Some(a[1]),
            _ => None,
        })
        .collect();
    assert!(!loads.is_empty());
    // No load from the separately declared local index means that input cannot
    // supply a logical AND63 lane path in this original shader.
    assert!(!ops.iter().any(|(op, a)| *op == 61 && a[2] == local[0]));
    let constant = |id, expected| {
        ops.iter()
            .any(|(op, a)| *op == 43 && a.len() == 3 && a[1] == id && a[2] == expected)
    };
    let bits: Vec<_> = ops
        .iter()
        .filter_map(|(op, a)| match *op {
            199 if loads.contains(&a[2]) && constant(a[3], 31) => Some(a[1]),
            _ => None,
        })
        .collect();
    assert!(!bits.is_empty());
    assert!(ops.iter().any(|(op, a)| *op == 194 && bits.contains(&a[3])));
    if wave64 {
        let halves: Vec<_> = ops
            .iter()
            .filter_map(|(op, a)| match *op {
                176 if loads.contains(&a[2]) && constant(a[3], 32) => Some(a[1]),
                _ => None,
            })
            .collect();
        assert!(!halves.is_empty());
        let masks: Vec<_> = ops
            .iter()
            .filter_map(|(op, a)| match *op {
                169 if halves.contains(&a[2]) => Some(a[1]),
                _ => None,
            })
            .collect();
        assert!(
            ops.iter()
                .any(|(op, a)| { *op == 194 && masks.contains(&a[2]) && bits.contains(&a[3]) })
        );
    }
}

#[test]
fn cross_lane_and_wave32_operations_keep_the_native_subgroup_contract() {
    let words = masked_words(true, 135);
    let module = compiled(&words, description(WaveSize::Wave64, true, [64, 1, 1]));
    assert_eq!(module.manifest().required_subgroup_size(), Some(64));
    assert!(module.manifest().required_features().subgroup_basic());
    assert!(module.manifest().required_features().subgroup_ballot());
    assert_eq!(builtins(&module, 41).len(), 1);
    native_mask_lane(&module, true);
    let limits = Limits::default();
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    let no_subgroup = Profile::new(StageDescription::Compute(description(
        WaveSize::Wave64,
        false,
        [64, 1, 1],
    )))
    .unwrap();
    assert!(matches!(
        compile(&program, &no_subgroup, &limits),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::RequiresExactSubgroup,
            ..
        }))
    ));
    let words = masked_words(false, 135);
    let module = compiled(&words, description(WaveSize::Wave32, true, [64, 1, 1]));
    assert_eq!(module.manifest().required_subgroup_size(), Some(32));
    assert!(module.manifest().required_features().subgroup_basic());
    assert_eq!(builtins(&module, 41).len(), 1);
    native_mask_lane(&module, false);
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    let no_subgroup = Profile::new(StageDescription::Compute(description(
        WaveSize::Wave32,
        false,
        [64, 1, 1],
    )))
    .unwrap();
    assert!(matches!(
        compile(&program, &no_subgroup, &limits),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::RequiresExactSubgroup,
            ..
        }))
    ));
}
