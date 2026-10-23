// SPDX-License-Identifier: GPL-2.0-only
//! Original indirect argument -> native shader/index/pixel consumers.
use super::*;
use crate::gpu::driver::{CommandBuilderAction, Driver, Function};
use pentax_gpu::graphics::{OrderedCommands, OrderedStep};

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
            .write(base + 0x4000, &bytes(&native_vertex()))
            .unwrap();
        memory
            .write(base + 0x5000, &bytes(&pixel_shader()))
            .unwrap();
        memory
            .write(base + 0x10000, &SENTINEL.repeat(WIDTH * HEIGHT))
            .unwrap();
    }
    (owner, base)
}
fn native_vertex() -> Vec<u32> {
    // Raw VertexIndex first consumes the actual signed SGPR0 patch. The
    // original native shader owns this add; Vulkan must not add it again.
    let mut words = vec![binary(0x25, 5, 0, 5)]; // V_ADD_NC_U32 v5,s0,v5.
    words.extend(
        vertex_shader()
            .into_iter()
            .filter(|word| *word != binary(3, 0, 0, 0)),
    );
    words.truncate(words.len() - 3); // Replace only the original final export/END.
    words.extend([
        unary(6, 6, 1),       // Actual firstInstance SGPR1 -> f32.
        binary(8, 6, 255, 6), // Horizontal shift = firstInstance * .125.
        0.125_f32.to_bits(),
        binary(3, 0, 262, 0),
    ]);
    words.extend(export(12, [0, 1, 2, 3]));
    words.push(0xbf81_0000);
    words
}
fn emit(
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    function: Function,
    a: u64,
    b: u64,
    count: usize,
) -> Vec<u32> {
    let at = base + 0x6000;
    let mut descriptor = [0; 56];
    descriptor[16..24].copy_from_slice(&at.to_le_bytes());
    descriptor[24..32].copy_from_slice(&(at + count as u64 * 4).to_le_bytes());
    memory.write(base + 0x100, &descriptor).unwrap();
    let mut driver = Driver::new(Default::default());
    assert!(matches!(driver.begin_command_builder(memory, function,
        [base + 0x100, a, b, 0, 0, 0], None).unwrap(),
        CommandBuilderAction::Returned(actual) if actual == at));
    read(memory, at, count * 4)
        .chunks_exact(4)
        .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}
fn stream(memory: &mut PausedNativeMemory<'_>, base: u64, args: u64) -> Vec<u32> {
    let mut words = commands(base + 0x4000, base + 0x5000, base + 0x10000);
    let first = words.iter().position(|word| *word == 0xc001_2d00).unwrap();
    words.truncate(first);
    set(&mut words, 0x79, 0x24b, &[0]);
    set(&mut words, 0x76, 0x8b, &[0x0003_0006]); // Three real user SGPRs.
    words.extend([
        0xc000_2a00,
        0,
        0xc001_2600,
        (base + 0x8000) as u32,
        ((base + 0x8000) >> 32) as u32,
        0xc000_1300,
        3,
    ]);
    words.extend(emit(
        memory,
        base,
        Function::AgcDcbSetBaseIndirectArgs,
        0,
        args,
        4,
    ));
    words.extend(emit(
        memory,
        base,
        Function::AgcDcbDrawIndexIndirect,
        0,
        7 | (1 << 19) | (2 << 14),
        5,
    ));
    words
}
fn resolve(
    memory: &PausedNativeMemory<'_>,
    base: u64,
    words: &[u32],
) -> pentax_gpu::graphics::Draw {
    let data = bytes(words);
    let mut live = OrderedCommands::prepare(
        &[&data],
        &[CommandRange {
            address: base + 0x1000,
            size: data.len() as u64,
        }],
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    assert!(matches!(
        live.next_step().unwrap(),
        Some(OrderedStep::IndirectDraw)
    ));
    let draw = live
        .resolve_indirect(&crate::gpu::Memory(memory))
        .unwrap()
        .unwrap();
    assert!(live.next_step().unwrap().is_none());
    draw
}

#[test]
fn actual_indirect_encoder_prepares_native_patches_signed_offsets_and_exact_index_span() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let args = base + 0x9000;
    memory
        .write(base + 0x8000, &bytes(&[0x0003_ffff, 0x0005_0004]))
        .unwrap();
    memory
        .write(args, &bytes(&[9, 2, 1, (-3i32) as u32, 5]))
        .unwrap();
    let words = stream(&mut memory, base, args);
    let draw = resolve(&memory, base, &words);
    let prepared = pentax_gpu::graphics::prepare_draw(
        &draw,
        &crate::gpu::Memory(&memory),
        &Default::default(),
    )
    .unwrap();
    assert_eq!((prepared.vertex_count(), prepared.instance_count()), (3, 2));
    assert_eq!(prepared.indices().unwrap().range().address, base + 0x8002);
    assert_eq!(prepared.indices().unwrap().bytes(), &[3, 0, 4, 0, 5, 0]);
    let offsets = prepared.native_indirect_offsets().unwrap();
    assert_eq!(
        (
            offsets.base_vertex(),
            offsets.first_instance(),
            offsets.first_index()
        ),
        (-3, 5, 1)
    );
    assert_eq!(
        (prepared.vertex_offset(), prepared.first_instance()),
        (0, 0)
    );
    assert!(
        !prepared
            .vertex()
            .manifest()
            .required_features()
            .draw_parameters()
    );
    // Values are actual source patch inputs in the compiled push manifest.
    let layout = prepared.vertex().manifest().push_data().unwrap();
    for (register, expected) in [(0, (-3i32) as u32), (1, 5)] {
        let index = layout
            .user_sgprs
            .iter()
            .position(|found| *found == register)
            .unwrap();
        let actual = u32::from_le_bytes(
            prepared.vertex_push_data()[index * 4..index * 4 + 4]
                .try_into()
                .unwrap(),
        );
        assert_eq!(actual, expected);
    }
    assert_eq!(
        read(&memory, base + 0x10000, TARGET_BYTES),
        SENTINEL.repeat(WIDTH * HEIGHT)
    );
}

