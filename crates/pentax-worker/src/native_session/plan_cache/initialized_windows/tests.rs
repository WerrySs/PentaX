// SPDX-License-Identifier: GPL-2.0-only
//! Original immutable-source lookups, fallbacks and live-backing refusals.
use super::*;
use pentax_codegen::declare_materialized_source;
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn image(length: usize, overlap: bool) -> Vec<u8> {
    let mut bytes = vec![0; length];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (at, value) in [
        (16, 3_u16),
        (18, 62),
        (52, 64),
        (54, 56),
        (56, if overlap { 2 } else { 1 }),
    ] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(20, 1_u32), (64, 1), (68, 5)] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (24, 0x200_u64),
        (32, 64),
        (96, length as u64),
        (104, 0x4000),
        (112, 0x4000),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    if overlap {
        bytes.copy_within(64..120, 120);
    }
    bytes[0x200..].fill(0xc3);
    bytes
}

#[test]
fn initialized_window_memo_keys_exact_limits_collisions_and_new_epochs_without_caching_errors() {
    let bytes = image(0x1000, false);
    let inputs = [
        ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000),
        ModuleInput::new(open_executable(&bytes).unwrap(), 0x110000),
    ];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let source = declare_materialized_source(&linked, &[], &Default::default()).unwrap();
    let mut work = 0;
    let mut memo = Memo::new(&source, MAX_PLAN_BYTES, &mut work, u64::MAX);
    assert!(memo.bytes != 0);
    for (pc, maximum, expected) in [
        (0x100200, 64, (0, 64)),
        (0x100200, 64, (0, 64)),
        (0x100200, 17, (0, 17)),
        (0x100ffd, 64, (0, 3)),
        (0x100ffd, 7, (0, 3)),
    ] {
        assert_eq!(memo.get(pc, maximum).unwrap(), expected);
    }
    assert_eq!(memo.scans, 4);
    // These distinct original module PCs share one full4096-entry hash slot.
    let hash = |pc: u64| ((pc ^ (pc >> 8) ^ (pc >> 16) ^ (pc >> 32)) as usize) & (SLOTS - 1);
    assert_eq!(hash(0x100200), hash(0x110300));
    for (pc, expected) in [
        (0x100200, (0, 64)),
        (0x110300, (1, 64)),
        (0x100200, (0, 64)),
    ] {
        assert_eq!(memo.get(pc, 64).unwrap(), expected);
    }
    let before = memo.scans;
    for _ in 0..2 {
        for pc in [0x101000, 0x103fff, 0x108000] {
            // RX BSS and the module gap.
            assert!(matches!(
                memo.get(pc, 64),
                Err(NativeSessionError::SourceWindow)
            ));
        }
        assert!(matches!(
            memo.get(0x100200, 0),
            Err(NativeSessionError::Budget("source window"))
        ));
    }
    assert_eq!(memo.scans, before + 8);
    let shorter = image(0x800, false);
    let next_inputs = [
        ModuleInput::new(open_executable(&shorter).unwrap(), 0x100000),
        ModuleInput::new(open_executable(&bytes).unwrap(), 0x120000),
    ];
    let next = plan_symbolic(&next_inputs, &SymbolicLimits::default()).unwrap();
    let declaration = declare_materialized_source(&next, &[], &Default::default()).unwrap();
    let mut fresh = Memo::new(&declaration, MAX_PLAN_BYTES, &mut work, u64::MAX);
    assert!(matches!(
        fresh.get(0x100ffd, 64),
        Err(NativeSessionError::SourceWindow)
    ));
    assert_eq!(fresh.get(0x120200, 64).unwrap(), (1, 64));
    assert_eq!(fresh.scans, 2);
    // The typed source itself rejects overlapping logical segments, before
    // either the scanner or a memo can derive a successful window from them.
    let overlapping = image(0x1000, true);
    let inputs = [ModuleInput::new(
        open_executable(&overlapping).unwrap(),
        0x100000,
    )];
    assert!(matches!(
        plan_symbolic(&inputs, &SymbolicLimits::default()),
        Err(pentax_linker::SymbolicError::Relocation {
            error: pentax_linker::RelocationError::OverlappingSegments { .. },
            ..
        })
    ));
}

