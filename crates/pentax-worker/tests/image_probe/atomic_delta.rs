// SPDX-License-Identifier: GPL-2.0-only

//! Original native sources: a new delta stops at use, then a later real module
//! can replace its exact marker without adding a guest handle or initializer.
use super::*;

fn load(code: &mut Vec<u8>, branches: &mut Vec<usize>, path: i32, handle: u8) {
    rip(code, 0x100, &[0x48, 0x8d, 0x3d], path);
    code.extend([
        0x31, 0xf6, 0x31, 0xd2, 0x31, 0xc9, 0x45, 0x31, 0xc0, 0x45, 0x31, 0xc9,
    ]);
    rip(code, 0x100, &[0xff, 0x15], 0xf00);
    code.extend([0x83, 0xf8, handle]);
    checked(code, branches);
}

fn main_image(call_before_real_module: bool) -> Input {
    let mut code = vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 32];
    let mut branches = Vec::new();
    load(&mut code, &mut branches, 0xd00, 2);
    if !call_before_real_module {
        // The intervening generated delta must not consume guest handle 3.
        load(&mut code, &mut branches, 0xd40, 3);
    }
    rip(&mut code, 0x100, &[0xff, 0x15], 0xf08);
    code.extend([0x48, 0x83, 0xc4, 32, 0x5d, 0xc3]);
    finish_checks(&mut code, branches);
    assert!(code.len() < 0x100);
    let mut bytes = parameter_bytes(&code, None);
    put(&mut bytes, 0x415, b"wzvqT4UqKX8"); // Public LoadStartModule.
    put(&mut bytes, 0xd00, b"/app0/original-delta.sprx\0");
    put(&mut bytes, 0xd40, b"/app0/original-real.sprx\0");
    put(&mut bytes, 0x480, b"AAAAAAAAAAA#D#C\0");
    put(&mut bytes, 0x218, &0x90_u64.to_le_bytes());
    put(&mut bytes, 0x238, &72_u64.to_le_bytes());
    put(&mut bytes, 0x630, &0x80_u32.to_le_bytes());
    bytes[0x634] = 0x12;
    put(&mut bytes, 0x698, &0xf08_u64.to_le_bytes());
    put(&mut bytes, 0x6a0, &((2_u64 << 32) | 7).to_le_bytes());
    put(&mut bytes, 0x288, &48_u64.to_le_bytes());
    create_input(bytes)
}

fn library(imports_new_function: bool) -> Input {
    let mut bytes = parameter_bytes(&[0xb8, 7, 0, 0, 0, 0xc3], None);
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    bytes[0x200..0x300].fill(0);
    bytes[0x600..0x680].fill(0);
    let mut names = b"\0libkernel\0libkernel\0".to_vec();
    let export_name = names.len() as u32;
    names.extend_from_slice(if imports_new_function {
        b"AAAAAAAAAAA#D#C\0"
    } else {
        b"BBBBBBBBBBA#D#C\0"
    });
    let import_name = names.len() as u32;
    if imports_new_function {
        names.extend_from_slice(b"BBBBBBBBBBA#D#C\0");
    }
    put(&mut bytes, 0x400, &names);
    put(&mut bytes, 0x618, &export_name.to_le_bytes());
    bytes[0x61c] = if imports_new_function { 0x22 } else { 0x12 };
    bytes[0x61d] = 3;
    put(&mut bytes, 0x61e, &1_u16.to_le_bytes());
    put(&mut bytes, 0x620, &0x300_u64.to_le_bytes());
    let mut function = Vec::new();
    if imports_new_function {
        function.push(0x55);
        rip(&mut function, 0x300, &[0xff, 0x15], 0xf00);
        function.extend([0x5d, 0xc3]);
        put(&mut bytes, 0x630, &import_name.to_le_bytes());
        bytes[0x634] = 0x12;
        put(&mut bytes, 0x680, &0xf00_u64.to_le_bytes());
        put(&mut bytes, 0x688, &((2_u64 << 32) | 7).to_le_bytes());
    } else {
        function.extend([0xb8, 42, 0, 0, 0, 0xc3]);
    }
    put(&mut bytes, 0x628, &(function.len() as u64).to_le_bytes());
    put(&mut bytes, 0x300, &function);
    let module = 1_u64 | (1 << 32) | (1 << 40) | (2 << 48);
    let library = 11_u64 | (1 << 32) | (3 << 48);
    let mut entries = vec![
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, if imports_new_function { 72 } else { 48 }),
        (11, 24),
        (pentax_linker::DT_SCE_MODULE_INFO, module),
        (pentax_linker::DT_SCE_EXPORT_LIB, library),
        (7, 0x680),
        (8, if imports_new_function { 24 } else { 0 }),
        (9, 24),
    ];
    if imports_new_function {
        // Separate owned import declarations retain exact byte/version scope.
        entries.extend([
            (pentax_linker::DT_SCE_NEEDED_MODULE, module),
            (pentax_linker::DT_SCE_IMPORT_LIB, library),
        ]);
    }
    entries.extend([(12, 0x100), (0, 0)]);
    assert!(0x200 + entries.len() * 16 <= 0x300);
    for at in [152, 160] {
        put(&mut bytes, at, &(entries.len() as u64 * 16).to_le_bytes());
    }
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}

#[test]
fn original_atomic_delta_stops_at_native_use_then_later_real_module_retires_its_marker() {
    for call_before_real_module in [true, false] {
        let main = main_image(call_before_real_module);
        let first = library(true);
        let later = library(false);
        let (status, fields, text) = probe_dynamic(
            &main,
            &[
                ("/app0/original-delta.sprx", &first),
                ("/app0/original-real.sprx", &later),
            ],
        );
        assert_eq!(fields["unavailable_functions"], "1", "{text}");
        assert_eq!(fields["service_stops"], "2", "{text}");
        if call_before_real_module {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "unavailable-function", "{text}");
            assert_eq!(fields["initializer_calls"], "1", "{text}");
            assert_eq!(fields["reference_match"], "unlisted-reference", "{text}");
            assert!(!fields.contains_key("value"), "{text}");
            assert!(!fields.contains_key("public_function"), "{text}");
            assert!(!fields.contains_key("reference_function"), "{text}");
            assert!(!text.contains("BBBBBBBBBBA"));
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42", "{text}");
            assert_eq!(fields["initializer_calls"], "2", "{text}");
        }
    }
}
