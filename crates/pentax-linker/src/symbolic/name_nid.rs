// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded streaming SHA-1/NID transformation.
// Adapted from pinned libKernel.cpp; see docs/upstream-provenance.md.

use crate::{NidToken, SELECTOR_ALPHABET};

const SALT: [u8; 16] = [
    0x51, 0x8d, 0x64, 0xa6, 0x35, 0xde, 0xd8, 0xc1, 0xe6, 0xb0, 0x39, 0xb1, 0xc3, 0xe5, 0x52, 0x30,
];

/// Caller admits length/block work before entering. No allocation or I/O.
pub(super) fn for_name(name: &[u8]) -> NidToken {
    let length = name.len() + SALT.len();
    let padded = (length + 9).div_ceil(64) * 64;
    let bits = (length as u64 * 8).to_be_bytes();
    let mut h = [
        0x6745_2301_u32,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    for block in 0..padded / 64 {
        let mut words = [0_u32; 80];
        for index in 0..64 {
            let at = block * 64 + index;
            let byte = if at < name.len() {
                name[at]
            } else if at < length {
                SALT[at - name.len()]
            } else if at == length {
                0x80
            } else if at >= padded - 8 {
                bits[at - (padded - 8)]
            } else {
                0
            };
            words[index / 4] |= u32::from(byte) << ((3 - index % 4) * 8);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (index, &word) in words.iter().enumerate() {
            let (f, k) = match index {
                0..20 => ((b & c) | (!b & d), 0x5a82_7999_u32),
                20..40 => (b ^ c ^ d, 0x6ed9_eba1),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (value, addition) in h.iter_mut().zip([a, b, c, d, e]) {
            *value = value.wrapping_add(addition);
        }
    }
    let mut bytes = [0; 8];
    bytes[..4].copy_from_slice(&h[0].to_be_bytes());
    bytes[4..].copy_from_slice(&h[1].to_be_bytes());
    let digest = u64::from_le_bytes(bytes);
    let mut token = [0; 11];
    for (index, byte) in token.iter_mut().take(10).enumerate() {
        *byte = SELECTOR_ALPHABET[((digest >> (58 - index * 6)) & 63) as usize];
    }
    token[10] = SELECTOR_ALPHABET[((digest & 15) * 4) as usize];
    NidToken::parse(&token).expect("fixed canonical encoding")
}
