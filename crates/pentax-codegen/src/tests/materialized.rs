// SPDX-License-Identifier: GPL-2.0-only

//! Original pure relocation/decode fixtures. No backing owner or executor.

use super::*;
use pentax_linker::SymbolicLinkPlan;

const SECOND: u64 = BIAS + 0x2_0000;
// Nonuniform individually decodable bytes also exercise partial write offsets.
const DECLARED: u64 = 0x9090_5951_9058_5090;

fn put(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn images() -> [Vec<u8>; 2] {
    let mut first = with_relocation(8, ENTRY as u64 + 2, 0x55);
    let mut code = vec![0x48, 0xb8];
    code.extend_from_slice(&[0; 8]);
    code.extend_from_slice(&[0x48, 0xbb]);
    code.extend_from_slice(&[0; 8]);
    code.push(0xc3);
    first[ENTRY..ENTRY + code.len()].copy_from_slice(&code);
    put(&mut first, 0x818, 48); // Two regular RELA records.
    put(&mut first, 0x918, ENTRY as u64 + 12);
    put(&mut first, 0x920, 18); // TP offset; symbol zero intentionally unproven.
    let second = with_relocation(16, ENTRY as u64 + 2, 0);
    [first, second]
}

fn declarations(plan: &SymbolicLinkPlan<'_>) -> Vec<DeclaredAppliedWrite> {
    let mut result = plan
        .deferred()
        .iter()
        .enumerate()
        .map(|(deferred_index, deferred)| {
            let module = &plan.modules()[deferred.module().index()];
            DeclaredAppliedWrite {
                deferred_index,
                consumer: deferred.module(),
                source: deferred.source(),
                address: module.load_bias() + deferred.source().record.offset,
                value: if deferred.module().index() == 0 {
                    DECLARED
                } else {
                    2
                },
            }
        })
        .collect::<Vec<_>>();
    result.sort_unstable_by_key(|write| write.address);
    result
}

fn first_snapshot() -> Vec<u8> {
    let mut bytes = vec![0x48, 0xb8];
    bytes.extend_from_slice(&(BIAS + 0x55).to_le_bytes());
    bytes.extend_from_slice(&[0x48, 0xbb]);
    bytes.extend_from_slice(&DECLARED.to_le_bytes());
    bytes.push(0xc3);
    bytes
}

fn planned<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    pc: u64,
    bytes: &[u8],
) -> Result<MaterializedBlockPlan<'view, 'plan, 'image>, CodegenError> {
    plan_materialized_block(
        source,
        0,
        pc,
        bytes,
        placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
}

