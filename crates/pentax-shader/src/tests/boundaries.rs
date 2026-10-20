// SPDX-License-Identifier: GPL-2.0-only
// New synthetic truncation, ownership and budget tests for the larger
// multi-word metadata of the PentaX Rust port. No guest/game fixtures.

use std::mem::size_of;

use super::memory::{Mimg, buf1, ds, ds1, flat, flat1, mubuf, smem, smem1};
use super::vector::{src3, vop1, vop2, vop3};
use super::*;
use crate::model::Record;

/// Complete instructions and, for each shorter prefix, the total length the
/// decoder can prove it needs at that point (literal needs are only known
/// once the modifier word that selects them has been read).
fn cases() -> Vec<(Vec<u32>, Vec<usize>)> {
    let sdwa_scalar_literal = 255 | (6 << 8) | (6 << 16) | (1 << 23);
    vec![
        (vec![vop1(0x01, 0, 255), 1], vec![2]),
        (vec![vop1(0x01, 0, 249), sdwa_scalar_literal, 1], vec![2, 3]),
        (vec![vop2(0x2c, 0, 0, 250), 0xff00_0000, 1], vec![2, 3]),
        (
            vec![vop3(0x141, 0, 0, 0, 0), src3(1, 255, 2, 0, 0), 1],
            vec![2, 3],
        ),
        (vec![smem(0x00, 0, 0, 0), smem1(0, 4)], vec![2]),
        (
            vec![mubuf(0x00, 0, 0, 0, 0, 0), buf1(0, 0, 0, 0, 0, 0)],
            vec![2],
        ),
        (vec![flat(0x0c, 0, 0, 0), flat1(0, 0, 0, 0)], vec![2]),
        (vec![ds(0x36, 0, 0, 0), ds1(0, 0, 0, 0)], vec![2]),
        (
            Mimg {
                nsa: 3,
                ..Mimg::new(0x20, 1)
            }
            .words(),
            vec![5, 5, 5, 5],
        ),
        (vec![0xf800_000f, 0], vec![2]),
    ]
}

#[test]
fn every_prefix_of_every_multiword_form_is_truncated_without_partial_output() {
    for (words, required) in cases() {
        let family = instruction_family(words[0]);
        // A preceding one-word instruction proves the error names the right start.
        let mut input = vec![0xbf80_0000];
        input.extend_from_slice(&words);
        let span = decoded(&input);
        assert_eq!(span.instruction_count(), 2, "{family:?}");
        assert_eq!(span.instruction(1).unwrap().raw_words(), &words[..]);
        assert_eq!(span.instruction(1).unwrap().word_offset(), 1);
        for (available, required_words) in (1..words.len()).zip(required) {
            assert_eq!(
                decode(
                    &input[..=available],
                    ShaderStage::Compute,
                    &Limits::default()
                )
                .unwrap_err(),
                DecodeError::Truncated {
                    at_word: 1,
                    family,
                    required_words,
                    available_words: available,
                },
                "{family:?} with {available} words"
            );
        }
    }
}

#[test]
fn larger_records_are_charged_by_size_and_fit_the_default_budget() {
    let limits = Limits::default();
    assert!(size_of::<Record>() * limits.max_instructions <= limits.max_record_bytes);
    let words = Mimg {
        nsa: 3,
        ..Mimg::new(0x20, 1)
    }
    .words();
    let exact = Limits {
        max_record_bytes: size_of::<Record>(),
        ..Limits::default()
    };
    let span = decode(&words, ShaderStage::Compute, &exact).unwrap();
    assert_eq!(span.record_capacity_bytes(), size_of::<Record>());
    let short = Limits {
        max_record_bytes: size_of::<Record>() - 1,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words, ShaderStage::Compute, &short).unwrap_err(),
        DecodeError::LimitExceeded {
            at_word: 0,
            kind: LimitKind::RecordBytes
        }
    );
}

