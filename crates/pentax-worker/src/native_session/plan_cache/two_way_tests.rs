// SPDX-License-Identifier: GPL-2.0-only
//! Original source plans exercise paired retention without execution authority.
use super::*;
use pentax_codegen::declare_materialized_source;
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

const A: u64 = 0x100200;
const B: u64 = 0x101210;
const C: u64 = 0x102220;

fn with_source(run: impl FnOnce(&MaterializedSource<'_, '_>)) {
    let mut bytes = super::tests::image();
    bytes.resize(0x3000, 0xc3);
    bytes[96..104].copy_from_slice(&0x3000_u64.to_le_bytes());
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
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

fn hash(pc: u64) -> usize {
    ((pc ^ (pc >> 8) ^ (pc >> 16) ^ (pc >> 32)) as usize) & (SLOTS - 1)
}

#[test]
fn two_way_plan_cache_keeps_two_hot_collisions_and_evicts_the_actual_lru_victim() {
    assert_eq!(hash(A), hash(B));
    assert_eq!(hash(A), hash(C));
    with_source(|source| {
        let mut work = 0;
        let mut cache = PlanCache::new(
            source,
            TranslationPolicy::x64_sse2(),
            BlockLimits::default(),
            &mut work,
            u64::MAX,
        )
        .unwrap();
        for stamp in 1..=16 {
            let pc = if stamp % 2 == 1 { A } else { B };
            let (plan, hit, observations) = cache
                .prepare(0, pc, &[0xc3; 64], placement(), stamp, &mut work, u64::MAX)
                .unwrap();
            assert_eq!(hit, stamp > 2);
            assert_eq!(plan.guest_pc(), pc);
            assert_eq!(plan.placement(), placement());
            assert!(std::ptr::eq(plan.declaration(), source));
            assert_eq!(observations.occupied_collisions, 0);
        }
        let primary = hash(A);
        assert_eq!(
            (cache.recency[primary], cache.recency[primary ^ 1]),
            (15, 16)
        );
        assert!(
            !cache
                .prepare(0, C, &[0xc3; 64], placement(), 17, &mut work, u64::MAX)
                .unwrap()
                .1
        );
        assert_eq!(cache.slots[primary].as_ref().unwrap().guest_pc(), C);
        assert_eq!(cache.slots[primary ^ 1].as_ref().unwrap().guest_pc(), B);
        assert!(
            cache
                .prepare(0, B, &[0xc3; 64], placement(), 18, &mut work, u64::MAX)
                .unwrap()
                .1
        );
        assert!(
            !cache
                .prepare(0, A, &[0xc3; 64], placement(), 19, &mut work, u64::MAX)
                .unwrap()
                .1
        );
        assert_eq!(cache.observations.occupied_collisions, 2);
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 2);
        assert_eq!(cache.slots.len(), 4096);
        assert_eq!(cache.max_bytes, 8 * 1024 * 1024);

        // Equal stamps use the incoming hash's primary deterministically,
        // including at u64::MAX: no separate clock can wrap.
        cache
            .prepare(
                0,
                A,
                &[0xc3; 64],
                placement(),
                u64::MAX,
                &mut work,
                u64::MAX,
            )
            .unwrap();
        cache
            .prepare(
                0,
                B,
                &[0xc3; 64],
                placement(),
                u64::MAX,
                &mut work,
                u64::MAX,
            )
            .unwrap();
        cache
            .prepare(
                0,
                C,
                &[0xc3; 64],
                placement(),
                u64::MAX,
                &mut work,
                u64::MAX,
            )
            .unwrap();
        assert_eq!(cache.slots[primary].as_ref().unwrap().guest_pc(), C);
    });
}

#[test]
fn two_way_plan_cache_replans_changed_placement_and_preserves_hints_on_refusal() {
    with_source(|source| {
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
            .prepare(0, A, &[0xc3; 64], placement(), 10, &mut work, u64::MAX)
            .unwrap();
        let moved = CachePlacement {
            address: placement().address + 0x10000,
            ..placement()
        };
        let (plan, hit, observations) = cache
            .prepare(0, A, &[0xc3; 64], moved, 11, &mut work, u64::MAX)
            .unwrap();
        assert!(!hit);
        assert_eq!(plan.placement(), moved);
        assert_eq!(observations.same_key_replacements, 1);
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);
        assert_eq!(cache.recency[hash(A)], 11);
        assert_eq!(cache.recency[hash(A) ^ 1], 0);
        cache
            .prepare(0, B, &[0xc3; 64], placement(), 12, &mut work, u64::MAX)
            .unwrap();
        let before_recency = cache.recency;
        let before_observations = cache.observations;
        let mut changed = [0xc3; 64];
        changed[63] = 0x90;
        assert!(matches!(
            cache.prepare(0, C, &changed, placement(), 100, &mut work, u64::MAX),
            Err(NativeSessionError::Codegen(
                CodegenError::MaterializedWindowMismatch { .. }
            ))
        ));
        let before_work = work;
        assert!(
            cache
                .prepare(0, A, &[0xc3; 64], moved, 101, &mut work, before_work)
                .is_err()
        );
        assert_eq!(work, before_work);
        cache.max_bytes = 0;
        assert!(matches!(
            cache.prepare(0, C, &[0xc3; 64], placement(), 102, &mut work, u64::MAX),
            Err(NativeSessionError::Budget("plan cache bytes"))
        ));
        assert_eq!(cache.recency, before_recency);
        assert_eq!(cache.observations, before_observations);
        assert_eq!(cache.slots[hash(A)].as_ref().unwrap().guest_pc(), A);
        assert_eq!(cache.slots[hash(A)].as_ref().unwrap().placement(), moved);
        assert_eq!(cache.slots[hash(A) ^ 1].as_ref().unwrap().guest_pc(), B);
        cache.max_bytes = MAX_PLAN_BYTES;
        assert!(
            cache
                .prepare(0, A, &[0xc3; 64], moved, 13, &mut work, u64::MAX)
                .unwrap()
                .1
        );
        assert!(
            !cache
                .prepare(0, A, &[0xc3; 64], placement(), 14, &mut work, u64::MAX)
                .unwrap()
                .1
        );
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 2);
        assert_eq!(cache.observations.same_key_replacements, 2);
        assert_eq!(cache.observations.occupied_collisions, 0);

        cache.max_bytes = cache.retained_bytes;
        cache
            .prepare(0, A + 2, &[0xc3; 64], placement(), 99, &mut work, u64::MAX)
            .unwrap();
        assert_eq!(
            (
                cache.observations.byte_purges,
                cache.observations.purged_plans
            ),
            (1, 2)
        );
        assert_eq!(cache.recency[hash(A)], 0);
        assert_eq!(cache.recency[hash(A) ^ 1], 0);
        assert_eq!(cache.recency[hash(A + 2)], 99);
        assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 1);

        // A new cache never inherits hints, even for the same declaration.
        let mut next = PlanCache::new(
            source,
            TranslationPolicy::x64_sse2(),
            BlockLimits::default(),
            &mut work,
            u64::MAX,
        )
        .unwrap();
        assert!(next.recency.iter().all(|stamp| *stamp == 0));
        assert!(
            !next
                .prepare(0, A, &[0xc3; 64], placement(), 15, &mut work, u64::MAX)
                .unwrap()
                .1
        );
    });
}
