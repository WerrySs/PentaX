// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumption of both exact interpolant mapping exports.
use super::*;
const GS: i32 = 0x4100;
const PS: i32 = 0x4200;
const OUTPUT: i32 = 0x4300;
const GS_SEM: i32 = 0x4500;
const PS_SEM: i32 = 0x4600;

fn word(p: &mut Program, field: i32, expected: u64) {
    p.path(7, field);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(expected);
}
#[test]
fn native_interpolant_exports_preserve_source_f16_differences_identity_tail_and_exact_scope() {
    for (module, library, version) in [
        ("Graphics5", "Graphics5", 1),
        ("libSceAgc", "libSceAgc", 1),
        ("Graphics5", "Graphics5", 2),
        ("Graphics5Driver", "Graphics5Driver", 1),
    ] {
        let mut p = Program::new();
        for (field, target) in [(GS + 56, GS_SEM), (PS + 48, PS_SEM)] {
            p.path(0, target);
            p.path(7, field);
            p.code.extend([0x48, 0x89, 0x07]);
        }
        for (slot, expected) in [(0, 0x0228_023f_u64), (1, 0x0228_011f)] {
            p.path(7, OUTPUT);
            p.path(6, GS);
            p.path(2, PS);
            p.call(slot);
            p.expect(0);
            word(&mut p, OUTPUT, (expected << 32) | 0x191);
            word(&mut p, OUTPUT + 8, (1_u64 << 32) | 0x192);
            word(&mut p, OUTPUT + 31 * 8, (31_u64 << 32) | 0x1b0);
            word(&mut p, OUTPUT - 8, 0xa5a5_a5a5_a5a5_a5a5);
            word(&mut p, OUTPUT + 256, 0xa5a5_a5a5_a5a5_a5a5);
        }
        p.path(7, OUTPUT);
        p.reg(6, u64::MAX);
        p.reg(2, 0);
        p.call(1);
        p.expect(0);
        word(&mut p, OUTPUT, 0x191);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["HV4j+E0MBHE", "dbOlWdppb4o"],
            module,
            library,
            version,
        );
        let mut bytes = original.bytes.clone();
        bytes.resize(0x5000, 0);
        put(&mut bytes, 56, &3_u16.to_le_bytes());
        put(&mut bytes, 176, &1_u32.to_le_bytes());
        put(&mut bytes, 180, &6_u32.to_le_bytes());
        for (at, value) in [
            (184, 0x4000_u64),
            (192, 0x4000),
            (208, 0x1000),
            (216, 0x4000),
            (224, 0x4000),
        ] {
            put(&mut bytes, at, &value.to_le_bytes());
        }
        put(&mut bytes, GS as usize + 86, &1_u16.to_le_bytes());
        put(&mut bytes, PS as usize + 80, &1_u32.to_le_bytes());
        put(&mut bytes, GS_SEM as usize, &0x0020_1f37_u32.to_le_bytes());
        put(&mut bytes, PS_SEM as usize, &0x6020_0137_u32.to_le_bytes());
        for at in [OUTPUT as usize - 8, OUTPUT as usize + 256] {
            put(&mut bytes, at, &0xa5a5_a5a5_a5a5_a5a5_u64.to_le_bytes());
        }
        let original = create_input(bytes);
        let (status, fields, text) = probe(&original);
        if version == 2 {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "import-version-mismatch");
        } else if module == "Graphics5Driver" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "unavailable-function");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
            assert_eq!(fields["unavailable_functions"], "0");
        }
    }
}