#[test]
fn every_body_and_literal_word_is_charged_as_work_before_it_is_read() {
    // 5 copied + 1 header + 4 body words; no prior records to move.
    let words = Mimg {
        nsa: 3,
        ..Mimg::new(0x20, 1)
    }
    .words();
    let ok = Limits {
        max_work: 10,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words, ShaderStage::Compute, &ok)
            .unwrap()
            .work_units(),
        10
    );
    let short = Limits {
        max_work: 9,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words, ShaderStage::Compute, &short).unwrap_err(),
        DecodeError::LimitExceeded {
            at_word: 0,
            kind: LimitKind::Work
        }
    );
    // SDWA + literal: 3 copied + header + modifier + literal = 6.
    let words = [
        vop1(0x01, 0, 249),
        255 | (6 << 8) | (6 << 16) | (1 << 23),
        7,
    ];
    assert_eq!(decoded(&words).work_units(), 6);
    let short = Limits {
        max_work: 5,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words, ShaderStage::Compute, &short).unwrap_err(),
        DecodeError::LimitExceeded {
            at_word: 0,
            kind: LimitKind::Work
        }
    );
    // Truncation is reported before the missing word is charged.
    let tight = Limits {
        max_work: 4,
        ..Limits::default()
    };
    assert_eq!(
        decode(&words[..2], ShaderStage::Compute, &tight).unwrap_err(),
        DecodeError::Truncated {
            at_word: 0,
            family: Family::Vop1,
            required_words: 3,
            available_words: 2
        }
    );
}

#[test]
fn image_and_operand_metadata_are_owned_by_the_decoded_span() {
    let mut input = Mimg {
        nsa: 2,
        ..Mimg::new(0x20, 1)
    }
    .words();
    input.push(vop1(0x01, 0, 250));
    input.push(9 | (0x1ab << 8));
    let span = decoded(&input);
    input.fill(0);
    let image = span.instruction(0).unwrap();
    assert_eq!(image.raw_words(), &span.source_words()[..4]);
    assert_eq!(
        image.image().unwrap().nsa_addresses(),
        &[10, 11, 12, 13, 14, 15, 16, 17]
    );
    let dpp = span.instruction(1).unwrap().sources()[0].dpp().unwrap();
    assert_eq!(dpp.control(), 0x1ab);
    assert_eq!(span.instruction(1).unwrap().word_offset(), 4);
}

#[test]
fn unsupported_reasons_are_bounded_static_diagnostics() {
    let error = decode(
        &[ds(0x35, 0, 0, 0xe0), 0],
        ShaderStage::Compute,
        &Limits::default(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "unsupported Ds opcode 53 at word 0: DS swizzle FFT mode is not implemented"
    );
}

#[test]
fn random_words_in_every_family_never_panic_or_overrun() {
    let limits = Limits {
        max_source_words: 16,
        max_instructions: 16,
        max_record_bytes: 16 * size_of::<Record>(),
        max_work: 128,
    };
    let headers = [
        0x7e00_0000,
        0,
        0x7c00_0000,
        0xd400_0000,
        0xcc00_0000,
        0xf400_0000,
        0xe000_0000,
        0xe800_0000,
        0xdc00_0000,
        0xd800_0000,
        0xf000_0000,
    ];
    let mut seed = 0x5eed_1234_u32;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed
    };
    for header in headers {
        for _ in 0..512 {
            let mut words = [0u32; 6];
            for word in &mut words {
                *word = next();
            }
            // Keep the family bits, randomize the remaining header fields.
            let mask = if header & 0x8000_0000 == 0 {
                0x01ff_ffff
            } else {
                0x03ff_ffff
            };
            words[0] = header | (words[0] & mask);
            let length = 1 + (next() as usize % words.len());
            if let Ok(span) = decode(&words[..length], ShaderStage::Unknown, &limits) {
                let total: usize = span.instructions().map(|i| i.raw_words().len()).sum();
                assert_eq!(total, length);
                for inst in span.instructions() {
                    assert!((1..=5).contains(&inst.raw_words().len()));
                    assert!(inst.sources().len() <= 4);
                }
            }
        }
    }
}
