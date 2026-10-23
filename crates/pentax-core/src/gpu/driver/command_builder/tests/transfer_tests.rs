// SPDX-License-Identifier: GPL-2.0-only
//! Original current-read, real-prefix and full-output transfer consumers.
use super::*;
use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};

fn source() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x20000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    (owner, base)
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn descriptor(memory: &mut PausedNativeMemory<'_>, base: u64, words: &[u32]) {
    memory.write(base + 0x1000, &bytes(words)).unwrap();
    memory
        .write(base + 0x100, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x108, &(words.len() as u32).to_le_bytes())
        .unwrap();
}
fn write(destination: u64, values: &[u32]) -> Vec<u32> {
    let mut words = vec![
        0xc000_3700 | (values.len() as u32 + 2) << 16,
        0x100,
        destination as u32,
        (destination >> 32) as u32,
    ];
    words.extend(values);
    words
}
fn copy(source: u64, destination: u64, wide: bool) -> Vec<u32> {
    vec![
        0xc004_4000,
        0x101 | if wide { 1 << 16 } else { 0 },
        source as u32,
        (source >> 32) as u32,
        destination as u32,
        (destination >> 32) as u32,
    ]
}
fn dma(source: u64, destination: u64, size: u32, fill: bool) -> Vec<u32> {
    vec![
        0xc005_5000,
        if fill { 0x4000_0000 } else { 0 },
        source as u32,
        (source >> 32) as u32,
        destination as u32,
        (destination >> 32) as u32,
        size,
    ]
}
fn submit(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
) -> Result<u64, Failure> {
    driver.submit(memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0])
}

#[test]
fn transfers_read_actual_prior_writes_feed_live_arrays_and_repeat_whole_immediate_words() {
    for graphics in [false, true] {
        for array in [false, true] {
            let (mut owner, base) = source();
            let mut memory = owner.pause().unwrap();
            let source = base + 0x8000;
            let middle = base + 0x9000;
            let target = base + 0xa000;
            let fill = base + 0xb000;
            memory.write(source, &[0xa5; 8]).unwrap();
            memory.write(target - 4, &[0x69; 16]).unwrap();
            memory.write(fill - 4, &[0x96; 24]).unwrap();
            let register = if graphics { 0x318 } else { 0x20c };
            let mut words = write(source, &[register, 17]);
            words.extend(copy(source, middle, true));
            words.extend(dma(middle, target, 8, false));
            words.extend(dma(0xffff_ffff_0123_4567, fill, 16, true));
            // Normalized immediate10 stores only low32; high source bits are
            // not an address and must not be dereferenced or copied as64 bits.
            memory.write(base + 0xd000 - 4, &[0x5a; 12]).unwrap();
            words.extend([
                0xc004_4000,
                0x105,
                0x89ab_cdef,
                u32::MAX,
                (base + 0xd000) as u32,
                ((base + 0xd000) >> 32) as u32,
            ]);
            if array {
                words.extend([
                    if graphics { 0xc003_9f00 } else { 0xc003_6300 },
                    target as u32,
                    (target >> 32) as u32,
                    0x8000_0000,
                    1,
                ]);
            }
            words.extend([
                0xc005_3c00,
                0x13,
                (target + 4) as u32,
                ((target + 4) >> 32) as u32,
                17,
                u32::MAX,
                0,
            ]);
            let mut driver = Driver::new(Default::default());
            if graphics {
                // Actual nested command capture retains BOTH root and child.
                memory.write(base + 0x2000, &bytes(&words)).unwrap();
                let root = [
                    0xc002_3f00,
                    (base + 0x2000) as u32,
                    ((base + 0x2000) >> 32) as u32,
                    0x0f20_0000 | words.len() as u32,
                ];
                memory.write(base + 0x1000, &bytes(&root)).unwrap();
                assert_eq!(
                    driver.submit_graphics(
                        &mut memory,
                        &[NativeRange {
                            address: base + 0x1000,
                            size: 16
                        }]
                    ),
                    Ok(0)
                );
            } else {
                descriptor(&mut memory, base, &words);
                assert_eq!(submit(&mut driver, &mut memory, base), Ok(0));
            }
            assert_eq!(read(&memory, middle, 8), bytes(&[register, 17]));
            assert_eq!(
                read(&memory, target - 4, 16),
                [
                    [0x69; 4].to_vec(),
                    bytes(&[register, 17]),
                    [0x69; 4].to_vec()
                ]
                .concat()
            );
            assert_eq!(
                read(&memory, fill - 4, 24),
                [
                    [0x96; 4].to_vec(),
                    bytes(&[0x0123_4567; 4]),
                    [0x96; 4].to_vec()
                ]
                .concat()
            );
            assert_eq!(
                read(&memory, base + 0xd000 - 4, 12),
                [
                    [0x5a; 4].to_vec(),
                    0x89ab_cdefu32.to_le_bytes().to_vec(),
                    [0x5a; 4].to_vec()
                ]
                .concat()
            );
            assert_eq!(driver.usage().written_bytes, 44);
            assert_eq!(driver.usage().memory_commands, 6);
            assert_eq!(
                (
                    driver.usage().draws,
                    driver.usage().dispatches,
                    driver.usage().interrupts
                ),
                (0, 0, 0)
            );
            if array && graphics {
                let expected = draw::plan_commands(
                    &[&bytes(&[0xc001_6900, register, 17])],
                    draw::GraphicsState::empty(),
                    &Default::default(),
                )
                .unwrap()
                .final_state();
                assert_eq!(driver.graphics, expected);
            } else if array {
                let state_words = [0xc001_7600, register, 17];
                memory.write(base + 0xc000, &bytes(&state_words)).unwrap();
                let expected = compute::plan_commands(
                    &compute::ComputeState::empty(),
                    compute::CommandRange {
                        address: base + 0xc000,
                        size: 12,
                    },
                    &crate::gpu::Memory(&memory),
                    &Default::default(),
                )
                .unwrap()
                .final_state();
                assert_eq!(driver.queues[0], expected);
                assert!(
                    driver.queues[1..]
                        .iter()
                        .all(|state| *state == compute::ComputeState::empty())
                );
            }
            assert!(!driver.poisoned);
        }
    }
}

