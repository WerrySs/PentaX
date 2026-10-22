// SPDX-License-Identifier: GPL-2.0-only
//! Original same-length source-window consumers and optional paired measurements.
use super::*;
use pentax_platform::native_memory::{
    NATIVE_GUEST_PAGE, NativeAddressSpace, NativeMemoryLimits, NativePermissions,
};

#[derive(Clone, Copy)]
enum Arm {
    Baseline,
    Candidate,
}
impl Arm {
    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
        }
    }
    fn fetch<'storage>(
        self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
        length: u64,
        limit: u64,
        storage: &'storage mut Vec<u8>,
    ) -> Result<&'storage [u8], NativeSessionError> {
        match self {
            Self::Baseline => baseline_fetch_window_into(memory, address, length, limit, storage),
            Self::Candidate => fetch_window_into(memory, address, length, limit, storage),
        }
    }
}

#[test]
fn same_length_source_window_checks_the_whole_current_span_before_overwriting_and_refetches_changed_bytes()
 {
    verify_current_source_guard(Arm::Candidate);
}
fn verify_current_source_guard(arm: Arm) {
    let mut owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let address = base + NATIVE_GUEST_PAGE - 32;
    let original: [u8; 64] = std::array::from_fn(|index| index as u8);
    owner.pause().unwrap().write(address, &original).unwrap();
    owner
        .protect(base, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    let mut storage = vec![0xa5; 64];
    let allocation = storage.as_ptr();
    let capacity = storage.capacity();
    assert_eq!(
        arm.fetch(&owner.pause().unwrap(), address, 64, 64, &mut storage)
            .unwrap(),
        &original
    );

    // Change both halves, then restore execute only on the first page. A
    // forbidden partial prefix copy is now observable on the denied attempt.
    owner
        .protect(base, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let changed: [u8; 64] = std::array::from_fn(|index| 255 - index as u8);
    owner.pause().unwrap().write(address, &changed).unwrap();
    owner
        .protect(base, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    assert!(matches!(
        arm.fetch(&owner.pause().unwrap(), address, 64, 64, &mut storage),
        Err(NativeSessionError::Memory(
            NativeMemoryError::PermissionDenied(_)
        ))
    ));
    assert_eq!(storage.as_slice(), &original);
    assert_eq!(storage.len(), 64);
    assert_eq!(storage.as_ptr(), allocation);
    assert_eq!(storage.capacity(), capacity);

    owner
        .protect(
            base + NATIVE_GUEST_PAGE,
            NATIVE_GUEST_PAGE,
            NativePermissions::READ_EXECUTE,
        )
        .unwrap();
    assert_eq!(
        arm.fetch(&owner.pause().unwrap(), address, 64, 64, &mut storage)
            .unwrap(),
        &changed
    );
    assert_eq!(storage.as_ptr(), allocation);
    assert_eq!(storage.capacity(), capacity);
}

// Exact pre-change source-window implementation from the reviewed base.
fn baseline_fetch_window_into<'window>(
    memory: &PausedNativeMemory<'_>,
    address: u64,
    length: u64,
    allocation_limit: u64,
    storage: &'window mut Vec<u8>,
) -> Result<&'window [u8], NativeSessionError> {
    if length == 0 || length > allocation_limit || storage.capacity() as u64 > allocation_limit {
        return Err(NativeSessionError::Budget("window allocation"));
    }
    let length =
        usize::try_from(length).map_err(|_| NativeSessionError::Budget("window allocation"))?;
    memory.preflight_fetch_source(address, length as u64)?;
    if storage.capacity() < length {
        // Vec reservation is relative to length, including after a shorter
        // previous fetch. The capacity check above implies len < length here.
        storage
            .try_reserve_exact(length - storage.len())
            .map_err(|_| NativeSessionError::Allocation)?;
    }
    if storage.capacity() as u64 > allocation_limit {
        return Err(NativeSessionError::Budget("window allocation"));
    }
    storage.resize(length, 0);
    // Never return bytes from the previous attempt, including on a plan hit.
    memory.fetch_source(address, storage)?;
    Ok(storage.as_slice())
}

#[derive(Clone, Copy, Default)]
struct Sample {
    nanoseconds: u128,
    iterations: u64,
    checksum: u64,
}
fn measure(arm: Arm, iterations: u64) -> Sample {
    use std::{
        hint::black_box,
        time::{Duration, Instant},
    };
    let mut owner =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let original: Vec<u8> = (0..2 * NATIVE_GUEST_PAGE)
        .map(|index| (index as u8).wrapping_mul(17).wrapping_add(3))
        .collect();
    owner.pause().unwrap().write(base, &original).unwrap();
    owner
        .protect(base, 2 * NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
        .unwrap();
    let offsets = [
        0,
        7,
        64,
        511,
        NATIVE_GUEST_PAGE - 65,
        NATIVE_GUEST_PAGE - 64,
        NATIVE_GUEST_PAGE - 32,
        NATIVE_GUEST_PAGE - 1,
    ];
    let mut storage = vec![0; 64];
    let memory = owner.pause().unwrap();
    let mut expected_windows = [0_u64; 8];
    for (slot, offset) in offsets.iter().copied().enumerate() {
        let expected = &original[offset as usize..offset as usize + 64];
        assert_eq!(
            arm.fetch(&memory, base + offset, 64, 64, &mut storage)
                .unwrap(),
            expected
        );
        // Derive the expected digest directly from the independent source bytes,
        // not from either implementation's fetched output or digest helper.
        for word in 0..8 {
            let mut value = 0_u64;
            for byte in 0..8 {
                value |= u64::from(expected[word * 8 + byte]) << (byte * 8);
            }
            expected_windows[slot] =
                expected_windows[slot].wrapping_add(value.rotate_left(word as u32 * 7));
        }
    }
    let expected_checksum =
        expected_windows
            .iter()
            .enumerate()
            .fold(0_u64, |total, (index, value)| {
                let occurrences = iterations / 8 + u64::from((index as u64) < iterations % 8);
                total.wrapping_add(value.wrapping_mul(occurrences))
            });
    let mut checksum = 0_u64;
    let started = Instant::now();
    for index in 0..iterations {
        let address = base + offsets[(index & 7) as usize];
        let bytes = black_box(
            arm.fetch(&memory, black_box(address), 64, 64, &mut storage)
                .unwrap(),
        );
        let digest = bytes
            .chunks_exact(8)
            .enumerate()
            .fold(0_u64, |value, (position, word)| {
                value.wrapping_add(
                    u64::from_le_bytes(word.try_into().unwrap()).rotate_left(position as u32 * 7),
                )
            });
        checksum = checksum.wrapping_add(digest);
        if index & 4095 == 4095 {
            assert!(
                started.elapsed() < Duration::from_secs(2),
                "source-window measurement exceeded the two-second arm ceiling; reduce iterations"
            );
        }
    }
    let nanoseconds = started.elapsed().as_nanos();
    assert!(
        nanoseconds < 2_000_000_000,
        "source-window measurement exceeded the two-second arm ceiling; reduce iterations"
    );
    assert_eq!(checksum, expected_checksum);
    black_box(checksum);
    Sample {
        nanoseconds,
        iterations,
        checksum,
    }
}
fn report(arm: Arm, samples: &[Sample]) {
    let mut times = [0_u128; 5];
    assert!(samples.len() == 1 || samples.len() == 5);
    for (index, sample) in samples.iter().enumerate() {
        assert_eq!(sample.iterations, samples[0].iterations);
        assert_eq!(sample.checksum, samples[0].checksum);
        times[index] = sample.nanoseconds;
        println!(
            "source_window_bench_arm={} sample={} wall_ns={} iterations={} checksum={}",
            arm.name(),
            index,
            sample.nanoseconds,
            sample.iterations,
            sample.checksum
        );
    }
    let times = &mut times[..samples.len()];
    times.sort_unstable();
    println!(
        "source_window_bench_arm={} samples={} wall_median_ns={} wall_min_ns={} wall_max_ns={} wall_spread_ns={} iterations={} checksum={}",
        arm.name(),
        times.len(),
        times[times.len() / 2],
        times[0],
        times[times.len() - 1],
        times[times.len() - 1] - times[0],
        samples[0].iterations,
        samples[0].checksum
    );
}

#[test]
#[ignore = "bounded release-only paired measurement; requires explicit owner approval after title closure"]
fn original_same_length_source_window_paired_release_measurement() {
    assert!(
        !cfg!(debug_assertions),
        "run the ignored measurement with --release"
    );
    let iterations = match std::env::var("PENTAX_SOURCE_WINDOW_BENCH_ITERATIONS") {
        Ok(value) => value
            .parse::<u64>()
            .expect("iterations must be a decimal integer"),
        Err(std::env::VarError::NotPresent) => 1_000_000,
        Err(error) => panic!("invalid iteration setting: {error}"),
    };
    assert!((1..=100_000_000).contains(&iterations));
    let selection =
        std::env::var("PENTAX_SOURCE_WINDOW_BENCH_ARM").unwrap_or_else(|_| "paired".into());
    assert!(matches!(
        selection.as_str(),
        "paired" | "baseline" | "candidate"
    ));
    if selection != "paired" {
        // A private supervisor can measure CPU for this exact owned process.
        // Alternate separate baseline/candidate processes in five AB/BA pairs;
        // use identical iterations and retain their exit statuses and wall data.
        verify_current_source_guard(Arm::Baseline);
        verify_current_source_guard(Arm::Candidate);
        let arm = if selection == "baseline" {
            Arm::Baseline
        } else {
            Arm::Candidate
        };
        report(arm, &[measure(arm, iterations)]);
        return;
    }
    let mut baseline = [Sample::default(); 5];
    let mut candidate = [Sample::default(); 5];
    for pair in 0..5 {
        verify_current_source_guard(Arm::Baseline);
        verify_current_source_guard(Arm::Candidate);
        if pair & 1 == 0 {
            baseline[pair] = measure(Arm::Baseline, iterations);
            candidate[pair] = measure(Arm::Candidate, iterations);
        } else {
            candidate[pair] = measure(Arm::Candidate, iterations);
            baseline[pair] = measure(Arm::Baseline, iterations);
        }
        assert_eq!(baseline[pair].checksum, candidate[pair].checksum);
    }
    report(Arm::Baseline, &baseline);
    report(Arm::Candidate, &candidate);
}
