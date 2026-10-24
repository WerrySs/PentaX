// SPDX-License-Identifier: GPL-2.0-only
// Original guest programs and bounded post-SSA integer consumers. These do
// not execute GPU code or claim a hardware oracle for dispatch preparation.
use super::*;
use crate::ir::{Builtin, InstId, Ir, Op, Value};
use std::collections::BTreeMap;

fn description(wave: WaveSize, local: [u32; 3]) -> ComputeDescription {
    let StageDescription::Compute(mut cs) = profile(ShaderStage::Compute).description().clone()
    else {
        unreachable!()
    };
    cs.common.wave_size = wave;
    cs.common.subgroup = (wave == WaveSize::Wave32).then_some(SubgroupProfile { size: 32 });
    cs.common.user_sgpr_count = 4;
    cs.workgroup_size = local;
    cs.dispatch_thread_dimensions = true;
    cs
}

fn mask_words(overwrite: bool) -> Vec<u32> {
    let mut words = vec![
        fixtures::scalar_move(6, 126), // s6 = actual EXEC_LO.
        fixtures::scalar_move(7, 127), // s7 = actual EXEC_HI.
    ];
    if overwrite {
        words.push(0xbefe_04c1);
    } // S_MOV_B64 EXEC, -1.
    words.extend([
        fixtures::vector_unary(1, 1, 6),
        fixtures::vector_unary(1, 2, 7),
        fixtures::vector_unary(1, 3, 128),
    ]);
    words.extend(fixtures::store(1, 3));
    words.push(fixtures::vector_unary(1, 3, 132));
    words.extend(fixtures::store(2, 3));
    words.push(fixtures::END);
    words
}

fn lowered(words: &[u32], cs: ComputeDescription) -> (Ir, budget::Budget) {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(cs)).unwrap();
    let mut budget = budget::Budget::new(&limits);
    let (ir, _) = lower(&program, &profile, &mut budget).unwrap();
    (ir, budget)
}

fn resolve(
    ir: &Ir,
    budget: &budget::Budget,
    values: &BTreeMap<InstId, u32>,
    v: Value,
) -> Option<u32> {
    match ir.resolve(budget, v).unwrap() {
        Value::U1(x) => Some(u32::from(x)),
        Value::U32(x) | Value::F32(x) => Some(x),
        Value::Inst(id) => values.get(&id).copied(),
        _ => None,
    }
}

// Consume the actual lowered SSA feeding each original buffer store. Unknown
// resource handles/control flow are not interpreted; every consumed scalar
// and store predicate must be known in this straight-line integer fixture.
fn stores(
    ir: &Ir,
    budget: &budget::Budget,
    linear: u32,
    group: [u32; 3],
    extent: [u32; 3],
) -> Vec<(Option<u32>, bool)> {
    let mut values = BTreeMap::new();
    let mut stores = Vec::new();
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            let inst = ir.inst(id);
            let arg = |i| resolve(ir, budget, &values, ir.arg(id, i));
            let a = arg(0);
            let b = arg(1);
            let c = arg(2);
            let value = match inst.op {
                Op::Identity | Op::BitCastU32F32 | Op::BitCastF32U32 => a,
                Op::GetDispatchThreadExtent => Some(extent[inst.aux as usize]),
                Op::GetBuiltin if inst.aux == Builtin::LocalInvocationIndex.aux(0) => Some(linear),
                Op::GetBuiltin if inst.aux >> 8 == Builtin::WorkgroupId.aux(0) >> 8 => {
                    Some(group[(inst.aux & 255) as usize])
                }
                Op::LaneId => Some(linear & 63),
                Op::IAdd32 => a.zip(b).map(|(x, y)| x.wrapping_add(y)),
                Op::ISub32 => a.zip(b).map(|(x, y)| x.wrapping_sub(y)),
                Op::IMul32 => a.zip(b).map(|(x, y)| x.wrapping_mul(y)),
                Op::UDiv32 => a
                    .zip(b)
                    .map(|(x, y)| x.checked_div(y).expect("nonzero local axis")),
                Op::BitwiseAnd32 => a.zip(b).map(|(x, y)| x & y),
                Op::BitwiseOr32 => a.zip(b).map(|(x, y)| x | y),
                Op::BitwiseXor32 => a.zip(b).map(|(x, y)| x ^ y),
                Op::ShiftLeftLogical32 => a
                    .zip(b)
                    .map(|(x, y)| x.checked_shl(y).expect("defined shift")),
                Op::ShiftRightLogical32 => a
                    .zip(b)
                    .map(|(x, y)| x.checked_shr(y).expect("defined shift")),
                Op::ULessThan32 => a.zip(b).map(|(x, y)| u32::from(x < y)),
                Op::IEqual32 => a.zip(b).map(|(x, y)| u32::from(x == y)),
                Op::INotEqual32 => a.zip(b).map(|(x, y)| u32::from(x != y)),
                Op::LogicalAnd => a.zip(b).map(|(x, y)| u32::from(x != 0 && y != 0)),
                Op::LogicalNot => a.map(|x| u32::from(x == 0)),
                Op::SelectU1 | Op::SelectU32 => a.and_then(|x| if x != 0 { b } else { c }),
                Op::StoreBufferU32 => {
                    let active = arg(5).expect("actual store predicate") != 0;
                    let data = active.then(|| arg(4).expect("active stored actual EXEC word"));
                    stores.push((data, active));
                    None
                }
                _ => None,
            };
            if let Some(value) = value {
                values.insert(id, value);
            }
        }
    }
    stores
}

