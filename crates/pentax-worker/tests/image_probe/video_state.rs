// SPDX-License-Identifier: GPL-2.0-only
//! Original VideoOut float-ABI and no-pixels display-clock consumers.
use super::*;

fn captured_gamma(p: &mut Program, bits: u32) {
    p.reg(0, u64::from(bits));
    p.code.extend([0x66, 0x0f, 0x6e, 0xc0]); // MOVD xmm0,eax: actual float argument.
    p.reg(6, 0xffff_ffff_0000_0000); // Poison the unrelated GPR argument slot.
}

#[test]
fn native_video_color_uses_actual_xmm0_and_exact_four_byte_source_updates() {
    for (bits, expected) in [
        (0x3dcc_cccd_u32, 0),
        (0x4000_0000, 0),
        (0x7fc0_1234, 0),
        (0xff80_1234, 0),
        (0x4000_0001, 0x8029_0001),
    ] {
        let mut p = Program::new();
        p.reg(7, 255);
        p.reg(6, 0);
        p.reg(2, 0);
        p.call(0);
        p.expect(1);
        p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
        p.save(16);
        p.save(24);
        captured_gamma(&mut p, bits);
        p.ptr(7, 16);
        p.call(1);
        p.expect(expected);
        let field = if expected == 0 {
            u64::from(bits) | 0xa5a5_a5a5_0000_0000
        } else {
            0xa5a5_a5a5_a5a5_a5a5
        };
        p.word(16, field);
        p.word(24, 0xa5a5_a5a5_a5a5_a5a5);
        p.reg(7, 1);
        p.ptr(6, 16);
        p.call(2);
        p.expect(0);
        p.reg(7, 1);
        p.call(3);
        p.expect(0);
        p.reg(7, 1);
        p.reg(6, 0);
        p.call(2);
        p.expect(0x8029_0002);
        p.reg(7, 1);
        p.ptr(6, 16);
        p.call(2);
        p.expect(0x8029_000b);
        let (code, calls) = p.finish(None);
        let source = scoped_system_image(
            &code,
            &["Up36PTk687E", "DYhhWbJSeRg", "pv9CI5VC+R0", "uquVH4-Du78"],
            "VideoOut",
            "VideoOut",
            1,
        );
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
    }
}

fn wait_image(code: &[u8]) -> Input {
    // First three imports are canonical VideoOut; last two are canonical
    // libkernel. Each symbol retains its actual distinct library/module scope.
    let source = scoped_system_image(
        code,
        &[
            "Up36PTk687E",
            "Xru92wHJRmg",
            "1FZBKy8HeNU",
            "D0OdFMjp46I",
            "fzyMKs9kim0",
        ],
        "VideoOut",
        "VideoOut",
        1,
    );
    let mut bytes = source.bytes.clone();
    let old = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    put(&mut bytes, 0x400 + old, b"libkernel\0libkernel\0");
    put(&mut bytes, 0x218, &((old + 20) as u64).to_le_bytes());
    for index in 3..5 {
        let symbol = 0x600 + (index + 1) * 24;
        let name = u32::from_le_bytes(bytes[symbol..symbol + 4].try_into().unwrap()) as usize;
        put(&mut bytes, 0x400 + name + 11, b"#F#E");
    }
    for (index, (tag, value)) in [
        (
            0x6100_000f_i64,
            old as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x6100_0015, (old as u64 + 10) | (1 << 32) | (5 << 48)),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + (10 + index) * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + (10 + index) * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(13_u64 * 16).to_le_bytes());
    }
    create_input(bytes)
}

#[test]
fn native_video_wait_equeue_receives_elapsed_vblank_before_any_pixels_or_window() {
    let mut p = Program::new();
    p.ptr(7, 8);
    p.path(6, NAME);
    p.call(3);
    p.expect(0); // Actual owned equeue.
    p.reg(7, 255);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(0);
    p.expect(1);
    p.load(7, 8);
    p.reg(6, 1);
    p.reg(2, 27);
    p.call(1);
    p.expect(0);
    // No query, flip, registered surface or presenter occurs before this wait.
    // Registration is inactive and this straight-line prefix cannot reach a
    // scheduler quantum; WaitEqueue must park before the first clock pump.
    p.load(7, 8);
    p.ptr(6, 64);
    p.reg(2, 1);
    p.ptr(1, 104);
    p.reg(8, 0);
    p.call(4);
    p.expect(0);
    p.word(64, 1);
    p.word(88, 27);
    p.load(0, 80);
    p.code.extend([0x48, 0xc1, 0xe8, 16, 0x48, 0x85, 0xc0]);
    p.code.extend([0x0f, 0x84, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4); // Actual payload count is positive.
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(8);
    p.save(56);
    p.reg(7, 1);
    p.ptr(6, 16);
    p.call(2);
    p.expect(0);
    p.word(8, 0xa5a5_a5a5_a5a5_a5a5);
    p.word(48, 0);
    p.word(56, 0xa5a5_a5a5_a5a5_a5a5);
    p.load(0, 16);
    p.code.extend([0x48, 0x85, 0xc0]);
    p.code.extend([0x0f, 0x84, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
    let (code, calls) = p.finish(None);
    let source = wait_image(&code);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
    assert_eq!(fields["scheduling_pending_waits"], "0", "{text}");
    assert_eq!(fields["scheduling_returned"], "0", "{text}");
    for field in [
        "graphics_draw_attempts",
        "graphics_dispatch_attempts",
        "video_registered_buffers",
        "video_flip_ready_with_frame",
        "video_flip_acknowledged",
    ] {
        assert_eq!(fields[field], "0", "{field}: {text}");
    }
    assert_eq!(fields["video_open_ports"], "1", "{text}");
    assert_eq!(fields["video_vblank_subscriptions"], "1", "{text}");
    assert_eq!(fields["initializer_calls"], "0", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
}

#[test]
fn native_video_gamma_readonly_destination_stops_without_success() {
    let mut p = Program::new();
    captured_gamma(&mut p, 0x3fc0_0000);
    p.path(7, 0x800);
    p.call(0);
    let (code, _) = p.finish(None);
    let source = scoped_system_image(&code, &["DYhhWbJSeRg"], "VideoOut", "VideoOut", 1);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service", "{text}");
    assert_eq!(fields["runtime_failure"], "graphics-memory", "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    assert_eq!(fields["service_stops"], "1", "{text}");
}
