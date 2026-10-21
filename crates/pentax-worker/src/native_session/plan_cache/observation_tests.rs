// SPDX-License-Identifier: GPL-2.0-only
//! Original retention operations distinguish collisions, replacements and purges.
use super::*;
use pentax_codegen::declare_materialized_source;
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

const A: u64 = 0x100200;
const B: u64 = 0x101210;
const OTHER_MODULE: u64 = 0x200230;

fn with_original_source(run: impl FnOnce(&MaterializedSource<'_, '_>)) {
    let mut bytes = super::tests::image();
    bytes.resize(0x3000, 0xc3);
    bytes[96..104].copy_from_slice(&0x3000_u64.to_le_bytes());
    let inputs = [
        ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000),
        ModuleInput::new(open_executable(&bytes).unwrap(), 0x200000),
    ];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let source = declare_materialized_source(&linked, &[], &BlockLimits::default()).unwrap();
    run(&source);
}
fn placement() -> CachePlacement {
    CachePlacement {
        address: 0x800000,
        capacity: 4096,
    }
}

#[test]
fn committed_plan_collisions_and_same_key_changes_are_distinct_from_exact_hits() {
    with_original_source(|source| {
        let mut work = 0;
        let mut cache = PlanCache::new(
            source,
            TranslationPolicy::x64_sse2(),
            BlockLimits::default(),
            &mut work,
            u64::MAX,
        )
        .unwrap();
        let (plan, hit, first) = cache
            .prepare(0, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(first.retained_bytes, plan.usage().allocation_bytes);
        assert_eq!(first.peak_retained_bytes, first.retained_bytes);
        assert_eq!(first.occupied_collisions, 0);
        assert_eq!(first.same_key_replacements, 0);
        assert_eq!(
            cache
                .prepare(0, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
                .unwrap()
                .2,
            first
        );

        // These original PCs really collide in the existing4096-slot table;
        // no placement/replacement policy is changed for this observation.
        let (plan, hit, second) = cache
            .prepare(0, B, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(plan.guest_pc(), B);
        assert_eq!(
            (second.occupied_collisions, second.same_key_replacements),
            (1, 0)
        );
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);
        let (_, hit, third) = cache
            .prepare(
                1,
                OTHER_MODULE,
                &[0xc3; 64],
                placement(),
                &mut work,
                u64::MAX,
            )
            .unwrap();
        assert!(!hit);
        assert_eq!(
            (third.occupied_collisions, third.same_key_replacements),
            (2, 0)
        );
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);

        let moved = CachePlacement {
            address: placement().address + 0x4000,
            ..placement()
        };
        let (_, hit, changed_placement) = cache
            .prepare(1, OTHER_MODULE, &[0xc3; 64], moved, &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(
            (
                changed_placement.occupied_collisions,
                changed_placement.same_key_replacements
            ),
            (2, 1)
        );
        let (plan, hit, shortened) = cache
            .prepare(1, OTHER_MODULE, &[0xc3; 32], moved, &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(
            (
                shortened.occupied_collisions,
                shortened.same_key_replacements
            ),
            (2, 2)
        );
        assert_eq!(shortened.retained_bytes, plan.usage().allocation_bytes);
        assert!(shortened.peak_retained_bytes >= shortened.retained_bytes);
        assert_eq!((shortened.byte_purges, shortened.purged_plans), (0, 0));
        assert!(!shortened.overflow);
        assert_eq!(
            cache
                .prepare(1, OTHER_MODULE, &[0xc3; 32], moved, &mut work, u64::MAX)
                .unwrap()
                .2,
            shortened
        );

        // A fresh per-epoch cache inherits no earlier diagnostic history.
        let mut rebound = PlanCache::new(
            source,
            TranslationPolicy::x64_sse2(),
            BlockLimits::default(),
            &mut work,
            u64::MAX,
        )
        .unwrap();
        let fresh = rebound
            .prepare(0, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap()
            .2;
        assert_eq!(
            (
                fresh.occupied_collisions,
                fresh.same_key_replacements,
                fresh.byte_purges
            ),
            (0, 0, 0)
        );
    });
}

#[test]
fn plan_refusals_preserve_observations_and_successful_byte_purges_count_actual_victims() {
    with_original_source(|source| {
        let mut work = 0;
        let mut cache = PlanCache::new(
            source,
            TranslationPolicy::x64_sse2(),
            BlockLimits::default(),
            &mut work,
            u64::MAX,
        )
        .unwrap();
        cache
            .prepare(0, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        let first = cache.observations;
        let mut changed = [0xc3; 64];
        changed[63] = 0x90; // Unconsumed bytes still require the exact source.
        assert!(matches!(
            cache.prepare(0, B, &changed, placement(), &mut work, u64::MAX),
            Err(NativeSessionError::Codegen(
                CodegenError::MaterializedWindowMismatch { .. }
            ))
        ));
        assert_eq!(cache.observations, first);
        assert!(
            cache
                .prepare(2, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
                .is_err()
        );
        assert_eq!(cache.observations, first);
        let previous_work = work;
        assert!(
            cache
                .prepare(0, B, &[0xc3; 64], placement(), &mut work, previous_work)
                .is_err()
        );
        assert_eq!(work, previous_work);
        assert_eq!(cache.observations, first);
        cache.max_bytes = first.retained_bytes - 1;
        assert!(matches!(
            cache.prepare(0, B, &[0xc3; 64], placement(), &mut work, u64::MAX),
            Err(NativeSessionError::Budget("plan cache bytes"))
        ));
        assert_eq!(cache.observations, first);

        cache.max_bytes = 2 * first.retained_bytes;
        cache
            .prepare(0, A + 1, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        assert_eq!(cache.retained_bytes, cache.max_bytes);
        let before_purge = cache.observations;
        let admission_only = work + 64 + 8;
        assert!(
            cache
                .prepare(
                    0,
                    A + 2,
                    &[0xc3; 64],
                    placement(),
                    &mut work,
                    admission_only
                )
                .is_err()
        );
        assert_eq!(work, admission_only); // Original attempted lookup charge survives.
        assert_eq!(cache.observations, before_purge);
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 2);
        let (plan, hit, purged) = cache
            .prepare(0, A + 2, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!((purged.byte_purges, purged.purged_plans), (1, 2));
        assert_eq!(
            (purged.occupied_collisions, purged.same_key_replacements),
            (0, 0)
        );
        assert_eq!(purged.retained_bytes, plan.usage().allocation_bytes);
        assert_eq!(purged.peak_retained_bytes, before_purge.retained_bytes);
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);

        // Incoming-slot cause and byte-pressure action are independent: this
        // real larger valid plan both collides with A and purges all old plans.
        cache
            .prepare(0, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        let full = cache.retained_bytes;
        let expanded = plan_materialized_block(
            source,
            0,
            B,
            &[0xc3; 128],
            placement(),
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert!(expanded.usage().allocation_bytes > first.retained_bytes);
        assert!(expanded.usage().allocation_bytes <= full);
        let expanded_bytes = expanded.usage().allocation_bytes;
        drop(expanded);
        let (_, hit, overlap) = cache
            .prepare(0, B, &[0xc3; 128], placement(), &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(
            (
                overlap.occupied_collisions,
                overlap.byte_purges,
                overlap.purged_plans
            ),
            (1, 2, 4)
        );
        assert_eq!(overlap.retained_bytes, expanded_bytes);
    });
}

#[test]
fn diagnostic_saturation_does_not_prevent_real_retention_or_change_its_gauge() {
    with_original_source(|source| {
        let mut work = 0;
        let mut cache = PlanCache::new(
            source,
            TranslationPolicy::x64_sse2(),
            BlockLimits::default(),
            &mut work,
            u64::MAX,
        )
        .unwrap();
        cache
            .prepare(0, A, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        cache.observations.occupied_collisions = u64::MAX;
        let (_, hit, saturated) = cache
            .prepare(0, B, &[0xc3; 64], placement(), &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(saturated.occupied_collisions, u64::MAX);
        assert!(saturated.overflow);
        assert_eq!(cache.slots.iter().flatten().next().unwrap().guest_pc(), B);
        assert_eq!(saturated.retained_bytes, cache.retained_bytes);
        cache.observations.same_key_replacements = u64::MAX;
        let moved = CachePlacement {
            address: placement().address + 0x4000,
            ..placement()
        };
        let (plan, hit, saturated) = cache
            .prepare(0, B, &[0xc3; 64], moved, &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(plan.placement(), moved);
        assert_eq!(saturated.same_key_replacements, u64::MAX);
        assert!(saturated.overflow);
        assert_eq!(saturated.retained_bytes, plan.usage().allocation_bytes);
        cache.max_bytes = 2 * cache.retained_bytes;
        cache
            .prepare(0, B + 1, &[0xc3; 64], moved, &mut work, u64::MAX)
            .unwrap();
        cache.observations.byte_purges = u64::MAX;
        cache.observations.purged_plans = u64::MAX;
        let (plan, hit, saturated) = cache
            .prepare(0, B + 2, &[0xc3; 64], moved, &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(
            (saturated.byte_purges, saturated.purged_plans),
            (u64::MAX, u64::MAX)
        );
        assert!(saturated.overflow);
        assert_eq!(saturated.retained_bytes, plan.usage().allocation_bytes);
    });
}
