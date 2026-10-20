// SPDX-License-Identifier: GPL-2.0-only
//! Original native caller over exact shader-half query/fusion imports.
use super::*;

const FRONT: i32 = 0x4100;
const BACK: i32 = 0x4200;
const OUTPUT: i32 = 0x4300;
const FRONT_REGS: i32 = 0x4400;
const BACK_REGS: i32 = 0x4500;
const FRONT_SPECIALS: i32 = 0x4600;
const BACK_SPECIALS: i32 = 0x4620;
const SCRATCH: i32 = 0x4700;
const SIZE: i32 = 0x4780;
const CODE: i32 = 0x4800;
const USER: i32 = 0x4840;

fn assign(p: &mut Program, field: i32, target: i32) {
    p.path(0, target);
    p.path(7, field);
    p.code.extend([0x48, 0x89, 0x07]);
}
fn value(p: &mut Program, field: i32, expected: u64, narrow: bool) {
    p.path(7, field);
    if !narrow {
        p.code.push(0x48);
    }
    p.code.extend([0x8b, 0x07]);
    p.expect(expected);
}
fn pointer(p: &mut Program, field: i32, target: i32) {
    p.path(11, target);
    p.path(7, field);
    p.code.extend([0x48, 0x8b, 0x07, 0x4c, 0x39, 0xd8]);
    checked(&mut p.code, &mut p.branches);
}

#[test]
fn native_shader_half_query_and_both_fusion_exports_retain_real_header_and_register_effects() {
    for (module, library, version) in [
        ("Graphics5", "Graphics5", 1),
        ("libSceAgc", "libSceAgc", 1),
        ("Graphics5", "Graphics5", 2),
        ("Graphics5Driver", "Graphics5Driver", 1),
    ] {
        let mut p = Program::new();
        for (field, target) in [
            (FRONT + 8, USER),
            (FRONT + 16, CODE),
            (FRONT + 32, FRONT_REGS),
            (FRONT + 40, FRONT_SPECIALS),
            (BACK + 32, BACK_REGS),
            (BACK + 40, BACK_SPECIALS),
        ] {
            assign(&mut p, field, target);
        }
        p.path(7, SIZE);
        p.path(6, FRONT);
        p.path(2, BACK);
        p.call(0);
        p.expect(0);
        value(&mut p, SIZE, 48, false);
        value(&mut p, SIZE + 8, 4, false);
        for modern in [true, false] {
            p.path(7, OUTPUT);
            p.path(6, FRONT);
            p.path(2, BACK);
            if modern {
                p.path(1, SCRATCH);
            } else {
                p.reg(1, 0);
            }
            p.call(if modern { 1 } else { 2 });
            p.expect(0);
            let registers = if modern { SCRATCH } else { BACK_REGS };
            pointer(&mut p, OUTPUT + 32, registers);
            if modern {
                value(&mut p, OUTPUT + 8, 0, false);
            } else {
                pointer(&mut p, OUTPUT + 8, USER);
            }
            for (field, expected) in [
                (4, 0x1122_3344),
                (12, 0x5566_7788),
                (20, 0x5100_0013),
                (28, if modern { 0x2a06_003f } else { 0xfa06_003f }),
            ] {
                value(&mut p, registers + field, expected, true);
            }
        }
        // A wrong pair returns the source error and preserves the old result.
        p.path(7, FRONT + 90);
        p.code.extend([0xc6, 0x07, 0]);
        p.path(7, SIZE);
        p.path(6, FRONT);
        p.path(2, BACK);
        p.call(0);
        p.expect(0x8a6c_0008);
        value(&mut p, SIZE, 48, false);
        let (code, calls) = p.finish(None);
        let original = scoped_system_image(
            &code,
            &["dolOmWH+huQ", "fd5Bp5tGTgo", "nApJjpKNBl4"],
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
        bytes[FRONT as usize + 90] = 4;
        bytes[BACK as usize + 90] = 6;
        bytes[FRONT as usize + 92] = 6;
        bytes[BACK as usize + 92] = 6;
        for (start, values) in [
            (
                FRONT_REGS,
                [0x1122_3344_u32, 0x5566_7788, 0xa000_0013, 0xf805_003e, 0, 0],
            ),
            (
                BACK_REGS,
                [
                    0x1111_2222,
                    0x3333_4444,
                    0x5100_0007,
                    0x1a02_0001,
                    0,
                    0xa5a5_a500,
                ],
            ),
        ] {
            for (i, (offset, value)) in [0x80_u32, 0x80, 0x8a, 0x8b, 0xc8, 0xc9]
                .into_iter()
                .zip(values)
                .enumerate()
            {
                put(&mut bytes, start as usize + i * 8, &offset.to_le_bytes());
                put(&mut bytes, start as usize + i * 8 + 4, &value.to_le_bytes());
            }
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
