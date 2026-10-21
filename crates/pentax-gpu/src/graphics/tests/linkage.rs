// SPDX-License-Identifier: GPL-2.0-only
//! Original sparse PARAM exports and actual input-register linkage.
use super::*;
use pentax_shader_compiler::{InputInterface, OutputInterface};

fn linked_source(locations: &[u32], attribute: u32, flat: bool) -> Source {
    let mut source = Source::new();
    let position = source.vertex.split_off(source.vertex.len() - 3);
    source.vertex.extend([
        unary(1, 6, 128),
        unary(1, 7, 242),
        unary(1, 8, 128),
        unary(1, 9, 242),
    ]);
    for location in locations {
        let mut parameter = export(0x20 + location, [6, 7, 8, 9]);
        parameter[0] &= !(1 << 11);
        source.vertex.extend(parameter);
    }
    source.vertex.extend(position);
    source.pixel.clear();
    for channel in 0..4 {
        let common = 0xc800_0000 | ((6 + channel) << 18) | (attribute << 10) | (channel << 8);
        if flat {
            source.pixel.push(common | (2 << 16) | 2);
        } else {
            source.pixel.extend([common, common | (1 << 16) | 1]);
        }
    }
    source.pixel.extend(export(0, [6, 7, 8, 9]));
    source.pixel.push(END);
    source
}

fn linked_commands(count: u32, controls: &[u32], span: u32, flat: bool) -> Vec<u32> {
    let mut words = commands(&[]);
    words.truncate(words.len() - 3);
    for (register, value) in [
        (0x1b1, (span - 1) << 1),
        (0x1b6, 0x8000 | count),
        (0x1b3, if flat { 0 } else { 2 }),
        (0x1b4, if flat { 0 } else { 2 }),
    ] {
        set(&mut words, RegisterSpace::Context, register, &[value]);
    }
    if !controls.is_empty() {
        set(&mut words, RegisterSpace::Context, 0x191, controls);
    }
    words.extend([0xc001_2d00, 3, 2]);
    words
}

#[test]
fn parameter_linkage_admits_sparse_locations_aliases_and_all_32_declared_inputs() {
    for flat in [false, true] {
        for location in [0, 1, 7, 31] {
            let source = linked_source(&[location], 31, flat);
            let control = location | if flat { 0x400 } else { 0 };
            let planned = plan(&linked_commands(32, &[control; 32], location + 1, flat));
            let prepared = prepare_draw(&planned.draws()[0], &source, &Limits::default()).unwrap();
            assert!(
                prepared
                    .vertex()
                    .manifest()
                    .outputs()
                    .contains(&OutputInterface::Parameter {
                        index: location,
                        location
                    })
            );
            assert_eq!(
                prepared.pixel().manifest().inputs(),
                [InputInterface::Parameter {
                    input: 31,
                    location,
                    flat
                }]
            );
            let interfaces =
                validate_shader_interfaces(prepared.vertex(), prepared.pixel()).unwrap();
            assert_eq!(
                (interfaces.vertex_locations, interfaces.pixel_locations),
                (1 << location, 1 << location)
            );
            assert_eq!(
                (interfaces.vertex_components, interfaces.pixel_components),
                (8, 4)
            );
            assert_eq!(source.snapshots.get(), 1);
            assert!(prepared.usage().host_bytes < prepared.usage().peak_host_bytes);
        }
        // Two distinct ATTRs share one host vec4 at the same class/location.
        let mut source = linked_source(&[7], 0, flat);
        // Make ATTR0 live as red and ATTR1 live as green instead of overwriting
        // the first result with a second full color.
        source.pixel.clear();
        for (attribute, destination) in [(0, 6), (1, 7)] {
            let common = 0xc800_0000 | (destination << 18) | (attribute << 10);
            if flat {
                source.pixel.push(common | (2 << 16) | 2);
            } else {
                source.pixel.extend([common, common | (1 << 16) | 1]);
            }
        }
        source.pixel.extend([unary(1, 8, 128), unary(1, 9, 242)]);
        source.pixel.extend(export(0, [6, 7, 8, 9]));
        source.pixel.push(END);
        let control = 7 | if flat { 0x400 } else { 0 };
        let planned = plan(&linked_commands(2, &[control; 2], 8, flat));
        let prepared = prepare_draw(&planned.draws()[0], &source, &Limits::default()).unwrap();
        assert_eq!(prepared.pixel().manifest().inputs().len(), 2);
        assert_eq!(
            validate_shader_interfaces(prepared.vertex(), prepared.pixel())
                .unwrap()
                .pixel_components,
            4
        );
        let spirv = prepared.pixel().spirv();
        let mut at = 5;
        let mut declarations = 0;
        while at < spirv.len() {
            let count = (spirv[at] >> 16) as usize;
            assert!(count != 0 && at + count <= spirv.len());
            // OpDecorate(target, Location=30, location=7).
            if spirv[at] & 0xffff == 71 && count == 4 && spirv[at + 2] == 30 && spirv[at + 3] == 7 {
                declarations += 1;
            }
            at += count;
        }
        assert_eq!(declarations, 1);
    }
}

