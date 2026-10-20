// SPDX-License-Identifier: GPL-2.0-only
//! Original assertions over real licensed font data and hostile synthetic input.

use super::*;
use crate::Limits;

fn budget() -> Budget {
    Budget::new(Limits::default()).unwrap()
}

#[test]
fn licensed_fallback_draws_real_coverage_space_and_missing_codepoint() {
    let mut work = budget();
    let letter = rasterize(None, 16.0, u32::from(b'A'), 0, &mut work).unwrap();
    assert!(letter.width > 0 && letter.height > 0);
    assert!(letter.pixels.iter().any(|value| *value > 0));
    assert!(letter.pixels.iter().any(|value| *value > 0 && *value < 255));
    assert!(letter.metrics.values[4] >= letter.metrics.values[0]);
    assert_eq!(letter.pixels.len(), IMAGE_BYTES);
    let space = rasterize(None, 16.0, u32::from(b' '), 0, &mut work).unwrap();
    assert!(space.pixels.iter().all(|value| *value == 0));
    assert!(space.metrics.values[4] > 0.0);
    let replacement = rasterize(None, 16.0, u32::from(b'?'), 0, &mut work).unwrap();
    let invalid = rasterize(None, 16.0, 0x11_0000, 0, &mut work).unwrap();
    assert_eq!(replacement.pixels, invalid.pixels);
    assert_eq!(replacement.metrics, invalid.metrics);
    assert!(work.usage().scratch_peak >= IMAGE_BYTES);
}

#[test]
fn source_height_policy_and_font_scratch_work_refuse_before_success() {
    let ordinary = metrics(None, 16.0, u32::from(b'A'), 0, &mut budget()).unwrap();
    let defaulted = metrics(None, 1.0, u32::from(b'A'), 0, &mut budget()).unwrap();
    assert_eq!(ordinary, defaulted);
    let tall = metrics(None, 32.0, u32::from(b'A'), 0, &mut budget()).unwrap();
    assert!(tall.values[1] > ordinary.values[1]);
    assert_eq!(tall.values[7], 32.0);
    assert!(metrics(None, f32::NAN, u32::from(b'A'), 0, &mut budget()).is_err());
    assert!(metrics(None, f32::MAX, u32::from(b'A'), 0, &mut budget()).is_err());
    let mut limited = Budget::new(Limits {
        max_work: 1,
        ..Limits::default()
    })
    .unwrap();
    assert!(matches!(
        rasterize(None, 16.0, 65, 0, &mut limited),
        Err(Failure::Limit(_))
    ));
    let mut limited = Budget::new(Limits {
        max_scratch_bytes: 1024,
        max_io_bytes: 1024,
        ..Limits::default()
    })
    .unwrap();
    assert!(matches!(
        rasterize(None, 16.0, 65, 0, &mut limited),
        Err(Failure::Limit(_))
    ));
    let layout = horizontal_layout(None, 16.0, 0, &mut budget()).unwrap();
    assert!(layout[0] > 0.0 && layout[1] >= layout[0]);
    assert_eq!(layout[1], layout[2]);
}

#[test]
fn malformed_supported_font_magic_and_cff_expansion_do_not_become_blank_success() {
    let malformed = [0, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    assert!(rasterize(Some(&malformed), 16.0, 65, 0, &mut budget()).is_err());
    let empty = Index::Empty;
    let bounded = CffBudget {
        inner: (&[], &empty, &empty, &empty),
        remaining: Cell::new(3),
    };
    assert!(bounded.span(&[1, 2]).is_ok());
    assert!(bounded.span(&[3, 4]).is_err());
    // A failed expansion cannot underflow its remaining allowance.
    assert_eq!(bounded.remaining.get(), 1);
}

#[test]
fn zero_target_cff2_blend_exposure_is_admitted_across_all_variation_indices_before_evaluation() {
    // Original ItemVariationStore: index0 has one region, index1 has64,
    // each with64 axes. No glyph/font bytes or outside fixture are used.
    let region_start = 16;
    let first = region_start + 4 + 64 * 64 * 6;
    let second = first + 8;
    let mut bytes = vec![0; second + 6 + 64 * 2];
    bytes[..2].copy_from_slice(&1_u16.to_be_bytes());
    bytes[2..6].copy_from_slice(&(region_start as u32).to_be_bytes());
    bytes[6..8].copy_from_slice(&2_u16.to_be_bytes());
    bytes[8..12].copy_from_slice(&(first as u32).to_be_bytes());
    bytes[12..16].copy_from_slice(&(second as u32).to_be_bytes());
    bytes[region_start..region_start + 2].copy_from_slice(&64_u16.to_be_bytes());
    bytes[region_start + 2..region_start + 4].copy_from_slice(&64_u16.to_be_bytes());
    bytes[first + 4..first + 6].copy_from_slice(&1_u16.to_be_bytes());
    bytes[second + 4..second + 6].copy_from_slice(&64_u16.to_be_bytes());
    for index in 0..64 {
        let at = second + 6 + index * 2;
        bytes[at..at + 2].copy_from_slice(&(index as u16).to_be_bytes());
    }
    let store = ItemVariationStore::read(bytes.as_slice().into()).unwrap();
    // Zero target values do not fail the evaluator's operand-count check:
    // the public evaluator really accepts blend and visits its region scalars.
    let empty = Index::Empty;
    let context = CffBudget {
        inner: (&[], &empty, &empty, &empty),
        remaining: Cell::new(CFF_BYTES),
    };
    let mut null = skrifa::outline::pen::NullPen;
    let mut sink = NopFilterSink::new(&mut null);
    let blend = BlendState::new(store.clone(), &[], 1).unwrap();
    assert!(cs::evaluate(&context, Some(blend), &[139, 16, 139, 16], &mut sink).is_ok());
    // Admission must see index1 even if the initial subfont index is0. The
    // full cumulative expansion envelope exceeds this bounded work allowance.
    let mut work = budget();
    assert!(matches!(
        cff_blend_work(Some(&store), 0, &mut work),
        Err(Failure::Limit(_))
    ));
    // Common nonvariable CFF remains within the same default allowance.
    assert!(cff_blend_work(None, 0, &mut budget()).is_ok());
}
