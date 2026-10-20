// SPDX-License-Identifier: GPL-2.0-only
//! Original phase and pixel admission proofs without Vulkan execution.
use super::*;

#[test]
fn compiler_peak_is_temporary_but_actual_products_and_pixels_remain_charged() {
    let memory = Source::new();
    let planned = plan(&commands(&[]));
    let limits = Limits {
        max_host_bytes: 24 * 1024 * 1024,
        ..Default::default()
    };
    let prepared = prepare_draw(&planned.draws()[0], &memory, &limits).unwrap();
    let usage = prepared.usage();
    assert_eq!(usage.pixel_bytes, prepared.input.capacity());
    assert_eq!(
        usage.host_bytes,
        usage.pixel_bytes
            + prepared.vertex().retained_host_bytes()
            + prepared.pixel().retained_host_bytes()
    );
    assert!(usage.peak_host_bytes >= 2 * limits.compiler_phase_bytes);
    assert!(usage.peak_host_bytes <= limits.max_host_bytes);
    assert!(usage.host_bytes < usage.peak_host_bytes);
    assert_eq!(memory.snapshots.get(), 1);

    // A real temporary phase remains enforced even though the final product
    // is much smaller. Refusal must precede the target LOAD.
    let memory = Source::new();
    assert!(matches!(
        prepare_draw(
            &planned.draws()[0],
            &memory,
            &Limits {
                max_host_bytes: 2 * limits.compiler_phase_bytes - 1,
                ..limits
            }
        ),
        Err(Error::Limit(_))
    ));
    assert_eq!(memory.snapshots.get(), 0);
}

#[test]
fn full_hd_tiled_load_and_backend_pixel_allowance_share_a_finite_phase_budget() {
    let width = 1920_u32;
    let height = 1080_u32;
    let padded = 1920 * 1152 * 4;
    let tight = 1920 * 1080 * 4;
    let mut memory = Source::new();
    memory.target = vec![0xa5; padded];
    let planned = plan(&commands(&[
        (
            RegisterSpace::Context,
            0x3b0,
            ((width - 1) << 14) | (height - 1),
        ),
        (RegisterSpace::Context, 0x3b8, 0x0106_c000),
        (RegisterSpace::Context, 0x0d, width | (height << 16)),
        (RegisterSpace::Context, 0x82, width | (height << 16)),
        (RegisterSpace::Context, 0x91, width | (height << 16)),
        (RegisterSpace::Context, 0x95, width | (height << 16)),
        (RegisterSpace::Context, 0x10f, 960_f32.to_bits()),
        (RegisterSpace::Context, 0x110, 960_f32.to_bits()),
        (RegisterSpace::Context, 0x111, 540_f32.to_bits()),
        (RegisterSpace::Context, 0x112, 540_f32.to_bits()),
    ]));
    // A modeled live original Frame + consumer decode + mapped upload and
    // inline bytes are counted, not a future forty-MiB reservation. This
    // arithmetic is not evidence of any actual device allocation/display.
    let external = 3 * tight + 65536;
    let limits = Limits {
        max_host_bytes: 64 * 1024 * 1024,
        max_pixel_bytes: 64 * 1024 * 1024 - external,
        ..Default::default()
    };
    let prepared = prepare_draw_with_target(
        &planned.draws()[0],
        &memory,
        &limits,
        |baseline, geometry, remaining| {
            assert_eq!(baseline.len(), padded);
            assert_eq!(
                (geometry.pitch(), geometry.width(), geometry.height()),
                (1920, 1920, 1080)
            );
            assert_eq!(
                (geometry.guest_bytes(), geometry.tight_bytes()),
                (padded, tight)
            );
            assert!(remaining.max_host_bytes >= tight);
            // This original constant baseline has the same value under the
            // source tiling permutation; the real conversion remains in core.
            Ok(DecodedTarget {
                bytes: vec![0xa5; tight],
                work: tight as u64,
            })
        },
    )
    .unwrap();
    let usage = prepared.usage();
    assert_eq!(
        usage.pixel_bytes,
        prepared.input.capacity() + prepared.target_snapshot.as_ref().unwrap().capacity()
    );
    assert!(external + usage.pixel_bytes + 2 * tight <= 64 * 1024 * 1024);
    assert!(usage.host_bytes < 24 * 1024 * 1024);
    assert_eq!(memory.snapshots.get(), 1);

    let memory = Source::new();
    assert!(matches!(
        prepare_draw_with_target(
            &planned.draws()[0],
            &memory,
            &Limits {
                max_pixel_bytes: padded + 3 * tight - 1,
                ..limits
            },
            |_, _, _| panic!("pixel refusal reached target decoder")
        ),
        Err(Error::Limit(_))
    ));
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(memory.snapshots.get(), 0);
}
