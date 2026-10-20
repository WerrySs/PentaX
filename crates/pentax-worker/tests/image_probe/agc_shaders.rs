// SPDX-License-Identifier: GPL-2.0-only
//! Original native callers consume actual in-place AGC shader header effects.
use super::*;

const HEADER: i32 = 0x4100;
const USER: i32 = 0x4000;
const REGISTERS: i32 = 0x4300;
const CODE: i32 = 0x4800;

#[test]
fn long_startup_admits_shader_initialization_call_volume_with_compact_profile_unchanged() {
    let mut p = Program::new();
    p.reg(12, 257);
    let start = p.code.len();
    p.call(0);
    p.expect(0);
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]);
    let displacement = start as i32 - (p.code.len() as i32 + 4);
    p.code.extend(displacement.to_le_bytes());
    let (code, _) = p.finish(None);
    let original = scoped_system_image(&code, &["23LRUSvYu1M"], "Graphics5", "Graphics5", 1);
    for profile in ["startup", "long-startup"] {
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            original.path.as_os_str().to_owned(),
            "--execution-profile".into(),
            profile.into(),
        ]);
        assert_eq!(fields["service_stops"], "257", "{text}");
        if profile == "startup" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["runtime_failure"], "graphics-budget");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
        }
    }
}

fn compare(p: &mut Program) {
    p.code.extend([0x4c, 0x39, 0xd8]); // CMP RAX,R11
    checked(&mut p.code, &mut p.branches);
}
fn pointer(p: &mut Program, field: i32, target: i32) {
    p.path(0, target);
    p.path(7, field);
    p.code.extend([0x4c, 0x8b, 0x1f]);
    compare(p);
}
fn shader_image(
    code: &[u8],
    module: &str,
    library: &str,
    version: u64,
    bad_program: bool,
) -> Input {
    let original = scoped_system_image(code, &["f3dg2CSgRKY"], module, library, version);
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
    let h = HEADER as usize;
    put(&mut bytes, h, &0x3433_3231_u32.to_le_bytes());
    put(&mut bytes, h + 4, &0x18_u32.to_le_bytes());
    put(
        &mut bytes,
        h + 8,
        &(USER as u64).wrapping_sub(HEADER as u64 + 8).to_le_bytes(),
    );
    put(
        &mut bytes,
        h + 32,
        &(REGISTERS as u64 - HEADER as u64 - 32).to_le_bytes(),
    );
    put(&mut bytes, h + 64, &96_u32.to_le_bytes());
    put(&mut bytes, h + 68, &32_u32.to_le_bytes());
    bytes[h + 90] = 0;
    bytes[h + 92] = 2;
    put(
        &mut bytes,
        USER as usize,
        &(0x3f80_u64.wrapping_sub(USER as u64)).to_le_bytes(),
    );
    put(&mut bytes, REGISTERS as usize, &0x20c_u32.to_le_bytes());
    put(&mut bytes, REGISTERS as usize + 4, &2_u32.to_le_bytes());
    put(
        &mut bytes,
        REGISTERS as usize + 8,
        &(if bad_program { 0x555_u32 } else { 0x20d }).to_le_bytes(),
    );
    put(
        &mut bytes,
        REGISTERS as usize + 12,
        &0xa5a5_a500_u32.to_le_bytes(),
    );
    create_input(bytes)
}

#[test]
fn native_shader_creation_publishes_exact_source_relocations_program_addresses_and_error_effects() {
    for (module, library, version) in [
        ("Graphics5", "Graphics5", 1),
        ("libSceAgc", "libSceAgc", 1),
        ("Graphics5", "Graphics5", 2),
        ("Graphics5Driver", "Graphics5Driver", 1),
    ] {
        for case in 0..if version == 1 && module != "Graphics5Driver" {
            3
        } else {
            1
        } {
            let bad = case == 1;
            let mut p = Program::new();
            p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
            p.save(24);
            p.save(32);
            p.save(40);
            if case == 2 {
                p.path(7, HEADER + 16);
            } else {
                p.ptr(7, 32);
            }
            p.path(6, HEADER);
            p.path(2, CODE);
            p.call(0);
            p.expect(if bad { 0x8a6c_0005 } else { 0 });
            p.word(24, 0xa5a5_a5a5_a5a5_a5a5);
            p.word(40, 0xa5a5_a5a5_a5a5_a5a5);
            if case == 0 {
                p.path(0, HEADER);
                p.load(11, 32);
                compare(&mut p);
            } else {
                p.word(32, 0xa5a5_a5a5_a5a5_a5a5);
            }
            pointer(&mut p, HEADER + 8, USER);
            pointer(&mut p, HEADER + 16, if case == 2 { HEADER } else { CODE });
            pointer(&mut p, HEADER + 32, REGISTERS);
            pointer(&mut p, USER, 0x3f80);
            if !bad {
                p.path(0, CODE + 512);
                p.code.extend([0x48, 0xc1, 0xe8, 8, 0x89, 0xc0]);
                p.path(7, REGISTERS + 4);
                p.code.extend([0x44, 0x8b, 0x1f]);
                compare(&mut p);
                p.path(0, CODE + 512);
                p.code.extend([0x48, 0xc1, 0xe8, 40]);
                p.reg(10, 0xa5a5_a500);
                p.code.extend([0x4c, 0x09, 0xd0]);
                p.path(7, REGISTERS + 12);
                p.code.extend([0x44, 0x8b, 0x1f]);
                compare(&mut p);
            }
            let (code, calls) = p.finish(None);
            let original = shader_image(&code, module, library, version, bad);
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
}
