// SPDX-License-Identifier: GPL-2.0-only
//! Original readonly stage resources on actual paused native owners.
//! Refusal/admission fixtures do not enter Vulkan or invent a completion.
use super::*;
use crate::gpu::graphics::{Error, execute_draw_admitted, preflight_read_source};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryError, NativeMemoryTransaction,
};
use std::cell::Cell;

fn buffer_vertex_shader() -> Vec<u32> {
    // The loaded float changes actual coverage. No descriptor address word is
    // consumed as a floating-point offset or supplied as a replacement result.
    let mut original = vertex_shader();
    original[9] = binary(3, 0, 260, 0); // Replace v0 += s0 with v0 += v4.
    // VS s2/s3 are entry-ABI values, so its descriptor uses s[4..7].
    let mut words = vec![0xe030_0000, 0x8001_0400]; // Raw DWORD -> v4, s[4..7].
    words.extend(original);
    words
}

fn buffer_pixel_shader(offset: u32) -> Vec<u32> {
    assert!(offset <= 4095);
    let mut words = vec![0xe038_0000 | offset, 0x8000_0000]; // Raw DWORDX4 -> v0..3.
    words.extend(export(0, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}

fn resource_commands(
    base: u64,
    vertex: Option<CommandRange>,
    pixel: Option<CommandRange>,
) -> Vec<u32> {
    let mut words = commands(base + 0x4000, base + 0x5000, base + 0x10000);
    let first = words.iter().position(|word| *word == 0xc001_2d00).unwrap();
    words.truncate(first);
    for (resource, first, resources2, value) in
        [(vertex, 0x90, 0x8b, 0x0003_0010), (pixel, 0x0c, 0x0b, 8)]
    {
        if let Some(resource) = resource {
            set(&mut words, 0x76, resources2, &[value]); // VS eight / PS four user SGPRs.
            set(
                &mut words,
                0x76,
                first,
                &[
                    resource.address as u32,
                    (resource.address >> 32) as u32,
                    resource.size as u32,
                    0,
                ],
            );
        }
    }
    words.extend([0xc001_2d00, 3, 2]);
    words
}

fn resource_plan(
    base: u64,
    vertex: Option<CommandRange>,
    pixel: Option<CommandRange>,
) -> pentax_gpu::graphics::CommandPlan {
    let encoded = bytes(&resource_commands(base, vertex, pixel));
    plan_commands(
        &[&encoded],
        GraphicsState::empty(),
        &CommandLimits::default(),
    )
    .unwrap()
}

fn stage_bytes(values: [f32; 4]) -> Vec<u8> {
    bytes(&values.map(f32::to_bits))
}

fn shaders(memory: &mut PausedNativeMemory<'_>, base: u64, vertex: bool, pixel: bool, offset: u32) {
    memory
        .write(
            base + 0x4000,
            &bytes(&if vertex {
                buffer_vertex_shader()
            } else {
                vertex_shader()
            }),
        )
        .unwrap();
    memory
        .write(
            base + 0x5000,
            &bytes(&if pixel {
                buffer_pixel_shader(offset)
            } else {
                pixel_shader()
            }),
        )
        .unwrap();
}

fn original_owner() -> (NativeAddressSpace, u64) {
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
    memory.write(base + 0x8000, &stage_bytes([0.0; 4])).unwrap();
    memory
        .write(base + 0x9000, &stage_bytes([1.0, 0.0, 0.0, 1.0]))
        .unwrap();
    memory
        .write(base + 0x10000, &SENTINEL.repeat(WIDTH * HEIGHT))
        .unwrap();
    drop(memory);
    (owner, base)
}

#[test]
fn readonly_resource_admission_checks_full_cross_page_gpu_access_and_work_before_effects() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    for (address, readable) in [(base, true), (base + 0x4000, false), (base + 0x8000, true)] {
        owner
            .map_at(
                address,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(readable, address == base + 0x8000),
            )
            .unwrap();
    }
    let mut memory = owner.pause().unwrap();
    memory.write(base + 0x3ffc, &[0xa5; 8]).unwrap();
    memory.write(base + 0x8000, &[0x3b; 64]).unwrap();
    let target = NativeRange {
        address: base + 0x8000,
        size: 64,
    };
    for source in [
        CommandRange {
            address: base + 0x3ffc,
            size: 8,
        },
        CommandRange {
            address: base + 0xbffc,
            size: 8,
        },
    ] {
        assert!(matches!(
            preflight_read_source(&memory, source, target, 4096),
            Err(Error::Memory(_))
        ));
    }
    for source in [
        CommandRange {
            address: base,
            size: 0,
        },
        CommandRange {
            address: u64::MAX - 3,
            size: 8,
        },
    ] {
        assert!(matches!(
            preflight_read_source(&memory, source, target, 4096),
            Err(Error::Limit)
        ));
    }
    assert!(matches!(
        preflight_read_source(
            &memory,
            CommandRange {
                address: base,
                size: 8
            },
            target,
            0
        ),
        Err(Error::Limit)
    ));
    assert_eq!(read(&memory, base + 0x3ffc, 8), [0xa5; 8]);
    assert_eq!(read(&memory, target.address, 64), [0x3b; 64]);
}

#[test]
fn readonly_stage_sources_allow_mutual_physical_aliases_but_reject_target_aliases() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base + 0x20000,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
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
    for address in [base, base + 0x10000] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, false),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    memory.write(base, &[0x5a; 64]).unwrap();
    memory.write(base + 0x20000, &[0x3b; 64]).unwrap();
    let target = NativeRange {
        address: base + 0x20000,
        size: 64,
    };
    let vertex = CommandRange {
        address: base,
        size: 64,
    };
    let pixel = CommandRange {
        address: base + 0x10000,
        size: 64,
    };
    let vertex_work = preflight_read_source(&memory, vertex, target, 4096).unwrap();
    let pixel_work = preflight_read_source(&memory, pixel, target, 4096).unwrap();
    assert!(vertex_work > 0 && pixel_work > 0);
    assert_eq!(
        read(&memory, vertex.address, 64),
        read(&memory, pixel.address, 64)
    );
    for (source, alias) in [(vertex, pixel), (pixel, vertex)] {
        assert!(matches!(
            preflight_read_source(
                &memory,
                source,
                NativeRange {
                    address: alias.address,
                    size: alias.size
                },
                4096
            ),
            Err(Error::Memory(NativeMemoryError::Overlap))
        ));
    }
    assert!(
        memory
            .preflight_gpu_write(vertex.address, vertex.size)
            .is_err()
    );
    assert_eq!(read(&memory, target.address, 64), [0x3b; 64]);
}

