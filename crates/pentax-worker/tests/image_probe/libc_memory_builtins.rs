// SPDX-License-Identifier: GPL-2.0-only
//! Original library bodies and callers compare native and checked ABI routes.
use super::*;
use pentax_core::process::hle::KnownFunction;

const ENTRIES: [i32; 3] = [0x100, 0x500, 0x580];
const SOURCE: i32 = 0x4000;
const DEST: i32 = 0x4040;

fn short_branch(code: &mut [u8], displacement: usize, target: usize) {
    code[displacement] = i8::try_from(target as isize - displacement as isize - 1).unwrap() as u8;
}

fn copy_body(move_overlap: bool) -> Vec<u8> {
    let mut code = vec![0x48, 0x89, 0xf8, 0x48, 0x85, 0xd2, 0x74, 0]; // RAX=dst; zero length.
    if move_overlap {
        code.extend([0x48, 0x39, 0xf7, 0x76, 0]); // dst<=src selects forward.
        let forward = code.len() - 1;
        code.extend([0x48, 0x8d, 0x74, 0x16, 0xff, 0x48, 0x8d, 0x7c, 0x17, 0xff]);
        let start = code.len();
        code.extend([
            0x8a, 0x0e, 0x88, 0x0f, 0x48, 0xff, 0xce, 0x48, 0xff, 0xcf, 0x48, 0xff, 0xca, 0x75, 0,
        ]);
        let repeat = code.len() - 1;
        short_branch(&mut code, repeat, start);
        code.push(0xc3);
        let target = code.len();
        short_branch(&mut code, forward, target);
    }
    let start = code.len();
    code.extend([
        0x8a, 0x0e, 0x88, 0x0f, 0x48, 0xff, 0xc6, 0x48, 0xff, 0xc7, 0x48, 0xff, 0xca, 0x75, 0,
    ]);
    let repeat = code.len() - 1;
    short_branch(&mut code, repeat, start);
    let done = code.len();
    short_branch(&mut code, 7, done);
    code.push(0xc3);
    code
}

fn set_body() -> Vec<u8> {
    let mut code = vec![0x48, 0x89, 0xf8, 0x48, 0x85, 0xd2, 0x74, 0];
    let start = code.len();
    code.extend([
        0x40, 0x88, 0x37, 0x48, 0xff, 0xc7, 0x48, 0xff, 0xca, 0x75, 0,
    ]);
    let repeat = code.len() - 1;
    short_branch(&mut code, repeat, start);
    let done = code.len();
    short_branch(&mut code, 7, done);
    code.push(0xc3);
    code
}

fn call(p: &mut Program, index: usize) {
    let next = 0x800 + p.code.len() as i32 + 5;
    p.code.push(0xe8);
    p.code.extend((ENTRIES[index] - next).to_le_bytes());
}

fn expect_pointer(p: &mut Program, target: i32) {
    p.path(11, target);
    p.code.extend([0x4c, 0x39, 0xd8]);
    checked(&mut p.code, &mut p.branches);
}

fn expect_word(p: &mut Program, at: i32, value: u64) {
    p.path(0, at);
    p.code.extend([0x48, 0x8b, 0x00]);
    p.expect(value);
}

