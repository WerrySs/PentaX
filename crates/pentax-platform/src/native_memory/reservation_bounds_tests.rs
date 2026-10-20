// SPDX-License-Identifier: GPL-2.0-only
//! Original pure reservation admission proofs; no host allocation or scan.
use super::*;

#[test]
fn constrained_window_checks_exclusive_end_alignment_capacity_and_overflow() {
    let granule = 65536;
    let window = NativeRange {
        address: granule,
        size: (1 << 40) - granule,
    };
    assert_eq!(
        checked_reservation_window(2 * granule, window, granule, 2 * granule),
        Ok(1 << 40)
    );
    assert_eq!(
        checked_reservation_window(
            granule,
            NativeRange {
                address: NATIVE_ADDRESS_LIMIT - granule,
                size: granule
            },
            granule,
            granule
        ),
        Ok(NATIVE_ADDRESS_LIMIT)
    );
    for (size, range, alignment, cap) in [
        (0, window, granule, granule),
        (NATIVE_GUEST_PAGE, window, granule, granule),
        (granule, window, 0, granule),
        (granule, window, 3 * NATIVE_GUEST_PAGE, granule),
        (granule, window, granule, granule - 1),
        (
            granule,
            NativeRange {
                address: 0,
                size: granule,
            },
            granule,
            granule,
        ),
        (
            granule,
            NativeRange {
                address: granule + 1,
                size: granule,
            },
            granule,
            granule,
        ),
        (
            granule,
            NativeRange {
                address: granule,
                size: 0,
            },
            granule,
            granule,
        ),
        (
            granule,
            NativeRange {
                address: granule,
                size: granule - 1,
            },
            granule,
            granule,
        ),
        (
            2 * granule,
            NativeRange {
                address: granule,
                size: granule,
            },
            granule,
            2 * granule,
        ),
        (
            granule,
            NativeRange {
                address: NATIVE_ADDRESS_LIMIT,
                size: granule,
            },
            granule,
            granule,
        ),
        (
            granule,
            NativeRange {
                address: u64::MAX - granule + 1,
                size: granule,
            },
            granule,
            granule,
        ),
    ] {
        assert_eq!(
            checked_reservation_window(size, range, alignment, cap),
            Err(NativeMemoryError::InvalidRange)
        );
    }
}

#[test]
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
fn constrained_reservation_never_constructs_an_owner_on_unsupported_hosts() {
    for size in [0, 65536] {
        assert!(matches!(
            NativeAddressSpace::reserve_within(
                Default::default(),
                size,
                NativeRange {
                    address: 65536,
                    size: (1 << 40) - 65536
                }
            ),
            Err(NativeMemoryError::UnsupportedHost)
        ));
    }
}
