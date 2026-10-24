// SPDX-License-Identifier: GPL-2.0-only
// Original AMD programs and bounded post-SSA consumers. No device execution
// or general shader interpreter is claimed by these CPU-only contract tests.
use super::*;
use crate::ir::{Builtin, InstId, Ir, Op, Value};
use std::collections::{BTreeMap, BTreeSet};

fn description(bytes: u32, size: u32) -> ComputeDescription {
    let StageDescription::Compute(mut cs) = profile(ShaderStage::Compute).description().clone()
    else {
        unreachable!()
    };
    cs.shared_memory_bytes = bytes;
    cs.workgroup_size = [size, 1, 1];
    cs.common.wave_size = WaveSize::Wave64;
    cs.common.subgroup = None;
    cs.common.user_sgpr_count = 6;
    cs
}

fn ds(
    op: u32,
    offset0: u32,
    offset1: u32,
    address: u32,
    first: u32,
    second: u32,
    dst: u32,
) -> [u32; 2] {
    [
        0xd800_0000 | (op << 18) | (offset1 << 8) | offset0,
        address | (first << 8) | (second << 16) | (dst << 24),
    ]
}

fn instructions(words: &[u32]) -> Vec<(u16, &[u32])> {
    let mut out = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count != 0 && at + count <= words.len());
        out.push((words[at] as u16, &words[at + 1..at + count]));
        at += count;
    }
    out
}

fn depends_on(
    id: u32,
    loads: &[u32],
    definitions: &BTreeMap<u32, Vec<u32>>,
    visited: &mut BTreeSet<u32>,
) -> bool {
    if loads.contains(&id) {
        return true;
    }
    if !visited.insert(id) {
        return false;
    }
    definitions.get(&id).is_some_and(|args| {
        args.iter()
            .any(|&arg| depends_on(arg, loads, definitions, visited))
    })
}

fn value(
    ir: &Ir,
    budget: &budget::Budget,
    values: &BTreeMap<InstId, u64>,
    v: Value,
) -> Option<u64> {
    match ir.resolve(budget, v).unwrap() {
        Value::U1(v) => Some(u64::from(v)),
        Value::U32(v) | Value::F32(v) => Some(u64::from(v)),
        Value::U64(v) => Some(v),
        Value::Inst(id) => values.get(&id).copied(),
    }
}

// Only the integer SSA slice used by these original straight-line shaders is
// interpreted. A DS instruction is visited across all invocations before the
// next instruction, just as the emitted unconditional Workgroup barriers
// require. Unknown operations stay unknown, and a consumed unknown fails.
fn stores(ir: &Ir, budget: &budget::Budget, count: usize, bytes: u32) -> Vec<u32> {
    let mut values = vec![BTreeMap::new(); count];
    let mut shared = vec![None; (bytes / 4) as usize];
    let mut out = vec![Vec::new(); count];
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            let inst = ir.inst(id);
            for (lane, (lane_values, lane_out)) in values.iter_mut().zip(out.iter_mut()).enumerate()
            {
                let arg = |i| value(ir, budget, lane_values, ir.arg(id, i));
                let a = arg(0);
                let b = arg(1);
                let c = arg(2);
                let scalar = match inst.op {
                    Op::Identity | Op::BitCastU32F32 | Op::BitCastF32U32 => a,
                    Op::GetBuiltin
                        if inst.aux == Builtin::LocalInvocationId.aux(0)
                            || inst.aux == Builtin::LocalInvocationIndex.aux(0) =>
                    {
                        Some(lane as u64)
                    }
                    Op::GetUserData => Some(match inst.aux {
                        4 => 0x8000_0000,
                        5 => 1,
                        _ => 0,
                    }),
                    Op::LaneId => Some((lane & 63) as u64),
                    Op::IAdd32 => a
                        .zip(b)
                        .map(|(x, y)| u64::from((x as u32).wrapping_add(y as u32))),
                    Op::BitwiseAnd32 => a.zip(b).map(|(x, y)| x & y),
                    Op::BitwiseOr32 => a.zip(b).map(|(x, y)| x | y),
                    Op::BitwiseXor32 => a.zip(b).map(|(x, y)| x ^ y),
                    Op::ShiftLeftLogical32 => a
                        .zip(b)
                        .map(|(x, y)| u64::from((x as u32).wrapping_shl(y as u32))),
                    Op::ShiftRightLogical32 => a
                        .zip(b)
                        .map(|(x, y)| u64::from((x as u32).wrapping_shr(y as u32))),
                    Op::IEqual32 => a.zip(b).map(|(x, y)| u64::from(x == y)),
                    Op::INotEqual32 => a.zip(b).map(|(x, y)| u64::from(x != y)),
                    Op::ULessThan32 => a.zip(b).map(|(x, y)| u64::from(x < y)),
                    Op::LogicalNot => a.map(|x| u64::from(x == 0)),
                    Op::LogicalAnd => a.zip(b).map(|(x, y)| u64::from(x != 0 && y != 0)),
                    Op::LogicalOr => a.zip(b).map(|(x, y)| u64::from(x != 0 || y != 0)),
                    Op::SelectU1 | Op::SelectU32 => a.and_then(|x| if x != 0 { b } else { c }),
                    Op::CompositeConstructU64 => a.zip(b).map(|(x, y)| x | (y << 32)),
                    Op::CompositeExtractU64 => a.map(|x| (x >> (inst.aux * 32)) & 0xffff_ffff),
                    Op::LoadSharedU32 => {
                        if b.unwrap() == 0 {
                            Some(0)
                        } else {
                            let index = a.unwrap() as usize / 4;
                            Some(if index < shared.len() {
                                shared[index].expect("read before original guest write")
                            } else {
                                0
                            })
                        }
                    }
                    Op::StoreSharedU32 => {
                        if c.unwrap() != 0 {
                            let index = a.unwrap() as usize / 4;
                            if index < shared.len() {
                                shared[index] = Some(b.unwrap());
                            }
                        }
                        None
                    }
                    Op::StoreBufferU32 => {
                        if arg(5).unwrap() != 0 {
                            lane_out
                                .push(arg(4).expect("actual output must depend on known LDS data")
                                    as u32);
                        }
                        None
                    }
                    _ => None,
                };
                if let Some(v) = scalar {
                    lane_values.insert(id, v);
                }
            }
        }
    }
    out.into_iter().flatten().collect()
}

