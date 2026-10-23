// SPDX-License-Identifier: GPL-2.0-only
//! Original native AMD MIMG shader and current reverse-linear texture storage.
//! Only real Vulkan draw/fence/readback supplies pixels; no LOD feedback claim.
use super::*;

const TEXTURE_BYTES: usize = 1536;
const BLUE: [u8; 4] = [0, 0, 255, 255];
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
    // no MIN_LOD, compression, sparse flags or mip-statistics counter.
    set(
        &mut words,
        0x76,
        0x0c,
        &[
            (texture >> 8) as u32,
            0xc380_0000 | ((texture >> 40) as u32 & 0xff),
            0x0000_c000,
            0x9001_0fac,
            0,
            0x10,
            0,
            0,
        ],
    );
    // Clamp-to-edge XYZ, nearest texels/mips, effectiveLOD0..1, bias0.
    set(&mut words, 0x76, 0x14, &[0x92, 0x0010_0000, 0x0400_0000, 0]);
    set(&mut words, 0x76, 0x18, &[lod.to_bits()]);
    words.extend([0xc001_2d00, 3, 2]);
    words
}
#[test]
#[ignore = "requires an actual Vulkan sampled-image device; run exactly alone under an owning180-second gate"]
fn actual_pixel_mimg_samples_reverse_mips_and_fresh_owned_texels_after_real_fences() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x40000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let identity = memory.owner_id();
    let texture = base + 0x8000;
    let target = base + 0x10000;
    let command = base + 0x1000;
    memory
        .write(base + 0x4000, &bytes(&vertex_shader()))
        .unwrap();
    let shader = bytes(&sampled_pixel());
    memory.write(base + 0x5000, &shader).unwrap();
    let mut original = original_texture();
    memory.write(texture, &original).unwrap();
    memory.write(texture - 16, &[0x91; 16]).unwrap();
    memory
        .write(texture + TEXTURE_BYTES as u64, &[0xd7; 16])
        .unwrap();
    memory
        .write(target, &SENTINEL.repeat(WIDTH * HEIGHT))
        .unwrap();
    memory.write(target - 16, &[0x9b; 16]).unwrap();
    memory
        .write(target + TARGET_BYTES as u64, &[0xe7; 16])
        .unwrap();
    let limits = Limits::default();
    let gpu_limits = GpuLimits::default();
    let mut previous_submission = 0;
    for (turn, (lod, expected)) in [(0.0, RED), (1.0, GREEN), (1.0, BLUE)]
        .into_iter()
        .enumerate()
    {
        if turn == 2 {
            // The only changed guest source is this one real mip1 texel. The
            // shader, T#/S#, coordinates and LOD1 are otherwise identical.
            original[256..260].copy_from_slice(&BLUE);
            memory.write(texture + 256, &BLUE).unwrap();
        }
        let encoded = bytes(&sampled_commands(base, texture, target, lod));
        assert!(encoded.len() < 0x3000);
        memory.write(command, &encoded).unwrap();
        let mut captured = vec![0; encoded.len()];
        memory.read_gpu(command, &mut captured).unwrap();
        assert_eq!(captured, encoded);
        let plan = plan_commands(
            &[&captured],
            GraphicsState::empty(),
            &CommandLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.draws().len(), 1);
        let ranges = [CommandRange {
            address: command,
            size: captured.len() as u64,
        }];
        {
            let prepared = pentax_gpu::graphics::prepare_draw(
                &plan.draws()[0],
                &crate::gpu::Memory(&memory),
                &limits,
            )
            .unwrap();
            assert_eq!(prepared.pixel().manifest().images().len(), 1);
            assert_eq!(prepared.pixel().manifest().images()[0].binding, 58);
            assert_eq!(prepared.pixel().manifest().samplers().len(), 1);
            assert_eq!(prepared.pixel().manifest().samplers()[0].binding, 74);
            assert!(
                !prepared
                    .pixel()
                    .manifest()
                    .required_features()
                    .image_query()
            );
            assert_eq!(prepared.pixel_images()[0].descriptor().counter_id, None);
            assert_eq!(
                prepared.pixel_images()[0].source(),
                CommandRange {
                    address: texture,
                    size: TEXTURE_BYTES as u64
                }
            );
            assert_eq!(prepared.pixel_images()[0].pixels().len(), 80); // Logical4x4 +2x2 RGBA8.
        }
        // This is the actual core path called by Driver: it admits live owner,
        // rights/aliases and real backend work, waits for its GPU fence, then
        // publishes whole owned target bytes and an attested completion receipt.
        let report =
            execute_draw(&mut memory, &plan.draws()[0], &limits, &gpu_limits, &ranges).unwrap();
        assert_eq!(report.written_bytes, TARGET_BYTES);
        assert!(report.completion.owner() == identity);
        assert_eq!(
            report.completion.target(),
            NativeRange {
                address: target,
                size: TARGET_BYTES as u64
            }
        );
        assert_eq!(report.completion.layout(), TargetLayout::LinearRgba8);
        assert!(report.completion.submission() > previous_submission);
        previous_submission = report.completion.submission();
        assert!(
            report.gpu.commands > 0 && report.gpu.shader_words > 0 && report.gpu.device_bytes > 0
        );
        let output = read(&memory, target, TARGET_BYTES);
        for (x, y) in [(24, 24), (36, 20)] {
            assert_eq!(pixel(&output, x, y), expected);
        }
        for (x, y) in [(52, 20), (56, 56), (0, 0)] {
            assert_eq!(pixel(&output, x, y), SENTINEL);
        }
        assert!(
            output
                .chunks_exact(4)
                .all(|p| p == SENTINEL || p == expected)
        );
        assert_eq!(read(&memory, target - 16, 16), [0x9b; 16]);
        assert_eq!(read(&memory, target + TARGET_BYTES as u64, 16), [0xe7; 16]);
        assert_eq!(read(&memory, texture - 16, 16), [0x91; 16]);
        assert_eq!(
            read(&memory, texture + TEXTURE_BYTES as u64, 16),
            [0xd7; 16]
        );
        assert_eq!(read(&memory, texture, TEXTURE_BYTES), original);
        assert_eq!(read(&memory, command, encoded.len()), encoded);
        assert_eq!(read(&memory, base + 0x5000, shader.len()), shader);
    }
    println!(
        "agc_sampled_image_gpu_available=true agc_sampled_image_gpu_draws=3 agc_sampled_image_gpu_mip_levels=2 agc_sampled_image_gpu_fresh_source_reads=1"
    );
}