#[test]
fn current_indirect_arguments_refuse_late_permissions_and_physical_output_alias_before_gpu() {
    use crate::gpu::graphics::{Error, execute_draw_admitted};
    use std::cell::Cell;
    for case in 0..4 {
        let (mut owner, base) = owner();
        let args = if case == 0 {
            base + 0xfff0
        } else {
            base + 0x9000
        };
        let words = {
            let mut memory = owner.pause().unwrap();
            if case == 3 {
                use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};
                for effect in [
                    NativeMemoryEffect::Unmap {
                        address: base + 0x8000,
                        size: 0x4000,
                    },
                    NativeMemoryEffect::Unmap {
                        address: base + 0x10000,
                        size: 0x4000,
                    },
                    NativeMemoryEffect::AllocatePhysical {
                        physical: 0,
                        size: 0x4000,
                    },
                    NativeMemoryEffect::MapPhysical {
                        address: base + 0x8000,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                    },
                    NativeMemoryEffect::MapPhysical {
                        address: base + 0x10000,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                    },
                ] {
                    memory
                        .apply_transaction(
                            NativeMemoryTransaction {
                                effect,
                                writes: &[],
                            },
                            &Default::default(),
                        )
                        .unwrap();
                }
            }
            memory.write(args, &bytes(&[3, 1, 0, 0, 0])).unwrap();
            memory.write(base + 0x8000, &[0; 6]).unwrap();
            let mut words = stream(&mut memory, base, args);
            if case == 3 {
                let at = words.iter().position(|word| *word == 0xc001_2600).unwrap();
                words[at + 1] = (base + 0x18000) as u32;
                words[at + 2] = ((base + 0x18000) >> 32) as u32;
                memory.write(base + 0x18000, &[0; 6]).unwrap();
            }
            words
        };
        if case == 0 {
            owner
                .protect(base + 0x10000, 0x4000, NativePermissions::READ_WRITE)
                .unwrap();
        }
        let mut memory = owner.pause().unwrap();
        if case == 0 {
            let data = bytes(&words);
            let mut live = OrderedCommands::prepare(
                &[&data],
                &[CommandRange {
                    address: base + 0x1000,
                    size: data.len() as u64,
                }],
                GraphicsState::empty(),
                &Default::default(),
            )
            .unwrap();
            assert!(matches!(
                live.next_step().unwrap(),
                Some(OrderedStep::IndirectDraw)
            ));
            assert!(matches!(
                live.resolve_indirect(&crate::gpu::Memory(&memory)),
                Err(pentax_gpu::graphics::Error::Memory(_))
            ));
            assert!(live.resolve_indirect(&crate::gpu::Memory(&memory)).is_err());
        } else {
            let draw = resolve(&memory, base, &words);
            let admitted = Cell::new(false);
            let limits = if case == 1 {
                Limits {
                    max_vertices: 2,
                    ..Default::default()
                }
            } else {
                Limits::default()
            };
            let result = execute_draw_admitted(
                &mut memory,
                &draw,
                &limits,
                &GpuLimits::default(),
                &[],
                |_, _, _| {
                    admitted.set(true);
                    Err(Error::Limit)
                },
            );
            assert!(result.is_err());
            if case == 3 {
                assert!(matches!(
                    result,
                    Err(Error::Memory(
                        pentax_platform::native_memory::NativeMemoryError::Overlap
                    ))
                ));
            }
            assert_eq!(admitted.get(), case == 2); // Actual full preparation reaches explicit no-GPU admission stop.
        }
    }
}

