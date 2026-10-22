// SPDX-License-Identifier: GPL-2.0-only
//! Optional cold traces share the existing plan ledger; cache hits never encode.
use super::*;
use pentax_codegen::declare_materialized_source;
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn policy() -> TranslationPolicy {
    TranslationPolicy::x64_sse2()
        .with_independent_memory()
        .with_register_loop_trace()
}
fn placement() -> CachePlacement {
    CachePlacement {
        address: 0x200000,
        capacity: 500,
    }
}

#[test]
fn register_trace_is_retained_and_charged_once_with_full_live_window_validation() {
    let mut bytes = tests::image();
    let code = [0x00, 0xc4, 0x11, 0xc3, 0x90, 0x75, 0xf9];
    bytes[0x200..0x207].copy_from_slice(&code);
    bytes[0x300..0x307].copy_from_slice(&code);
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = BlockLimits::default();
    let source = declare_materialized_source(&linked, &[], &limits).unwrap();
    let live = &bytes[0x200..0x240];
    let mut work = 0;
    let mut cache = PlanCache::new(&source, policy(), limits, &mut work, u64::MAX).unwrap();
    let before = work;
    let (plan, hit, observation) = cache
        .prepare(0, 0x100200, live, placement(), 1, &mut work, u64::MAX)
        .unwrap();
    assert!(!hit);
    let ordinary = plan.usage();
    let allocated = plan.retained_allocation_bytes();
    let trace = plan.register_loop_trace().unwrap();
    let preparation = trace.preparation_work();
    let trace_bytes = trace.bytes().to_vec();
    assert!(allocated > ordinary.allocation_bytes);
    assert_eq!(work - before, live.len() as u64 + 8 + preparation);
    assert_eq!(observation.retained_bytes, allocated);
    let before = work;
    let (plan, hit, after) = cache
        .prepare(0, 0x100200, live, placement(), 2, &mut work, u64::MAX)
        .unwrap();
    assert!(hit);
    assert_eq!(plan.usage(), ordinary);
    assert_eq!(plan.register_loop_trace().unwrap().bytes(), trace_bytes);
    assert_eq!(work - before, live.len() as u64 + 8);
    assert_eq!(after, observation);
    let retained = cache.retained_bytes;
    let recency = cache.recency;
    let mut changed = live.to_vec();
    changed[1] ^= 1;
    assert!(
        cache
            .prepare(0, 0x100200, &changed, placement(), 3, &mut work, u64::MAX)
            .is_err()
    );
    assert_eq!(cache.retained_bytes, retained);
    assert_eq!(cache.recency, recency);
    assert_eq!(cache.observations, observation);
    assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);
    // Real optional allocation participates in the unchanged global byte
    // purge, and its previously reserved fallback cost is committed exactly.
    cache.max_bytes = allocated;
    let before = work;
    let (next, hit, after) = cache
        .prepare(
            0,
            0x100300,
            &bytes[0x300..0x340],
            placement(),
            4,
            &mut work,
            u64::MAX,
        )
        .unwrap();
    assert!(!hit);
    assert!(next.register_loop_trace().is_some());
    assert_eq!(next.retained_allocation_bytes(), allocated);
    assert_eq!(
        work - before,
        live.len() as u64
            + 8
            + next.register_loop_trace().unwrap().preparation_work()
            + SLOTS as u64
    );
    assert_eq!(after.byte_purges, 1);
    assert_eq!(after.purged_plans, 1);
    assert_eq!(after.retained_bytes, allocated);
    assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);
}

#[test]
fn register_trace_work_decline_and_failed_attempt_retain_an_ordinary_cached_plan() {
    for eligible in [true, false] {
        let mut bytes = tests::image();
        // An RSP register input is deliberately ineligible but reaches the bounded scan.
        let code = if eligible {
            vec![0x00, 0xc4, 0x11, 0xc3, 0x90, 0x75, 0xf9]
        } else {
            vec![0x48, 0x89, 0xe0, 0xff, 0xc9, 0x90, 0x75, 0xf8]
        };
        bytes[0x200..0x200 + code.len()].copy_from_slice(&code);
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
        let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        let limits = BlockLimits::default();
        let source = declare_materialized_source(&linked, &[], &limits).unwrap();
        let live = &bytes[0x200..0x240];
        let mut work = 0;
        let mut cache = PlanCache::new(&source, policy(), limits, &mut work, u64::MAX).unwrap();
        let initial = work;
        // Optional preparation must reserve a possible existing SLOTS purge,
        // even when this particular ordinary plan has room in an empty table.
        let allowance = initial + live.len() as u64 + 8 + SLOTS as u64;
        let (plan, hit, _) = cache
            .prepare(0, 0x100200, live, placement(), 1, &mut work, allowance)
            .unwrap();
        assert!(!hit);
        assert!(plan.register_loop_trace().is_none());
        assert_eq!(work, initial + live.len() as u64 + 8);
        assert_eq!(
            plan.retained_allocation_bytes(),
            plan.usage().allocation_bytes
        );
        // More later work does not repeatedly encode an already retained plan.
        let before = work;
        let (plan, hit, _) = cache
            .prepare(0, 0x100200, live, placement(), 2, &mut work, u64::MAX)
            .unwrap();
        assert!(hit && plan.register_loop_trace().is_none());
        assert_eq!(work - before, live.len() as u64 + 8);

        let mut full_work = 0;
        let mut full = PlanCache::new(&source, policy(), limits, &mut full_work, u64::MAX).unwrap();
        let before = full_work;
        let (plan, _, _) = full
            .prepare(0, 0x100200, live, placement(), 1, &mut full_work, u64::MAX)
            .unwrap();
        assert_eq!(plan.register_loop_trace().is_some(), eligible);
        if !eligible {
            assert!(full_work - before > live.len() as u64 + 8);
            assert!(plan.register_loop_trace_work_bound(&limits).is_none());
        }
        let before = full_work;
        let (_, hit, _) = full
            .prepare(0, 0x100200, live, placement(), 2, &mut full_work, u64::MAX)
            .unwrap();
        assert!(hit);
        assert_eq!(full_work - before, live.len() as u64 + 8);
    }
}
