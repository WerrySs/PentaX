// SPDX-License-Identifier: GPL-2.0-only
//! Original decoded storage program; structural compiler proof, not device execution.
use super::*;
use crate::ir::{Ir, Op, Value};
use std::collections::BTreeSet;

fn depends_on_buffer_load(
    ir: &Ir,
    budget: &budget::Budget,
    value: Value,
    seen: &mut BTreeSet<crate::ir::InstId>,
) -> bool {
    let Value::Inst(id) = ir.resolve(budget, value).unwrap() else {
        return false;
    };
    if !seen.insert(id) {
        return false;
    }
    if ir.inst(id).op == Op::LoadBufferU32 {
        return true;
    }
    (0..ir.nargs(id)).any(|index| depends_on_buffer_load(ir, budget, ir.arg(id, index), seen))
}

fn instructions(words: &[u32]) -> Vec<(u16, &[u32])> {
    let mut records = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        assert!(count > 0 && at + count <= words.len());
        records.push((words[at] as u16, &words[at + 1..at + count]));
        at += count;
    }
    records
}

#[test]
fn compute_workgroup_barrier_preserves_storage_dependency_and_effective_coherence() {
    for (barrier, store_completion) in [(false, false), (true, false), (true, true)] {
        let mut words = vec![
            fixtures::vector_unary(1, 1, 137), // v1=9, original prefix store.
            fixtures::vector_unary(1, 2, 128), // v2=0 byte address.
            fixtures::vector_unary(1, 3, 128), // Define old destination before the masked load.
        ];
        words.extend(fixtures::store(1, 2));
        if store_completion {
            words.push(0xbbfd_0000); // Actual VSCNT null,0 before uniform workgroup barrier.
        }
        if barrier {
            words.push(0xbf8a_0000); // Actual workgroup S_BARRIER, full EXEC.
        }
        // Read that storage location after the barrier, add7, store at byte4.
        words.extend([0xe030_1000, (128 << 24) | (3 << 8) | 2]);
        words.push(0xbf8c_0000); // Actual S_WAITCNT0; not a new wait implementation.
        words.push(fixtures::vector_binary(0x25, 4, 135, 3));
        let mut output = fixtures::store(4, 2);
        output[0] |= 4;
        words.extend(output);
        words.push(fixtures::END);
        let StageDescription::Compute(mut description) =
            profile(ShaderStage::Compute).description().clone()
        else {
            unreachable!()
        };
        description.common.wave_size = WaveSize::Wave64;
        description.common.subgroup = None;
        description.common.user_sgpr_count = 4;
        description.common.buffers[0].access = BufferAccess::ReadWrite;
        description.workgroup_size = [64, 2, 1];
        let profile = Profile::new(StageDescription::Compute(description)).unwrap();
        let limits = Limits::default();
        let program = ShaderProgram::decode(&words, ShaderStage::Compute, &limits).unwrap();
        let mut budget = budget::Budget::new(&limits);
        let (ir, _) = crate::lower(&program, &profile, &mut budget).unwrap();
        let stores: Vec<_> = (0..ir.blocks.len() as u32)
            .flat_map(|block| ir.block_insts(block))
            .filter(|&id| ir.inst(id).op == Op::StoreBufferU32)
            .collect();
        assert_eq!(stores.len(), 2);
        assert!(depends_on_buffer_load(
            &ir,
            &budget,
            ir.arg(stores[1], 4),
            &mut BTreeSet::new()
        ));
        let module = compile(&program, &profile, &limits).unwrap();
        let name = match (barrier, store_completion) {
            (true, true) => "storage-barrier-with-store-completion",
            (true, false) => "storage-barrier-without-subgroup-scope",
            _ => "storage-no-barrier",
        };
        validate(name, &module);
        let buffer = module.manifest().buffers()[0];
        assert!(buffer.read && buffer.written);
        assert!(!buffer.coherent); // No guest coherent flag was admitted.
        assert_eq!(buffer.effective_coherent, barrier);
        assert_eq!(module.manifest().required_subgroup_size(), None);
        assert_eq!(
            module.manifest().required_features().subgroup_basic(),
            store_completion
        );
        assert!(!module.manifest().required_features().subgroup_ballot());
        let ops = instructions(module.spirv());
        let constant = |id, value| {
            ops.iter()
                .any(|(op, args)| *op == 43 && args[1] == id && args[2] == value)
        };
        let barriers: Vec<_> = ops.iter().filter(|(op, _)| *op == 224).collect();
        assert_eq!(
            barriers.len(),
            usize::from(barrier) + usize::from(store_completion)
        );
        assert_eq!(
            ops.iter().any(|(op, args)| *op == 17 && *args == [61]),
            store_completion
        );
        if barrier {
            let args = barriers
                .iter()
                .find(|(_, args)| constant(args[0], 2))
                .unwrap()
                .1;
            assert!(constant(args[0], 2)); // Workgroup execution.
            assert!(constant(args[1], 2)); // Workgroup memory scope.
            assert!(constant(args[2], 0x8 | 0x40 | 0x100));
        }
        if store_completion {
            let args = barriers
                .iter()
                .find(|(_, args)| constant(args[0], 3))
                .unwrap()
                .1;
            assert!(constant(args[1], 1)); // Actual host subgroup / Device memory.
            assert!(constant(args[2], 0x4 | 0x40 | 0x800));
        }
        let storage_variables: Vec<_> = ops
            .iter()
            .filter_map(|(op, args)| match (*op, *args) {
                (59, [_, id, 12]) => Some(*id),
                _ => None,
            })
            .collect();
        assert_eq!(storage_variables.len(), 1);
        assert_eq!(
            ops.iter().any(|(op, args)| matches!((*op, *args),
                (71, [id, 23]) if storage_variables.contains(id))),
            barrier
        );
    }
}