fn compiled(words: &[u32], cs: ComputeDescription) -> CompiledShader {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(cs)).unwrap();
    let module = compile(&program, &profile, &limits).unwrap();
    validate("original-workgroup-lds", &module);
    module
}

#[test]
fn logical_wave64_lds_cross_half_data_and_exec_use_real_workgroup_storage() {
    let mut words = vec![
        fixtures::vector_unary(1, 4, 128),
        fixtures::vector_binary(0x25, 1, 135, 0),
        fixtures::vector_binary(0x1a, 2, 130, 0),
        fixtures::vector_binary(0x1d, 3, 160, 0),
        fixtures::vector_binary(0x1a, 3, 130, 3),
    ];
    words.extend(ds(0x0d, 0, 0, 2, 1, 0, 0));
    words.push(0xbf8a_0000); // Actual workgroup S_BARRIER.
    words.push(0xbefe_0404); // EXEC s[4:5]: lanes31 and32.
    words.extend(ds(0x36, 0, 0, 3, 0, 0, 4));
    words.push(0xbefe_04c1); // Restore EXEC to all lanes before output.
    words.extend(fixtures::store(4, 2));
    words.push(fixtures::END);
    let cs = description(512, 64);
    let limits = Limits::default();
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(cs.clone())).unwrap();
    let mut budget = budget::Budget::new(&limits);
    let (ir, _) = crate::lower(&program, &profile, &mut budget).unwrap();
    let actual = stores(&ir, &budget, 64, 512);
    let mut expected = vec![0; 64];
    expected[31] = 70; // (31 xor32)+7, across the native32 boundary.
    expected[32] = 7; // (32 xor32)+7.
    assert_eq!(actual, expected);
    let module = compiled(&words, cs);
    assert_eq!(module.manifest().shared_memory_bytes(), 512);
    assert_eq!(module.manifest().required_subgroup_size(), None);
    assert!(!module.manifest().required_features().subgroup_basic());
    assert!(!module.manifest().required_features().subgroup_ballot());
    assert!(
        module
            .manifest()
            .inputs()
            .contains(&InputInterface::LocalInvocationIndex)
    );
    assert!(
        !module
            .manifest()
            .inputs()
            .contains(&InputInterface::SubgroupLocalInvocationId)
    );
    let ops = instructions(module.spirv());
    let constant = |id, x| {
        ops.iter()
            .any(|(op, a)| *op == 43 && a[1] == id && a[2] == x)
    };
    let shared: Vec<_> = ops
        .iter()
        .filter_map(|(op, a)| match (*op, *a) {
            (59, [_, id, 4]) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(shared.len(), 1);
    let pointers: Vec<_> = ops
        .iter()
        .filter_map(|(op, a)| match (*op, *a) {
            (65, [_, id, object, _]) if shared.contains(object) => Some(*id),
            _ => None,
        })
        .collect();
    assert!(
        ops.iter()
            .any(|(op, a)| *op == 62 && pointers.contains(&a[0]))
    );
    assert!(
        ops.iter()
            .any(|(op, a)| *op == 61 && pointers.contains(&a[2]))
    );
    let loads: Vec<_> = ops
        .iter()
        .filter_map(|(op, a)| match (*op, *a) {
            (61, [_, id, pointer]) if pointers.contains(pointer) => Some(*id),
            _ => None,
        })
        .collect();
    let definitions: BTreeMap<_, _> = ops
        .iter()
        .filter_map(|(op, a)| match *op {
            61 | 124 | 128 | 169 | 194 | 196 | 199 | 245 => Some((a[1], a[2..].to_vec())),
            _ => None,
        })
        .collect();
    assert!(ops.iter().any(|(op, a)| *op == 62
        && !pointers.contains(&a[0])
        && depends_on(a[1], &loads, &definitions, &mut BTreeSet::new())));

    let barriers: Vec<_> = ops.iter().filter(|(op, _)| *op == 224).collect();
    assert_eq!(barriers.len(), 3); // WRITE completion, real S_BARRIER, READ completion.
    assert!(
        barriers
            .iter()
            // DS completions order LDS; S_BARRIER also orders storage buffers.
            .zip([0x108, 0x148, 0x108])
            .all(|((_, a), semantics)| constant(a[0], 2)
                && constant(a[1], 2)
                && constant(a[2], semantics))
    );
    assert!(ops.iter().any(|(op, a)| *op == 176 && constant(a[3], 128)));
}

#[test]
fn paired_lds_offsets_alias_snapshots_and_out_of_range_read_reach_real_outputs() {
    for (write, read, offsets, expected) in [
        (0x0e, 0x37, [0, 1], [35, 63]),
        (0x0f, 0x38, [1, 2], [35, 0]), // second ST64 offset512 is outside LDS.
        (0x0e, 0x37, [0, 0], [35, 35]), // primary first write wins coincident offsets.
    ] {
        let mut words = vec![
            fixtures::vector_unary(1, 1, 163),
            fixtures::vector_unary(1, 2, 128),
            fixtures::vector_unary(1, 3, 191),
            fixtures::vector_unary(1, 5, 132),
        ];
        words.extend(ds(write, offsets[0], offsets[1], 2, 1, 3, 0));
        // v2 is BOTH the address and first destination; the second read must
        // retain its original address before v2 is overwritten by35.
        words.extend(ds(read, offsets[0], offsets[1], 2, 0, 0, 2));
        words.extend(fixtures::store(2, 0));
        words.extend(fixtures::store(3, 5));
        words.push(fixtures::END);
        let cs = description(512, 1);
        let limits = Limits::default();
        let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
        let profile = Profile::new(StageDescription::Compute(cs.clone())).unwrap();
        let mut budget = budget::Budget::new(&limits);
        let (ir, _) = crate::lower(&program, &profile, &mut budget).unwrap();
        assert_eq!(stores(&ir, &budget, 1, 512), expected);
        let module = compiled(&words, cs.clone());
        let mut unspecialized = cs;
        unspecialized.common.buffers.clear();
        let staged = analyze(
            &program,
            Profile::new(StageDescription::Compute(unspecialized)).unwrap(),
            &limits,
        )
        .unwrap()
        .finish(&profile.common().buffers)
        .unwrap();
        assert_eq!(staged.manifest().shared_memory_bytes(), 512);
        assert_eq!(staged.spirv(), module.spirv());
    }
}

#[test]
fn lds_unknown_gds_control_flow_and_allocation_are_typed_refusals() {
    let limits = Limits::default();
    let mut valid = vec![fixtures::vector_unary(1, 2, 128)];
    valid.extend(ds(0x36, 0, 0, 2, 0, 0, 2));
    valid.extend(fixtures::store(2, 0));
    valid.push(fixtures::END);
    for (words, bytes, reason) in [
        (valid.clone(), 0, UnsupportedReason::SharedMemoryAllocation),
        (
            {
                let mut w = valid.clone();
                w[1] |= 1 << 17;
                w
            },
            512,
            UnsupportedReason::GlobalDataShare,
        ),
        (
            {
                let mut w = valid.clone();
                w[1] = (w[1] & !(0xff << 18)) | (0x76 << 18);
                w
            },
            512,
            UnsupportedReason::LocalDataShare,
        ),
        (
            {
                let mut w = valid.clone();
                w.insert(1, 0xbf88_0002);
                w
            },
            512,
            UnsupportedReason::SharedMemoryControlFlow,
        ),
    ] {
        let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
        let profile = Profile::new(StageDescription::Compute(description(bytes, 64))).unwrap();
        assert!(matches!(compile(&program,&profile,&limits),
            Err(CompileError::Unsupported(Unsupported { reason: actual, .. })) if actual == reason));
    }
    for bytes in [1, 511, 513, 65_537, 131_072] {
        assert!(matches!(
            Profile::new(StageDescription::Compute(description(bytes, 64))),
            Err(ProfileError::SharedMemorySize)
        ));
    }
    assert!(Profile::new(StageDescription::Compute(description(65_536, 64))).is_ok());
    // Existing true cross-lane/native-wave requirements stay unchanged.
    let mut cs = description(0, 64);
    let words = [
        fixtures::vector_unary(1, 1, 135),
        fixtures::vector_unary(1, 2, 128),
        0xbefe_0404,
        0xbf88_0002,
        fixtures::store(1, 2)[0],
        fixtures::store(1, 2)[1],
        fixtures::END,
    ];
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    assert!(matches!(
        compile(
            &program,
            &Profile::new(StageDescription::Compute(cs.clone())).unwrap(),
            &limits
        ),
        Err(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::RequiresExactSubgroup,
            ..
        }))
    ));
    cs.common.subgroup = Some(SubgroupProfile { size: 64 });
    let module = compiled(&words, cs);
    assert_eq!(module.manifest().required_subgroup_size(), Some(64));
    assert!(module.manifest().required_features().subgroup_ballot());
}
