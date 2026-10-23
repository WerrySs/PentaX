// SPDX-License-Identifier: GPL-2.0-only
//! Original implicit MIMG samples and real clamped/unclamped LOD query results.
//! Compact FragCoord inputs create derivatives; no host shader or queried-value substitute.
use super::*;
use pentax_shader_compiler::InputInterface;

const TEXTURE_BYTES: usize = 1536;

fn lod_pixel_shader() -> Vec<u32> {
    // CX input enables0x300 compact actual FragCoord.xy into v0/v1.
    // The only varying input is real user SGPR12: normalized coord units/pixel.
    let mut words = vec![
        binary(8, 0, 12, 0), // v0 = s12 * FragCoord.x
        binary(8, 1, 12, 1), // v1 = s12 * FragCoord.y
        0xf080_0f08,
        0x0040_0400, // IMAGE_SAMPLE v4..7,v0..1,T#s0,S#s8
        0xf180_0308,
        0x0040_0800,           // IMAGE_GET_LOD v8..9,v0..1,T#s0,S#s8
        0xbf8c_0000,           // Actual S_WAITCNT before consuming sample/query values.
        binary(3, 10, 242, 9), // requested/unclamped query.y +1
        binary(8, 10, 255, 10),
        0.125_f32.to_bits(),
        binary(8, 11, 255, 8),
        0.125_f32.to_bits(), // clamped query.x /8
    ];
    // R/G are actual texture channels. B/A independently consume BOTH actual
    // query components, so constants, an omitted query, or swapped outputs fail.
    words.extend(export(0, [4, 5, 10, 11]));
    words.push(0xbf81_0000);
    words
}
fn original_mip_colors() -> Vec<u8> {
    // Source linear storage is reverse-mip with256-byte row pitches. Mip1 is
    //2x2 green atoffset0; mip0 is4x4 red atoffset512. Padding stays0xa5.
    let mut data = vec![0xa5; TEXTURE_BYTES];
    for y in 0..2 {
        for x in 0..2 {
            let at = y * 256 + x * 4;
            data[at..at + 4].copy_from_slice(&GREEN);
        }
    }
    for y in 0..4 {
        for x in 0..4 {
            let at = 512 + y * 256 + x * 4;
            data[at..at + 4].copy_from_slice(&RED);
        }
    }
    data
}
fn lod_commands(base: u64, texture: u64, target: u64, scale: f32) -> Vec<u32> {
    let mut words = commands(base + 0x4000, base + 0x5000, target);
    let first = words.iter().position(|word| *word == 0xc001_2d00).unwrap();
    words.truncate(first);
    // No barycentric inputs/PARAM requirements: only genuine compact XY.
    set(&mut words, 0x69, 0x1b3, &[0x300, 0x300]);
    // Allocation encoding 2 covers the original shader destinations v0..v11.
    set(&mut words, 0x76, 0x0a, &[0x002c_0002]);
    set(&mut words, 0x76, 0x0b, &[26]); // Thirteen PS user SGPRs.
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
    ); //4x4 RGBA8, physical/view mip0..1, no image MIN_LOD or counters.
    // Repeat XYZ; nearest texels/mips; sampler LOD0..1; bias0.
    set(&mut words, 0x76, 0x14, &[0, 0x0010_0000, 0x0400_0000, 0]);
    set(&mut words, 0x76, 0x18, &[scale.to_bits()]);
    words.extend([0xc001_2d00, 3, 2]);
    words
}
fn expected_sample_and_query(pixel: &[u8], second: bool) -> bool {
    // Public Vulkan scale-factor bounds permit rho..sqrt(2)*rho, rather than
    // an exact implementation-specific logarithm. Minimum mipmapPrecisionBits4
    // allows1/16 rounding; these byte bands add a small UNORM conversion guard.
    // scale1/8: requestedLOD[-1,-.5], sampler-clamped mip0;
    // scale1:   requestedLOD[2,2.5], sampler-clamped mip1.
    // After B=(requested+1)/8, bands remain disjoint from default query0 ->32.
    // UNORM conversion permits either closest integer: 0.125*255 is31.875.
    if second {
        pixel[0] == 0
            && pixel[1] == 255
            && (93..=115).contains(&pixel[2])
            && (31..=32).contains(&pixel[3])
    } else {
        pixel[0] == 255 && pixel[1] == 0 && pixel[2] <= 20 && pixel[3] == 0
    }
}

