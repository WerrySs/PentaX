// SPDX-License-Identifier: GPL-2.0-only
//! Original checked VideoOut state consumers, with actual owned memory.
use super::super::tests::{PAGE, backing};
use super::*;
use crate::gpu::driver::{Driver, Limits};

fn vectors(bits: u32) -> Option<[[u8; 16]; 8]> {
    let mut captured = [[0xa5; 16]; 8];
    captured[0][..4].copy_from_slice(&bits.to_le_bytes());
    Some(captured)
}

#[test]
fn gamma_uses_only_captured_xmm0_with_exact_source_boundaries_and_reserved_bytes() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Limits::default());
    for (bits, expected) in [
        (0x3dcc_cccd_u32, 0),
        (0x4000_0000, 0),
        (0x3fc0_0000, 0),
        (0x7fc0_1234, 0),
        (0xff80_1234, 0),
        (0x3dcc_cccc, INVALID_VALUE),
        (0x4000_0001, INVALID_VALUE),
        (0x8000_0000, INVALID_VALUE),
        (0x7f80_0000, INVALID_VALUE),
    ] {
        memory.write(base, &[0xa5; 24]).unwrap();
        assert_eq!(
            driver.submit_video_gamma(&mut memory, [base + 4, u64::MAX, 0, 0, 0, 0], vectors(bits)),
            Ok(expected)
        );
        let mut bytes = [0; 24];
        memory.read(base, &mut bytes).unwrap();
        let mut wanted = [0xa5; 24];
        if expected == 0 {
            wanted[4..8].copy_from_slice(&bits.to_le_bytes());
        }
        assert_eq!(bytes, wanted);
    }
    let mut before = [0; 24];
    memory.read(base, &mut before).unwrap();
    assert!(matches!(
        driver.submit_video_gamma(&mut memory, [base + 4, 0x3f80_0000, 0, 0, 0, 0], None),
        Err(Failure::Kernel(pentax_kernel::Failure::Unsupported(_)))
    ));
    assert_eq!(
        driver.submit_video_gamma(&mut memory, [0; 6], None),
        Ok(INVALID_ADDRESS)
    );
    let mut after = [0; 24];
    memory.read(base, &mut after).unwrap();
    assert_eq!(after, before);
}

#[test]
fn adjust_color_reads_only_gamma_after_owned_handle_checks_and_preserves_source_color_on_reopen() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::VideoAdjustColor, [99, 0, 0, 0, 0, 0]),
        Ok(INVALID_ADDRESS)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoAdjustColor,
            [99, u64::MAX, 0, 0, 0, 0]
        ),
        Ok(INVALID_HANDLE)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [0; 6]),
        Ok(1)
    );
    assert_eq!(driver.video_port(1).unwrap().gamma_bits, 0x3f80_0000);
    let address = base + PAGE - 4;
    // Reserved settings bytes are unmapped: AdjustColor reads exactly four.
    memory
        .write(address, &0x7f80_0000_u32.to_le_bytes())
        .unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoAdjustColor,
            [1, address, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(driver.video_port(1).unwrap().gamma_bits, 0x7f80_0000);
    assert!(
        driver
            .submit(
                &mut memory,
                Function::VideoAdjustColor,
                [1, base + PAGE, 0, 0, 0, 0]
            )
            .is_err()
    );
    assert_eq!(driver.video_port(1).unwrap().gamma_bits, 0x7f80_0000);
    assert_eq!(
        driver.submit(&mut memory, Function::VideoClose, [1, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoAdjustColor,
            [1, address, 0, 0, 0, 0]
        ),
        Ok(INVALID_HANDLE)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [0; 6]),
        Ok(1)
    );
    assert_eq!(driver.video_port(1).unwrap().gamma_bits, 0x7f80_0000);
}

