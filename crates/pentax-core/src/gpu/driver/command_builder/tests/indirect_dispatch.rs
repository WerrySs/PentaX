// SPDX-License-Identifier: GPL-2.0-only
//! Original producer-to-owned-compute consumers; no Vulkan device is used.
use super::*;

fn source() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x10000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    (owner, base)
}
fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn build(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    descriptor: u64,
    at: u64,
    function: Function,
    a: u64,
    b: u64,
) -> Vec<u8> {
    setup(memory, descriptor, at, 16, 0);
    memory.write(at - 4, &[0x5a; 24]).unwrap();
    assert_eq!(
        returned(
            driver
                .begin_command_builder(memory, function, [descriptor, a, b, 0, 0, 0], None)
                .unwrap()
        ),
        at
    );
    let size = if function == Function::AgcDcbDispatchIndirect {
        12
    } else {
        16
    };
    assert_eq!(read(memory, at - 4, 4), [0x5a; 4]);
    assert_eq!(read(memory, at + size, 4), [0x5a; 4]);
    read(memory, at, size as usize)
}
fn submit(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    stream: &[u8],
) -> Result<u64, Failure> {
    memory.write(base + 0x1000, stream).unwrap();
    memory
        .write(base + 0x100, &(base + 0x1000).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x108, &(stream.len() as u32 / 4).to_le_bytes())
        .unwrap();
    driver.submit(memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0])
}

#[test]
fn indirect_builders_capture_arguments_across_real_grow_and_preserve_source_null_boundaries() {
    let (mut owner, base) = source();
    let mut memory = owner.pause().unwrap();
    let old = base + 0x4000;
    let moved = base + 0x5000;
    for (function, a, b, expected) in [
        (
            Function::AgcAcbDispatchIndirect,
            0xfedc_ba98_7654_3213,
            u64::MAX,
            vec![0xc002_1600, 0x7654_3213, 0xfedc_ba98, 0xa079],
        ),
        (
            Function::AgcDcbDispatchIndirect,
            0xffff_ffff_1234_567b,
            u64::MAX,
            vec![0xc001_1600, 0x1234_567b, 0xa079],
        ),
        (
            Function::AgcDcbSetBaseIndirectArgs,
            0xffff_ffff_8000_0003,
            0xfedc_ba98_7654_321f,
            vec![0xc002_1106, 1, 0x7654_3218, 0xfedc_ba98],
        ),
    ] {
        let words = expected.len() as u64;
        let mut driver = Driver::new(Default::default());
        setup(&mut memory, base, old, 3, base + 0x80);
        memory.write(old, &[0x69; 20]).unwrap();
        let CommandBuilderAction::Grow(pending) = driver
            .begin_command_builder(&mut memory, function, [base, a, b, 0, 0, 0], None)
            .unwrap()
        else {
            panic!("actual grow");
        };
        assert_eq!(pending.args(), [base, words + 3, 0x7123]);
        // Descriptor migration is read after the actual callback. All call
        // scalars were captured before it; output is still just packet data.
        setup(&mut memory, base, moved, words + 3, 0);
        memory.write(moved - 4, &[0x96; 24]).unwrap();
        assert_eq!(
            returned(
                driver
                    .complete_command_builder(&mut memory, pending, 0x101)
                    .unwrap()
            ),
            moved
        );
        assert_eq!(read(&memory, moved, words as usize * 4), bytes(&expected));
        assert_eq!(read(&memory, moved - 4, 4), [0x96; 4]);
        assert_eq!(read(&memory, moved + words * 4, 4), [0x96; 4]);
        assert_eq!(read(&memory, old, 20), [0x69; 20]);
        assert_eq!(
            read(&memory, base + 16, 8),
            (moved + words * 4).to_le_bytes()
        );
        for null_buffer in [true, false] {
            let mut driver = Driver::new(Default::default());
            setup(&mut memory, base, old, 3, 0);
            let result = driver.begin_command_builder(
                &mut memory,
                function,
                [if null_buffer { 0 } else { base }, a, b, 0, 0, 0],
                None,
            );
            if function == Function::AgcAcbDispatchIndirect {
                assert_eq!(returned(result.unwrap()), 0);
            } else {
                assert!(matches!(result, Err(Failure::UnsupportedQueue)));
            }
        }
    }
}

