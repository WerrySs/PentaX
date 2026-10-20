// SPDX-License-Identifier: GPL-2.0-only
//! Original byte conversion and padding proofs; no GPU or guest code.
use super::*;
use crate::gpu::video::retile_32;

#[test]
fn tiled_load_reverse_overlay_preserves_every_padding_byte_and_refuses_before_mutation() {
    let layout = ImageLayout {
        kind: SurfaceLayout::RenderTarget64Kb,
        width: 192,
        height: 129,
        pitch: 256,
    };
    let limits = PresentationLimits::default();
    let mut baseline = vec![0x9b; 262144];
    let mut tight: Vec<u8> = (0..192 * 129)
        .flat_map(|n| [n as u8, (n >> 8) as u8, 47, 63])
        .collect();
    let mut work = 0;
    retile_32_into(&tight, layout, &mut baseline, &limits, &mut work).unwrap();
    let initial = baseline.clone();
    assert_eq!(
        detile_32(&baseline, layout, baseline.capacity(), &limits, &mut work).unwrap(),
        tight
    );
    // Independent public-source offset vectors across all four64KiB blocks.
    for (n, (x, y, offset)) in [
        (0, 0, 0),
        (128, 0, 65536),
        (0, 128, 131072),
        (128, 128, 196608),
        (127, 127, 0xf0fc),
    ]
    .into_iter()
    .enumerate()
    {
        let value = [17 + n as u8, 31, 255, 127];
        let linear = (y * 192 + x) * 4;
        tight[linear..linear + 4].copy_from_slice(&value);
        retile_32_into(&tight, layout, &mut baseline, &limits, &mut work).unwrap();
        assert_eq!(&baseline[offset..offset + 4], &value);
    }
    assert_eq!(
        detile_32(&baseline, layout, baseline.capacity(), &limits, &mut work).unwrap(),
        tight
    );
    let active = retile_32(
        &vec![255; tight.len()],
        layout,
        baseline.capacity() + tight.capacity(),
        &limits,
        &mut work,
    )
    .unwrap();
    for (n, byte) in baseline.iter().enumerate() {
        if active[n] == 0 {
            assert_eq!((*byte, initial[n]), (0x9b, 0x9b));
        }
    }
    let before = baseline.clone();
    let mut empty_work = 0;
    assert!(
        retile_32_into(
            &tight[..tight.len() - 1],
            layout,
            &mut baseline,
            &limits,
            &mut empty_work
        )
        .is_err()
    );
    assert_eq!(baseline, before);
    assert!(
        retile_32_into(
            &tight,
            layout,
            &mut baseline,
            &PresentationLimits {
                max_work: (tight.len() * 4 - 1) as u64,
                ..limits
            },
            &mut empty_work
        )
        .is_err()
    );
    assert_eq!(baseline, before);
}
