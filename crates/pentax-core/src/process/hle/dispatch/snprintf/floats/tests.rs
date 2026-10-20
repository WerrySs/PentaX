// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX inputs and directed expectations for the pinned public profile.

use super::*;

fn render(
    value: f64,
    spec: u8,
    width: u32,
    precision: u32,
    flags: u32,
) -> Result<Vec<u8>, MemoryServiceError> {
    let mut bytes = Vec::new();
    format(value, spec, width, precision, flags, &mut |byte| {
        bytes.push(byte);
        Ok(())
    })?;
    Ok(bytes)
}

#[test]
fn fixed_rounding_sign_width_and_default_precision_follow_the_pinned_profile() {
    assert_eq!(render(1.5, b'f', 0, 0, 0).unwrap(), b"1.500000");
    assert_eq!(render(-0.0, b'F', 0, 0, 0).unwrap(), b"0.000000");
    assert_eq!(render(1.5, b'f', 0, 0, FLAGS_PRECISION).unwrap(), b"2");
    assert_eq!(render(2.5, b'f', 0, 0, FLAGS_PRECISION).unwrap(), b"2");
    assert_eq!(render(2.25, b'f', 0, 1, FLAGS_PRECISION).unwrap(), b"2.2");
    assert_eq!(render(2.75, b'f', 0, 1, FLAGS_PRECISION).unwrap(), b"2.8");
    assert_eq!(
        render(
            1.5,
            b'f',
            10,
            2,
            FLAGS_PRECISION | FLAGS_PLUS | FLAGS_ZEROPAD
        )
        .unwrap(),
        b"+000001.50"
    );
    assert_eq!(
        render(1.5, b'f', 6, 1, FLAGS_PRECISION | FLAGS_LEFT).unwrap(),
        b"1.5   "
    );
    assert_eq!(
        render(1.0, b'f', 0, 0, FLAGS_PRECISION | FLAGS_HASH).unwrap(),
        b"1"
    );
}

#[test]
fn exponential_and_adaptive_cases_share_the_real_source_lowering() {
    assert_eq!(render(1.0, b'e', 0, 0, 0).unwrap(), b"1.000000e+00");
    assert_eq!(render(-1.5, b'E', 0, 0, 0).unwrap(), b"-1.500000E+00");
    assert_eq!(render(1.0, b'g', 0, 0, 0).unwrap(), b"1.00000");
    assert_eq!(render(1.0, b'G', 0, 0, 0).unwrap(), b"1.00000");
    // Source adaptive choice is based on [1e-4,1e6), not the usual precision
    // threshold. It also retains fractional trailing zeros above.
    assert_eq!(
        render(100000.0, b'g', 0, 3, FLAGS_PRECISION).unwrap(),
        b"100000"
    );
    let automatic = render(1e10, b'f', 0, 0, 0).unwrap();
    assert!(automatic.contains(&b'e'));
    assert!(automatic.iter().all(u8::is_ascii));
}

#[test]
fn all_six_conversions_keep_lowercase_specials_and_source_sign_rules() {
    for spec in *b"fFeEgG" {
        assert_eq!(render(f64::NAN, spec, 0, 0, 0).unwrap(), b"nan");
        assert_eq!(
            render(f64::INFINITY, spec, 0, 0, FLAGS_PLUS).unwrap(),
            b"+inf"
        );
        assert_eq!(
            render(f64::INFINITY, spec, 0, 0, FLAGS_SPACE).unwrap(),
            b"inf"
        );
        assert_eq!(render(f64::NEG_INFINITY, spec, 0, 0, 0).unwrap(), b"-inf");
    }
    assert_eq!(
        render(f64::NEG_INFINITY, b'F', 7, 0, 0).unwrap(),
        b"   -inf"
    );
}

#[test]
fn fixed_buffer_precision_limits_are_defined_and_source_undefined_indexing_refuses() {
    assert_eq!(
        render(0.0, b'f', 0, 10, FLAGS_PRECISION).unwrap(),
        b"0.0000000000"
    );
    // At precision41 the reversed zero suffix fills the source's whole buffer;
    // even its decimal point/whole part no longer fit. This is source behavior.
    assert_eq!(
        render(1.0, b'f', 0, 41, FLAGS_PRECISION).unwrap(),
        [b'0'; 32]
    );
    let mut emitted = 0;
    assert_eq!(
        format(1.0, b'f', 0, 42, FLAGS_PRECISION, &mut |_| {
            emitted += 1;
            Ok(())
        }),
        Err(MemoryServiceError::UnsupportedFormat("floating precision"))
    );
    assert_eq!(emitted, 0);
    // Missing PRECISION means the supplied word is ignored in favor of6.
    assert_eq!(render(1.0, b'f', 0, u32::MAX, 0).unwrap(), b"1.000000");
}

#[test]
fn emitter_failure_propagates_without_substituting_an_empty_success() {
    let mut bytes = Vec::new();
    let result = format(1.5, b'f', 0, 0, 0, &mut |byte| {
        if bytes.len() == 3 {
            return Err(MemoryServiceError::Allocation);
        }
        bytes.push(byte);
        Ok(())
    });
    assert_eq!(result, Err(MemoryServiceError::Allocation));
    assert_eq!(bytes, b"1.5");
    assert_eq!(
        render(1.0, b'a', 0, 0, 0),
        Err(MemoryServiceError::UnsupportedFormat("floating conversion"))
    );
}