fn module(words: &[u32], cs: ComputeDescription) -> CompiledShader {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(cs)).unwrap();
    let module = compile(&program, &profile, &limits).unwrap();
    validate("thread-dimensions", &module);
    module
}

fn opcodes(module: &CompiledShader) -> Vec<u16> {
    let mut at = 5;
    let mut out = Vec::new();
    while at < module.spirv().len() {
        let word = module.spirv()[at];
        let count = (word >> 16) as usize;
        assert!(count > 0 && at + count <= module.spirv().len());
        out.push(word as u16);
        at += count;
    }
    out
}

#[test]
fn initial_exec64_covers_both_halves_and_multidimensional_partial_groups() {
    let words = mask_words(false);
    let cs = description(WaveSize::Wave64, [8, 4, 4]);
    let (ir, budget) = lowered(&words, cs.clone());
    // In group(1,1,0), only x0..4/y0/z0..2 are within requested13x5x3.
    // X-major linear bits are0..4 and32..36, then64..68 in the next wave.
    for (linear, expected, active) in [
        (0, [0x1f, 0x1f], true),
        (31, [0x1f, 0x1f], false),
        (32, [0x1f, 0x1f], true),
        (63, [0x1f, 0x1f], false),
        (64, [0x1f, 0], true),
        (95, [0x1f, 0], false),
        (96, [0x1f, 0], false),
        (127, [0x1f, 0], false),
    ] {
        assert_eq!(
            stores(&ir, &budget, linear, [1, 1, 0], [13, 5, 3]),
            [
                (active.then_some(expected[0]), active),
                (active.then_some(expected[1]), active)
            ]
        );
    }
    let compiled = module(&words, cs);
    assert_eq!(compiled.manifest().required_subgroup_size(), None);
    assert!(!compiled.manifest().required_features().subgroup_ballot());
    assert!(!compiled.manifest().required_features().subgroup_basic());
    assert!(!opcodes(&compiled).iter().any(|op| (333..=366).contains(op)));
    assert!(compiled.manifest().dispatch_thread_extent_dword().is_some());
}

#[test]
fn wave32_and_later_guest_exec_writes_preserve_initial_only_tail_semantics() {
    for wave in [WaveSize::Wave32, WaveSize::Wave64] {
        let words = mask_words(true);
        let cs = description(wave, [64, 1, 1]);
        let (ir, budget) = lowered(&words, cs.clone());
        // The initial mask is captured to s6/s7 before the guest explicitly
        // enables all lanes. Even padded lane63 then reaches both stores.
        let expected = if wave == WaveSize::Wave64 {
            [u32::MAX, 0x3fff_ffff]
        } else {
            [0x3fff_ffff, 0]
        };
        assert_eq!(
            stores(&ir, &budget, 63, [0, 0, 0], [62, 1, 1]),
            [(Some(expected[0]), true), (Some(expected[1]), true)]
        );
        let compiled = module(&words, cs);
        assert!(compiled.manifest().dispatch_thread_extent_dword().is_some());
        assert!(!opcodes(&compiled).contains(&252)); // No OpKill substitute.
        assert_eq!(
            opcodes(&compiled).iter().filter(|&&op| op == 253).count(),
            1
        );
    }
    let mut words = vec![
        0xbefe_04c1,
        fixtures::vector_unary(1, 1, 129),
        fixtures::vector_unary(1, 2, 128),
    ];
    words.extend(fixtures::store(1, 2));
    words.push(fixtures::END);
    let compiled = module(&words, description(WaveSize::Wave64, [64, 1, 1]));
    // All initial-EXEC data is dead when guest code replaces it before use.
    assert_eq!(compiled.manifest().dispatch_thread_extent_dword(), None);

    let mut unsafe_words = vec![
        fixtures::vector_unary(1, 1, 129),
        fixtures::vector_unary(1, 2, 128),
        0xbefe_04c1,
    ];
    unsafe_words.extend(fixtures::store(1, 2));
    unsafe_words.push(fixtures::END);
    let limits = Limits::default();
    let program = ShaderProgram::decode(&unsafe_words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(description(
        WaveSize::Wave64,
        [64, 1, 1],
    )))
    .unwrap();
    // These VGPRs were written only under the initial tail mask. Enabling
    // previously inactive lanes without recomputing them must still refuse.
    assert!(matches!(
        compile(&program, &profile, &limits),
        Err(CompileError::UndefinedRegister {
            register: Register::Vgpr(_),
            ..
        })
    ));
    let mut undefined_sgpr = vec![
        fixtures::vector_unary(1, 1, 20),
        fixtures::vector_unary(1, 2, 128),
    ];
    undefined_sgpr.extend(fixtures::store(1, 2));
    undefined_sgpr.push(fixtures::END);
    let program = ShaderProgram::decode(&undefined_sgpr, ShaderStage::Compute, &limits).unwrap();
    assert!(matches!(
        compile(&program, &profile, &limits),
        Err(CompileError::UndefinedRegister {
            register: Register::Sgpr(20),
            ..
        })
    ));
    let loop_program =
        ShaderProgram::decode(&fixtures::scalar_loop(), ShaderStage::Compute, &limits).unwrap();
    // Initial EXEC is synthetic-entry invariant even across a genuine scalar
    // loop. Its guard can still prove the final first-VOP store is defined.
    assert!(compile(&loop_program, &profile, &limits).is_ok());
    let mut changing = vec![
        fixtures::scalar_move(10, 128),
        fixtures::scalar_move(11, 130),
        0xbefe_040a,
        fixtures::vector_unary(1, 1, 135),
        fixtures::scalar_add(10, 10, 129),
        0xbf00_0000 | (0x0a << 16) | (11 << 8) | 10,
        0xbf85_fffb,
        0xbefe_04c1,
        fixtures::vector_unary(1, 2, 128),
    ];
    changing.extend(fixtures::store(1, 2));
    changing.push(fixtures::END);
    let changing = ShaderProgram::decode(&changing, ShaderStage::Compute, &limits).unwrap();
    // This EXEC condition depends on an actual loop Phi. Same id in different
    // iterations is never used as a contradiction to waive an undefined read.
    assert!(matches!(
        compile(&changing, &profile, &limits),
        Err(CompileError::UndefinedRegister {
            register: Register::Vgpr(_),
            ..
        })
    ));
}