#[test]
fn exact_two_module_declarations_preserve_ordinary_writes_and_explicit_unproved_status() {
    let original = images();
    let before = original.clone();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    assert!(matches!(
        plan_block(
            &plan,
            0,
            BIAS + ENTRY as u64,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::DeferredRelocations { count: 2 })
    ));
    let mut writes = declarations(&plan);
    let view = declare_materialized_source(&plan, &writes, &BlockLimits::default()).unwrap();
    assert!(std::ptr::eq(view.source_plan(), &plan));
    assert!(std::ptr::eq(view.deferred_relocations(), plan.deferred()));
    assert_eq!(
        view.assurance(),
        MaterializedSourceAssurance::CallerDeclared
    );
    // Deliberately no TLS symbol/template proof exists. This path must never
    // relabel the original two deferred records as semantically resolved.
    assert_eq!(view.deferred_relocations().len(), 2);
    writes[0].value = 0; // View owns its declaration; caller cannot replace it.
    let snapshot = first_snapshot();
    let block = planned(&view, BIAS + ENTRY as u64, &snapshot).unwrap();
    assert_eq!(block.relocated_source_bytes(), snapshot);
    assert_eq!(block.original_bytes(), &original[0][ENTRY..ENTRY + 21]);
    assert!(std::ptr::eq(block.declaration(), &view));
    assert!(std::ptr::eq(block.source_plan(), &plan));
    assert_eq!(block.module(), plan.modules()[0].id());
    assert!(matches!(
        block.exit().kind(),
        ExitKind::Return { adjust: 0 }
    ));
    let mut decoded = Decoder::with_ip(64, block.bytes(), CACHE, DecoderOptions::NONE);
    assert_eq!(decoded.decode().immediate64(), BIAS + 0x55);
    assert_eq!(decoded.decode().immediate64(), DECLARED);
    assert_eq!(decoded.decode().code(), Code::Ud2);
    let mut second = vec![0x48, 0xb8];
    second.extend_from_slice(&2_u64.to_le_bytes());
    second.push(0xc3);
    let block = plan_materialized_block(
        &view,
        1,
        SECOND + ENTRY as u64,
        &second,
        placement(),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    assert_eq!(block.module(), plan.modules()[1].id());
    assert_eq!(block.relocated_source_bytes(), second);
    assert_eq!(original, before);
    assert_eq!(plan.deferred_count(), 2);
}

#[test]
fn sorted_targets_can_have_nonmonotonic_original_deferred_indices() {
    let original = images();
    let plan = link(&[(&original[0], SECOND), (&original[1], BIAS)]);
    let writes = declarations(&plan);
    assert_eq!([writes[0].deferred_index, writes[1].deferred_index], [1, 0]);
    let view = declare_materialized_source(&plan, &writes, &BlockLimits::default()).unwrap();
    assert_eq!(view.declared_writes(), writes);
}

#[test]
fn full_bijection_refuses_missing_extra_duplicate_and_unknown_records() {
    let original = images();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    let writes = declarations(&plan);
    assert!(matches!(
        declare_materialized_source(&plan, &writes[..1], &BlockLimits::default()),
        Err(CodegenError::MaterializedDeclarationCount {
            expected: 2,
            actual: 1
        })
    ));
    let mut extra = writes.clone();
    extra.push(writes[0]);
    assert!(matches!(
        declare_materialized_source(&plan, &extra, &BlockLimits::default()),
        Err(CodegenError::MaterializedDeclarationCount {
            expected: 2,
            actual: 3
        })
    ));
    let duplicate = [writes[0], writes[0]];
    assert!(matches!(
        declare_materialized_source(&plan, &duplicate, &BlockLimits::default()),
        Err(CodegenError::DuplicateMaterializedWrite { deferred_index: 0 })
    ));
    let unknown = with_relocation(19, ENTRY as u64 + 2, 0);
    let plan = link(&[(&unknown, BIAS)]);
    let writes = declarations(&plan);
    assert!(matches!(
        declare_materialized_source(&plan, &writes, &BlockLimits::default()),
        Err(CodegenError::UnsupportedMaterializedRelocation {
            deferred_index: 0,
            kind: 19
        })
    ));
}

#[test]
fn identities_checked_addresses_and_target_order_cannot_be_substituted() {
    let original = images();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    let writes = declarations(&plan);
    for reason in [
        MaterializedWriteError::DeferredIndex,
        MaterializedWriteError::Consumer,
        MaterializedWriteError::SourceRecord,
        MaterializedWriteError::Address,
        MaterializedWriteError::OrderingOrOverlap,
    ] {
        let mut wrong = writes.clone();
        match reason {
            MaterializedWriteError::DeferredIndex => wrong[0].deferred_index = 2,
            MaterializedWriteError::Consumer => wrong[0].consumer = plan.modules()[1].id(),
            MaterializedWriteError::SourceRecord => wrong[0].source.record.addend += 1,
            MaterializedWriteError::Address => wrong[0].address += 1,
            MaterializedWriteError::OrderingOrOverlap => wrong.reverse(),
            _ => unreachable!(),
        }
        assert!(
            matches!(declare_materialized_source(&plan, &wrong, &BlockLimits::default()),
            Err(CodegenError::InvalidMaterializedWrite { reason: actual, .. }) if actual == reason)
        );
    }
    let overflow = with_relocation(18, u64::MAX, 0);
    let plan = link(&[(&overflow, BIAS)]);
    let record = plan.deferred()[0];
    let write = DeclaredAppliedWrite {
        deferred_index: 0,
        consumer: record.module(),
        source: record.source(),
        address: 0,
        value: 0,
    };
    assert!(matches!(
        declare_materialized_source(&plan, &[write], &BlockLimits::default()),
        Err(CodegenError::InvalidMaterializedWrite {
            reason: MaterializedWriteError::Address,
            ..
        })
    ));
}

#[test]
fn logical_holes_and_ordinary_patch_overlap_are_refused_before_a_source_view() {
    let outside = with_relocation(18, DATA + PAGE - 4, 0);
    let plan = link(&[(&outside, BIAS)]);
    assert!(matches!(
        declare_materialized_source(&plan, &declarations(&plan), &BlockLimits::default()),
        Err(CodegenError::InvalidMaterializedWrite {
            reason: MaterializedWriteError::LogicalTarget,
            ..
        })
    ));
    let mut original = images();
    put(&mut original[0], 0x918, ENTRY as u64 + 4);
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    assert!(matches!(
        declare_materialized_source(&plan, &declarations(&plan), &BlockLimits::default()),
        Err(CodegenError::InvalidMaterializedWrite {
            reason: MaterializedWriteError::OrdinaryPatchOverlap,
            ..
        })
    ));
    let mut overlap = images()[0].clone();
    put(&mut overlap, 0x908, 18); // Both records are now declared fixed-width writes.
    put(&mut overlap, 0x918, ENTRY as u64 + 6);
    let plan = link(&[(&overlap, BIAS)]);
    assert!(matches!(
        declare_materialized_source(&plan, &declarations(&plan), &BlockLimits::default()),
        Err(CodegenError::InvalidMaterializedWrite {
            reason: MaterializedWriteError::OrderingOrOverlap,
            ..
        })
    ));
}

#[test]
fn staged_window_equality_initialization_and_decode_boundaries_remain_strict() {
    let original = images();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    let view =
        declare_materialized_source(&plan, &declarations(&plan), &BlockLimits::default()).unwrap();
    let snapshot = first_snapshot();
    for outside in [&[][..], &[0x90; 4096][..]] {
        assert!(matches!(
            planned(&view, BIAS + ENTRY as u64, outside),
            Err(CodegenError::MaterializedWindowRange { .. })
        ));
    }
    let mut wrong = snapshot.clone();
    wrong[12] ^= 1;
    assert!(matches!(
        planned(&view, BIAS + ENTRY as u64, &wrong),
        Err(CodegenError::MaterializedWindowMismatch { offset: 12, .. })
    ));
    assert!(matches!(
        planned(&view, BIAS + 0x2000, &[0x90]),
        Err(CodegenError::MissingInitializedExecutableSource { .. })
    ));
    assert!(matches!(
        planned(&view, BIAS + ENTRY as u64, &snapshot[..9]),
        Err(CodegenError::TruncatedInstruction { .. })
    ));
    assert!(matches!(
        plan_materialized_block(
            &view,
            0,
            BIAS + ENTRY as u64,
            &snapshot,
            CachePlacement {
                address: BIAS + PAGE - 1,
                capacity: 2
            },
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default()
        ),
        Err(CodegenError::CacheOverlap { .. })
    ));
}

#[test]
fn partial_windows_resume_exact_instruction_and_keep_relocation_attestation() {
    let original = images();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    let view =
        declare_materialized_source(&plan, &declarations(&plan), &BlockLimits::default()).unwrap();
    let snapshot = first_snapshot();
    let pc = BIAS + ENTRY as u64;
    let policy = TranslationPolicy::x64_sse2().with_partial_window_splits();
    for size in 1..20 {
        let block = plan_materialized_block(
            &view,
            0,
            pc,
            &snapshot[..size],
            placement(),
            &policy,
            &BlockLimits::default(),
        );
        if size < 10 {
            assert!(
                matches!(block, Err(CodegenError::TruncatedInstruction { guest_pc }) if guest_pc == pc)
            );
            continue;
        }
        let block = block.unwrap();
        assert_eq!(block.instructions().len(), 1);
        assert_eq!(
            block.instructions()[0].instruction().mnemonic(),
            Mnemonic::Mov
        );
        assert!(
            matches!(block.exit().kind(), ExitKind::Fallthrough { target } if target == pc + 10)
        );
        assert_eq!(block.relocated_source_bytes(), &snapshot[..size]);
        let next = plan_materialized_block(
            &view,
            0,
            pc + 10,
            &snapshot[10..],
            placement(),
            &policy,
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(next.instructions().len(), 2);
        assert!(matches!(next.exit().kind(), ExitKind::Return { .. }));
    }
    let mut wrong = snapshot[..19].to_vec();
    wrong[18] ^= 1; // Even the unconsumed partial instruction must be attested.
    assert!(matches!(
        plan_materialized_block(
            &view,
            0,
            pc,
            &wrong,
            placement(),
            &policy,
            &BlockLimits::default()
        ),
        Err(CodegenError::MaterializedWindowMismatch { offset: 18, .. })
    ));
}

#[test]
fn partial_declared_patch_intersections_use_the_correct_little_endian_subranges() {
    let original = images();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    let view =
        declare_materialized_source(&plan, &declarations(&plan), &BlockLimits::default()).unwrap();
    let snapshot = first_snapshot();
    let block = planned(&view, BIAS + ENTRY as u64 + 15, &snapshot[15..]).unwrap();
    assert_eq!(
        block.relocated_source_bytes(),
        &[0x90, 0x51, 0x59, 0x90, 0x90, 0xc3]
    );
    assert_eq!(block.instructions().len(), 6);
    assert!(matches!(block.exit().kind(), ExitKind::Return { .. }));
    let block = planned(&view, BIAS + ENTRY as u64 + 12, &snapshot[12..16]).unwrap();
    assert_eq!(block.relocated_source_bytes(), &[0x90, 0x50, 0x58, 0x90]);
    assert!(
        matches!(block.exit().kind(), ExitKind::Fallthrough { target } if target == BIAS + ENTRY as u64 + 16)
    );
}

#[test]
fn declaration_and_per_block_aggregate_budgets_cannot_be_bypassed_by_retained_views() {
    let original = images();
    let plan = link(&[(&original[0], BIAS), (&original[1], SECOND)]);
    let writes = declarations(&plan);
    for kind in [Budget::Patches, Budget::AllocationBytes, Budget::Work] {
        let mut limits = BlockLimits::default();
        match kind {
            Budget::Patches => limits.max_patches = 2,
            Budget::AllocationBytes => limits.max_allocation_bytes = 0,
            Budget::Work => limits.max_work_units = 0,
            _ => unreachable!(),
        }
        assert!(
            matches!(declare_materialized_source(&plan, &writes, &limits),
            Err(CodegenError::Budget { kind: actual, .. }) if actual == kind)
        );
    }
    let view = declare_materialized_source(&plan, &writes, &BlockLimits::default()).unwrap();
    let usage = view.declaration_usage();
    let inclusive = BlockLimits {
        max_work_units: usage.work_units,
        max_allocation_bytes: usage.allocation_bytes,
        ..Default::default()
    };
    declare_materialized_source(&plan, &writes, &inclusive).unwrap();
    let snapshot = first_snapshot();
    let tight = BlockLimits {
        max_patches: 2,
        ..Default::default()
    };
    assert!(matches!(
        plan_materialized_block(
            &view,
            0,
            BIAS + ENTRY as u64,
            &snapshot,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &tight
        ),
        Err(CodegenError::Budget {
            kind: Budget::Patches,
            ..
        })
    ));
    let tight = BlockLimits {
        max_source_bytes: 20,
        ..Default::default()
    };
    assert!(matches!(
        plan_materialized_block(
            &view,
            0,
            BIAS + ENTRY as u64,
            &snapshot,
            placement(),
            &TranslationPolicy::x64_sse2(),
            &tight
        ),
        Err(CodegenError::Budget {
            kind: Budget::SourceBytes,
            ..
        })
    ));
}
