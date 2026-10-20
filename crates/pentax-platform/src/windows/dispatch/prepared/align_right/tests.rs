// SPDX-License-Identifier: GPL-2.0-only
//! Independent literal concatenations distinguish lane boundaries and zero-fill.
use super::*;

#[test]
fn all_unsigned_immediates_select_literal_lane_windows_without_count_masking() {
    let low = std::array::from_fn(|i| if i < 16 { i as u8 + 1 } else { i as u8 + 0x31 });
    let high = std::array::from_fn(|i| i as u8 + 0x81);
    let lower = [
        1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86,
        0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d, 0x8e, 0x8f, 0x90, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0,
    ];
    let upper = [
        0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e, 0x4f,
        0x50, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b, 0x9c, 0x9d, 0x9e,
        0x9f, 0xa0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    for size in [16, 32] {
        for immediate in 0..=255_u8 {
            let mut expected = [0; 32];
            if immediate < 32 {
                let offset = usize::from(immediate);
                expected[..16].copy_from_slice(&lower[offset..offset + 16]);
                if size == 32 {
                    expected[16..].copy_from_slice(&upper[offset..offset + 16]);
                }
            }
            assert_eq!(
                calculate(
                    Form {
                        vector_bytes: size,
                        legacy: false
                    },
                    &high,
                    &low,
                    immediate
                ),
                expected,
                "size={size} immediate={immediate}"
            );
        }
    }
}
