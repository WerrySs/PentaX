// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::gpu::driver::{Driver, Limits};
use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryLimits, NativePermissions};
pub(super) const PAGE: u64 = 16384;

pub(super) fn backing() -> (NativeAddressSpace, u64) {
    let mut memory =
        NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 4 * PAGE).unwrap();
    let base = memory.reserved_ranges().next().unwrap().address;
    memory
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    (memory, base)
}

#[test]
fn video_buffer_attributes_capture_both_stack_arguments_before_overlapping_output() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    memory.write(base + 8, &[0xa5; 96]).unwrap();
    // The two source stack words overlap the destination's first16bytes.
    memory
        .write(base + 16, &0xdead_beef_1234_5678_u64.to_le_bytes())
        .unwrap();
    memory
        .write(base + 24, &0xfedc_ba98_7654_3210_u64.to_le_bytes())
        .unwrap();
    let mut driver = Driver::new(Default::default());
    driver
        .submit_with_stack(
            &mut memory,
            Function::VideoSetBufferAttribute2,
            [
                base + 16,
                0x1122_3344_5566_7788,
                0xdead_beef_0000_0002,
                0xdead_beef_0000_0780,
                0xdead_beef_0000_0438,
                0x8000_0000_0000_0001,
            ],
            Some(base + 8),
        )
        .unwrap();
    let mut expected = Vec::new();
    for word in [0_u32, 2, 0, 1920, 1080, 0] {
        expected.extend(word.to_le_bytes());
    }
    for word in [
        0x8000_0000_0000_0001_u64,
        0x1122_3344_5566_7788,
        0xfedc_ba98_7654_3210,
    ] {
        expected.extend(word.to_le_bytes());
    }
    for word in [0x1234_5678_u32, 0] {
        expected.extend(word.to_le_bytes());
    }
    expected.extend([0; 24]);
    let mut bytes = [0; 96];
    memory.read(base + 8, &mut bytes).unwrap();
    assert_eq!(&bytes[8..88], expected);
    assert_eq!(&bytes[..8], &[0xa5; 8]);
    assert_eq!(&bytes[88..], &[0xa5; 8]);
    driver
        .submit(
            &mut memory,
            Function::VideoAddBufferAttributeOption,
            [base + 16, 0x4000_0000_0000_0002, 0, 0, 0, 0],
        )
        .unwrap();
    expected[24..32].copy_from_slice(&0xc000_0000_0000_0003_u64.to_le_bytes());
    memory.read(base + 8, &mut bytes).unwrap();
    assert_eq!(&bytes[8..88], expected);
    assert_eq!(driver.video_port(1), None);
    assert_eq!(driver.usage().dispatches, 0);
}

#[test]
fn video_attribute_argument_and_output_refusals_do_not_publish_partial_headers() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    memory.write(base + 8, &[0; 16]).unwrap();
    let mut driver = Driver::new(Default::default());
    for (stack, destination, expected) in [
        (None, base + 256, Failure::UnsupportedVideo),
        (Some(u64::MAX), base + 256, Failure::Memory),
        (Some(base + PAGE - 16), base + 256, Failure::Memory),
        (Some(base), base + PAGE - 79, Failure::Memory),
    ] {
        let size = if destination == base + 256 { 80 } else { 79 };
        memory.write(destination, &vec![0xa5; size]).unwrap();
        assert_eq!(
            driver.submit_with_stack(
                &mut memory,
                Function::VideoSetBufferAttribute2,
                [destination, 0, 0, 0, 0, 0],
                stack
            ),
            Err(expected)
        );
        let mut bytes = vec![0; size];
        memory.read(destination, &mut bytes).unwrap();
        assert_eq!(bytes, vec![0xa5; size]);
    }
    let mut limited = Driver::new(Limits {
        max_preparation_work: 255,
        ..Default::default()
    });
    assert_eq!(
        limited.submit_with_stack(
            &mut memory,
            Function::VideoSetBufferAttribute2,
            [base + 256, 0, 0, 0, 0, 0],
            Some(base)
        ),
        Err(Failure::Limit)
    );
    // AddOption accesses exactly its8-byte field, never an entire80-byte header.
    let field = base + PAGE - 8;
    memory
        .write(field, &0x8000_0000_0000_0001_u64.to_le_bytes())
        .unwrap();
    driver
        .submit(
            &mut memory,
            Function::VideoAddBufferAttributeOption,
            [field - 24, 2, 0, 0, 0, 0],
        )
        .unwrap();
    let mut bytes = [0; 8];
    memory.read(field, &mut bytes).unwrap();
    assert_eq!(u64::from_le_bytes(bytes), 0x8000_0000_0000_0003);
}