#[test]
#[ignore = "requires a local Vulkan device and an outer owning timeout gate"]
fn actual_indirect_native_shader_consumes_signed_base_first_instance_and_live_first_index() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let args = base + 0x9000;
    let indices = [0xffffu16, 3, 4, 5]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    memory.write(base + 0x8000, &indices).unwrap();
    memory.write(args, &[0; 20]).unwrap();
    let mut words = stream(&mut memory, base, args);
    // Populate the previously-zero argument payload in its actual ordered turn.
    let indirect = words.len() - 5;
    words.splice(
        indirect..indirect,
        [
            0xc007_3700,
            0x100,
            args as u32,
            (args >> 32) as u32,
            9,
            2,
            1,
            (-3i32) as u32,
            5,
        ],
    );
    memory.write(base + 0x1000, &bytes(&words)).unwrap();
    memory
        .write(base + 0x20, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x28, &(words.len() as u32).to_le_bytes())
        .unwrap();
    memory.write(base + 4, &0u32.to_le_bytes()).unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(driver.usage().draws, 1);
    assert_eq!(driver.usage().vertices, 6);
    assert_eq!(driver.usage().memory_commands, 1);
    assert_eq!(driver.usage().written_bytes, TARGET_BYTES as u64 + 20);
    let actual = read(&memory, base + 0x10000, TARGET_BYTES);
    assert_eq!(pixel(&actual, 44, 24), RED);
    assert_eq!(pixel(&actual, 24, 24), SENTINEL);
    assert_eq!(pixel(&actual, 60, 60), SENTINEL);
    assert_eq!(read(&memory, base + 0x8000, indices.len()), indices);
    assert_eq!(
        read(&memory, args, 20),
        bytes(&[9, 2, 1, (-3i32) as u32, 5])
    );
    println!(
        "agc_draw_index_indirect_gpu_available=true agc_draw_index_indirect_gpu_draws=1 agc_draw_index_indirect_gpu_instances=2"
    );
}

