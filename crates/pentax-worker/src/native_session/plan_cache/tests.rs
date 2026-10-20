// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use pentax_codegen::declare_materialized_source;
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn image() -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 1)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(20, 1_u32), (64, 1), (68, 5)] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (24, 0x200_u64),
        (32, 64),
        (96, 4096),
        (104, 0x4000),
        (112, 0x4000),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x200..].fill(0xc3); // Original RET instructions.
    bytes
}

#[test]
fn plan_cache_keeps_formerly_colliding_pcs_in_distinct_native_placements() {
    let mut bytes = image();
    bytes[0x200..0x203].copy_from_slice(&[0x31, 0xc0, 0xc3]); // XOR EAX,EAX; RET.
    bytes[0x604..0x607].copy_from_slice(&[0x31, 0xdb, 0xc3]); // XOR EBX,EBX; RET.
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = BlockLimits::default();
    let source = declare_materialized_source(&linked, &[], &limits).unwrap();
    let mut work = 0;
    let mut cache = PlanCache::new(
        &source,
        TranslationPolicy::x64_sse2(),
        limits,
        &mut work,
        u64::MAX,
    )
    .unwrap();
    // Both PCs selected slot 530 in the former 1024-entry table. Their native
    // hash slots are 530 and 1554 in the runtime's 2 MiB / 4096-slot cache.
    let cases = [
        (0x100200_u64, 530_usize, pentax_codegen::Register::EAX),
        (0x100604_u64, 1554_usize, pentax_codegen::Register::EBX),
    ];
    assert_eq!(cases[0].1 & 1023, cases[1].1 & 1023);
    let mut hits = 0;
    let mut misses = 0;
    for round in 0..8 {
        for (pc, native_slot, destination) in cases {
            let placement = CachePlacement {
                address: 0x200000 + native_slot as u64 * 512,
                capacity: 500, // Native stride minus its 8-byte cell and 4-byte tail growth.
            };
            let offset = (pc - 0x100000) as usize;
            let live = &bytes[offset..offset + 64];
            let (plan, hit) = cache
                .prepare(0, pc, live, placement, &mut work, u64::MAX)
                .unwrap();
            assert_eq!(hit, round != 0);
            if hit {
                hits += 1;
            } else {
                misses += 1;
            }
            assert_eq!(plan.guest_pc(), pc);
            assert_eq!(plan.placement(), placement);
            assert_eq!(plan.relocated_source_bytes(), live);
            assert_eq!(
                plan.instructions()[0].instruction().op0_register(),
                destination
            );
            assert!(std::ptr::eq(plan.declaration(), &source));
        }
    }
    assert_eq!((misses, hits), (2, 14));
    assert!(cache.slots[cases[0].1].is_some());
    assert!(cache.slots[cases[1].1].is_some());
    assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 2);
    assert_eq!(cache.max_bytes, 8 * 1024 * 1024);
    assert!(cache.retained_bytes <= cache.max_bytes);
}

