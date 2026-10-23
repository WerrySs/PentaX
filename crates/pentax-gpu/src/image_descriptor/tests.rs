// SPDX-License-Identifier: GPL-2.0-only
//! Original literal descriptor, odd-mip upload and strict-profile consumers.
use super::*;
const LIMITS: Limits = Limits {
    max_dimension: 16384,
    max_levels: 16,
    max_bytes: 1 << 30,
};
// Original13x5 RGBA8 image, physical3mips/view1..2, absoluteMIN_LOD1.5,
// counter201 enabled and sourceperformancehint7. Address is data, not a mapping.
const IMAGE: [u32; 8] = [
    0x3456789a, 0x03818012, 0x00010003, 0x90021fac, 0, 0x02700020, 201, 0,
];
#[test]
fn literal_tsharp_reversed_odd_mips_drive_original_padded_texture_upload() {
    let decoded = decode_image(IMAGE, &LIMITS).unwrap();
    assert_eq!(
        (decoded.address, decoded.width, decoded.height),
        (0x123456789a00, 13, 5)
    );
    assert_eq!(
        (
            decoded.base_level,
            decoded.last_level,
            decoded.level_count,
            decoded.min_lod_u4_8
        ),
        (1, 2, 3, 384)
    );
    assert_eq!(decoded.counter_id, Some(201));
    assert_eq!(decoded.perf_mod, 7);
    assert_eq!(
        decoded.swizzle,
        [
            Component::Red,
            Component::Green,
            Component::Blue,
            Component::Alpha
        ]
    );
    assert_eq!(
        (
            decoded.mips[0].offset,
            decoded.mips[0].size,
            decoded.mips[0].row_pitch_bytes,
            decoded.mips[0].storage_height
        ),
        (1280, 1280, 256, 5)
    );
    assert_eq!(
        (
            decoded.mips[1].offset,
            decoded.mips[1].size,
            decoded.mips[1].row_pitch_bytes,
            decoded.mips[1].storage_height
        ),
        (512, 768, 256, 3)
    );
    assert_eq!(
        (
            decoded.mips[2].offset,
            decoded.mips[2].size,
            decoded.mips[2].row_pitch_bytes,
            decoded.mips[2].storage_height
        ),
        (0, 512, 256, 2)
    );
    assert_eq!(decoded.total_bytes, 2560);
    assert_eq!(decoded.mips[3..], [MipLevel::default(); 13]);
    // Literal offsets/rowcounts independently populate source storage. Logical
    // mip rows are FLOOR-sized, storage rows CEIL-sized; padding stays visible
    // sentinel and must not become uploaded texels.
    let colors = [[241, 13, 29, 255], [7, 223, 43, 255], [17, 31, 211, 255]];
    let mut storage = vec![0xa5; 2560];
    for (level, (offset, width, height)) in [(1280, 13, 5), (512, 6, 2), (0, 3, 1)]
        .into_iter()
        .enumerate()
    {
        for y in 0..height {
            for x in 0..width {
                let at = offset + y * 256 + x * 4;
                storage[at..at + 4].copy_from_slice(&colors[level]);
            }
        }
    }
    for (level, expected_bytes) in [260, 48, 12].into_iter().enumerate() {
        let mip = decoded.mips[level];
        let mut upload = Vec::new();
        for y in 0..mip.height {
            let start = (mip.offset + u64::from(y) * u64::from(mip.row_pitch_bytes)) as usize;
            upload.extend_from_slice(&storage[start..start + mip.width as usize * 4]);
        }
        assert_eq!(upload.len(), expected_bytes);
        assert!(
            upload
                .chunks_exact(4)
                .all(|p| p == colors[level].as_slice())
        );
    }
    let exact = Limits {
        max_bytes: 2560,
        ..LIMITS
    };
    assert_eq!(decode_image(IMAGE, &exact).unwrap().total_bytes, 2560);
    let one_short = Limits {
        max_bytes: 2559,
        ..LIMITS
    };
    assert_eq!(
        decode_image(IMAGE, &one_short),
        Err(Error::Limit("image storage bytes"))
    );
}
#[test]
fn tsharp_unknown_sampling_metadata_views_and_budgets_refuse_without_normalizing_fields() {
    for (word, mask) in [
        (1, 1 << 29),
        (2, 1 << 12),
        (2, 1 << 31),
        (3, 1 << 20),
        (3, 1 << 25),
        (4, 1),
        (5, 1 << 8),
        (5, 1 << 23),
        (5, 1 << 26),
        (6, 1 << 21),
        (7, 1),
    ] {
        let mut raw = IMAGE;
        raw[word] |= mask;
        assert!(matches!(
            decode_image(raw, &LIMITS),
            Err(Error::Unsupported(_))
        ));
    }
    for (word, value) in [
        (1, IMAGE[1] ^ (1 << 20)),
        (3, (IMAGE[3] & !7) | 2),
        (3, (IMAGE[3] & !(15 << 28)) | (13 << 28)),
        (3, (IMAGE[3] & !(15 << 12)) | (3 << 12)),
        (3, (IMAGE[3] & !(15 << 16)) | (3 << 16)),
        (5, (IMAGE[5] & !(7 << 20)) | (1 << 20)),
    ] {
        let mut raw = IMAGE;
        raw[word] = value;
        assert!(matches!(
            decode_image(raw, &LIMITS),
            Err(Error::Unsupported(_))
        ));
    }
    let mut null = IMAGE;
    null[0] = 0;
    null[1] &= !0xff;
    assert_eq!(
        decode_image(null, &LIMITS),
        Err(Error::Unsupported("null sampled image"))
    );
    assert_eq!(
        decode_image(
            IMAGE,
            &Limits {
                max_dimension: 12,
                ..LIMITS
            }
        ),
        Err(Error::Limit("image dimensions"))
    );
    assert_eq!(
        decode_image(
            IMAGE,
            &Limits {
                max_levels: 2,
                ..LIMITS
            }
        ),
        Err(Error::Limit("image mip levels"))
    );
    let mut disabled = IMAGE;
    disabled[5] &= !(1 << 25);
    assert_eq!(decode_image(disabled, &LIMITS).unwrap().counter_id, None);
    let mut swizzle = IMAGE;
    swizzle[3] = (swizzle[3] & !0xfff) | 0x9c8;
    assert_eq!(
        decode_image(swizzle, &LIMITS).unwrap().swizzle,
        [
            Component::Zero,
            Component::One,
            Component::Alpha,
            Component::Red
        ]
    );
}
#[test]
fn literal_sampler_decodes_signed_bias_filters_addressing_and_effective_mip_clamps() {
    let raw = [0x190, 0x00280080, 0x08103e80, 0x80000000];
    let decoded = decode_sampler(raw).unwrap();
    assert_eq!(
        decoded.address_modes,
        [
            AddressMode::Repeat,
            AddressMode::ClampToEdge,
            AddressMode::ClampToBorder
        ]
    );
    assert_eq!(
        (decoded.mag_filter, decoded.min_filter, decoded.mip_filter),
        (Filter::Linear, Filter::Nearest, MipFilter::Linear)
    );
    assert_eq!(
        (
            decoded.min_lod_u4_8,
            decoded.max_lod_u4_8,
            decoded.lod_bias_s6_8
        ),
        (128, 640, -384)
    );
    assert_eq!(decoded.border, BorderColor::OpaqueWhite);
    let disabled = decode_sampler([0x49, 0xfff, 0x2000, 0]).unwrap();
    assert_eq!(disabled.address_modes, [AddressMode::MirroredRepeat; 3]);
    assert_eq!(
        (
            disabled.min_lod_u4_8,
            disabled.max_lod_u4_8,
            disabled.lod_bias_s6_8
        ),
        (0, 0, -8192)
    );
    assert_eq!(
        decode_sampler([0, 0, 0x1fff, 1 << 30])
            .unwrap()
            .lod_bias_s6_8,
        8191
    );
    assert_eq!(
        decode_sampler([0, 0, 0, 1 << 30]).unwrap().border,
        BorderColor::OpaqueBlack
    );
}
#[test]
fn sampler_unsupported_features_and_invalid_ranges_stay_typed_refusals() {
    for (word, mask) in [
        (0, 1 << 9),
        (0, 1 << 12),
        (0, 1 << 15),
        (0, 1 << 20),
        (0, 1 << 29),
        (1, 1 << 24),
        (2, 1 << 14),
        (2, 1 << 24),
        (2, 1 << 28),
        (3, 1),
    ] {
        let mut raw = [0; 4];
        raw[word] = mask;
        assert!(matches!(decode_sampler(raw), Err(Error::Unsupported(_))));
    }
    for address in [3, 4, 5, 7] {
        assert_eq!(
            decode_sampler([address, 0, 0, 0]),
            Err(Error::Unsupported("sampler address mode"))
        );
    }
    assert_eq!(
        decode_sampler([0, 0, 2 << 20, 0]),
        Err(Error::Unsupported("sampler anisotropic filter"))
    );
    assert_eq!(
        decode_sampler([0, 0, 3 << 26, 0]),
        Err(Error::Unsupported("sampler mip filter"))
    );
    assert_eq!(
        decode_sampler([0, 0x100, 1 << 26, 0]),
        Err(Error::Unsupported("sampler inverted LOD clamp"))
    );
    assert_eq!(
        decode_sampler([0, 0, 0, 3 << 30]),
        Err(Error::Unsupported("sampler border color table"))
    );
}