#[test]
#[ignore = "requires actual Vulkan implicit sampling and LOD queries; run exactly alone under an owning180-second gate"]
fn actual_implicit_mimg_uses_fragment_gradients_and_exports_both_real_lod_query_components() {
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
    let shader = bytes(&lod_pixel_shader());
    let texture_bytes = original_mip_colors();
    memory
        .write(base + 0x4000, &bytes(&vertex_shader()))
        .unwrap();
    memory.write(base + 0x5000, &shader).unwrap();
    memory.write(texture, &texture_bytes).unwrap();
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
    let mut previous_submission = 0;
    let mut first_coverage = Vec::new();
    for (turn, scale) in [0.125_f32, 1.0_f32].into_iter().enumerate() {
        let encoded = bytes(&lod_commands(base, texture, target, scale));
        assert!(encoded.len() < 0x3000);
        memory.write(command, &encoded).unwrap();
        let mut current_commands = vec![0; encoded.len()];
        memory.read_gpu(command, &mut current_commands).unwrap();
        assert_eq!(current_commands, encoded);
        let plan = plan_commands(
            &[&current_commands],
            GraphicsState::empty(),
            &CommandLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.draws().len(), 1);
        {
            let product = pentax_gpu::graphics::prepare_draw(
                &plan.draws()[0],
                &crate::gpu::Memory(&memory),
                &Limits::default(),
            )
            .unwrap();
            let manifest = product.pixel().manifest();
            assert!(manifest.inputs().contains(&InputInterface::FragCoord));
            assert!(manifest.required_features().image_query());
            assert_eq!(manifest.images().len(), 1);
            assert_eq!(manifest.samplers().len(), 1);
            assert_eq!(manifest.images()[0].binding, 58);
            assert_eq!(manifest.samplers()[0].binding, 74);
            assert_eq!(product.pixel_images()[0].descriptor().min_lod_u4_8, 0);
            assert_eq!(product.pixel_images()[0].descriptor().counter_id, None);
            assert_eq!(
                product.pixel_images()[0].source(),
                CommandRange {
                    address: texture,
                    size: TEXTURE_BYTES as u64
                }
            );
            assert_eq!(product.pixel_images()[0].pixels().len(), 80);
            let push = manifest.push_data().unwrap();
            let scale_slot = push.user_sgprs.iter().position(|&reg| reg == 12).unwrap();
            let start = (push.start_dword as usize + scale_slot) * 4;
            assert_eq!(
                &product.pixel_push_data()[start..start + 4],
                scale.to_bits().to_le_bytes()
            );
        }
        let report = execute_draw(
            &mut memory,
            &plan.draws()[0],
            &Limits::default(),
            &GpuLimits::default(),
            &[CommandRange {
                address: command,
                size: current_commands.len() as u64,
            }],
        )
        .unwrap();
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
            assert!(expected_sample_and_query(pixel(&output, x, y), turn == 1));
        }
        for (x, y) in [(52, 20), (56, 56), (0, 0)] {
            assert_eq!(pixel(&output, x, y), SENTINEL);
        }
        assert!(
            output
                .chunks_exact(4)
                .all(|p| p == SENTINEL || expected_sample_and_query(p, turn == 1))
        );
        let coverage: Vec<_> = output.chunks_exact(4).map(|p| p != SENTINEL).collect();
        assert!(coverage.iter().any(|&covered| covered));
        if turn == 0 {
            first_coverage = coverage;
        } else {
            assert_eq!(coverage, first_coverage);
        }
        for (address, value) in [
            (texture - 16, 0x91),
            (texture + TEXTURE_BYTES as u64, 0xd7),
            (target - 16, 0x9b),
            (target + TARGET_BYTES as u64, 0xe7),
        ] {
            assert_eq!(read(&memory, address, 16), [value; 16]);
        }
        assert_eq!(read(&memory, texture, TEXTURE_BYTES), texture_bytes);
        assert_eq!(read(&memory, command, encoded.len()), encoded);
        assert_eq!(read(&memory, base + 0x5000, shader.len()), shader);
    }
    println!(
        "agc_implicit_sample_gpu_available=true agc_implicit_sample_gpu_draws=2 agc_implicit_sample_gpu_query_draws=2 agc_implicit_sample_gpu_distinct_mip_colors=2"
    );
}
