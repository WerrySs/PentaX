// SPDX-License-Identifier: GPL-2.0-only
//! Original glyph lifecycles, source-order output and real coverage readback.

use super::*;
use crate::{
    CallArgs, Limits,
    tests::{ARG, PAGE, TestGuest},
};

struct Fixture {
    render: RenderState,
    objects: Objects,
    memory: MemoryState,
    guest: TestGuest,
    budget: Budget,
    font: u64,
}
fn call(service: Service, args: &[u64], xy: Option<[f32; 2]>) -> FontCall {
    let mut words = [0; 6];
    words[..args.len()].copy_from_slice(args);
    let vector_args = xy.map(|xy| {
        let mut vectors = [[0x5a; 16]; 8];
        vectors[0][..4].copy_from_slice(&xy[0].to_le_bytes());
        vectors[1][..4].copy_from_slice(&xy[1].to_le_bytes());
        vectors
    });
    FontCall {
        service,
        args: CallArgs(words),
        vector_args,
        stack_pointer: None,
    }
}
impl Fixture {
    fn new() -> Self {
        let limits = Limits::default();
        let mut value = Self {
            render: RenderState::default(),
            objects: Objects::default(),
            memory: MemoryState::new(limits).unwrap(),
            guest: TestGuest::new(),
            budget: Budget::new(limits).unwrap(),
            font: 0,
        };
        assert_eq!(
            value.objects.invoke(
                call(Service::FontOpenFontSet, &[0, 0, 0, 0, ARG], None),
                &mut value.memory,
                &mut value.guest,
                &mut value.budget,
                0
            ),
            Ok(0)
        );
        value.font = value.guest.u64(0);
        value
    }
    fn invoke(
        &mut self,
        service: Service,
        args: &[u64],
        xy: Option<[f32; 2]>,
    ) -> Result<u64, Failure> {
        let held = self.objects.metadata_bytes() + self.render.metadata_bytes();
        self.render.invoke(
            call(service, args, xy),
            &mut self.objects,
            &mut self.memory,
            &mut self.guest,
            &mut self.budget,
            held,
        )
    }
}