#[test]
fn both_stage_snapshots_observe_current_bytes_and_each_upload_consumes_its_actual_extent() {
    let (mut owner, base) = original_owner();
    let mut memory = owner.pause().unwrap();
    let source = CommandRange {
        address: base + 0x8000,
        size: 16,
    };
    shaders(&mut memory, base, true, true, 0);
    let plan = resource_plan(base, Some(source), Some(source));
    let before = pentax_gpu::graphics::prepare_draw(
        &plan.draws()[0],
        &crate::gpu::Memory(&memory),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(
        before.vertex_buffer().unwrap().bytes(),
        stage_bytes([0.0; 4])
    );
    assert_eq!(
        before.pixel_buffer().unwrap().bytes(),
        stage_bytes([0.0; 4])
    );
    let changed = stage_bytes([0.5, 0.25, 0.75, 1.0]);
    memory.write(source.address, &changed).unwrap();
    let after = pentax_gpu::graphics::prepare_draw(
        &plan.draws()[0],
        &crate::gpu::Memory(&memory),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(after.vertex_buffer().unwrap().bytes(), changed);
    assert_eq!(after.pixel_buffer().unwrap().bytes(), changed);
    assert_eq!(
        before.vertex_buffer().unwrap().bytes(),
        stage_bytes([0.0; 4])
    );
    assert_eq!(after.vertex_buffer().unwrap().range(), source);
    assert_eq!(after.pixel_buffer().unwrap().range(), source);
    drop(before);
    drop(after);
    for (vertex, pixel, uploads) in [(true, false, 16), (false, true, 16), (true, true, 32)] {
        shaders(&mut memory, base, vertex, pixel, 0);
        let plan = resource_plan(base, vertex.then_some(source), pixel.then_some(source));
        for short in [true, false] {
            let admitted = Cell::new(false);
            let gpu = GpuLimits {
                max_host_bytes: 2 * TARGET_BYTES as u64 + uploads - u64::from(short),
                ..Default::default()
            };
            let result = execute_draw_admitted(
                &mut memory,
                &plan.draws()[0],
                &Limits::default(),
                &gpu,
                &[],
                |_, _, _| {
                    admitted.set(true);
                    Err(Error::Limit)
                },
            );
            assert!(matches!(result, Err(Error::Limit)));
            assert_eq!(admitted.get(), !short);
            assert_eq!(
                read(&memory, base + 0x10000, TARGET_BYTES),
                SENTINEL.repeat(WIDTH * HEIGHT)
            );
        }
    }
}

#[test]
fn each_stage_refuses_late_gpu_read_unmapped_foreign_and_virtual_target_overlap_before_admission() {
    for vertex in [true, false] {
        for case in 0..5 {
            let (mut owner, base) = original_owner();
            let (_foreign, foreign_base) = original_owner();
            let address = match case {
                0 | 1 => base + 0xbffc,
                2 => foreign_base + 0x8000,
                3 => base + 0x10100,
                _ => base + 0x8001,
            };
            if case == 0 {
                owner
                    .protect(
                        base + 0xc000,
                        0x4000,
                        NativePermissions::READ_WRITE.with_gpu(false, false),
                    )
                    .unwrap();
            }
            let mut memory = owner.pause().unwrap();
            if case == 1 {
                memory
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Unmap {
                                address: base + 0xc000,
                                size: 0x4000,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
            }
            shaders(&mut memory, base, vertex, !vertex, 0);
            let resource = CommandRange { address, size: 16 };
            let plan = resource_plan(
                base,
                vertex.then_some(resource),
                (!vertex).then_some(resource),
            );
            let admitted = Cell::new(false);
            assert!(
                execute_draw_admitted(
                    &mut memory,
                    &plan.draws()[0],
                    &Limits::default(),
                    &GpuLimits::default(),
                    &[],
                    |_, _, _| {
                        admitted.set(true);
                        Err(Error::Limit)
                    }
                )
                .is_err()
            );
            assert!(!admitted.get());
            assert_eq!(
                read(&memory, base + 0x10000, TARGET_BYTES),
                SENTINEL.repeat(WIDTH * HEIGHT)
            );
        }
    }
}

#[test]
fn actual_prepared_vertex_and_pixel_resources_cannot_alias_target_physical_backing() {
    let (mut owner, base) = original_owner();
    let mut memory = owner.pause().unwrap();
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::Unmap {
                    address: base + 0x10000,
                    size: 0x4000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::Unmap {
                    address: base + 0x30000,
                    size: 0x4000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
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
    for (address, writable) in [(base + 0x10000, true), (base + 0x30000, false)] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, writable),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    memory
        .write(base + 0x10000, &SENTINEL.repeat(WIDTH * HEIGHT))
        .unwrap();
    let resource = CommandRange {
        address: base + 0x30000,
        size: 16,
    };
    for (vertex, pixel) in [(true, false), (false, true), (true, true)] {
        shaders(&mut memory, base, vertex, pixel, 0);
        let plan = resource_plan(base, vertex.then_some(resource), pixel.then_some(resource));
        let admitted = Cell::new(false);
        assert!(matches!(
            execute_draw_admitted(
                &mut memory,
                &plan.draws()[0],
                &Limits::default(),
                &GpuLimits::default(),
                &[],
                |_, _, _| {
                    admitted.set(true);
                    Err(Error::Limit)
                }
            ),
            Err(Error::Memory(NativeMemoryError::Overlap))
        ));
        assert!(!admitted.get());
        assert_eq!(
            read(&memory, base + 0x10000, TARGET_BYTES),
            SENTINEL.repeat(WIDTH * HEIGHT)
        );
    }
}

#[test]
fn ordered_prefix_resource_writes_survive_a_late_pixel_read_failure_and_cannot_be_replayed() {
    use crate::gpu::driver::{Driver, Failure, Function, Limits as DriverLimits};
    let (mut owner, base) = original_owner();
    owner
        .protect(
            base + 0xc000,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(false, false),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    shaders(&mut memory, base, true, true, 0);
    let vertex = CommandRange {
        address: base + 0x8000,
        size: 16,
    };
    let pixel = CommandRange {
        address: base + 0xc000,
        size: 16,
    };
    let changed = [
        0.5f32.to_bits(),
        0.25f32.to_bits(),
        0.75f32.to_bits(),
        1.0f32.to_bits(),
    ];
    let mut words = vec![
        0xc006_3700,
        0x100,
        vertex.address as u32,
        (vertex.address >> 32) as u32,
    ];
    words.extend(changed);
    words.extend(resource_commands(base, Some(vertex), Some(pixel)));
    let encoded = bytes(&words);
    memory.write(base + 0x1000, &encoded).unwrap();
    memory.write(base + 4, &u32::MAX.to_le_bytes()).unwrap();
    memory
        .write(base + 0x20, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x28, &(words.len() as u32).to_le_bytes())
        .unwrap();
    memory.write(base + 0x2c, &0u32.to_le_bytes()).unwrap();
    let mut driver = Driver::new(DriverLimits::default());
    let args = [base, base + 0x20, 0, 0, 0, 0];
    assert!(matches!(
        driver.submit(&mut memory, Function::CommandBuffer, args),
        Err(Failure::GraphicsPreparation(
            pentax_gpu::graphics::Error::Memory(_)
        ))
    ));
    assert_eq!(read(&memory, vertex.address, 16), bytes(&changed));
    assert_eq!(
        read(&memory, base + 0x10000, TARGET_BYTES),
        SENTINEL.repeat(WIDTH * HEIGHT)
    );
    assert_eq!(
        (
            driver.usage().memory_commands,
            driver.usage().memory_written_bytes,
            driver.usage().draws
        ),
        (1, 16, 1)
    );
    let usage = driver.usage();
    assert_eq!(
        driver.submit(&mut memory, Function::CommandBuffer, args),
        Err(Failure::Poisoned)
    );
    assert_eq!(driver.usage().memory_commands, usage.memory_commands);
    assert_eq!(read(&memory, vertex.address, 16), bytes(&changed));
}

#[test]
#[ignore = "requires a local Vulkan device"]
fn actual_readonly_vertex_and_pixel_buffers_change_coverage_color_and_zero_oob_after_fences() {
    let (mut owner, base) = original_owner();
    let mut memory = owner.pause().unwrap();
    let identity = memory.owner_id();
    shaders(&mut memory, base, true, true, 0);
    let vertex = CommandRange {
        address: base + 0x8000,
        size: 16,
    };
    let pixel_source = CommandRange {
        address: base + 0x9000,
        size: 16,
    };
    let plan = resource_plan(base, Some(vertex), Some(pixel_source));
    memory.write(base + 0x10000 - 16, &[0x9b; 16]).unwrap();
    memory
        .write(base + 0x10000 + TARGET_BYTES as u64, &[0xe7; 16])
        .unwrap();
    let first = execute_draw(
        &mut memory,
        &plan.draws()[0],
        &Limits::default(),
        &GpuLimits::default(),
        &[],
    )
    .unwrap();
    assert!(first.completion.owner() == identity && first.completion.submission() > 0);
    assert_eq!(first.written_bytes, TARGET_BYTES);
    let first_pixels = read(&memory, base + 0x10000, TARGET_BYTES);
    assert_eq!(pixel(&first_pixels, 24, 24), RED);
    assert_eq!(pixel(&first_pixels, 36, 20), RED);
    assert_eq!(pixel(&first_pixels, 52, 20), SENTINEL);
    memory
        .write(vertex.address, &stage_bytes([0.5, 0.0, 0.0, 0.0]))
        .unwrap();
    memory
        .write(pixel_source.address, &stage_bytes([0.0, 1.0, 0.0, 1.0]))
        .unwrap();
    let second = execute_draw(
        &mut memory,
        &plan.draws()[0],
        &Limits::default(),
        &GpuLimits::default(),
        &[],
    )
    .unwrap();
    assert!(second.completion.submission() > first.completion.submission());
    let second_pixels = read(&memory, base + 0x10000, TARGET_BYTES);
    assert_eq!(pixel(&second_pixels, 24, 24), RED);
    assert_eq!(pixel(&second_pixels, 36, 20), GREEN);
    assert_eq!(pixel(&second_pixels, 52, 20), GREEN);
    assert_eq!(pixel(&second_pixels, 56, 56), SENTINEL);
    // A real translated load past the descriptor's own extent returns zero;
    // the host does not provide replacement colors or a wider resource view.
    shaders(&mut memory, base, true, true, 16);
    let third = execute_draw(
        &mut memory,
        &plan.draws()[0],
        &Limits::default(),
        &GpuLimits::default(),
        &[],
    )
    .unwrap();
    assert!(third.completion.submission() > second.completion.submission());
    let third_pixels = read(&memory, base + 0x10000, TARGET_BYTES);
    assert_eq!(pixel(&third_pixels, 24, 24), RED);
    assert_eq!(pixel(&third_pixels, 36, 20), [0; 4]);
    assert_eq!(pixel(&third_pixels, 52, 20), [0; 4]);
    assert_eq!(pixel(&third_pixels, 56, 56), SENTINEL);
    assert_eq!(read(&memory, base + 0x10000 - 16, 16), [0x9b; 16]);
    assert_eq!(
        read(&memory, base + 0x10000 + TARGET_BYTES as u64, 16),
        [0xe7; 16]
    );
    assert_eq!(
        read(&memory, vertex.address, 16),
        stage_bytes([0.5, 0.0, 0.0, 0.0])
    );
    assert_eq!(
        read(&memory, pixel_source.address, 16),
        stage_bytes([0.0, 1.0, 0.0, 1.0])
    );
}
