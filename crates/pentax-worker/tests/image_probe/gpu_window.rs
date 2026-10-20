// SPDX-License-Identifier: GPL-2.0-only
//! Original AMD draw and tagged R_FLIP through the real worker-owned window.
//! No commercial inputs, CPU SubmitFlip, substitute shaders or fake receipts.

use super::*;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;
const PADDED_BYTES: u64 = 1920 * 1152 * 4;
const STAGING: i32 = PADDED_BYTES as i32 + 0x4000;
const COMMANDS: i32 = STAGING + 0x200;
const ATTRIBUTE: i32 = 0x1e00;
const TEMPLATE: i32 = 0x2000;
const BASELINE: u64 = 0x3f2f_1f11_3f2f_1f11;
const GUARD: u64 = 0x9b67_d1e5_c38a_f024;
const NIDS: [&str; 8] = [
    "Up36PTk687E", // VideoOutOpen
    "rKBUtgRrtbk", // VideoOutRegisterBuffers2
    "b4fpgH5ZXxQ", // Graphics5Driver CommandBuffer
    "SbU3dwp80lQ", // VideoOutGetFlipStatus
    "zgXifHT9ErY", // VideoOutIsFlipPending
    "uquVH4-Du78", // VideoOutClose
    "IWIBBdTHit4", // KernelMapFlexibleMemory
    "QcteRwbsnV0", // KernelUsleep: real cooperative timed park
];

