// SPDX-License-Identifier: GPL-2.0-only
//! Original indexed resources reuse the actual guest shader and tiled fixtures.
use super::*;
use crate::gpu::graphics::{Error, execute_draw_admitted, preflight_indices, preflight_target};
use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};
use std::cell::Cell;

fn indexed_commands(base: u64, index: u64, kind: u32) -> Vec<u32> {
    let mut original =
        commands_with_layout(base + 0x4000, base + 0x5000, base + 0x10000, 0x0106_c000);
    let first = original
        .iter()
        .position(|word| *word == 0xc001_2d00)
        .unwrap();
    original.truncate(first);
    set(&mut original, 0x79, 0x24b, &[0]); // Explicitly disable primitive restart.
    original.extend([
        0xc000_2a00,
        kind,
        0xc004_2700,
        3,
        index as u32,
        (index >> 32) as u32,
        3,
        0,
    ]);
    original
}

fn owner() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x40000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    {
        let mut memory = owner.pause().unwrap();
        memory
            .write(base + 0x4000, &bytes(&vertex_shader()))
            .unwrap();
        memory
            .write(base + 0x5000, &bytes(&pixel_shader()))
            .unwrap();
        memory.write(base + 0x10000, &[0x9b; 65536]).unwrap();
    }
    (owner, base)
}

#[test]
fn indexed_draw_late_span_permissions_owner_and_upload_quotas_refuse_before_gpu() {
    for case in 0..6 {
        let (mut owner, base) = owner();
        let (_foreign, foreign_base) = self::owner();
        let index = match case {
            0 | 1 => base + 0xbffc, // Full6B read crosses the final index page.
            2 => foreign_base + 0x8000,
            4 => base + 0x4000, // Valid read/read overlap with actual VS bytes.
            _ => base + 0x8000,
        };
        let encoded = bytes(&indexed_commands(base, index, 0));
        let plan = plan_commands(
            &[&encoded],
            GraphicsState::empty(),
            &CommandLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.draws().len(), 1);
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
        let admitted = Cell::new(false);
        let gpu = GpuLimits {
            max_host_bytes: if case == 3 {
                2 * TARGET_BYTES as u64 + 5
            } else {
                GpuLimits::default().max_host_bytes
            },
            ..Default::default()
        };
        let command_sources = if case == 5 {
            vec![CommandRange {
                address: index,
                size: 6,
            }]
        } else {
            Vec::new()
        };
        let result = execute_draw_admitted(
            &mut memory,
            &plan.draws()[0],
            &Limits::default(),
            &gpu,
            &command_sources,
            |_, _, _| {
                admitted.set(true);
                Err(Error::Limit)
            },
        );
        assert!(result.is_err());
        assert_eq!(admitted.get(), matches!(case, 4 | 5));
        if matches!(case, 3..=5) {
            assert!(matches!(result, Err(Error::Limit)));
        }
        assert_eq!(read(&memory, base + 0x10000, 65536), [0x9b; 65536]);
    }
}

#[test]
fn indexed_target_alias_checks_use_physical_backing_and_allow_shared_read_sources() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base + 0x18000,
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
    for (address, writable) in [(base, false), (base + 0x10000, true)] {
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
    memory.write(base, &[0x3b; 16]).unwrap();
    let indices = CommandRange {
        address: base,
        size: 6,
    };
    let alias = NativeRange {
        address: base + 0x10000,
        size: 0x4000,
    };
    assert!(matches!(
        preflight_indices(&memory, indices, alias, 4096),
        Err(Error::Memory(
            pentax_platform::native_memory::NativeMemoryError::Overlap
        ))
    ));
    let target = NativeRange {
        address: base + 0x18000,
        size: 64,
    };
    assert!(preflight_target(&memory, target, &[indices], &[indices], 4096).is_ok());
    assert!(preflight_indices(&memory, indices, target, 4096).is_ok());
    assert!(matches!(
        preflight_indices(&memory, indices, target, 0),
        Err(Error::Limit)
    ));
    assert!(memory.preflight_gpu_write(base, 6).is_err()); // Read-only GPU source.
    assert_eq!(read(&memory, base, 16), [0x3b; 16]);
}