#[test]
fn video_output_status_uses_admitted_display_policy_and_live_mode_without_counting_frames() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    for (attribute, width, height, resolution) in [
        (None, 1920, 1080, 2_u32),
        (Some(0), 1920, 1080, 2),
        (Some(4), 1920, 1080, 1),
        (Some(4), 3840, 1080, 2),
        (Some(4), 1920, 2160, 2),
        (Some(8), 1920, 1080, 2),
    ] {
        for (follows, hz, mode, refresh) in [
            (false, 120, 1, 3_u64),
            (true, 118, 1, 3),
            (true, 119, 1, 13),
            (false, 119, 15, 13),
        ] {
            let mut driver = Driver::new(Limits {
                video_vblank_hz: hz,
                video_display: DisplayPolicy {
                    resolution: pentax_config::Resolution { width, height },
                    application_attribute3: attribute,
                    refresh_follows_vblank: follows,
                },
                ..Default::default()
            });
            driver
                .submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0])
                .unwrap();
            driver
                .submit(
                    &mut memory,
                    Function::VideoConfigureOutput,
                    [1, mode, 0, 0, 0, 0],
                )
                .unwrap();
            let before = driver.video_port(1);
            memory.write(base, &[0xa5; 64]).unwrap();
            assert_eq!(
                driver.submit(
                    &mut memory,
                    Function::VideoGetOutputStatus,
                    [1, base + 8, 0, 0, 0, 0]
                ),
                Ok(0)
            );
            let mut bytes = [0; 64];
            memory.read(base, &mut bytes).unwrap();
            let mut expected = [0_u8; 48];
            expected[..4].copy_from_slice(&resolution.to_le_bytes());
            expected[4..8].copy_from_slice(&1_u32.to_le_bytes());
            expected[8..16].copy_from_slice(&refresh.to_le_bytes());
            assert_eq!(&bytes[8..56], &expected);
            assert_eq!(&bytes[..8], &[0xa5; 8]);
            assert_eq!(&bytes[56..], &[0xa5; 8]);
            assert_eq!(driver.video_port(1), before);
            assert_eq!(driver.frame_state().boundaries, 0);
            assert_eq!(driver.usage().dispatches, 0);
        }
    }
}

#[test]
fn video_output_status_error_order_and_late_faults_preserve_exact_output_and_port() {
    let (mut owner, base) = backing();
    let mut driver = Driver::new(Default::default());
    {
        let mut memory = owner.pause().unwrap();
        memory.write(base + PAGE - 48, &[0xa5; 48]).unwrap();
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::VideoGetOutputStatus,
                [9, 0, 0, 0, 0, 0]
            ),
            Ok(INVALID_ADDRESS)
        );
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::VideoGetOutputStatus,
                [9, u64::MAX, 0, 0, 0, 0]
            ),
            Ok(INVALID_HANDLE)
        );
        driver
            .submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0])
            .unwrap();
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::VideoGetOutputStatus,
                [1, base + PAGE - 47, 0, 0, 0, 0]
            ),
            Err(Failure::Memory)
        );
        let mut tail = [0; 48];
        memory.read(base + PAGE - 48, &mut tail).unwrap();
        assert_eq!(tail, [0xa5; 48]);
    }
    let before = driver.video_port(1);
    owner.protect(base, PAGE, NativePermissions::READ).unwrap();
    assert_eq!(
        driver.submit(
            &mut owner.pause().unwrap(),
            Function::VideoGetOutputStatus,
            [1, base + PAGE - 48, 0, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(driver.video_port(1), before);
}

#[test]
fn video_ports_retain_generations_master_lifetime_and_source_error_order() {
    let (mut owner, _) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [1, 99, 1, 0, 0, 0]),
        Err(Failure::UnsupportedVideo)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [255, 99, 1, 0, 0, 0]),
        Ok(INVALID_VALUE)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [0, 0, 1, 0, 0, 0]),
        Err(Failure::UnsupportedVideo)
    );
    for bus in 0..3 {
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::VideoOpen,
                [255, bus, 0, u64::MAX, 0, 0]
            ),
            Ok(bus + 1)
        );
        assert_eq!(driver.video_port(bus as i32 + 1).unwrap().generation, 1);
    }
    let initial = driver.video_port(1).unwrap();
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [0, 0, 0, 0, 0, 0]),
        Ok(BUSY)
    );
    assert_eq!(driver.video_port(1), Some(initial));
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoSetFlipMaster,
            [1, 1, 0, 0, 0, 0]
        ),
        Ok(UNSUPPORTED_OPERATION)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoSetFlipMaster,
            [2, 1, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(driver.video_port(1).unwrap().slave_count, 1);
    assert_eq!(driver.video_port(2).unwrap().master_handle, Some(1));
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoSetFlipMaster,
            [1, 3, 0, 0, 0, 0]
        ),
        Ok(UNSUPPORTED_OPERATION)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoClose, [1, 0, 0, 0, 0, 0]),
        Err(Failure::UnsupportedVideo)
    );
    assert_eq!(driver.video_port(1).unwrap().slave_count, 1);
    assert_eq!(
        driver.submit(&mut memory, Function::VideoSetFlipRate, [2, 2, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoClose, [2, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_eq!(driver.video_port(2), None);
    assert_eq!(driver.video_port(1).unwrap().slave_count, 0);
    assert_eq!(
        driver.submit(&mut memory, Function::VideoClose, [2, 0, 0, 0, 0, 0]),
        Ok(INVALID_HANDLE)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [0, 1, 0, 0, 0, 0]),
        Ok(2)
    );
    assert_eq!(
        driver.video_port(2).unwrap(),
        PortState {
            generation: 3,
            ..Default::default()
        }
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoSetFlipRate, [0, 99, 0, 0, 0, 0]),
        Ok(INVALID_HANDLE)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoSetFlipRate,
            [2, u64::MAX, 0, 0, 0, 0]
        ),
        Ok(INVALID_VALUE)
    );
    assert_eq!(driver.frame_state().boundaries, 0);
    assert_eq!(driver.usage().dispatches, 0);
}

