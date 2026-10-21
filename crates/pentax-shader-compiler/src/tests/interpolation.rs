// SPDX-License-Identifier: GPL-2.0-only
//! Original interpolation programs prove coordinate/partial provenance.
use super::*;

fn compile_words(words: &[u32]) -> Result<CompiledShader, CompileError> {
    let limits = Limits::default();
    let program = ShaderProgram::decode(words, ShaderStage::Pixel, &limits).unwrap();
    compile(&program, &profile(ShaderStage::Pixel), &limits)
}

#[test]
fn interpolation_requires_matched_unmodified_center_coordinates_and_p1_partial() {
    for case in 0..9 {
        let mut words = fixtures::fragment();
        match case {
            0 => {
                words.insert(0, fixtures::vector_unary(1, 6, 128));
            }
            1 => {
                words.insert(0, fixtures::vector_unary(1, 7, 128));
            }
            2 => {
                words[1] |= 1 << 10;
            } // Different ATTR in P2.
            3 => {
                words[1] |= 1 << 8;
            } // Different channel in P2.
            4 => {
                words.insert(1, fixtures::vector_unary(1, 0, 128));
            }
            5 => {
                words.remove(0);
            }
            6 => {
                words[0] = (words[0] & !255) | 7;
            }
            7 => {
                words[1] = (words[1] & !255) | 6;
            }
            8 => {
                words.remove(1);
            } // Live unconsumed P1 is never an emitted placeholder.
            _ => unreachable!(),
        }
        let result = compile_words(&words);
        if case == 8 {
            assert!(matches!(result, Err(CompileError::Unsupported(_))));
        } else {
            assert!(result.is_err(), "provenance case {case}");
        }
    }
    let direct = compile_words(&fixtures::fragment()).unwrap();
    validate("interpolation-center-provenance", &direct);
    // Plain register copies preserve genuine center-coordinate SSA values.
    let mut copied = fixtures::fragment();
    copied[0] = (copied[0] & !255) | 8;
    copied[1] = (copied[1] & !255) | 9;
    copied.splice(
        ..0,
        [
            fixtures::vector_unary(1, 8, 262),
            fixtures::vector_unary(1, 9, 263),
        ],
    );
    let copied = compile_words(&copied).unwrap();
    assert_eq!(direct.manifest().inputs(), copied.manifest().inputs());
    // A genuine partial can also pass through a plain register copy.
    let mut partial = fixtures::fragment();
    partial[1] |= 10 << 18;
    partial.insert(1, fixtures::vector_unary(1, 10, 256));
    partial.insert(3, fixtures::vector_unary(1, 0, 266));
    assert_eq!(
        compile_words(&partial).unwrap().manifest().inputs(),
        direct.manifest().inputs()
    );
}