#[test]
fn transfer_late_backing_faults_preserve_prior_prefix_and_leave_current_output_atomic() {
    for case in 0..5 {
        let (mut owner, base) = source();
        let source = base + 0x8000;
        let target = match case {
            2 => base + 0x1fffc,
            3 => source,
            4 => base + 0x1000,
            _ => base + 0x10000,
        };
        {
            let mut memory = owner.pause().unwrap();
            memory.write(source, &[0x96; 8]).unwrap();
            if case != 3 && case != 4 {
                memory
                    .write(target, &[0x69; 8][..if case == 2 { 4 } else { 8 }])
                    .unwrap();
            }
            let mut words = write(base + 0x7000, &[17]);
            words.extend(dma(source, target, 8, false));
            descriptor(&mut memory, base, &words);
        }
        if case == 0 {
            owner
                .protect(
                    source,
                    0x4000,
                    NativePermissions::READ_WRITE.with_gpu(false, true),
                )
                .unwrap();
        }
        if case == 1 {
            owner
                .protect(
                    target,
                    0x4000,
                    NativePermissions::READ_WRITE.with_gpu(true, false),
                )
                .unwrap();
        }
        let mut memory = owner.pause().unwrap();
        let before = read(&memory, target, if case == 2 { 4 } else { 8 });
        let mut driver = Driver::new(Default::default());
        assert_eq!(submit(&mut driver, &mut memory, base), Err(Failure::Memory));
        assert_eq!(read(&memory, target, before.len()), before);
        assert_eq!(read(&memory, base + 0x7000, 4), 17_u32.to_le_bytes());
        assert_eq!(driver.usage().written_bytes, 4);
        assert!(driver.poisoned);
        let usage = driver.usage();
        assert_eq!(
            submit(&mut driver, &mut memory, base),
            Err(Failure::Poisoned)
        );
        assert_eq!(driver.usage(), usage);
    }
}