#[test]
#[ignore = "requires a local Vulkan device"]
fn actual_indexed_u16_u32_draws_observe_changed_indices_and_preserve_tiled_load_padding() {
    use crate::gpu::video::{
        ImageLayout, PresentationLimits, SurfaceLayout, detile_32, retile_32, retile_32_into,
    };
    for kind in [0, 1] {
        let (mut owner, base) = owner();
        let index = base + 0x8000;
        owner
            .protect(
                index,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(true, false),
            )
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let layout = ImageLayout {
            kind: SurfaceLayout::RenderTarget64Kb,
            width: 64,
            height: 64,
            pitch: 128,
        };
        let conversion = PresentationLimits::default();
        let mut work = 0;
        let mut baseline = vec![0x9b; 65536];
        retile_32_into(
            &SENTINEL.repeat(WIDTH * HEIGHT),
            layout,
            &mut baseline,
            &conversion,
            &mut work,
        )
        .unwrap();
        let active = retile_32(
            &vec![255; TARGET_BYTES],
            layout,
            baseline.capacity(),
            &conversion,
            &mut work,
        )
        .unwrap();
        let target = base + 0x10000;
        memory.write(target, &baseline).unwrap();
        let encoded = bytes(&indexed_commands(base, index, kind));
        memory.write(base + 0x1000, &encoded).unwrap();
        let plan = plan_commands(
            &[&encoded],
            GraphicsState::empty(),
            &CommandLimits::default(),
        )
        .unwrap();
        assert_eq!(plan.draws().len(), 1);
        let mut serial = 0;
        for (case, values) in [[0_u32, 0, 0], [0, 1, 3]].into_iter().enumerate() {
            let indices: Vec<u8> = if kind == 0 {
                values
                    .into_iter()
                    .flat_map(|value| (value as u16).to_le_bytes())
                    .collect()
            } else {
                values.into_iter().flat_map(u32::to_le_bytes).collect()
            };
            // Only these source bytes change; the same plan, shaders and
            // target are reused. VertexIndex3 is valid even though count=3.
            memory.write(index, &indices).unwrap();
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
            assert!(
                report.gpu.commands > 0
                    && report.gpu.device_bytes > 0
                    && report.gpu.shader_words > 0
            );
            assert_eq!(report.written_bytes, 65536);
            assert_eq!(report.completion.owner(), memory.owner_id());
            assert_eq!(
                report.completion.layout(),
                TargetLayout::RenderTarget64KbRgba8
            );
            assert_eq!(
                report.completion.target(),
                NativeRange {
                    address: target,
                    size: 65536
                }
            );
            assert!(report.completion.submission() > serial);
            serial = report.completion.submission();
            let tiled = read(&memory, target, 65536);
            for (at, byte) in tiled.iter().enumerate() {
                if active[at] == 0 {
                    assert_eq!(*byte, baseline[at]);
                }
            }
            let visible =
                detile_32(&tiled, layout, tiled.capacity(), &conversion, &mut work).unwrap();
            if case == 0 {
                assert_eq!(tiled, baseline);
            }
            assert_eq!(
                pixel(&visible, 36, 24),
                if case == 0 { SENTINEL } else { RED }
            );
            assert_eq!(pixel(&visible, 24, 36), SENTINEL);
            assert_eq!(pixel(&visible, 56, 56), SENTINEL);
            assert_eq!(read(&memory, index, indices.len()), indices);
        }
    }
}

#[test]
fn actual_0600_encoder_selects_owned_index_width_for_real_shader_and_index_preparation() {
    use crate::gpu::driver::{CommandBuilderAction, Driver, Function};
    for (kind, index_type) in [
        (0, pentax_gpu::graphics::IndexType::U16),
        (1, pentax_gpu::graphics::IndexType::U32),
    ] {
        let (mut owner, base) = owner();
        let mut memory = owner.pause().unwrap();
        let index = base + 0x8000;
        let command = base + 0x6000;
        let buffer = base + 0x100;
        let indices = if kind == 0 {
            [0u16, 1, 2]
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        } else {
            [0u32, 1, 2]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>()
        };
        memory.write(index, &indices).unwrap();
        for flag in [0, 1] {
            let mut descriptor = [0; 56];
            descriptor[16..24].copy_from_slice(&command.to_le_bytes());
            descriptor[24..32].copy_from_slice(&(command + 12).to_le_bytes());
            memory.write(buffer, &descriptor).unwrap();
            let mut driver = Driver::new(Default::default());
            assert!(
                matches!(driver.begin_command_builder(&mut memory,Function::AgcDcbSetIndexSize0600,[buffer,kind,3,flag,0,0],None).unwrap(),CommandBuilderAction::Returned(actual) if actual==command)
            );
            let mut words = commands(base + 0x4000, base + 0x5000, base + 0x10000);
            let first = words.iter().position(|word| *word == 0xc0012d00).unwrap();
            words.truncate(first);
            set(&mut words, 0x79, 0x24b, &[0]);
            let mut encoded = bytes(&words);
            encoded.extend(read(&memory, command, 12));
            encoded.extend(bytes(&[
                0xc0042700,
                3,
                index as u32,
                (index >> 32) as u32,
                3,
                0,
            ]));
            let plan = plan_commands(
                &[&encoded],
                GraphicsState::empty(),
                &CommandLimits::default(),
            )
            .unwrap();
            assert_eq!(plan.draws().len(), 1);
            let prepared = pentax_gpu::graphics::prepare_draw(
                &plan.draws()[0],
                &crate::gpu::Memory(&memory),
                &Default::default(),
            )
            .unwrap();
            let payload = prepared.indices().unwrap();
            assert_eq!(payload.index_type(), index_type);
            assert_eq!(payload.bytes(), indices);
            assert_eq!(prepared.vertex().spirv()[0], 0x07230203);
            assert_eq!(prepared.pixel().spirv()[0], 0x07230203);
            assert_eq!(driver.usage().draws, 0);
            assert_eq!(read(&memory, index, indices.len()), indices);
            assert_eq!(read(&memory, base + 0x10000, 65536), [0x9b; 65536]);
        }
    }
}