fn kernel() -> pentax_kernel::Kernel {
    pentax_kernel::Kernel::new(
        Default::default(),
        pentax_kernel::ProcessIdentity {
            process_id: 101,
            initial_thread_id: 7,
        },
        Vec::new(),
    )
    .unwrap()
}
fn clock(counter: u64) -> pentax_kernel::DisplayClock {
    pentax_kernel::DisplayClock {
        counter,
        microseconds: counter * 1_000_000 / 60_000,
        raw_ticks: 10_000_000 + counter,
        frequency: 60_000,
    }
}
fn call(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    kernel: &mut pentax_kernel::Kernel,
    function: Function,
    args: [u64; 6],
    counter: u64,
) -> Result<u64, Failure> {
    driver.submit_video_clocked(memory, function, args, kernel, |_, _| Ok(clock(counter)))
}
fn status(memory: &PausedNativeMemory<'_>, address: u64) -> [u8; 40] {
    let mut bytes = [0; 40];
    memory.read(address, &mut bytes).unwrap();
    bytes
}
fn count(memory: &PausedNativeMemory<'_>, address: u64) -> u64 {
    u64::from_le_bytes(status(memory, address)[..8].try_into().unwrap())
}

#[test]
fn elapsed_grid_advances_without_pixels_and_consumer_ticks_cannot_double_count() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    let mut kernel = kernel();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoOpen,
            [0; 6],
            0
        ),
        Ok(1)
    );
    for counter in [0, 999] {
        assert_eq!(
            call(
                &mut driver,
                &mut memory,
                &mut kernel,
                Function::VideoGetVblankStatus,
                [1, base, 0, 0, 0, 0],
                counter
            ),
            Ok(0)
        );
        assert_eq!(status(&memory, base), [0; 40]);
    }
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [1, base, 0, 0, 0, 0],
            3_500
        ),
        Ok(0)
    );
    let mut expected = [0; 40];
    for (offset, value) in [(0, 3_u64), (8, 50_000), (16, 10_003_000), (24, 3_000)] {
        expected[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(status(&memory, base), expected);
    let generation = driver.video_port(1).unwrap().generation;
    driver
        .video_observe_vblank(&mut memory, &mut kernel, 1, generation, 9_999, clock(3_500))
        .unwrap();
    driver
        .video_observe_vblank(
            &mut memory,
            &mut kernel,
            1,
            generation,
            10_000,
            clock(3_500),
        )
        .unwrap();
    assert!(
        driver
            .video_observe_vblank(
                &mut memory,
                &mut kernel,
                1,
                generation,
                10_000,
                clock(3_500)
            )
            .is_err()
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [1, base, 0, 0, 0, 0],
            3_500
        ),
        Ok(0)
    );
    assert_eq!(status(&memory, base), expected);
    assert_eq!(
        driver.video_flip_status(&memory, 1).unwrap().unwrap().count,
        0
    );
    assert_eq!(driver.usage().dispatches, 0);
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoClose,
            [1, 0, 0, 0, 0, 0],
            3_500
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoOpen,
            [0; 6],
            4_000
        ),
        Ok(1)
    );
    assert_ne!(driver.video_port(1).unwrap().generation, generation);
    assert!(
        driver
            .video_observe_vblank(
                &mut memory,
                &mut kernel,
                1,
                generation,
                10_001,
                clock(4_000)
            )
            .is_err()
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [1, base, 0, 0, 0, 0],
            4_999
        ),
        Ok(0)
    );
    assert_eq!(status(&memory, base), [0; 40]);
}

#[test]
fn global_mode_and_main_close_rebase_at_the_same_boundary_without_retroactive_jumps() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Limits {
        video_vblank_hz: 120,
        ..Default::default()
    });
    let mut kernel = kernel();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoOpen,
            [0; 6],
            0
        ),
        Ok(1)
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoOpen,
            [0, 1, 0, 0, 0, 0],
            0
        ),
        Ok(2)
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoConfigureOutput,
            [1, 15, 0, 0, 0, 0],
            60_000
        ),
        Ok(0)
    );
    for (counter, expected) in [(60_000, 60), (66_000, 72)] {
        for handle in [1, 2] {
            assert_eq!(
                call(
                    &mut driver,
                    &mut memory,
                    &mut kernel,
                    Function::VideoGetVblankStatus,
                    [handle, base, 0, 0, 0, 0],
                    counter
                ),
                Ok(0)
            );
            assert_eq!(count(&memory, base), expected);
        }
    }
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoClose,
            [1, 0, 0, 0, 0, 0],
            66_000
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [2, base, 0, 0, 0, 0],
            72_000
        ),
        Ok(0)
    );
    assert_eq!(count(&memory, base), 78);
    assert_eq!(driver.video_port(2).unwrap().output_mode, 1);
}