#[test]
fn initialized_window_memo_optional_cost_and_capacity_never_displace_existing_plan_capacity() {
    let bytes = image(0x1000, false);
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let source = declare_materialized_source(&linked, &[], &Default::default()).unwrap();
    let requested = SLOTS as u64 * std::mem::size_of::<Entry>() as u64;
    let mut work = 7;
    let mut no_work = Memo::new(&source, MAX_PLAN_BYTES, &mut work, 7 + requested - 1);
    assert_eq!((no_work.bytes, no_work.entries.capacity(), work), (0, 0, 7));
    assert_eq!(no_work.get(0x100200, 64).unwrap(), (0, 64));
    assert_eq!(no_work.get(0x100200, 64).unwrap(), (0, 64));
    assert_eq!(no_work.scans, 2);
    let mut no_space = Memo::new(&source, requested - 1, &mut work, u64::MAX);
    assert_eq!((no_space.bytes, work), (0, 7));
    assert_eq!(no_space.get(0x100200, 64).unwrap(), (0, 64));
    let mut memo = Memo::new(&source, MAX_PLAN_BYTES, &mut work, u64::MAX);
    assert_eq!(
        memo.bytes,
        memo.entries.capacity() as u64 * std::mem::size_of::<Entry>() as u64
    );
    assert_eq!(work, 7 + memo.bytes);
    memo.yield_to_plans(0, 1, memo.bytes);
    assert_eq!((memo.bytes, memo.entries.capacity()), (0, 0));
    assert_eq!(memo.get(0x100200, 64).unwrap(), (0, 64));
    let mut cache = PlanCache::new(
        &source,
        TranslationPolicy::x64_sse2(),
        BlockLimits::default(),
        &mut work,
        u64::MAX,
    )
    .unwrap();
    let placement = CachePlacement {
        address: 0x200000,
        capacity: 4096,
    };
    cache
        .prepare(0, 0x100200, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    let plan_bytes = cache.retained_bytes;
    assert!(cache.initialized_windows.bytes != 0);
    assert!(cache.retained_bytes + cache.initialized_windows.bytes <= cache.max_bytes);
    cache.max_bytes = 2 * plan_bytes;
    cache
        .prepare(0, 0x100201, &[0xc3; 64], placement, &mut work, u64::MAX)
        .unwrap();
    assert_eq!(cache.initialized_windows.bytes, 0);
    assert_eq!(cache.retained_bytes, 2 * plan_bytes);
    assert_eq!(cache.slots.iter().filter(|slot| slot.is_some()).count(), 2);
    // The exact former constructor work bound still admits the original plan
    // table and leaves an allocation-free initialized-window scanner fallback.
    let original_table_work =
        SLOTS as u64 * std::mem::size_of::<Option<MaterializedBlockPlan<'_, '_, '_>>>() as u64;
    let mut exact_work = 0;
    let mut exact = PlanCache::new(
        &source,
        TranslationPolicy::x64_sse2(),
        BlockLimits::default(),
        &mut exact_work,
        original_table_work,
    )
    .unwrap();
    assert_eq!(exact_work, original_table_work);
    assert_eq!(exact.initialized_windows.bytes, 0);
    assert_eq!(exact.initialized_window(0x100200, 64).unwrap(), (0, 64));
    eprintln!("initialized_window_memo_bytes={requested}");
}

#[test]
fn initialized_window_memo_never_reuses_live_bytes_or_protection_checks() {
    use pentax_platform::native_memory::{
        NativeAddressSpace, NativeMemoryLimits, NativePermissions,
    };
    let bytes = image(0x1000, false);
    let mut owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    owner.pause().unwrap().write(base, &bytes).unwrap();
    owner
        .protect(base, 0x4000, NativePermissions::READ_EXECUTE)
        .unwrap();
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), base)];
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let source = declare_materialized_source(&linked, &[], &Default::default()).unwrap();
    let mut work = 0;
    let mut cache = PlanCache::new(
        &source,
        TranslationPolicy::x64_sse2(),
        BlockLimits::default(),
        &mut work,
        u64::MAX,
    )
    .unwrap();
    let pc = base + 0x200;
    let mut storage = Vec::new();
    let placement = CachePlacement {
        address: base + 0x20000,
        capacity: 4096,
    };
    let (module, length) = cache.initialized_window(pc, 64).unwrap();
    let live = fetch_window_into(&owner.pause().unwrap(), pc, length, 64, &mut storage).unwrap();
    cache
        .prepare(module, pc, live, placement, &mut work, u64::MAX)
        .unwrap();
    let retained = cache.retained_bytes;
    let scans = cache.initialized_windows.scans;
    owner
        .protect(base, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    owner.pause().unwrap().write(pc + 63, &[0x90]).unwrap();
    assert_eq!(cache.initialized_window(pc, 64).unwrap(), (module, length));
    assert_eq!(cache.initialized_windows.scans, scans);
    assert!(matches!(
        fetch_window_into(&owner.pause().unwrap(), pc, length, 64, &mut storage),
        Err(NativeSessionError::Memory(
            NativeMemoryError::PermissionDenied(_)
        ))
    ));
    owner
        .protect(base, 0x4000, NativePermissions::READ_EXECUTE)
        .unwrap();
    let live = fetch_window_into(&owner.pause().unwrap(), pc, length, 64, &mut storage).unwrap();
    assert_eq!(live[63], 0x90);
    assert!(matches!(
        cache.prepare(module, pc, live, placement, &mut work, u64::MAX),
        Err(NativeSessionError::Codegen(
            CodegenError::MaterializedWindowMismatch { .. }
        ))
    ));
    assert_eq!(cache.retained_bytes, retained);
    owner.unmap(base, 0x4000).unwrap();
    assert_eq!(cache.initialized_window(pc, 64).unwrap(), (module, length));
    assert!(matches!(
        fetch_window_into(&owner.pause().unwrap(), pc, length, 64, &mut storage),
        Err(NativeSessionError::Memory(NativeMemoryError::Unmapped(_)))
    ));
}