fn original_library(
    outer_memcpy: bool,
    module_name: &str,
    library_name: &str,
    large_fill: bool,
) -> Input {
    let mut p = Program::new();
    p.stack(0x89, 12, 0x78); // Preserve the initializer's own incoming R12.
    p.reg(12, 0x1a2b_3c4d_5566_7788);
    if large_fill {
        // One byte above the optional helper's default span allowance. The
        // original REP body must finish this call before later small helpers.
        p.path(7, 0x8000);
        p.reg(6, 0x17e);
        p.reg(2, 65_537);
        call(&mut p, 2);
        expect_pointer(&mut p, 0x8000);
        expect_word(&mut p, 0x8000, 0x7e7e_7e7e_7e7e_7e7e);
        expect_word(&mut p, 0x8000 + 65_537 - 8, 0x7e7e_7e7e_7e7e_7e7e);
    }
    p.path(7, DEST);
    p.path(6, SOURCE);
    p.reg(2, 32);
    call(&mut p, 0);
    expect_pointer(&mut p, DEST);
    expect_word(&mut p, DEST, 0x0706_0504_0302_0100);
    expect_word(&mut p, DEST + 24, 0x1f1e_1d1c_1b1a_1918);
    p.path(7, DEST + 8);
    p.path(6, DEST);
    p.reg(2, 16);
    call(&mut p, 1);
    expect_pointer(&mut p, DEST + 8);
    expect_word(&mut p, DEST + 8, 0x0706_0504_0302_0100);
    expect_word(&mut p, DEST + 16, 0x0f0e_0d0c_0b0a_0908);
    p.path(7, DEST + 32);
    p.reg(6, 0x16d); // Standard memset consumes only the low byte.
    p.reg(2, 16);
    call(&mut p, 2);
    expect_pointer(&mut p, DEST + 32);
    expect_word(&mut p, DEST + 32, 0x6d6d_6d6d_6d6d_6d6d);
    p.code.extend([0x4c, 0x89, 0xe0]); // Callee-saved R12 survived every route.
    p.expect(0x1a2b_3c4d_5566_7788);
    p.stack(0x8b, 12, 0x78);
    // Restore this initializer's frame and tail-jump to a zero-length memcpy.
    // Its RET must complete the actual outer initializer call, not a syscall.
    for reg in [7, 6, 2] {
        p.reg(reg, 0);
    }
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xe9]);
    let next = 0x800 + p.code.len() as i32 + 4;
    p.code.extend((ENTRIES[0] - next).to_le_bytes());
    finish_checks(&mut p.code, p.branches);
    let code = p.code;
    assert!(code.len() < 0x500);
    let functions = [
        KnownFunction::Memcpy,
        KnownFunction::Memmove,
        KnownFunction::Memset,
    ];
    let names = functions.map(|function| std::str::from_utf8(function.nid()).unwrap());
    let original = scoped_system_image(&code, &names, module_name, library_name, 1);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x5000, 0);
    // Replace imports with GLOBAL/PROTECTED function definitions. There are no
    // relocation consumers competing with the generated provider's exports.
    let fill = if large_fill {
        // R8 preserves the original destination while AL/RCX/RDI serve STOSB.
        vec![
            0x49, 0x89, 0xf8, 0x89, 0xf0, 0x48, 0x89, 0xd1, 0xf3, 0xaa, 0x4c, 0x89, 0xc0, 0xc3,
        ]
    } else {
        set_body()
    };
    let bodies = [copy_body(false), copy_body(true), fill];
    for (index, body) in bodies.iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        bytes[symbol + 5] = 3;
        put(&mut bytes, symbol + 6, &1_u16.to_le_bytes());
        put(
            &mut bytes,
            symbol + 8,
            &(ENTRIES[index] as u64).to_le_bytes(),
        );
        put(&mut bytes, symbol + 16, &(body.len() as u64).to_le_bytes());
        put(&mut bytes, ENTRIES[index] as usize, body);
    }
    put(&mut bytes, 0x200 + 5 * 16, &0x6100_000d_u64.to_le_bytes());
    put(&mut bytes, 0x200 + 6 * 16, &0x6100_0013_u64.to_le_bytes());
    put(&mut bytes, 0x208 + 8 * 16, &0_u64.to_le_bytes()); // Empty DT_RELASZ.
    put(&mut bytes, 0x200 + 10 * 16, &12_u64.to_le_bytes()); // DT_INIT.
    put(
        &mut bytes,
        0x208 + 10 * 16,
        &(if outer_memcpy {
            ENTRIES[0] as u64
        } else {
            0x800
        })
        .to_le_bytes(),
    );
    put(&mut bytes, 0x200 + 11 * 16, &[0; 16]);
    put(&mut bytes, 152, &(12_u64 * 16).to_le_bytes());
    put(&mut bytes, 160, &(12_u64 * 16).to_le_bytes());
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (offset, value) in [
        (8, 0x4000_u64),
        (16, 0x4000),
        (32, 0x1000),
        (40, if large_fill { 0x20000 } else { 0x4000 }),
        (48, 0x4000),
    ] {
        put(&mut bytes, 176 + offset, &value.to_le_bytes());
    }
    for index in 0..32 {
        bytes[SOURCE as usize + index] = index as u8;
    }
    create_input(bytes)
}

#[test]
fn checked_libc_memory_builtins_preserve_original_callers_and_real_outer_returns() {
    compare_profiles("LibcInternal", "LibcInternal", false);
}

#[test]
fn checked_standard_libc_pair_uses_real_helpers_and_preserves_original_callers_and_outer_returns() {
    compare_profiles("libc", "libc", false);
}

#[test]
fn checked_libc_memory_builtins_leave_oversized_calls_native_without_consuming_helper_budget() {
    compare_profiles("libc", "libc", true);
}

fn compare_profiles(module_name: &str, library_name: &str, large_fill: bool) {
    for outer_memcpy in [false, true] {
        let main = module_input(&[0xb8, 42, 0, 0, 0, 0xc3], false, None);
        let library = original_library(outer_memcpy, module_name, library_name, large_fill);
        let mut ordinary = None;
        for profile in ["off", "checked"] {
            let (status, fields, text) = probe_arguments(&[
                "--image".into(),
                main.path.as_os_str().to_owned(),
                "--preload-libc".into(),
                library.path.as_os_str().to_owned(),
                "--libc-memory-builtins".into(),
                profile.into(),
            ]);
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42", "{text}");
            assert_eq!(fields["initializer_calls"], "1", "{text}");
            assert_eq!(
                fields["libc_builtin_bound_entries"],
                if profile == "checked" { "3" } else { "0" },
                "{text}"
            );
            for (index, name) in ["memcpy", "memmove", "memset"].iter().enumerate() {
                let expected = if profile == "off" || (outer_memcpy && index != 0) {
                    "0"
                } else if !outer_memcpy && index == 0 {
                    "2"
                } else {
                    "1"
                };
                assert_eq!(
                    fields[&format!("libc_builtin_attempted_{name}")],
                    expected,
                    "{text}"
                );
                assert_eq!(
                    fields[&format!("libc_builtin_completed_{name}")],
                    expected,
                    "{text}"
                );
            }
            let counts = (
                fields["block_attempts"].parse::<u64>().unwrap(),
                fields["native_instructions"].parse::<u64>().unwrap(),
            );
            if profile == "off" {
                ordinary = Some(counts);
            } else {
                let ordinary = ordinary.unwrap();
                assert!(counts.0 < ordinary.0 && counts.1 < ordinary.1, "{text}");
            }
        }
    }
}