#[test]
fn parameter_missing_reserved_and_cross_stage_mismatches_refuse_before_target_capture() {
    let source = linked_source(&[7], 0, false);
    // Register/declaration failures precede every shader read.
    for (count, controls, extra) in [
        (1, vec![], None),
        (33, vec![], None),
        (1, vec![7 | 0x20], None),
        (1, vec![7 | 0x800], None),
        (2, vec![7, 7 | 0x400], None),
        (1, vec![7], Some((0x1b4, 0))),
        (1, vec![7], Some((0x1b8, 0x0100_0000))),
        (1, vec![7], Some((0x1b3, 4))),
        (1, vec![7], Some((0x1b1, 1))),
    ] {
        let mut words = linked_commands(count, &controls, 8, false);
        if let Some((register, value)) = extra {
            words.truncate(words.len() - 3);
            set(&mut words, RegisterSpace::Context, register, &[value]);
            words.extend([0xc001_2d00, 3, 2]);
        }
        let planned = plan(&words);
        assert!(prepare_draw(&planned.draws()[0], &source, &Limits::default()).is_err());
        assert!(source.reads.borrow().is_empty());
    }
    for (location, span) in [(1, 8), (7, 7)] {
        let planned = plan(&linked_commands(1, &[location], span, false));
        assert!(matches!(
            prepare_draw(&planned.draws()[0], &source, &Limits::default()),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(source.snapshots.get(), 0);
    }
    let mut undefined = linked_source(&[7], 1, false);
    let planned = plan(&linked_commands(1, &[7], 8, false));
    assert!(prepare_draw(&planned.draws()[0], &undefined, &Limits::default()).is_err());
    assert_eq!(undefined.snapshots.get(), 0);
    undefined.pixel.insert(0, unary(1, 1, 128)); // Computed j is not implicit interpolation.
    let planned = plan(&linked_commands(2, &[7, 7], 8, false));
    assert!(matches!(
        prepare_draw(&planned.draws()[0], &undefined, &Limits::default()),
        Err(Error::Compile)
    ));
    assert_eq!(undefined.snapshots.get(), 0);
}

#[test]
fn parameter_linkage_keeps_all_32_simultaneously_live_inputs_and_profile_capacity_bounded() {
    for flat in [false, true] {
        let locations: Vec<u32> = (0..32).collect();
        let mut source = linked_source(&locations, 0, flat);
        source.pixel = vec![unary(1, 10, 128)];
        for attribute in 0..32 {
            let common = 0xc800_0000 | (6 << 18) | (attribute << 10);
            if flat {
                source.pixel.push(common | (2 << 16) | 2);
            } else {
                source.pixel.extend([common, common | (1 << 16) | 1]);
            }
            source.pixel.push(binary(3, 10, 262, 10));
        }
        source
            .pixel
            .extend([unary(1, 7, 128), unary(1, 8, 128), unary(1, 9, 242)]);
        source.pixel.extend(export(0, [10, 7, 8, 9]));
        source.pixel.push(END);
        assert!(source.vertex.len() <= 256 && source.pixel.len() <= 256);
        let controls: Vec<u32> = locations
            .iter()
            .map(|location| *location | if flat { 0x400 } else { 0 })
            .collect();
        let planned = plan(&linked_commands(32, &controls, 32, flat));
        let prepared = prepare_draw(&planned.draws()[0], &source, &Limits::default()).unwrap();
        let interfaces = validate_shader_interfaces(prepared.vertex(), prepared.pixel()).unwrap();
        assert_eq!(prepared.pixel().manifest().inputs().len(), 32);
        assert_eq!(
            (interfaces.vertex_locations, interfaces.pixel_locations),
            (u32::MAX, u32::MAX)
        );
        assert_eq!(
            (interfaces.vertex_components, interfaces.pixel_components),
            (132, 128)
        );
        assert_eq!(source.snapshots.get(), 1);
    }
    // The fixed profile-vector admission itself is fallible and capacity
    // charged independently of the opaque compiler temporary envelope.
    let planned = plan(&linked_commands(32, &[7; 32], 8, false));
    let mut budget = Budget {
        limits: &Limits::default(),
        usage: Usage::default(),
    };
    let registers =
        super::super::linkage::PixelRegisters::read(&planned.draws()[0].state, &mut budget)
            .unwrap();
    let before = budget.usage.host_bytes;
    let (profile, charged) = registers.profile(0, &mut budget).unwrap();
    assert_eq!(
        charged,
        profile.inputs.capacity() * std::mem::size_of::<pentax_shader_compiler::PixelInput>()
    );
    assert_eq!(budget.usage.host_bytes, before + charged);
    drop(profile);
    budget.release(charged).unwrap();
    assert_eq!(budget.usage.host_bytes, before);
    let low_limits = Limits {
        max_host_bytes: 63,
        ..Default::default()
    };
    let mut refused = Budget {
        limits: &low_limits,
        usage: Usage::default(),
    };
    assert!(registers.profile(0, &mut refused).is_err());
    assert_eq!(refused.usage.host_bytes, 0);
}