#[test]
fn font_surface_initialization_scissor_and_nulls_write_only_exact_source_fields() {
    let mut f = Fixture::new();
    f.guest.put(0x100, &[0xa5; 136]);
    assert_eq!(
        f.invoke(
            Service::FontRenderSurfaceInit,
            &[ARG + 0x100, u64::MAX, 123, 0x1ff, u32::MAX as u64, 7],
            None
        ),
        Ok(0)
    );
    let mut expected = [0; 128];
    expected[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    expected[8..12].copy_from_slice(&123_i32.to_le_bytes());
    expected[12] = 0xff;
    expected[20..24].copy_from_slice(&7_i32.to_le_bytes());
    expected[36..40].copy_from_slice(&7_u32.to_le_bytes());
    assert_eq!(f.guest.get(ARG + 0x100, 128), expected);
    assert_eq!(f.guest.get(ARG + 0x180, 8), [0xa5; 8]);
    assert_eq!(
        f.invoke(
            Service::FontRenderSurfaceSetScissor,
            &[ARG + 0x100, 1, 2, 3, 4],
            None
        ),
        Ok(0)
    );
    for (index, value) in [1_u32, 2, 3, 4].iter().enumerate() {
        expected[24 + index * 4..28 + index * 4].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(f.guest.get(ARG + 0x100, 128), expected);
    assert_eq!(
        f.invoke(
            Service::FontRenderSurfaceInit,
            &[0, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX],
            None
        ),
        Ok(0)
    );
    assert_eq!(
        f.invoke(
            Service::FontRenderSurfaceSetScissor,
            &[0, u64::MAX, u64::MAX, u64::MAX, u64::MAX],
            None
        ),
        Ok(0)
    );
}

#[test]
fn generated_glyph_owns_pixels_after_font_close_and_delete_is_atomic_retryable() {
    let mut f = Fixture::new();
    f.guest.put(0x80, &[0xa5; 16]);
    f.guest.refuse = true;
    assert!(
        f.invoke(
            Service::FontGenerateCharGlyph,
            &[f.font, 65, u64::MAX, ARG + 0x80],
            None
        )
        .is_err()
    );
    assert_eq!(f.render.retained_handle_count(), 0);
    assert_eq!(f.render.metadata_bytes(), 0);
    assert_eq!(f.guest.get(ARG + 0x80, 16), [0xa5; 16]);
    assert_eq!(f.budget.usage().handles_live, 2);
    f.guest.refuse = false;
    assert_eq!(
        f.invoke(
            Service::FontGenerateCharGlyph,
            &[f.font, 65, u64::MAX, ARG + 0x80],
            None
        ),
        Ok(0)
    );
    let glyph = f.guest.u64(0x80);
    let pixels = f.guest.get(glyph + IMAGE_OFFSET, engine::IMAGE_BYTES);
    assert!(pixels.iter().any(|value| *value != 0));
    assert_eq!(f.guest.get(ARG + 0x88, 8), [0xa5; 8]);
    assert_eq!(f.budget.usage().handles_live, 4);
    assert_eq!(
        f.invoke(
            Service::FontGlyphDefineAttribute,
            &[glyph, 7, ARG + 0xa0],
            None
        ),
        Ok(0)
    );
    assert_eq!(f.guest.get(ARG + 0xa0, 4), 0_i32.to_le_bytes());
    f.guest.refuse = true;
    assert!(
        f.invoke(
            Service::FontGlyphDefineAttribute,
            &[glyph, 9, ARG + 0xa0],
            None
        )
        .is_err()
    );
    f.guest.refuse = false;
    assert_eq!(
        f.invoke(
            Service::FontGlyphDefineAttribute,
            &[glyph, 11, ARG + 0xa0],
            None
        ),
        Ok(0)
    );
    assert_eq!(f.guest.get(ARG + 0xa0, 4), 7_i32.to_le_bytes());
    let held = f.objects.metadata_bytes() + f.render.metadata_bytes();
    assert_eq!(
        f.objects.invoke(
            call(Service::FontCloseFont, &[f.font], None),
            &mut f.memory,
            &mut f.guest,
            &mut f.budget,
            held
        ),
        Ok(0)
    );
    assert_eq!(
        f.guest.get(glyph + IMAGE_OFFSET, engine::IMAGE_BYTES),
        pixels
    );
    f.guest.refuse = true;
    assert!(
        f.invoke(Service::FontDeleteGlyph, &[u64::MAX, ARG + 0x80], None)
            .is_err()
    );
    assert_eq!(f.guest.u64(0x80), glyph);
    assert_eq!(f.render.retained_handle_count(), 1);
    f.guest.refuse = false;
    assert_eq!(
        f.invoke(Service::FontDeleteGlyph, &[u64::MAX, ARG + 0x80], None),
        Ok(0)
    );
    assert_eq!(f.guest.u64(0x80), 0);
    assert_eq!(f.render.retained_handle_count(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
    assert!(f.guest.read(glyph, &mut [0]).is_err());
}

#[test]
fn real_coverage_scissor_result_padding_and_late_output_refusal_share_one_commit() {
    let mut f = Fixture::new();
    let surface = ARG + 0x100;
    let metrics = ARG + 0x200;
    let result = ARG + 0x300;
    let buffer = ARG + 0x800;
    assert_eq!(
        f.invoke(
            Service::FontRenderSurfaceInit,
            &[surface, buffer, 128, 2, 64, 64],
            None
        ),
        Ok(0)
    );
    assert_eq!(
        f.invoke(
            Service::FontRenderSurfaceSetScissor,
            &[surface, 12, 18, 18, 28],
            None
        ),
        Ok(0)
    );
    f.guest.put(0x200, &[0xa5; 40]);
    f.guest.put(0x300, &[0xa5; 72]);
    f.guest.put(0x800, &[0x35; 8192]);
    let initial = f.guest.get(f.font, OBJECT_BYTES);
    assert!(
        f.invoke(
            Service::FontRenderCharGlyphImageHorizontal,
            &[f.font, 65, surface, metrics, ARG + PAGE - 32],
            Some([10.0, 30.0])
        )
        .is_err()
    );
    assert_eq!(f.guest.get(f.font, OBJECT_BYTES), initial);
    assert_eq!(f.guest.get(metrics, 40), [0xa5; 40]);
    assert_eq!(f.guest.get(buffer, 8192), [0x35; 8192]);
    assert_eq!(
        f.invoke(
            Service::FontRenderCharGlyphImageHorizontal,
            &[f.font, 65, surface, metrics, result],
            Some([10.0, 30.0])
        ),
        Ok(0)
    );
    let glyph = engine::rasterize(
        None,
        16.0,
        65,
        0,
        &mut Budget::new(Limits::default()).unwrap(),
    )
    .unwrap();
    assert_eq!(f.guest.get(metrics, 32), glyph.metrics.bytes());
    assert_eq!(f.guest.get(metrics + 32, 8), [0xa5; 8]);
    let output = f.guest.get(result, 72);
    assert_eq!(&output[22..24], &[0xa5; 2]);
    assert_eq!(&output[64..72], &[0xa5; 8]);
    assert_eq!(u64::from_le_bytes(output[..8].try_into().unwrap()), f.font);
    assert_eq!(
        u64::from_le_bytes(output[8..16].try_into().unwrap()),
        buffer
    );
    let x = integer(10.0 + glyph.metrics.values[2]).unwrap().max(0);
    let y = integer(30.0 - glyph.metrics.values[3]).unwrap().max(0);
    let mut expected = vec![0x35; 8192];
    let mut modified = 0;
    for yy in 18_i32..28 {
        for xx in 12_i32..18 {
            if xx < x || yy < y || xx >= x + glyph.width as i32 || yy >= y + glyph.height as i32 {
                continue;
            }
            let alpha = glyph.pixels[(yy - y) as usize * 128 + (xx - x) as usize];
            if alpha != 0 {
                let at = yy as usize * 128 + xx as usize * 2;
                expected[at..at + 2].fill(alpha);
                modified += 1;
            }
        }
    }
    assert!(modified > 0);
    assert_eq!(f.guest.get(buffer, 8192), expected);
}

#[test]
fn source_order_normalization_preserves_unwritten_holes_and_rollback() {
    let mut guest = TestGuest::new();
    let mut budget = Budget::new(Limits::default()).unwrap();
    guest.put(0, &[0x61; 16]);
    let mut plan = Plan::new(0, &mut budget).unwrap();
    plan.store(ARG + 2, &[1, 2, 3, 4], &mut budget).unwrap();
    plan.store(ARG + 4, &[9, 8, 7, 6], &mut budget).unwrap();
    plan.store(ARG + 10, &[5, 5], &mut budget).unwrap();
    let mut staged = [0; 12];
    plan.read(&guest, ARG, &mut staged, &mut budget).unwrap();
    assert_eq!(staged, [0x61, 0x61, 1, 2, 9, 8, 7, 6, 0x61, 0x61, 5, 5]);
    guest.refuse = true;
    assert!(plan.publish(&mut guest, &mut budget).is_err());
    assert_eq!(guest.get(ARG, 16), [0x61; 16]);
    guest.refuse = false;
    let mut plan = Plan::new(0, &mut budget).unwrap();
    plan.store(ARG + 2, &[1, 2, 3, 4], &mut budget).unwrap();
    plan.store(ARG + 4, &[9, 8, 7, 6], &mut budget).unwrap();
    plan.store(ARG + 10, &[5, 5], &mut budget).unwrap();
    plan.publish(&mut guest, &mut budget).unwrap();
    assert_eq!(guest.get(ARG, 12), staged);
    // Original zero-alpha preservation is checked by coverage readback above;
    // no complete surface snapshot is needed to prepare destination writes.
}