#[test]
fn transfer_physical_aliases_and_captured_child_command_outputs_are_live_refusals() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base + 0x20000,
            0x10000,
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
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    memory.write(base + 0x800, &[0x96; 8]).unwrap();
    // Different virtual ranges refer to exactly the same physical bytes.
    let child = dma(base + 0x800, base + 0x10800, 8, false);
    memory.write(base + 0x22000, &bytes(&child)).unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit_graphics(
            &mut memory,
            &[NativeRange {
                address: base + 0x22000,
                size: child.len() as u64 * 4
            }]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, base + 0x10800, 8), [0x96; 8]);
    assert_eq!(driver.usage().written_bytes, 0);
    // Captured child bytes must also be excluded through a physical alias.
    let child = dma(0x0123_4567, base + 0x10800, 8, true);
    memory.write(base + 0x800, &bytes(&child)).unwrap();
    let root = [
        0xc002_3f00,
        (base + 0x800) as u32,
        ((base + 0x800) >> 32) as u32,
        0x0f20_0000 | child.len() as u32,
    ];
    memory.write(base + 0x22000, &bytes(&root)).unwrap();
    let before = read(&memory, base + 0x800, child.len() * 4);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit_graphics(
            &mut memory,
            &[NativeRange {
                address: base + 0x22000,
                size: 16
            }]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(read(&memory, base + 0x800, child.len() * 4), before);
    assert_eq!(driver.usage().written_bytes, 0);
    // Two complete granule-aligned virtual views of the same section make
    // a fully mapped output span that repeats physical bytes internally.
    let mut repeated_owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let repeated_base = repeated_owner.reserved_ranges().next().unwrap().address;
    repeated_owner
        .map_at(
            repeated_base + 0x20000,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut repeated_memory = repeated_owner.pause().unwrap();
    repeated_memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: 0x10000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    for address in [repeated_base, repeated_base + 0x10000] {
        repeated_memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x10000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    let repeated = dma(0x0123_4567, repeated_base, 0x20000, true);
    repeated_memory
        .write(repeated_base + 0x22000, &bytes(&repeated))
        .unwrap();
    let before = read(&repeated_memory, repeated_base, 0x20000);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit_graphics(
            &mut repeated_memory,
            &[NativeRange {
                address: repeated_base + 0x22000,
                size: 28
            }]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(read(&repeated_memory, repeated_base, 0x20000), before);
    assert_eq!(driver.usage().written_bytes, 0);
}

#[test]
fn transfer_structural_output_and_staging_admission_precede_all_effects_and_zero_touches_nothing() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let label = base + 0x7000;
    for refusal in 0..4 {
        memory.write(label, &[0xa5; 4]).unwrap();
        let mut words = write(label, &[17]);
        words.extend(if refusal == 0 {
            dma(base + 0x8000, base + 0xa000, 6, false)
        } else {
            dma(0x0123_4567, base + 0xa000, 64, true)
        });
        descriptor(&mut memory, base, &words);
        let mut limits = Limits::default();
        if refusal == 1 {
            limits.max_memory_write_bytes = 67;
        }
        if refusal == 2 {
            limits.commands.max_write_bytes = 67;
        }
        if refusal == 3 {
            limits.commands.max_host_bytes = 63;
        }
        let mut driver = Driver::new(limits);
        assert!(submit(&mut driver, &mut memory, base).is_err());
        assert_eq!(read(&memory, label, 4), [0xa5; 4]);
        assert_eq!(driver.usage().written_bytes, 0);
    }
    descriptor(
        &mut memory,
        base,
        &[
            0xc005_5000,
            u32::MAX,
            u32::MAX,
            u32::MAX,
            1,
            u32::MAX,
            0xfc00_0000,
        ],
    );
    let mut driver = Driver::new(Limits {
        max_memory_write_bytes: 0,
        ..Default::default()
    });
    assert_eq!(submit(&mut driver, &mut memory, base), Ok(0));
    assert_eq!(
        (
            driver.usage().written_bytes,
            driver.usage().interrupts,
            driver.usage().dispatches,
            driver.usage().draws
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(driver.usage().memory_commands, 1);
    assert!(!driver.poisoned);
}

#[test]
fn transfer_grow_retains_captured_stack_fields_and_source_ordered_publication() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let up = base + 0x4000;
    let stack = base + 0x300;
    let roles = [
        Function::AgcAcbCopyData,
        Function::AgcDcbCopyData,
        Function::AgcAcbDmaData,
        Function::AgcDcbDmaData,
    ];
    for function in roles {
        let copy = matches!(
            function,
            Function::AgcAcbCopyData | Function::AgcDcbCopyData
        );
        let dcb = function == Function::AgcDcbDmaData;
        let args = if dcb {
            [base, 3, 3, 2, 0x1234_5678_0000_9000, 0x14]
        } else {
            [
                base,
                3,
                2,
                0x1234_5678_0000_9000,
                if copy { 5 } else { 0x14 },
                1,
            ]
        };
        let extras = if copy {
            [0x89ab_cdef_0123_4567, 1, 1, 0, 0, 0]
        } else if dcb {
            [1, 0x89ab_cdef_0123_4567, 0x0400_0010, 1, 1, 1]
        } else {
            [0x89ab_cdef_0123_4567, 0x0400_0010, 1, 1, 0, 0]
        };
        setup(&mut memory, base, up, 0, 1);
        memory
            .write(
                stack + 8,
                &extras
                    .into_iter()
                    .flat_map(u64::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let mut driver = Driver::new(Default::default());
        let CommandBuilderAction::Grow(pending) = driver
            .begin_command_builder(&mut memory, function, args, Some(stack))
            .unwrap()
        else {
            panic!("real growth continuation");
        };
        let count = if copy { 6 } else { 7 };
        assert_eq!(pending.args()[1], count + 3);
        memory.write(stack + 8, &[0xff; 48]).unwrap();
        setup(&mut memory, base, up, count + 3, 1);
        assert_eq!(
            returned(
                driver
                    .complete_command_builder(&mut memory, pending, 1)
                    .unwrap()
            ),
            up
        );
        assert_eq!(read(&memory, base + 16, 8), (up + count * 4).to_le_bytes());
        let expected = match function {
            Function::AgcAcbCopyData => vec![
                0xc004_4000,
                0x0411_2305,
                0x0123_4567,
                0x89ab_cdef,
                0x9000,
                0x1234_5678,
            ],
            Function::AgcDcbCopyData => vec![
                0xc004_4000,
                0x4411_2102,
                0x0123_4567,
                0x89ab_cdef,
                0x9000,
                0x1234_5678,
            ],
            Function::AgcAcbDmaData => vec![
                0xc005_5000,
                0x0430_2000,
                0x30174,
                0,
                0x9000,
                0x1234_5678,
                0xc400_0010,
            ],
            _ => vec![
                0xc005_5000,
                0x8430_2001,
                0x30174,
                0,
                0x9000,
                0x1234_5678,
                0xc400_0010,
            ],
        };
        assert_eq!(read(&memory, up, count as usize * 4), bytes(&expected));
    }
}