#[test]
fn clock_output_and_foreign_owner_refusals_do_not_publish_grid_or_mode_changes() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Limits {
        video_vblank_hz: 120,
        ..Default::default()
    });
    let mut kernel = kernel();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoOpen,
            [0; 6],
            0
        ),
        Ok(1)
    );
    assert_eq!(
        driver.submit_video_clocked(
            &mut memory,
            Function::VideoConfigureOutput,
            [1, 15, 0, 0, 0, 0],
            &mut kernel,
            |_, _| Err(Failure::Memory)
        ),
        Err(Failure::Memory)
    );
    assert_eq!(driver.video_port(1).unwrap().output_mode, 1);
    assert!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [1, base + PAGE, 0, 0, 0, 0],
            5_000
        )
        .is_err()
    );
    // A failed output did not consume elapsed periods; the next valid output
    // receives the full five real boundaries, with its original 60 Hz cadence.
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [1, base, 0, 0, 0, 0],
            5_000
        ),
        Ok(0)
    );
    assert_eq!(count(&memory, base), 5);
    let mut bad = clock(6_000);
    bad.frequency += 1;
    assert!(
        driver
            .submit_video_clocked(
                &mut memory,
                Function::VideoGetVblankStatus,
                [1, base, 0, 0, 0, 0],
                &mut kernel,
                |_, _| Ok(bad)
            )
            .is_err()
    );
    assert_eq!(count(&memory, base), 5);
    assert_eq!(
        driver.submit_video_clocked(
            &mut memory,
            Function::VideoGetVblankStatus,
            [99, 0, 0, 0, 0, 0],
            &mut kernel,
            |_, _| panic!("null output must precede clock")
        ),
        Ok(INVALID_ADDRESS)
    );
    assert_eq!(
        driver.submit_video_clocked(
            &mut memory,
            Function::VideoGetVblankStatus,
            [99, u64::MAX, 0, 0, 0, 0],
            &mut kernel,
            |_, _| panic!("unknown handle must precede clock")
        ),
        Ok(INVALID_HANDLE)
    );
    let (mut other, other_base) = backing();
    let mut foreign = other.pause().unwrap();
    assert!(
        driver
            .submit_video_clocked(
                &mut foreign,
                Function::VideoGetVblankStatus,
                [1, other_base, 0, 0, 0, 0],
                &mut kernel,
                |_, _| panic!("foreign owner must precede clock")
            )
            .is_err()
    );
    assert_eq!(count(&memory, base), 5);
}

#[test]
fn event_budget_refusal_keeps_mode_and_elapsed_output_unpublished() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Limits {
        video_vblank_hz: 120,
        ..Default::default()
    });
    let mut kernel = pentax_kernel::Kernel::new(
        pentax_kernel::Limits {
            max_work: 0,
            ..Default::default()
        },
        pentax_kernel::ProcessIdentity {
            process_id: 101,
            initial_thread_id: 7,
        },
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoOpen,
            [0; 6],
            0
        ),
        Ok(1)
    );
    memory.write(base, &[0xa5; 40]).unwrap();
    assert!(matches!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoConfigureOutput,
            [1, 15, 0, 0, 0, 0],
            60_000
        ),
        Err(Failure::Kernel(pentax_kernel::Failure::Limit(_)))
    ));
    assert_eq!(driver.video_port(1).unwrap().output_mode, 1);
    assert_eq!(status(&memory, base), [0xa5; 40]);
    assert!(matches!(
        call(
            &mut driver,
            &mut memory,
            &mut kernel,
            Function::VideoGetVblankStatus,
            [1, base, 0, 0, 0, 0],
            60_000
        ),
        Err(Failure::Kernel(pentax_kernel::Failure::Limit(_)))
    ));
    assert_eq!(status(&memory, base), [0xa5; 40]);
    // No count was committed by the failed batch. The direct retained-state
    // reader is intentionally clock-free and observes the untouched snapshot.
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoGetVblankStatus,
            [1, base, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(status(&memory, base), [0; 40]);
}
