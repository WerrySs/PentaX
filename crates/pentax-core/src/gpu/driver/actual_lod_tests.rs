// SPDX-License-Identifier: GPL-2.0-only
//! Original real sampled feedback through Driver, GPU fences and owned reports.
//! No CPU-generated counters, replacement shader, synthetic ready flag or frame claim.
use super::*;
use pentax_gpu::lod_stats::{COUNTER_WORDS, REPORT_BYTES, reset_words};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

const WIDTH: usize = 64;
const HEIGHT: usize = 64;
const TARGET_BYTES: usize = WIDTH * HEIGHT * 4;
const TEXTURE_BYTES: usize = 1536;
const SENTINEL: [u8; 4] = [17, 31, 47, 63];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
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

fn set(words: &mut Vec<u32>, opcode: u32, register: u32, values: &[u32]) {
    assert!(!values.is_empty() && values.len() < 16384);
    words.push(0xc000_0000 | ((values.len() as u32) << 16) | (opcode << 8));
    words.push(register);
    words.extend_from_slice(values);
}

fn commands(vertex: u64, pixel: u64, target: u64) -> Vec<u32> {
    commands_with_layout(vertex, pixel, target, 1 << 24)
}
fn commands_with_layout(vertex: u64, pixel: u64, target: u64, layout: u32) -> Vec<u32> {
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
        (0x3b8, layout),
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

fn sampled_pixel() -> Vec<u32> {
    let mut words = vec![
        unary(1, 0, 255),
        0x3ec0_0000, // Actual coordinate x=.375, away from texel boundaries.
        unary(1, 1, 255),
        0x3f60_0000,     // Actual coordinate y=.875.
        unary(1, 2, 12), // Explicit LOD is an actual PS user SGPR12.
        0xf090_0f08,
        0x0040_0400, // IMAGE_SAMPLE_L v4..7,v0..2,T#s0,S#s8.
        0xbf8c_0000, // Original S_WAITCNT waits for the sample before export.
    ];
    words.extend(export(0, [4, 5, 6, 7]));
    words.push(0xbf81_0000);
    words
}
fn original_texture() -> Vec<u8> {
    // Physical mip1 first:2rows at256B, then mip0:4rows at256B.
    // Coordinates select mip0(1,3) or mip1(0,1). Other texels have different
    // colors, so dropping coordinates, reversing LOD, or reading padding fails.
    let mut source = vec![0xa5; TEXTURE_BYTES];
    for y in 0..2 {
        for x in 0..2 {
            let at = y * 256 + x * 4;
            source[at..at + 4].copy_from_slice(&[0, 255, 255, 255]);
        }
    }
    for y in 0..4 {
        for x in 0..4 {
            let at = 512 + y * 256 + x * 4;
            source[at..at + 4].copy_from_slice(&BLUE);
        }
    }
    source[256..260].copy_from_slice(&GREEN);
    source[1284..1288].copy_from_slice(&RED);
    source
}
fn sampled_commands(base: u64, texture: u64, target: u64, lod: f32) -> Vec<u32> {
    let mut words = commands(base + 0x4000, base + 0x5000, target);
    let first_draw = words.iter().position(|word| *word == 0xc001_2d00).unwrap();
    words.truncate(first_draw);
    set(&mut words, 0x76, 0x0b, &[26]); // Thirteen real PS user SGPRs, no fake defaults.
    // Original4x4 RGBA8UNORM T#, two physical mips, view0..1, linear,
    // MIN_LOD1 and counter17; no compression or sparse flags.
    set(
        &mut words,
        0x76,
        0x0c,
        &[
            (texture >> 8) as u32,
            0xc381_0000 | ((texture >> 40) as u32 & 0xff),
            0x0000_c000,
            0x9001_0fac,
            0,
            0x0200_0010,
            17,
            0,
        ],
    );
    // Clamp-to-edge XYZ, nearest texels/mips, effectiveLOD0..1, bias0.
    set(&mut words, 0x76, 0x14, &[0x92, 0x0010_0000, 0x0400_0000, 0]);
    set(&mut words, 0x76, 0x18, &[lod.to_bits()]);
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
fn counter_words(driver: &Driver, memory: &PausedNativeMemory<'_>) -> [u32; COUNTER_WORDS] {
    *driver
        .lod_stats
        .as_ref()
        .unwrap()
        .counters(memory.owner_id())
        .unwrap()
        .as_words()
}
fn current_report(driver: &Driver, memory: &PausedNativeMemory<'_>) -> Vec<u8> {
    driver
        .lod_stats
        .as_ref()
        .unwrap()
        .counters(memory.owner_id())
        .unwrap()
        .pack_report()
        .as_bytes()
        .to_vec()
}
fn submit(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    command: u64,
    encoded: &[u8],
) -> Result<u64, Failure> {
    memory.write(command, encoded).unwrap();
    memory.write(base + 0x304, &0u32.to_le_bytes()).unwrap(); // Selected graphics CP.
    let mut descriptor = [0u8; 16];
    descriptor[..8].copy_from_slice(&command.to_le_bytes());
    descriptor[8..12].copy_from_slice(&((encoded.len() / 4) as u32).to_le_bytes());
    memory.write(base + 0x380, &descriptor).unwrap();
    driver.submit(
        memory,
        Function::CommandBuffer,
        [base + 0x300, base + 0x380, 0, 0, 0, 0],
    )
}
fn report_packet(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    command: u64,
    output: u64,
    size: u32,
    reset: bool,
) -> Vec<u8> {
    let descriptor = base + 0x800;
    let stack = base + 0x900;
    for (offset, value) in [
        (0, command),
        (8, command + 64),
        (16, command),
        (24, command + 64),
        (32, 0),
        (40, 0),
    ] {
        memory
            .write(descriptor + offset, &value.to_le_bytes())
            .unwrap();
    }
    memory.write(descriptor + 48, &1u32.to_le_bytes()).unwrap();
    memory
        .write(stack + 8, &u64::from(reset).to_le_bytes())
        .unwrap();
    memory.write(stack + 16, &0u64.to_le_bytes()).unwrap();
    let result = driver
        .begin_command_builder(
            memory,
            Function::AgcDcbGetLodStats,
            [descriptor, 0, output, u64::from(size), 0, 0],
            Some(stack),
        )
        .unwrap();
    let CommandBuilderAction::Returned(pointer) = result else {
        panic!("fitting producer unexpectedly grew")
    };
    assert_eq!(pointer, command);
    assert_eq!(
        read(memory, descriptor + 16, 8),
        (command + 20).to_le_bytes()
    );
    let encoded = read(memory, command, 20);
    assert_eq!(
        encoded,
        bytes(&[
            0xc003_8e00,
            size,
            output as u32 & !63,
            (output >> 32) as u32,
            u32::from(reset) << 19
        ])
    );
    encoded
}
fn check_guards(memory: &PausedNativeMemory<'_>, texture: u64, target: u64, output: u64) {
    for (address, value) in [
        (texture - 16, 0x91),
        (texture + TEXTURE_BYTES as u64, 0xd7),
        (target - 16, 0x9b),
        (target + TARGET_BYTES as u64, 0xe7),
        (output - 16, 0x63),
        (output + REPORT_BYTES as u64, 0xc7),
    ] {
        assert_eq!(read(memory, address, 16), [value; 16]);
    }
}

#[test]
#[ignore = "requires actual Vulkan sampled-image feedback/reset; run exactly alone under an owning180-second gate"]
fn actual_driver_lod_feedback_clamps_samples_publishes_fenced_reports_and_resets_without_replay() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x80000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x40000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    // Two different VAs for the same real backing, reserved only for the final
    // report-versus-captured-command alias refusal. No fake alias predicate.
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: 0x4000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    for address in [base + 0x40000, base + 0x44000] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    let command = base + 0x1000;
    let texture = base + 0x8000;
    let target = base + 0x10000;
    let output = base + 0x30000;
    let shader = bytes(&sampled_pixel());
    let texture_bytes = original_texture();
    memory
        .write(base + 0x4000, &bytes(&vertex_shader()))
        .unwrap();
    memory.write(base + 0x5000, &shader).unwrap();
    memory.write(texture, &texture_bytes).unwrap();
    memory
        .write(target, &SENTINEL.repeat(WIDTH * HEIGHT))
        .unwrap();
    memory.write(output, &[0x6d; REPORT_BYTES]).unwrap();
    for (address, value) in [
        (texture - 16, 0x91),
        (texture + TEXTURE_BYTES as u64, 0xd7),
        (target - 16, 0x9b),
        (target + TARGET_BYTES as u64, 0xe7),
        (output - 16, 0x63),
        (output + REPORT_BYTES as u64, 0xc7),
    ] {
        memory.write(address, &[value; 16]).unwrap();
    }
    let mut driver = Driver::new(Limits::default());
    // First packet performs actual GPU fill/fence/readback but publishes no
    // guest report, even with a writable full-sized destination.
    let encoded = report_packet(
        &mut driver,
        &mut memory,
        base,
        command,
        output,
        REPORT_BYTES as u32,
        false,
    );
    let admitted_writes = driver.usage.memory_written_bytes; // CPU producer stores already charged.
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(read(&memory, output, REPORT_BYTES), [0x6d; REPORT_BYTES]);
    assert_eq!(counter_words(&driver, &memory), reset_words());
    assert_eq!(driver.usage.memory_written_bytes, admitted_writes);
    check_guards(&memory, texture, target, output);

    // Explicit requested LOD0 wants the red mip. Image MIN_LOD1 makes the real
    // GPU sample the green mip while its feedback records unclamped wanted0.
    let encoded = bytes(&sampled_commands(base, texture, target, 0.0));
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(driver.usage.draws, 1);
    let actual = counter_words(&driver, &memory);
    assert_eq!(actual[17], 0);
    assert!(actual[257 + 17] > 0); // Public units are subgroups, not pixel coverage.
    for index in 0..257 {
        if index != 17 {
            assert_eq!((actual[index], actual[257 + index]), (u32::MAX, 0));
        }
    }
    let pixels = read(&memory, target, TARGET_BYTES);
    for (x, y) in [(24, 24), (36, 20)] {
        let at = (y * WIDTH + x) * 4;
        assert_eq!(&pixels[at..at + 4], GREEN);
    }
    for (x, y) in [(52, 20), (56, 56), (0, 0)] {
        let at = (y * WIDTH + x) * 4;
        assert_eq!(&pixels[at..at + 4], SENTINEL);
    }
    assert!(pixels.chunks_exact(4).all(|p| p == GREEN || p == SENTINEL));
    let packed = current_report(&driver, &memory);
    let entry = u64::from_le_bytes(packed[64 + 17 * 8..64 + 18 * 8].try_into().unwrap());
    assert_eq!(entry >> 56, 0);
    assert_eq!((entry >> 24) & 0xffff_ffff, 17);
    assert_eq!(
        entry & 0x00ff_ffff,
        u64::from(actual[257 + 17].min(0x00ff_ffff))
    );
    assert_eq!(read(&memory, output, REPORT_BYTES), [0x6d; REPORT_BYTES]);

    // Initialized empty probe never touches this intentionally unbacked encoded
    // destination and preserves all514 words, including the unreported sink.
    let encoded = report_packet(&mut driver, &mut memory, base, command, u64::MAX, 0, false);
    let before_written = driver.usage.memory_written_bytes;
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(counter_words(&driver, &memory), actual);
    assert_eq!(driver.usage.memory_written_bytes, before_written);
    assert_eq!(read(&memory, output, REPORT_BYTES), [0x6d; REPORT_BYTES]);

    let encoded = report_packet(
        &mut driver,
        &mut memory,
        base,
        command,
        output,
        REPORT_BYTES as u32,
        false,
    );
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(read(&memory, output, REPORT_BYTES), packed);
    assert_eq!(counter_words(&driver, &memory), actual);
    // Partial completion writes exactly the admitted prefix, including a
    // non-dword tail, and leaves every following output byte untouched.
    memory.write(output, &[0x6d; REPORT_BYTES]).unwrap();
    let short = 64 + 17 * 8 + 7;
    let encoded = report_packet(
        &mut driver,
        &mut memory,
        base,
        command,
        output,
        short as u32,
        false,
    );
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(read(&memory, output, short), packed[..short]);
    assert_eq!(
        read(&memory, output + short as u64, REPORT_BYTES - short),
        vec![0x6d; REPORT_BYTES - short]
    );
    assert_eq!(counter_words(&driver, &memory), actual);

    // Report-and-reset publishes the pre-reset real snapshot. Only the actual
    // GPU reset readback replaces driver state; the following report consumes it.
    let encoded = report_packet(
        &mut driver,
        &mut memory,
        base,
        command,
        output,
        REPORT_BYTES as u32,
        true,
    );
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(read(&memory, output, REPORT_BYTES), packed);
    assert_eq!(counter_words(&driver, &memory), reset_words());
    let reset_report = current_report(&driver, &memory);
    assert_ne!(reset_report, packed);
    let encoded = report_packet(
        &mut driver,
        &mut memory,
        base,
        command,
        output,
        REPORT_BYTES as u32,
        false,
    );
    assert_eq!(
        submit(&mut driver, &mut memory, base, command, &encoded),
        Ok(0)
    );
    assert_eq!(read(&memory, output, REPORT_BYTES), reset_report);
    assert_eq!(counter_words(&driver, &memory), reset_words());
    assert_eq!(read(&memory, texture, TEXTURE_BYTES), texture_bytes);
    assert_eq!(read(&memory, base + 0x5000, shader.len()), shader);
    check_guards(&memory, texture, target, output);

    // A foreign paused owner cannot inherit the initialized counter state or
    // inspect the first owner's command/report pointers.
    let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let mut foreign_memory = foreign.pause().unwrap();
    assert_eq!(
        driver.submit(
            &mut foreign_memory,
            Function::CommandBuffer,
            [base + 0x300, base + 0x380, 0, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(counter_words(&driver, &memory), reset_words());

    // Real physical command alias fails before report/reset effects, after a
    // separate legitimate earlier WriteData prefix. Retrying cannot replay it.
    let prefix = base + 0x3c000;
    let aliased_command = base + 0x41000;
    let aliased_output = base + 0x45000;
    let report = report_packet(
        &mut driver,
        &mut memory,
        base,
        command,
        aliased_output,
        REPORT_BYTES as u32,
        false,
    );
    let mut stream = bytes(&[
        0xc003_3700,
        0x100,
        prefix as u32,
        (prefix >> 32) as u32,
        0x1234_5678,
    ]);
    stream.extend(report);
    memory.write(prefix, &0u32.to_le_bytes()).unwrap();
    assert_eq!(
        submit(&mut driver, &mut memory, base, aliased_command, &stream),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, prefix, 4), 0x1234_5678u32.to_le_bytes());
    assert_eq!(read(&memory, aliased_command, stream.len()), stream);
    assert_eq!(read(&memory, aliased_output, stream.len()), stream);
    assert_eq!(counter_words(&driver, &memory), reset_words());
    assert!(driver.poisoned);
    memory.write(prefix, &0x8765_4321u32.to_le_bytes()).unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base + 0x300, base + 0x380, 0, 0, 0, 0]
        ),
        Err(Failure::Poisoned)
    );
    assert_eq!(read(&memory, prefix, 4), 0x8765_4321u32.to_le_bytes());
    check_guards(&memory, texture, target, output);
    assert_eq!(read(&memory, target, TARGET_BYTES), pixels);
    println!(
        "agc_lod_feedback_gpu_available=true agc_lod_feedback_gpu_draws=1 agc_lod_feedback_gpu_reset_readbacks=2 agc_lod_feedback_gpu_real_reports=4 agc_lod_feedback_gpu_alias_refusals=1"
    );
}
