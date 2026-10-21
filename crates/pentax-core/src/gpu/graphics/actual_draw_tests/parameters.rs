// SPDX-License-Identifier: GPL-2.0-only
//! Original unequal-W interpolation, flat and compact system-input programs.
use super::*;

fn parameter_vertex() -> Vec<u32> {
    let mut words = vertex_shader();
    words.truncate(words.len() - 3);
    // NDC positions remain the original partial triangle. PARAM7=(x+.5,
    // y+.5,1,1), while clip W=1+VertexIndex gives genuinely perspective
    // interpolation rather than the equal-W affine special case.
    words.extend([
        binary(3, 6, 240, 0),
        binary(3, 7, 240, 1),
        unary(1, 8, 242),
        unary(1, 9, 242),
    ]);
    let mut parameter = export(0x27, [6, 7, 8, 9]);
    parameter[0] &= !(1 << 11);
    words.extend(parameter);
    words.extend([
        unary(6, 4, 261),
        binary(3, 4, 242, 4),
        binary(8, 0, 260, 0),
        binary(8, 1, 260, 1),
    ]);
    words.extend(export(12, [0, 1, 2, 4]));
    words.push(0xbf81_0000);
    words
}

fn parameter_pixel(flat: bool) -> Vec<u32> {
    let mut words = Vec::new();
    for channel in 0..4 {
        let common = 0xc800_0000 | ((6 + channel) << 18) | (channel << 8);
        if flat {
            words.push(common | (2 << 16) | 2);
        } else {
            words.extend([common, common | (1 << 16) | 1]);
        }
    }
    words.extend(export(0, [6, 7, 8, 9]));
    words.push(0xbf81_0000);
    words
}

fn system_vertex(reverse: bool) -> Vec<u32> {
    let mut words = vertex_shader();
    words.truncate(words.len() - 3);
    if reverse {
        // Exchange x/y to reverse winding without changing triangle coverage.
        words.extend([unary(1, 6, 256), unary(1, 0, 257), unary(1, 1, 262)]);
    }
    // Inline selector244 is exactly+2.0: both clipXY and clipW double,
    // retaining NDC coverage while FragCoord.w becomes0.5.
    words.extend([binary(8, 0, 244, 0), binary(8, 1, 244, 1), unary(1, 3, 244)]);
    words.extend(export(12, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}

fn system_pixel() -> Vec<u32> {
    // ENA XYZW/front compacts to v0..4. Keep Z live through B; front becomes
    // either0/1 from its actual float -1/+1 convention. W is the source ABI's
    // reciprocal FragCoord.w=2, so alpha=W*.125=.25. This stays away
    // from a UNORM midpoint even with the guest reciprocal's finite precision.
    let mut words = vec![
        binary(8, 6, 255, 0),
        (1_f32 / 64.0).to_bits(),
        binary(8, 7, 255, 1),
        (1_f32 / 64.0).to_bits(),
        binary(3, 8, 242, 4),
        binary(8, 8, 240, 8),
        binary(3, 8, 258, 8),
        binary(8, 9, 255, 3),
        0.125_f32.to_bits(),
    ];
    words.extend(export(0, [6, 7, 8, 9]));
    words.push(0xbf81_0000);
    words
}

fn one_command(vertex: u64, pixel: u64, target: u64, flat: Option<bool>) -> Vec<u32> {
    let mut words = commands(vertex, pixel, target);
    let first = words.iter().position(|word| *word == 0xc001_2d00).unwrap();
    words.truncate(first);
    // Actual VS user input remains zero; actual PS user pushes stay red, so
    // silently ignoring declared inputs cannot satisfy any literal below.
    if let Some(flat) = flat {
        for (register, value) in [
            (0x1b1, 7 << 1),
            (0x1b6, 0x8001),
            (0x1b3, if flat { 0 } else { 2 }),
            (0x1b4, if flat { 0 } else { 2 }),
            (0x191, 7 | if flat { 0x400 } else { 0 }),
        ] {
            set(&mut words, 0x69, register, &[value]);
        }
    } else {
        set(&mut words, 0x69, 0x1b3, &[0x1f00, 0x1f00]);
    }
    words.extend([0xc001_2d00, 3, 2]);
    words
}

#[test]
#[ignore = "requires a local Vulkan device"]
fn actual_parameter_linkage_interpolates_and_preserves_system_inputs() {
    let mut face_values = Vec::new();
    for case in 0..4 {
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
        let target = base + 0x8000;
        let flat = match case {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        let vertex = if flat.is_some() {
            parameter_vertex()
        } else {
            system_vertex(case == 3)
        };
        let pixel_words = flat.map_or_else(system_pixel, parameter_pixel);
        memory.write(base + 0x4000, &bytes(&vertex)).unwrap();
        memory.write(base + 0x5000, &bytes(&pixel_words)).unwrap();
        memory
            .write(target, &SENTINEL.repeat(WIDTH * HEIGHT))
            .unwrap();
        memory.write(target - 16, &[0x9b; 16]).unwrap();
        memory
            .write(target + TARGET_BYTES as u64, &[0xe7; 16])
            .unwrap();
        let encoded = bytes(&one_command(base + 0x4000, base + 0x5000, target, flat));
        memory.write(base + 0x1000, &encoded).unwrap();
        let plan = plan_commands(
            &[&encoded],
            GraphicsState::empty(),
            &CommandLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.draws().len(), 1);
        let report = execute_draw(
            &mut memory,
            &plan.draws()[0],
            &Limits::default(),
            &GpuLimits::default(),
            &[CommandRange {
                address: base + 0x1000,
                size: encoded.len() as u64,
            }],
        )
        .unwrap();
        assert_eq!(report.written_bytes, TARGET_BYTES);
        assert!(report.completion.owner() == memory.owner_id());
        assert_eq!(
            report.completion.target(),
            NativeRange {
                address: target,
                size: TARGET_BYTES as u64
            }
        );
        assert!(report.gpu.commands > 0 && report.gpu.device_bytes > 0);
        let image = read(&memory, target, TARGET_BYTES);
        match case {
            0 => {
                // Center barycentrics at(24,24) are30/64,17/64,17/64.
                // Dividing by W1,2,3 gives color(51/265,34/265,1).
                assert_eq!(pixel(&image, 24, 24), [49, 33, 255, 255]);
                assert_eq!(pixel(&image, 36, 20), [139, 20, 255, 255]);
            }
            1 => {
                // Fixed state selects first-vertex flat: not last vertex's
                // green, nor either perspective result, nor PS red push.
                assert_eq!(pixel(&image, 24, 24), [0, 0, 255, 255]);
                assert_eq!(pixel(&image, 36, 20), [0, 0, 255, 255]);
            }
            _ => {
                let first = pixel(&image, 24, 24);
                let second = pixel(&image, 36, 20);
                assert_eq!([first[0], first[1], first[3]], [98, 98, 64]);
                assert_eq!([second[0], second[1], second[3]], [145, 82, 64]);
                assert!(matches!(first[2], 0 | 255));
                assert_eq!(first[2], second[2]);
                face_values.push(first[2]);
            }
        }
        assert_eq!(pixel(&image, 56, 56), SENTINEL);
        assert_eq!(read(&memory, target - 16, 16), [0x9b; 16]);
        assert_eq!(read(&memory, target + TARGET_BYTES as u64, 16), [0xe7; 16]);
    }
    // Both raw face signs are observed; swapping x/y reverses only winding.
    face_values.sort_unstable();
    assert_eq!(face_values, [0, 255]);
    println!("local_gpu_parameter_linkage_available=true local_gpu_parameter_linkage_cases=4");
}