fn call(p: &mut Program, slot: i32) {
    rip(&mut p.code, 0x800, &[0xff, 0x15], 0x1f00 + slot * 8);
    p.calls += 1;
}
fn fail_if(p: &mut Program, condition: u8) {
    p.code.extend([0x0f, condition, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}
fn address(p: &mut Program, register: u8, offset: i32) {
    // MOV register,R12; ADD register,signed immediate32.
    p.code.extend([
        0x4c | u8::from(register >= 8),
        0x89,
        0xe0 | (register & 7),
        0x48 | u8::from(register >= 8),
        0x81,
        0xc0 | (register & 7),
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn store(p: &mut Program, offset: i32, wide: bool) {
    address(p, 11, offset);
    p.code.extend([if wide { 0x49 } else { 0x41 }, 0x89, 0x03]);
}
fn read(p: &mut Program, offset: i32, wide: bool, expected: u64) {
    address(p, 11, offset);
    p.code.extend([if wide { 0x49 } else { 0x41 }, 0x8b, 0x03]);
    p.expect(expected);
}

fn unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e00_0000 | (dst << 17) | (op << 9) | src
}
fn binary(op: u32, dst: u32, src: u32, vector: u32) -> u32 {
    (op << 25) | (dst << 17) | (vector << 9) | src
}
fn export(target: u32, registers: [u32; 4]) -> [u32; 2] {
    [
        0xf800_080f | (target << 4),
        registers[0] | (registers[1] << 8) | (registers[2] << 16) | (registers[3] << 24),
    ]
}
fn shaders() -> (Vec<u32>, Vec<u32>) {
    // Original native AMD instructions consume VertexIndex(v5), constructing
    // (-.5,-.5),(.5,-.5),(-.5,.5); s0 is an actual zero VS user argument.
    let mut vertex = vec![
        binary(0x1a, 0, 129, 5),
        binary(0x1b, 0, 130, 0),
        binary(0x1b, 1, 130, 5),
        unary(6, 0, 256),
        unary(6, 1, 257),
        binary(8, 0, 240, 0),
        binary(8, 1, 240, 1),
        binary(3, 0, 241, 0),
        binary(3, 1, 241, 1),
        binary(3, 0, 0, 0),
        unary(1, 2, 128),
        unary(1, 3, 242),
    ];
    vertex.extend(export(12, [0, 1, 2, 3]));
    vertex.push(0xbf81_0000);
    // Red comes from guest PS user SGPR0..2; alpha is the AMD inline one.
    let mut pixel = vec![
        unary(1, 0, 0),
        unary(1, 1, 1),
        unary(1, 2, 2),
        unary(1, 3, 242),
    ];
    pixel.extend(export(0, [0, 1, 2, 3]));
    pixel.push(0xbf81_0000);
    (vertex, pixel)
}
fn set(words: &mut Vec<u32>, opcode: u32, register: u32, values: &[u32]) -> usize {
    assert!(!values.is_empty() && values.len() < 16384);
    words.extend([
        0xc000_0000 | ((values.len() as u32) << 16) | (opcode << 8),
        register,
    ]);
    let first = words.len();
    words.extend_from_slice(values);
    first
}
fn commands() -> (Vec<u32>, [(usize, i32, u8); 6]) {
    let mut words = Vec::new();
    for register in [
        0x80, 0x1b1, 0x1b3, 0x1b4, 0x1b8, 0x1c4, 0x1e0, 0x200, 0x203, 0x205, 0x207, 0x293, 0x2f8,
        0x31b, 0x31d, 0x31e, 0x31f, 0x321, 0x325, 0x398, 0x3a0, 0x3a8,
    ] {
        set(&mut words, 0x69, register, &[0]);
    }
    for (register, value) in [
        (0x8e, 15),
        (0x8f, 15),
        (0x1b6, 0x8000),
        (0x1c3, 4),
        (0x1c5, 9),
        (0x202, 0x00cc_0010),
        (0x204, 1 << 19),
        (0x206, 0x3f),
        (0x292, 2),
        (0x2d5, 0x0040_0000),
        (0x31c, (10 << 2) | (1 << 15)),
        (0x3b0, ((WIDTH - 1) << 14) | (HEIGHT - 1)),
        (0x3b8, 0x0106_c000), // Declared one-level 32bpp RenderTarget64KB.
    ] {
        set(&mut words, 0x69, register, &[value]);
    }
    let target_low = set(&mut words, 0x69, 0x318, &[0]);
    let target_high = set(&mut words, 0x69, 0x390, &[0]);
    for (left, right, origin) in [
        (0x0c, 0x0d, 0),
        (0x81, 0x82, 0x8000_0000),
        (0x90, 0x91, 0x8000_0000),
        (0x94, 0x95, 0x8000_0000),
    ] {
        set(&mut words, 0x69, left, &[origin]);
        set(&mut words, 0x69, right, &[WIDTH | (HEIGHT << 16)]);
    }
    set(
        &mut words,
        0x69,
        0x10f,
        &[
            960_f32.to_bits(),
            960_f32.to_bits(),
            540_f32.to_bits(),
            540_f32.to_bits(),
            1_f32.to_bits(),
            0,
        ],
    );
    let vertex = set(&mut words, 0x76, 0xc8, &[0, 0]);
    set(&mut words, 0x76, 0x88, &[0, 0, 0x602c_0000, 0x0003_0002, 0]);
    let pixel = set(
        &mut words,
        0x76,
        0x08,
        &[0, 0, 0x002c_0000, 6, 1_f32.to_bits(), 0, 0],
    );
    set(&mut words, 0x79, 0x242, &[4]);
    set(&mut words, 0x79, 0x24a, &[0]);
    words.extend([0xc000_2f00, 1, 0xc001_2d00, 3, 2]);
    // This exact tagged source packet requires the genuine CURRENT fenced
    // tiled draw receipt. It is not a CPU flip or a numeric completion bypass.
    words.extend([0xc004_105c, 1, 0, 1, 7, 0]);
    let patches = [
        (target_low, 0, 8),
        (target_high, 0, 40),
        (vertex, STAGING, 8),
        (vertex + 1, STAGING, 40),
        (pixel, STAGING + 0x100, 8),
        (pixel + 1, STAGING + 0x100, 40),
    ];
    (words, patches)
}

fn original_gpu_window_image() -> Input {
    let (words, patches) = commands();
    let mut p = Program::new();
    p.set(16, 0);
    p.ptr(7, 16);
    p.reg(6, PADDED_BYTES + 0x30000);
    p.reg(2, 0x33); // Guest-requested CPU and GPU READ|WRITE.
    p.reg(1, 0);
    call(&mut p, 6);
    p.expect(0);
    p.load(12, 16);
    // Leave actual mapped prefix and suffix guards around a 64KiB-aligned target.
    p.code.extend([0x49, 0x81, 0xc4, 0xff, 0xff, 1, 0]);
    p.code.extend([0x49, 0x81, 0xe4, 0, 0, 0xff, 0xff]);
    address(&mut p, 7, 0);
    p.reg(0, BASELINE);
    p.reg(1, PADDED_BYTES / 8);
    p.code.extend([0xf3, 0x48, 0xab]); // Actual guest fills the entire padded LOAD.
    p.reg(0, GUARD);
    store(&mut p, -8, true);
    store(&mut p, PADDED_BYTES as i32, true);
    p.path(6, TEMPLATE);
    address(&mut p, 7, STAGING);
    p.reg(1, 0x1000);
    p.code.extend([0xf3, 0xa4]); // Original AMD/PM4 template to GPU-readable memory.
    for (index, offset, shift) in patches {
        address(&mut p, 0, offset);
        p.code.extend([0x48, 0xc1, 0xe8, shift]);
        store(&mut p, COMMANDS + index as i32 * 4, false);
    }
    p.reg(7, 255);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, 0);
    call(&mut p, 0);
    p.expect(1);
    p.set(0, 0); // Seventh and eighth source registration arguments.
    p.set(8, 0);
    address(&mut p, 0, 0);
    p.save(32);
    for offset in [40, 48, 56] {
        p.set(offset, 0);
    }
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 0);
    p.ptr(1, 32);
    p.reg(8, 1);
    p.path(9, ATTRIBUTE);
    call(&mut p, 1);
    p.expect(0);
    p.reg(7, 1);
    p.ptr(6, 0);
    call(&mut p, 3);
    p.expect(0);
    p.word(0, 0); // Registration alone cannot acknowledge a frame.

    address(&mut p, 0, COMMANDS);
    p.save(80);
    p.reg(0, words.len() as u64);
    p.save(88); // Descriptor: dword count plus zero flags/reserved.
    p.set(96, 0); // Context.queue at+4: actual graphics queue0.
    p.ptr(7, 96);
    p.ptr(6, 80);
    call(&mut p, 2);
    p.expect(0);

    // Independent literal tiled offsets (pitch1920,128x128/64KiB blocks):
    // (1080,338)=block38+0x6120 is well inside; (1560,338)=block42+0x6920
    // is outside. No production detiler or expected host triangle supplies data.
    read(&mut p, 2_515_232, false, 0xff00_00ff);
    read(&mut p, 2_779_424, false, BASELINE as u32 as u64);
    read(&mut p, 8_825_340, false, BASELINE as u32 as u64); // (1919,1079)
    read(&mut p, 7_870_208, false, BASELINE as u32 as u64); // Padding at(0,1080).
    read(&mut p, -8, true, GUARD);
    read(&mut p, PADDED_BYTES as i32, true, GUARD);

    p.reg(13, 0);
    let again = p.code.len();
    p.reg(7, 1);
    p.ptr(6, 0);
    call(&mut p, 3);
    p.expect(0);
    p.code.extend([0x49, 0xff, 0xc5]);
    p.load(0, 0);
    p.code.extend([0x48, 0x85, 0xc0, 0x0f, 0x85, 0, 0, 0, 0]);
    let done = p.code.len() - 4;
    p.code.extend([0x49, 0x81, 0xfd, 200, 0, 0, 0]);
    fail_if(&mut p, 0x83); // Finite 200-observation deadline, never timing success.
    p.reg(7, 5_000);
    call(&mut p, 7);
    p.expect(0);
    p.code.push(0xe9);
    let jump = p.code.len();
    p.code
        .extend((again as i32 - (jump as i32 + 4)).to_le_bytes());
    let end = p.code.len();
    put(
        &mut p.code,
        done,
        &(end as i32 - (done as i32 + 4)).to_le_bytes(),
    );
    p.expect(1); // Actual consumer completion, not only GPUReady or swap submit.
    p.word(24, 7);
    p.word(48, 0); // No GPU-pending or pending requests remain.
    p.word(56, 0);
    p.load(0, 40);
    p.load(10, 64);
    p.code.extend([0x4c, 0x39, 0xd0]);
    fail_if(&mut p, 0x82); // Real completion counter follows submit counter.
    p.reg(7, 1);
    call(&mut p, 4);
    p.expect(0);
    p.reg(7, 1);
    p.ptr(6, 0);
    call(&mut p, 3);
    p.expect(0);
    p.word(0, 1); // Another service boundary cannot replay the receipt.
    p.reg(7, 1);
    call(&mut p, 5);
    p.expect(0);
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert_eq!(p.calls, 10);
    assert!(0x800 + p.code.len() < ATTRIBUTE as usize);

    let mut input = scoped_system_image(&[], &NIDS, "VideoOut", "VideoOut", 1);
    input.bytes.resize(0x4000, 0);
    put(&mut input.bytes, 96, &0x4000_u64.to_le_bytes());
    put(&mut input.bytes, 0x800, &p.code);
    let mut names = b"\0VideoOut\0VideoOut\0".to_vec();
    let kernel_name = names.len() as u64;
    names.extend_from_slice(b"libkernel\0");
    let graphics_name = names.len() as u64;
    names.extend_from_slice(b"Graphics5Driver\0");
    for (index, nid) in NIDS.iter().enumerate() {
        put(
            &mut input.bytes,
            0x600 + (index + 1) * 24,
            &(names.len() as u32).to_le_bytes(),
        );
        let suffix = if index == 2 {
            "H#G"
        } else if index >= 6 {
            "F#E"
        } else {
            "D#C"
        };
        names.extend_from_slice(format!("{nid}#{suffix}\0").as_bytes());
        put(
            &mut input.bytes,
            0x700 + index * 24,
            &(0x1f00_u64 + index as u64 * 8).to_le_bytes(),
        );
    }
    assert!(names.len() < 0x200);
    input.bytes[0x400..0x600].fill(0);
    put(&mut input.bytes, 0x400, &names);
    put(&mut input.bytes, 0x218, &(names.len() as u64).to_le_bytes());
    for (at, tag, value) in [
        (
            0x2a0,
            0x6100_000f_i64,
            kernel_name | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x2b0, 0x6100_0015_i64, kernel_name | (1 << 32) | (5 << 48)),
        (
            0x2c0,
            0x6100_000f_i64,
            graphics_name | (1 << 32) | (1 << 40) | (6 << 48),
        ),
        (
            0x2d0,
            0x6100_0015_i64,
            graphics_name | (1 << 32) | (7 << 48),
        ),
        (0x2e0, 0, 0),
    ] {
        put(&mut input.bytes, at, &tag.to_le_bytes());
        put(&mut input.bytes, at + 8, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut input.bytes, at, &(15_u64 * 16).to_le_bytes());
    }
    let mut attribute = [0; 80];
    attribute[12..16].copy_from_slice(&WIDTH.to_le_bytes());
    attribute[16..20].copy_from_slice(&HEIGHT.to_le_bytes());
    attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000_u64.to_le_bytes());
    put(&mut input.bytes, ATTRIBUTE as usize, &attribute);
    let (vertex, pixel) = shaders();
    assert!(vertex.len() * 4 <= 0x100 && pixel.len() * 4 <= 0x100);
    assert!(0x200 + words.len() * 4 <= 0x1000);
    for (offset, stream) in [
        (0, vertex.as_slice()),
        (0x100, pixel.as_slice()),
        (0x200, words.as_slice()),
    ] {
        for (index, word) in stream.iter().enumerate() {
            put(
                &mut input.bytes,
                TEMPLATE as usize + offset + index * 4,
                &word.to_le_bytes(),
            );
        }
    }
    std::fs::write(&input.path, &input.bytes).unwrap();
    input
}

#[test]
#[ignore = "requires an actual Vulkan device, visible primary monitor and real presentation completion"]
fn original_native_gpu_draw_flip_is_presented_at_1080p_and_acknowledged_once() {
    let input = original_gpu_window_image();
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        input.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "startup".into(),
        "--present-window".into(),
        "yes".into(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["window_configured"], "1", "{text}");
    assert_eq!(fields["window_frames_presented"], "1", "{text}");
    assert_eq!(fields["window_frames_acknowledged"], "1", "{text}");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
    assert_eq!(
        fields["string_bytes"].parse::<u64>().unwrap(),
        PADDED_BYTES + 2 * 0x1000
    );
    let calls = fields["service_stops"].parse::<u64>().unwrap();
    assert!((9..=407).contains(&calls) && calls % 2 == 1, "{text}");
    println!("local_gpu_presenter_composed_available=true local_gpu_presenter_composed_frames=1");
}
