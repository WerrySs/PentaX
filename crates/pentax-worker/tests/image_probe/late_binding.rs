// SPDX-License-Identifier: GPL-2.0-only

//! Original native fixture: a real dynamically loaded export replaces a stop.
use super::*;

#[test]
fn original_late_global_or_weak_export_replaces_generated_stop_and_executes_real_function() {
    for binding in [0x12, 0x22] {
        let mut code = vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 32];
        rip(&mut code, 0x100, &[0x48, 0x8d, 0x3d], 0xd00);
        code.extend([
            0x31, 0xf6, 0x31, 0xd2, 0x31, 0xc9, 0x45, 0x31, 0xc0, 0x45, 0x31, 0xc9,
        ]);
        rip(&mut code, 0x100, &[0xff, 0x15], 0xf00);
        code.extend([0x83, 0xf8, 2]);
        let mut branches = Vec::new();
        checked(&mut code, &mut branches);
        rip(&mut code, 0x100, &[0xff, 0x15], 0xf08);
        code.extend([0x48, 0x83, 0xc4, 32, 0x5d, 0xc3]);
        finish_checks(&mut code, branches);
        let mut main = parameter_bytes(&code, None);
        put(&mut main, 0x415, b"wzvqT4UqKX8");
        put(&mut main, 0xd00, b"/app0/original-late.sprx\0");
        put(&mut main, 0x480, b"AAAAAAAAAAA#D#C\0");
        put(&mut main, 0x218, &0x90_u64.to_le_bytes());
        put(&mut main, 0x238, &72_u64.to_le_bytes());
        put(&mut main, 0x630, &0x80_u32.to_le_bytes());
        main[0x634] = 0x12;
        put(&mut main, 0x698, &0xf08_u64.to_le_bytes());
        put(&mut main, 0x6a0, &((2_u64 << 32) | 7).to_le_bytes());
        put(&mut main, 0x288, &48_u64.to_le_bytes());

        let mut library = parameter_bytes(&[0xb8, 7, 0, 0, 0, 0xc3], None);
        put(&mut library, 16, &3_u16.to_le_bytes());
        library[0x61c] = binding;
        put(&mut library, 0x415, b"AAAAAAAAAAA");
        put(
            &mut library,
            0x250,
            &pentax_linker::DT_SCE_MODULE_INFO.to_le_bytes(),
        );
        put(
            &mut library,
            0x260,
            &pentax_linker::DT_SCE_EXPORT_LIB.to_le_bytes(),
        );
        library[0x61d] = 3;
        put(&mut library, 0x61e, &1_u16.to_le_bytes());
        put(&mut library, 0x620, &0x300_u64.to_le_bytes());
        put(&mut library, 0x628, &6_u64.to_le_bytes());
        put(&mut library, 0x288, &0_u64.to_le_bytes());
        put(&mut library, 0x2a0, &12_i64.to_le_bytes());
        put(&mut library, 0x2a8, &0x100_u64.to_le_bytes());
        for at in [152, 160] {
            put(&mut library, at, &(12_u64 * 16).to_le_bytes());
        }
        put(&mut library, 0x300, &[0xb8, 42, 0, 0, 0, 0xc3]);
        let main = create_input(main);
        let library = create_input(library);
        let (code, fields, text) = probe_dynamic(&main, &[("/app0/original-late.sprx", &library)]);
        assert_eq!(code, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["initializer_calls"], "1");
        assert_eq!(fields["unavailable_functions"], "1");
        assert_eq!(fields["service_stops"], "1");
    }
}