#[test]
fn runtime_extent_layout_is_disjoint_and_mode_is_explicit_without_extent_specialization() {
    let words = mask_words(false);
    let mut cs = description(WaveSize::Wave64, [64, 1, 1]);
    let compiled = module(&words, cs.clone());
    let layout = compiled.manifest().push_data().unwrap();
    let extent = compiled.manifest().dispatch_thread_extent_dword().unwrap();
    assert_eq!(extent as usize, layout.user_sgprs.len());
    assert_eq!(layout.memory_offset_dword, extent + 3);
    let (ir, budget) = lowered(&words, cs.clone());
    assert_eq!(
        stores(&ir, &budget, 32, [0; 3], [33, 1, 1]),
        [(Some(u32::MAX), true), (Some(1), true)]
    );
    assert_eq!(
        stores(&ir, &budget, 32, [0; 3], [34, 1, 1]),
        [(Some(u32::MAX), true), (Some(3), true)]
    );
    cs.common.push_data_start_dword = 32;
    let fallback = module(&words, cs.clone());
    assert!(fallback.manifest().push_data().is_none());
    assert!(fallback.manifest().shader_data_binding().is_some());
    assert!(fallback.manifest().dispatch_thread_extent_dword().is_some());
    cs.dispatch_thread_dimensions = false;
    // Without thread dimensions, observing initial EXEC keeps the legacy
    // entry ballot live. It still needs an actual exact native subgroup.
    let limits = Limits::default();
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    let no_subgroup = Profile::new(StageDescription::Compute(cs.clone())).unwrap();
    assert!(matches!(
        compile(&program, &no_subgroup, &limits),
        Err(CompileError::Unsupported(Unsupported {
            pc: 0,
            opcode: None,
            reason: UnsupportedReason::RequiresExactSubgroup,
        }))
    ));
    cs.common.subgroup = Some(SubgroupProfile { size: 64 });
    let old = module(&words, cs);
    assert_eq!(old.manifest().required_subgroup_size(), Some(64));
    assert!(old.manifest().required_features().subgroup_basic());
    assert!(old.manifest().required_features().subgroup_ballot());
    assert_eq!(old.manifest().dispatch_thread_extent_dword(), None);
    assert_ne!(old.spirv(), compiled.spirv());
}

#[test]
fn analytic_mask_generation_keeps_existing_ir_work_and_output_limits() {
    let words = mask_words(false);
    let cs = description(WaveSize::Wave64, [8, 4, 4]);
    let compiled = module(&words, cs.clone());
    let limits = Limits::default();
    let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
    let profile = Profile::new(StageDescription::Compute(cs)).unwrap();
    let output_limit = Limits {
        max_spirv_words: compiled.spirv().len() - 1,
        ..limits
    };
    assert!(matches!(
        compile(&program, &profile, &output_limit),
        Err(CompileError::LimitExceeded {
            kind: LimitKind::SpirvWords,
            ..
        })
    ));
    let work_limit = Limits {
        max_work: compiled.work_units() - 1,
        ..limits
    };
    assert!(matches!(
        compile(&program, &profile, &work_limit),
        Err(CompileError::LimitExceeded {
            kind: LimitKind::Work,
            ..
        })
    ));
}