#[test]
#[ignore = "bounded comparative metadata timing; run explicitly under the validation gate"]
fn initialized_window_memo_original_lookup_measurement() {
    use std::hint::black_box;
    let bytes = image(0x1000, false);
    let inputs: Vec<_> = (0..16)
        .map(|module| {
            ModuleInput::new(
                open_executable(&bytes).unwrap(),
                0x100000 + module * 0x10000,
            )
        })
        .collect();
    let linked = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let source = declare_materialized_source(&linked, &[], &Default::default()).unwrap();
    let cases = [
        (0x100200, 64),
        (0x130604, 64),
        (0x170ffd, 64),
        (0x1f0200, 17),
    ];
    let mut work = 0;
    let mut memo = Memo::new(&source, MAX_PLAN_BYTES, &mut work, u64::MAX);
    let mut expected = 0_u64;
    let start = Instant::now();
    for _ in 0..12_500 {
        for (pc, maximum) in cases {
            let (module, length) =
                initialized_window(black_box(&source), black_box(pc), maximum).unwrap();
            expected = expected.wrapping_add(module as u64 + length);
            black_box((module, length));
        }
    }
    let scan_ns = start.elapsed().as_nanos();
    let mut actual = 0_u64;
    let start = Instant::now();
    for _ in 0..12_500 {
        for (pc, maximum) in cases {
            let (module, length) = memo.get(black_box(pc), maximum).unwrap();
            actual = actual.wrapping_add(module as u64 + length);
            black_box((module, length));
        }
    }
    let memo_ns = start.elapsed().as_nanos();
    assert_eq!(actual, expected);
    assert!(memo.bytes != 0);
    assert_eq!(memo.scans, 4);
    // No timing inequality: these measurements do not establish title speedup.
    eprintln!(
        "initialized_window_measurement_cases=50000 initialized_window_scan_ns={scan_ns} initialized_window_memo_ns={memo_ns} initialized_window_memo_scans={} initialized_window_memo_bytes={}",
        memo.scans, memo.bytes
    );
}
