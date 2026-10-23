// SPDX-License-Identifier: GPL-2.0-only
//! Original synthetic packets: no commercial bytes or inferred addresses.
use super::*;
fn packet(control: u32, source: u64, destination: u64, count: u32) -> Vec<u8> {
    [
        0xc005_5000,
        control,
        source as u32,
        (source >> 32) as u32,
        destination as u32,
        (destination >> 32) as u32,
        count,
    ]
    .into_iter()
    .flat_map(u32::to_le_bytes)
    .collect()
}
#[test]
fn gds_dma_metadata_keeps_distinct_offsets_and_exact_public_bounds() {
    let filled = decode(&packet((2 << 29) | (1 << 20), 0xdead_beef_1122_3344, 0, 8)).unwrap();
    assert!(filled.uses_gds());
    assert_eq!(filled.destination_gds_offset(), Some(0));
    assert_eq!(filled.source_address(), None);
    assert_eq!(filled.fill_word(), Some(0x1122_3344));
    let out = decode(&packet((1 << 29) | (3 << 20), 65532, 0x10000, 4)).unwrap();
    assert_eq!(out.source_gds_offset(), Some(65532));
    assert_eq!(out.destination_gds_offset(), None);
    assert_eq!(out.source_address(), None);
    let input = decode(&packet((3 << 29) | (1 << 20), 0x10000, 65532, 4)).unwrap();
    assert_eq!(input.source_address(), Some(0x10000));
    assert_eq!(input.destination_gds_offset(), Some(65532));
    assert_eq!(input.packet()[0], 0xc005_5000);
    for (src, dst, control, count) in [
        (0u64, 65532, (2 << 29) | (1 << 20), 8),
        (65532, 0x10000, (1 << 29) | (3 << 20), 8),
        (1, 0x10000, (1 << 29) | (3 << 20), 4),
        (0, 1, (2 << 29) | (1 << 20), 4),
        (0, 1u64 << 32, (2 << 29) | (1 << 20), 4),
    ] {
        assert!(decode(&packet(control, src, dst, count)).is_err());
    }
}
#[test]
fn gds_dma_unknown_address_modes_stay_refused_and_zero_retains_no_backing_access() {
    assert!(decode(&packet((1 << 29) | (1 << 20), 0, 4, 4)).is_err());
    for flags in [1 << 26, 1 << 27, 1 << 28, 1 << 29] {
        assert!(decode(&packet((3 << 29) | (3 << 20), 0x10000, 0x20000, 4 | flags)).is_err());
    }
    let zero = decode(&packet(u32::MAX, u64::MAX, u64::MAX, 0xfc00_0000)).unwrap();
    assert_eq!(zero.byte_count(), 0);
    assert!(!zero.uses_gds());
    assert_eq!(zero.source_address(), None);
    assert_eq!(zero.source_gds_offset(), None);
}