#[test]
fn video_options_preserve_exact_output_error_order_and_selected_mode_on_fault() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    memory.write(base, &[0xa5; 80]).unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoInitializeOptions,
            [base + 8, 0, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    let mut output = [0; 80];
    memory.read(base, &mut output).unwrap();
    assert_eq!(&output[..8], &[0xa5; 8]);
    assert_eq!(&output[8..72], &[0; 64]);
    assert_eq!(&output[72..], &[0xa5; 8]);
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0]),
        Ok(1)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoIsOutputSupported,
            [1, 1, base + 8, 0, 0, 0]
        ),
        Ok(1)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoIsOutputSupported,
            [1, 15, base + 8, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoConfigureOutput,
            [1, 15, 0, 0, 0, 0]
        ),
        Ok(UNAVAILABLE_MODE)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoIsOutputSupported,
            [9, 99, u64::MAX, 1, 1, 0]
        ),
        Ok(INVALID_HANDLE)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoIsOutputSupported,
            [1, 99, u64::MAX, 1, 0, 0]
        ),
        Ok(INVALID_VALUE)
    );
    memory.write(base + PAGE - 4, &1_u32.to_le_bytes()).unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoConfigureOutput,
            [1, 99, base + PAGE - 4, 0, 0, 0]
        ),
        Ok(INVALID_OPTION)
    );
    memory.write(base + PAGE - 4, &[0; 4]).unwrap();
    let before = driver.video_port(1);
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoConfigureOutput,
            [1, 1, base + PAGE - 4, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    assert_eq!(driver.video_port(1), before);
    memory.write(base + PAGE - 32, &[0xa5; 32]).unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoInitializeOptions,
            [base + PAGE - 32, 0, 0, 0, 0, 0]
        ),
        Err(Failure::Memory)
    );
    let mut tail = [0; 32];
    memory.read(base + PAGE - 32, &mut tail).unwrap();
    assert_eq!(tail, [0xa5; 32]);
    let mut fast = Driver::new(Limits {
        video_vblank_hz: 119,
        ..Default::default()
    });
    fast.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0])
        .unwrap();
    assert_eq!(
        fast.submit(
            &mut memory,
            Function::VideoConfigureOutput,
            [1, 15, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(fast.video_port(1).unwrap().output_mode, 15);
    assert_eq!(fast.frame_state().boundaries, 0);
}

#[test]
fn video_admission_and_generation_exhaustion_preserve_owned_port_state() {
    let (mut owner, _) = backing();
    let mut memory = owner.pause().unwrap();
    for limits in [
        Limits {
            max_calls: 0,
            ..Default::default()
        },
        Limits {
            max_preparation_work: 127,
            ..Default::default()
        },
    ] {
        let mut driver = Driver::new(limits);
        assert_eq!(
            driver.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0]),
            Err(Failure::Limit)
        );
        assert_eq!(driver.video_port(1), None);
    }
    let mut state = State::default();
    state.ports[0].state.generation = u64::MAX;
    assert_eq!(
        state.invoke(
            Function::VideoOpen,
            [255, 0, 0, 0, 0, 0],
            60,
            Default::default(),
            None,
            &mut memory
        ),
        Err(Failure::Limit)
    );
    assert!(state.port(1).is_none());
    state.ports[0].open = true;
    let before = state.port(1);
    assert_eq!(
        state.invoke(
            Function::VideoClose,
            [1, 0, 0, 0, 0, 0],
            60,
            Default::default(),
            None,
            &mut memory
        ),
        Err(Failure::Limit)
    );
    assert_eq!(state.port(1), before);
}