#[test]
fn encoded_indirect_dispatches_consume_prior_owned_writes_in_static_and_live_array_streams() {
    for relative in [false, true] {
        for array in [false, true] {
            let (mut owner, base) = source();
            let mut memory = owner.pause().unwrap();
            let dimensions = base + 0x8000;
            // The prior source dimensions would attempt real nonzero work.
            // Only the actual earlier WRITE_DATA makes this a zero-work dispatch.
            memory.write(dimensions, &bytes(&[1, 1, 1])).unwrap();
            let mut driver = Driver::new(Default::default());
            let mut stream = bytes(&[
                0xc005_3700,
                0x100,
                dimensions as u32,
                (dimensions >> 32) as u32,
                0,
                7,
                9,
            ]);
            if relative {
                stream.extend(build(
                    &mut driver,
                    &mut memory,
                    base,
                    base + 0x4000,
                    Function::AgcDcbSetBaseIndirectArgs,
                    1,
                    dimensions - 0x100,
                ));
            }
            if array {
                let table = base + 0x9000;
                memory.write(table, &bytes(&[0x20c, 17])).unwrap();
                stream.extend(bytes(&[
                    0xc003_6300,
                    table as u32,
                    (table >> 32) as u32,
                    0x8000_0000,
                    1,
                ]));
            }
            stream.extend(build(
                &mut driver,
                &mut memory,
                base,
                base + 0x5000,
                if relative {
                    Function::AgcDcbDispatchIndirect
                } else {
                    Function::AgcAcbDispatchIndirect
                },
                if relative { 0x100 } else { dimensions },
                0x8000,
            ));
            assert_eq!(submit(&mut driver, &mut memory, base, &stream), Ok(0));
            assert_eq!(read(&memory, dimensions, 12), bytes(&[0, 7, 9]));
            assert_eq!(driver.usage().dispatches, 1);
            assert_eq!(driver.usage().invocations, 0);
            assert_eq!(driver.usage().written_bytes, 12);
            assert!(!driver.poisoned);
            assert_eq!((driver.usage().draws, driver.usage().interrupts), (0, 0));
        }
    }
}

#[test]
fn emitted_dispatches_keep_live_dimension_permissions_and_distinct_receiver_profile_refusals() {
    let (mut owner, base) = source();
    // The 12B dimension range crosses into a page without GPU-read authority.
    // Encoding does not read it and does not grant authority to it.
    let dimensions = base + 0xfff8;
    let mut driver = Driver::new(Default::default());
    let plan = {
        let mut memory = owner.pause().unwrap();
        let encoded = build(
            &mut driver,
            &mut memory,
            base,
            base + 0x4000,
            Function::AgcAcbDispatchIndirect,
            dimensions,
            0x8000,
        );
        memory.write(base + 0x1000, &encoded).unwrap();
        compute::plan_commands(
            &compute::ComputeState::empty(),
            compute::CommandRange {
                address: base + 0x1000,
                size: encoded.len() as u64,
            },
            &crate::gpu::Memory(&memory),
            &Default::default(),
        )
        .unwrap()
    };
    let Some(compute::Command::Dispatch(dispatch)) = plan.command(0) else {
        panic!("actual dispatch");
    };
    {
        let memory = owner.pause().unwrap();
        assert!(matches!(
            compute::prepare_dispatch(dispatch, &crate::gpu::Memory(&memory), &Default::default()),
            Err(compute::Error::Memory(_))
        ));
        // A tiny preparation budget refuses before attempting the dimension read.
        let limited = compute::Limits {
            max_work: 11,
            ..Default::default()
        };
        assert!(matches!(
            compute::prepare_dispatch(dispatch, &crate::gpu::Memory(&memory), &limited),
            Err(compute::Error::Limit(_))
        ));
    }
    owner
        .map_at(base + 0x10000, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    {
        let mut memory = owner.pause().unwrap();
        memory.write(dimensions, &bytes(&[0, 2, 3])).unwrap();
        assert!(matches!(
            compute::prepare_dispatch(dispatch, &crate::gpu::Memory(&memory), &Default::default()),
            Err(compute::Error::Memory(_))
        ));
    }
    owner
        .protect(
            base + 0x10000,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let (product, usage) =
        compute::prepare_dispatch(dispatch, &crate::gpu::Memory(&memory), &Default::default())
            .unwrap()
            .into_parts();
    assert!(product.is_none());
    assert_eq!(usage.work, 12);
    // The same admitted packet resolves new dimensions, never saved values.
    memory.write(dimensions, &bytes(&[1, 2, 3])).unwrap();
    assert!(matches!(
        compute::prepare_dispatch(dispatch, &crate::gpu::Memory(&memory), &Default::default()),
        Err(compute::Error::Unsupported(_))
    ));
    // Encoder masks are not compute-profile promotion: mode0x41 is refused
    // before touching an otherwise invalid dimensions pointer.
    let encoded = build(
        &mut driver,
        &mut memory,
        base,
        base + 0x4000,
        Function::AgcAcbDispatchIndirect,
        4,
        0,
    );
    assert!(matches!(
        submit(&mut driver, &mut memory, base, &encoded),
        Err(Failure::Preparation(compute::Error::Unsupported(
            "wave32 dispatch initiator"
        )))
    ));
    let mut driver = Driver::new(Default::default());
    let base_packet = build(
        &mut driver,
        &mut memory,
        base,
        base + 0x4000,
        Function::AgcDcbSetBaseIndirectArgs,
        0,
        0,
    );
    assert!(submit(&mut driver, &mut memory, base, &base_packet).is_err());
    // The existing graphics profile still rejects SET_BASE and indirect draws.
    assert!(
        draw::plan_commands(
            &[&base_packet],
            draw::GraphicsState::empty(),
            &Default::default()
        )
        .is_err()
    );
}