#[test]
fn plan_cache_slot_metadata_is_charged_before_bounded_allocation() {
    let bytes = image();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = BlockLimits::default();
    let source = declare_materialized_source(&linked, &[], &limits).unwrap();
    let metadata_bytes = 4096_u64
        .checked_mul(std::mem::size_of::<Option<MaterializedBlockPlan<'_, '_, '_>>>() as u64)
        .unwrap();
    let mut work = 7;
    let exact = work + metadata_bytes;
    assert!(matches!(
        PlanCache::new(
            &source,
            TranslationPolicy::x64_sse2(),
            limits,
            &mut work,
            exact - 1,
        ),
        Err(NativeSessionError::Budget("admission work"))
    ));
    assert_eq!(work, 7);
    let cache = PlanCache::new(
        &source,
        TranslationPolicy::x64_sse2(),
        limits,
        &mut work,
        exact,
    )
    .unwrap();
    assert_eq!(work, exact);
    assert_eq!(cache.slots.len(), 4096);
    assert_eq!(cache.slots.capacity(), 4096);
    assert_eq!(cache.retained_bytes, 0);
    assert_eq!(cache.max_bytes, 8 * 1024 * 1024);
    // A target-derived observation for the local validation log, never guest data.
    eprintln!("plan_cache_slot_metadata_bytes={metadata_bytes}");
}

#[test]
fn plan_cache_reuses_only_exact_live_windows_and_is_bound_to_one_declaration() {
    let bytes = image();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = BlockLimits::default();
    let source = declare_materialized_source(&linked, &[], &limits).unwrap();
    let placement = CachePlacement {
        address: 0x200000,
        capacity: 4096,
    };
    let policy = TranslationPolicy::x64_sse2();
    let mut work = 0;
    let mut cache = PlanCache::new(&source, policy, limits, &mut work, u64::MAX).unwrap();
    let (_, hit) = cache
        .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    assert!(!hit);
    let (plan, hit) = cache
        .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    assert!(hit);
    assert_eq!(plan.placement(), placement);
    assert!(std::ptr::eq(plan.declaration(), &source));
    let moved = CachePlacement {
        address: placement.address + 0x10000,
        ..placement
    };
    let (plan, hit) = cache
        .prepare(0, 0x100200, &[0xc3; 64], moved, &mut work, u64::MAX)
        .unwrap();
    assert!(!hit);
    assert_eq!(plan.placement(), moved);
    assert!(
        !cache
            .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
            .unwrap()
            .1
    );
    let mut changed = [0xc3; 64];
    changed[63] = 0x90; // Even an unconsumed trailing byte is attested.
    assert!(matches!(
        cache.prepare(0, 0x100200, &changed, placement, &mut work, u64::MAX),
        Err(NativeSessionError::Codegen(
            CodegenError::MaterializedWindowMismatch { .. }
        ))
    ));
    assert!(
        cache
            .prepare(1, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
            .is_err()
    );
    assert!(
        cache
            .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
            .unwrap()
            .1
    );
    assert!(
        !cache
            .prepare(0, 0x100200, &[0xc3; 32], placement, &mut work, u64::MAX)
            .unwrap()
            .1
    );
    let other = declare_materialized_source(&linked, &[], &limits).unwrap();
    let mut next = PlanCache::new(&other, policy, limits, &mut work, u64::MAX).unwrap();
    let (plan, hit) = next
        .prepare(0, 0x100200, &[0xc3; 32], placement, &mut work, u64::MAX)
        .unwrap();
    assert!(!hit);
    assert!(std::ptr::eq(plan.declaration(), &other));
}

#[test]
fn plan_cache_bounds_retained_capacity_and_refuses_work_before_lookup() {
    let bytes = image();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = BlockLimits::default();
    let source = declare_materialized_source(&linked, &[], &limits).unwrap();
    let mut work = 0;
    let placement = CachePlacement {
        address: 0x200000,
        capacity: 4096,
    };
    let mut cache = PlanCache::new(
        &source,
        TranslationPolicy::x64_sse2(),
        limits,
        &mut work,
        u64::MAX,
    )
    .unwrap();
    cache
        .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    let entry_bytes = cache.retained_bytes;
    cache.max_bytes = 2 * entry_bytes;
    cache
        .prepare(0, 0x100201, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    assert_eq!(cache.retained_bytes, 2 * entry_bytes);
    cache
        .prepare(0, 0x100202, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    assert_eq!(cache.retained_bytes, entry_bytes);
    assert_eq!(
        cache.slots.iter().filter(|entry| entry.is_some()).count(),
        1
    );
    let before = work;
    assert!(
        cache
            .prepare(0, 0x100202, &[0xc3; 64], placement, &mut work, before)
            .is_err()
    );
    assert_eq!(work, before);
    assert_eq!(cache.retained_bytes, entry_bytes);
    assert!(
        !cache
            .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
            .unwrap()
            .1
    );
}