#[test]
#[ignore = "requires a local Vulkan device and an outer owning timeout gate"]
fn actual_indirect_native_v8_instances_consume_relative_ids_and_logical_first_instance_once() {
    use pentax_gpu::graphics::{InputInterface, VertexInstanceContract};

    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    // Use the actual native fetch convention: indices3,4,5 plus signed SGPR0=-3
    // select logical vertices0,1,2. Vulkan must not add that offset a second time.
    let mut vertex = vec![binary(0x25, 5, 0, 5)];
    vertex.extend(
        vertex_shader()
            .into_iter()
            .filter(|word| *word != binary(3, 0, 0, 0)),
    );
    vertex.truncate(vertex.len() - 3); // Replace only final export/END.
    vertex.extend([
        binary(8, 0, 240, 0), // x *= .5: each triangle is16 pixels wide.
        binary(8, 1, 240, 1), // y *= .5: each triangle is16 pixels high.
        binary(3, 0, 241, 0), // x -= .5.
        binary(3, 1, 241, 1), // y -= .5.
        unary(6, 6, 1),       // Logical firstInstance SGPR1 -> f32.
        binary(8, 6, 255, 6),
        0.125_f32.to_bits(),
        binary(3, 0, 262, 0), // x += firstInstance*.125.
        unary(6, 7, 264),     // Actual live relative-instance v8 -> f32.
        binary(3, 1, 263, 1), // y += relative instance0/1.
    ]);
    vertex.extend(export(12, [0, 1, 2, 3]));
    vertex.push(0xbf81_0000);
    memory.write(base + 0x4000, &bytes(&vertex)).unwrap();

    let args = base + 0x9000;
    let indices = [0xffffu16, 3, 4, 5]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let arguments = bytes(&[9, 2, 1, (-3i32) as u32, 5]);
    memory.write(base + 0x8000, &indices).unwrap();
    memory.write(args, &arguments).unwrap();
    memory.write(base + 0x10000 - 16, &[0x9b; 16]).unwrap();
    memory
        .write(base + 0x10000 + TARGET_BYTES as u64, &[0xe7; 16])
        .unwrap();
    let draw_words = stream(&mut memory, base, args);
    {
        // This is a genuine compile from the same sealed draw, current guest
        // shader and arguments used below. No context/feature setter or host
        // SPIR-V module is substituted. The driver compiles through this same
        // preparation path again at its actual ordered execution turn.
        let draw = resolve(&memory, base, &draw_words);
        let prepared = pentax_gpu::graphics::prepare_draw(
            &draw,
            &crate::gpu::Memory(&memory),
            &Default::default(),
        )
        .unwrap();
        let manifest = prepared.vertex().manifest();
        assert_eq!(
            manifest.vertex_instance_contract(),
            VertexInstanceContract::NativeIndirectFirstInstanceZero,
        );
        assert!(manifest.inputs().contains(&InputInterface::InstanceIndex));
        assert!(!manifest.inputs().contains(&InputInterface::BaseInstance));
        assert!(!manifest.required_features().draw_parameters());
        assert_eq!(prepared.validate_native_instance_contract(), Ok(()));
        assert_eq!(
            (prepared.vertex_offset(), prepared.first_instance()),
            (0, 0)
        );
        assert_eq!((prepared.vertex_count(), prepared.instance_count()), (3, 2));
        let offsets = prepared.native_indirect_offsets().unwrap();
        assert_eq!(
            (
                offsets.base_vertex(),
                offsets.first_instance(),
                offsets.first_index()
            ),
            (-3, 5, 1),
        );
        assert_eq!(prepared.indices().unwrap().bytes(), &[3, 0, 4, 0, 5, 0]);
        let layout = manifest.push_data().unwrap();
        for (register, expected) in [(0, (-3i32) as u32), (1, 5)] {
            let index = layout
                .user_sgprs
                .iter()
                .position(|found| *found == register)
                .unwrap();
            assert_eq!(
                &prepared.vertex_push_data()[index * 4..index * 4 + 4],
                &expected.to_le_bytes(),
            );
        }
    }
    assert_eq!(
        read(&memory, base + 0x10000, TARGET_BYTES),
        SENTINEL.repeat(WIDTH * HEIGHT),
    );
    // Reset the argument bytes before submission. Only the real earlier
    // WRITE_DATA action supplies the subsequent live20-byte indirect read.
    memory.write(args, &[0; 20]).unwrap();
    let mut words = draw_words;
    let indirect = words.len() - 5;
    words.splice(
        indirect..indirect,
        [
            0xc007_3700,
            0x100,
            args as u32,
            (args >> 32) as u32,
            9,
            2,
            1,
            (-3i32) as u32,
            5,
        ],
    );
    memory.write(base + 0x1000, &bytes(&words)).unwrap();
    memory
        .write(base + 0x20, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x28, &(words.len() as u32).to_le_bytes())
        .unwrap();
    memory.write(base + 4, &0u32.to_le_bytes()).unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0],
        ),
        Ok(0),
    );
    assert_eq!(driver.usage().draws, 1);
    assert_eq!(driver.usage().vertices, 6);
    assert_eq!(driver.usage().memory_commands, 1);
    assert_eq!(driver.usage().written_bytes, TARGET_BYTES as u64 + 20);
    let actual = read(&memory, base + 0x10000, TARGET_BYTES);
    // NDC x[-.125,.375] maps to pixels28..44 after logical SGPR1=5.
    // Relative v8=0/1 gives y8..24 and40..56. These interior points are
    // separated by an untouched gap; neither duplicated instance0 nor use of
    // the logical firstInstance5 as the instance ID can satisfy both.
    assert_eq!(pixel(&actual, 31, 14), RED);
    assert_eq!(pixel(&actual, 31, 46), RED);
    for (x, y) in [(31, 28), (31, 36), (24, 14), (24, 46), (48, 14), (60, 60)] {
        assert_eq!(pixel(&actual, x, y), SENTINEL);
    }
    assert_eq!(read(&memory, base + 0x10000 - 16, 16), [0x9b; 16]);
    assert_eq!(
        read(&memory, base + 0x10000 + TARGET_BYTES as u64, 16),
        [0xe7; 16],
    );
    assert_eq!(read(&memory, base + 0x8000, indices.len()), indices);
    assert_eq!(read(&memory, args, 20), arguments);
    println!(
        "agc_native_instance_gpu_available=true agc_native_instance_gpu_draws=1 agc_native_instance_gpu_instances=2"
    );
}
