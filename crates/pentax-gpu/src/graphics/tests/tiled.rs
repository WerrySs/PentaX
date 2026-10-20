// SPDX-License-Identifier: GPL-2.0-only
//! Original declared-layout preparation proofs; conversion math stays in core.
use super::*;

#[test]
fn tiled_geometry_retains_full_load_baseline_and_bounded_tight_decoder_product() {
    let mut memory = Source::new();
    memory.target = vec![0xa5; 65536];
    let draw = plan(&commands(&[(RegisterSpace::Context, 0x3b8, 0x0106_c000)]));
    let called = Cell::new(false);
    let prepared = prepare_draw_with_target(
        &draw.draws()[0],
        &memory,
        &Limits::default(),
        |baseline, geometry, remaining| {
            called.set(true);
            assert_eq!(geometry.layout(), TargetLayout::RenderTarget64KbRgba8);
            assert_eq!(
                (geometry.width(), geometry.height(), geometry.pitch()),
                (64, 64, 128)
            );
            assert_eq!(
                (geometry.guest_bytes(), geometry.tight_bytes()),
                (65536, 16384)
            );
            assert_eq!(baseline, &[0xa5; 65536]);
            assert!(remaining.max_host_bytes >= geometry.tight_bytes());
            // A constant original source is invariant under any legal swizzle;
            // this proves hook/ownership accounting without duplicating math.
            Ok(DecodedTarget {
                bytes: vec![0xa5; geometry.tight_bytes()],
                work: 99,
            })
        },
    )
    .unwrap();
    assert!(called.get());
    assert_eq!(prepared.input(), &[0xa5; 16384]);
    assert_eq!(prepared.target_snapshot(), memory.target);
    assert_eq!(prepared.usage().target_bytes, 65536);
    assert!(prepared.usage().host_bytes >= 65536 + 16384);
    assert_eq!(prepared.into_target_snapshot(), memory.target);
    assert_eq!(memory.snapshots.get(), 1);
}

#[test]
fn tiled_unknown_swizzles_layers_pipe_bits_alignment_and_full_span_limits_refuse() {
    let memory = Source::new();
    for (register, value) in [
        (0x3b8, 0x0106_c001),
        (0x3b8, 0x0506_c000),
        (0x3b8, 0x4106_c000),
        (0x3b8, 0x8106_c000),
        (0x3b8, 0x0106_8000),
        (0x318, ((TARGET + 256) >> 8) as u32),
    ] {
        let planned = plan(&commands(&[
            (RegisterSpace::Context, 0x3b8, 0x0106_c000),
            (RegisterSpace::Context, register, value),
        ]));
        assert!(matches!(
            prepare_draw_with_target(
                &planned.draws()[0],
                &memory,
                &Limits::default(),
                |_, _, _| panic!("unsupported state reached decoder")
            ),
            Err(Error::Unsupported(_))
        ));
    }
    let planned = plan(&commands(&[(RegisterSpace::Context, 0x3b8, 0x0106_c000)]));
    assert!(matches!(
        prepare_draw_with_target(
            &planned.draws()[0],
            &memory,
            &Limits {
                max_target_bytes: 16384,
                ..Default::default()
            },
            |_, _, _| panic!("padded span limit reached decoder")
        ),
        Err(Error::Limit(_))
    ));
    assert!(memory.reads.borrow().is_empty());
    assert_eq!(memory.snapshots.get(), 0);
}
