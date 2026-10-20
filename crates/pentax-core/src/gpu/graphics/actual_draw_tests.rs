// SPDX-License-Identifier: GPL-2.0-only
//! Original guest PM4 and AMD shaders exercising real Vulkan LOAD/writeback.
//! No imported guest data, host shader source or synthesized readback pixels.

use crate::gpu::graphics::{TargetLayout, execute_draw};
use pentax_gpu::{
    compute::CommandRange,
    graphics::{CommandLimits, GraphicsState, Limits, plan_commands},
};
use pentax_platform::{
    native_memory::{NativeAddressSpace, NativePermissions, NativeRange, PausedNativeMemory},
    vulkan::GpuLimits,
};

const WIDTH: usize = 64;
const HEIGHT: usize = 64;
const TARGET_BYTES: usize = WIDTH * HEIGHT * 4;
const SENTINEL: [u8; 4] = [17, 31, 47, 63];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];

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

fn vertex_shader() -> Vec<u32> {
    // VertexIndex is the source ABI's v5. The three positions are:
    // (-.5 + s0, -.5), (.5 + s0, -.5), (-.5 + s0, .5).
    // Inline float operands240/241 mean +.5/-.5. Source SGPR0 is a real
    // per-draw horizontal offset, carried by the vertex push-data manifest.
    let mut words = vec![
        binary(0x1a, 0, 129, 5), // V_LSHLREV_B32 v0, 1, v5.
        binary(0x1b, 0, 130, 0), // V_AND_B32 v0, 2, v0.
        binary(0x1b, 1, 130, 5), // V_AND_B32 v1, 2, v5.
        unary(6, 0, 256),        // V_CVT_F32_U32 v0, v0.
        unary(6, 1, 257),        // V_CVT_F32_U32 v1, v1.
        binary(8, 0, 240, 0),    // V_MUL_F32 v0, .5, v0.
        binary(8, 1, 240, 1),
        binary(3, 0, 241, 0), // V_ADD_F32 v0, -.5, v0.
        binary(3, 1, 241, 1),
        binary(3, 0, 0, 0), // V_ADD_F32 v0, s0, v0.
        unary(1, 2, 128),   // z=0.
        unary(1, 3, 242),   // w=1.
    ];
    words.extend(export(12, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}

fn pixel_shader() -> Vec<u32> {
    // Actual PS user SGPRs0..2 supply RGB; alpha is one. Neither the compiler
    // nor this test supplies a host-language shader or hard-coded GPU result.
    let mut words = vec![
        unary(1, 0, 0),
        unary(1, 1, 1),
        unary(1, 2, 2),
        unary(1, 3, 242),
    ];
    words.extend(export(0, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}

fn set(words: &mut Vec<u32>, opcode: u32, register: u32, values: &[u32]) {
    assert!(!values.is_empty() && values.len() < 16384);
    words.push(0xc000_0000 | ((values.len() as u32) << 16) | (opcode << 8));
    words.push(register);
    words.extend_from_slice(values);
}

fn commands(vertex: u64, pixel: u64, target: u64) -> Vec<u32> {
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
        (0x318, (target >> 8) as u32),
        (0x31c, (10 << 2) | (1 << 15)),
        (0x390, (target >> 40) as u32),
        (0x3b0, (63 << 14) | 63),
        (0x3b8, 1 << 24),
    ] {
        set(&mut words, 0x69, register, &[value]);
    }
    for (left, right, origin) in [
        (0x0c, 0x0d, 0),
        (0x81, 0x82, 0x8000_0000),
        (0x90, 0x91, 0x8000_0000),
        (0x94, 0x95, 0x8000_0000),
    ] {
        set(&mut words, 0x69, left, &[origin]);
        set(&mut words, 0x69, right, &[64 | (64 << 16)]);
    }
    set(
        &mut words,
        0x69,
        0x10f,
        &[
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            32_f32.to_bits(),
            1_f32.to_bits(),
            0,
        ],
    );
    set(
        &mut words,
        0x76,
        0xc8,
        &[(vertex >> 8) as u32, (vertex >> 40) as u32],
    );
    set(&mut words, 0x76, 0x88, &[0, 0, 0x602c_0000, 0x0003_0002, 0]);
    set(
        &mut words,
        0x76,
        0x08,
        &[
            (pixel >> 8) as u32,
            (pixel >> 40) as u32,
            0x002c_0000,
            6,
            1_f32.to_bits(),
            0,
            0,
        ],
    );
    set(&mut words, 0x79, 0x242, &[4]);
    set(&mut words, 0x79, 0x24a, &[0]);
    words.extend([0xc000_2f00, 1, 0xc001_2d00, 3, 2]);
    // Shift the second partial triangle right by16 pixels, and change the
    // actual PS RGB registers from red to green. Some first-draw pixels remain
    // outside this second triangle, proving LOAD observes prior guest writeback.
    set(&mut words, 0x76, 0x8c, &[0.5_f32.to_bits()]);
    set(&mut words, 0x76, 0x0c, &[0, 1_f32.to_bits(), 0]);
    words.extend([0xc001_2d00, 3, 2]);
    words
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn read(memory: &PausedNativeMemory<'_>, address: u64, size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    memory.read(address, &mut bytes).unwrap();
    bytes
}

fn pixel(image: &[u8], x: usize, y: usize) -> &[u8] {
    let offset = (y * WIDTH + x) * 4;
    &image[offset..offset + 4]
}

#[test]
#[ignore = "requires a local Vulkan device"]
fn actual_guest_draw_64x64_load_and_writeback_preserve_prior_pixels_and_live_stage_inputs() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let identity = memory.owner_id();
    let command_address = base + 0x1000;
    let vertex_address = base + 0x4000;
    let pixel_address = base + 0x5000;
    let target = base + 0x8000;
    memory
        .write(vertex_address, &bytes(&vertex_shader()))
        .unwrap();
    memory
        .write(pixel_address, &bytes(&pixel_shader()))
        .unwrap();
    memory
        .write(target, &SENTINEL.repeat(WIDTH * HEIGHT))
        .unwrap();
    memory.write(target - 16, &[0x9b; 16]).unwrap();
    memory
        .write(target + TARGET_BYTES as u64, &[0xe7; 16])
        .unwrap();
    let encoded = bytes(&commands(vertex_address, pixel_address, target));
    assert!(encoded.len() < 0x3000);
    memory.write(command_address, &encoded).unwrap();
    let mut snapshot = vec![0; encoded.len()];
    memory.read_gpu(command_address, &mut snapshot).unwrap();
    let plan = plan_commands(
        &[&snapshot],
        GraphicsState::empty(),
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.draws().len(), 2);
    let command_ranges = [CommandRange {
        address: command_address,
        size: snapshot.len() as u64,
    }];
    let target_range = NativeRange {
        address: target,
        size: TARGET_BYTES as u64,
    };
    let limits = Limits::default();
    let gpu_limits = GpuLimits::default();

    let first = execute_draw(
        &mut memory,
        &plan.draws()[0],
        &limits,
        &gpu_limits,
        &command_ranges,
    )
    .unwrap();
    assert_eq!(first.written_bytes, TARGET_BYTES);
    assert!(first.completion.owner() == identity);
    assert!(first.completion.target() == target_range);
    assert_eq!(first.completion.layout(), TargetLayout::LinearRgba8);
    assert!(first.completion.submission() > 0);
    assert!(first.gpu.commands > 0 && first.gpu.shader_words > 0 && first.gpu.device_bytes > 0);
    let first_pixels = read(&memory, target, TARGET_BYTES);
    assert_eq!(pixel(&first_pixels, 24, 24), RED);
    assert_eq!(pixel(&first_pixels, 36, 20), RED);
    assert_eq!(pixel(&first_pixels, 52, 20), SENTINEL);
    assert_eq!(pixel(&first_pixels, 56, 56), SENTINEL);
    assert!(
        first_pixels
            .chunks_exact(4)
            .all(|value| value == SENTINEL || value == RED)
    );
    assert_eq!(read(&memory, target - 16, 16), [0x9b; 16]);
    assert_eq!(read(&memory, target + TARGET_BYTES as u64, 16), [0xe7; 16]);

    let second = execute_draw(
        &mut memory,
        &plan.draws()[1],
        &limits,
        &gpu_limits,
        &command_ranges,
    )
    .unwrap();
    assert_eq!(second.written_bytes, TARGET_BYTES);
    assert!(second.completion.owner() == identity);
    assert!(second.completion.target() == target_range);
    assert_eq!(second.completion.layout(), TargetLayout::LinearRgba8);
    assert!(second.completion.submission() > first.completion.submission());
    let second_pixels = read(&memory, target, TARGET_BYTES);
    assert_eq!(pixel(&second_pixels, 24, 24), RED); // First-only region survives LOAD.
    assert_eq!(pixel(&second_pixels, 36, 20), GREEN); // Overlap sees the new PS input.
    assert_eq!(pixel(&second_pixels, 52, 20), GREEN); // New VS offset changes coverage.
    assert_eq!(pixel(&second_pixels, 56, 56), SENTINEL); // Neither draw covers this pixel.
    assert!(
        second_pixels
            .chunks_exact(4)
            .all(|value| value == SENTINEL || value == RED || value == GREEN)
    );
    assert_eq!(read(&memory, target - 16, 16), [0x9b; 16]);
    assert_eq!(read(&memory, target + TARGET_BYTES as u64, 16), [0xe7; 16]);
}
