// SPDX-License-Identifier: GPL-2.0-only
//! Original owned write -> actual draw -> two exact SYNC packets -> presentation.
//! No replacement shader, second draw, synthetic pixels or display receipt.
use super::*;
use pentax_platform::presenter::{self, Presenter};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires a local Vulkan device, visible primary monitor and presentation completion extensions"]
fn actual_encoded_draw_sync_flip_preserves_fenced_pixels_before_real_presentation_and_ack() {
    let (mut owner, base) = owner();
    let target = base + 0x10000;
    let mut driver = Driver::new(DriverLimits::default());
    let mut kernel = pentax_kernel::Kernel::new(
        Default::default(),
        pentax_kernel::ProcessIdentity {
            process_id: 7,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap();
    {
        let mut memory = owner.pause().unwrap();
        assert_eq!(
            driver.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0]),
            Ok(1)
        );
        let mut attribute = [0; 80];
        attribute[12..16].copy_from_slice(&64_u32.to_le_bytes());
        attribute[16..20].copy_from_slice(&64_u32.to_le_bytes());
        attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000_u64.to_le_bytes());
        memory.write(base + 0x80, &attribute).unwrap();
        let mut registration = [0; 32];
        registration[..8].copy_from_slice(&target.to_le_bytes());
        memory.write(base + 0x180, &registration).unwrap();
        memory.write(base + 8, &[0; 16]).unwrap();
        assert_eq!(
            driver.submit_with_stack(
                &mut memory,
                Function::VideoRegisterBuffers2,
                [1, 0, 0, base + 0x180, 1, base + 0x80],
                Some(base)
            ),
            Ok(0)
        );
        let layout = ImageLayout {
            kind: SurfaceLayout::RenderTarget64Kb,
            width: 64,
            height: 64,
            pitch: 128,
        };
        let mut tiled = vec![0x9b; 65536];
        let mut work = 0;
        retile_32_into(
            &SENTINEL.repeat(WIDTH * HEIGHT),
            layout,
            &mut tiled,
            &PresentationLimits::default(),
            &mut work,
        )
        .unwrap();
        memory.write(target, &tiled).unwrap();
        memory
            .write(base + 0x8000, &bytes(&vertex_shader()))
            .unwrap();
        memory
            .write(base + 0x9000, &bytes(&pixel_shader()))
            .unwrap();
        let mut commands = commands_with_layout(base + 0x8000, base + 0x9000, target, 0x0106_c000);
        let end = commands
            .iter()
            .position(|word| *word == 0xc001_2d00)
            .unwrap()
            + 3;
        commands.truncate(end);
        let label = base + 0x6000;
        memory.write(label, &0xa5a5_a5a5u32.to_le_bytes()).unwrap();
        let mut stream = vec![
            0xc003_3700,
            0x100,
            label as u32,
            (label >> 32) as u32,
            0xfeed_beef,
        ];
        stream.extend(commands);
        let commands = stream;
        let start = base + 0x1000;
        let encoded = bytes(&commands);
        memory.write(start, &encoded).unwrap();
        let cursor = start + encoded.len() as u64;
        let mut header = [0; 56];
        header[16..24].copy_from_slice(&cursor.to_le_bytes());
        header[24..32].copy_from_slice(&(base + 0x7000).to_le_bytes());
        memory.write(base + 0x300, &header).unwrap();
        memory.write(base + 0x408, &40u64.to_le_bytes()).unwrap();
        assert_eq!(
            driver.submit_with_stack(
                &mut memory,
                Function::AgcDcbAcquireMem,
                [base + 0x300, 1, 0, 0x1234, target, 65536],
                Some(base + 0x400)
            ),
            Ok(cursor)
        );
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::AgcDcbEventWrite,
                [base + 0x300, 7, u64::MAX, 0, 0, 0]
            ),
            Ok(cursor + 32)
        );
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::AgcDcbSetFlip,
                [base + 0x300, 1, 0, 1, 77, 0]
            ),
            Ok(cursor + 40)
        );
        let mut descriptor = [0; 16];
        descriptor[..8].copy_from_slice(&start.to_le_bytes());
        descriptor[8..12].copy_from_slice(&((encoded.len() as u32 + 64) / 4).to_le_bytes());
        memory.write(base + 0x200, &descriptor).unwrap();
        memory.write(base + 0x104, &[0; 4]).unwrap();
        let before = driver.usage();
        assert_eq!(
            driver.submit_with_kernel_clock(
                &mut memory,
                Function::CommandBuffer,
                [base + 0x100, base + 0x200, 0, 0, 0, 0],
                None,
                &mut kernel,
                counter
            ),
            Ok(0)
        );
        assert_eq!(driver.usage().draws - before.draws, 1);
        assert_eq!(driver.usage().memory_commands - before.memory_commands, 3);
        assert_eq!(driver.usage().written_bytes - before.written_bytes, 65540);
        assert_eq!(read(&memory, label, 4), 0xfeed_beefu32.to_le_bytes());
        assert_eq!(
            driver.usage().memory_written_bytes - before.memory_written_bytes,
            4
        );
        assert_eq!(driver.usage().interrupts - before.interrupts, 0);
        let status = driver.video_flip_status(&memory, 1).unwrap().unwrap();
        assert_eq!(
            (status.count, status.pending, status.gpu_pending),
            (0, 1, 1)
        );
        assert!(driver.video_peek_ready_frame(&memory).unwrap().is_some());
        let completed = read(&memory, target, 65536);
        let visible = detile_32(
            &completed,
            layout,
            completed.capacity(),
            &PresentationLimits::default(),
            &mut work,
        )
        .unwrap();
        assert_eq!(pixel(&visible, 36, 20), RED);
        assert_eq!(pixel(&visible, 52, 20), SENTINEL);
    }
    // Native owner is unpaused throughout the actual window/OS presentation.
    let mut presenter = Presenter::open(
        presenter::Config {
            width: 64,
            height: 64,
        },
        presenter::Limits::default(),
    )
    .unwrap();
    let generation = presenter.window_state().generation;
    let deadline = Instant::now() + Duration::from_secs(5);
    let tick = loop {
        match presenter.observe_tick() {
            Ok(Some(tick)) if tick.count() > 0 => break tick,
            Ok(_) | Err(presenter::Error::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            other => panic!("no actual display tick within the bounded fixture: {other:?}"),
        }
    };
    assert_eq!(tick.generation(), generation);
    let frame = driver
        .video_take_frame(&owner.pause().unwrap(), tick.count())
        .unwrap()
        .unwrap();
    assert_eq!(
        frame.source_format(),
        crate::gpu::video::PixelFormat::Rgba8Srgb
    );
    assert_eq!((frame.width(), frame.height()), (64, 64));
    assert_eq!(pixel(frame.rgba8(), 36, 20), RED);
    assert_eq!(pixel(frame.rgba8(), 52, 20), SENTINEL);
    assert!(
        frame
            .rgba8()
            .chunks_exact(4)
            .all(|value| value == RED || value == SENTINEL)
    );
    let accepted = loop {
        let view = presenter::FrameView {
            width: frame.width(),
            height: frame.height(),
            pixels: frame.rgba8(),
            alpha: if frame.premultiplied_alpha() {
                presenter::AlphaMode::Premultiplied
            } else {
                presenter::AlphaMode::Straight
            },
            encoding: presenter::ColorEncoding::Srgb,
        };
        match presenter.submit(1, view, frame.capacity()) {
            Ok(receipt) => break receipt,
            Err(presenter::Error::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            other => panic!("actual frame was not accepted: {other:?}"),
        }
    };
    assert_eq!(
        (accepted.generation(), accepted.sequence()),
        (generation, 1)
    );
    let receipt = loop {
        match presenter.poll_present() {
            Ok(presenter::Presentation::Presented(receipt)) => break receipt,
            Ok(presenter::Presentation::Pending) | Err(presenter::Error::WouldBlock)
                if Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(1))
            }
            other => panic!("actual presentation did not complete: {other:?}"),
        }
    };
    assert_eq!((receipt.generation(), receipt.sequence()), (generation, 1));
    assert_eq!(
        presenter.poll_present().unwrap(),
        presenter::Presentation::Idle
    );
    let mut memory = owner.pause().unwrap();
    let before = driver.video_flip_status(&memory, 1).unwrap().unwrap();
    assert_eq!(
        (before.count, before.pending, before.gpu_pending),
        (0, 1, 1)
    );
    let completion = driver
        .video_acknowledge_frame(&mut memory, &frame, |_| {
            let clock = kernel
                .process_clock(&Clock)
                .map_err(crate::gpu::driver::Failure::Kernel)?;
            Ok((clock.microseconds, clock.counter))
        })
        .unwrap();
    assert_eq!(
        (
            completion.count(),
            completion.buffer(),
            completion.flip_arg()
        ),
        (1, 0, 77)
    );
    let after = driver.video_flip_status(&memory, 1).unwrap().unwrap();
    assert_eq!(
        (
            after.count,
            after.pending,
            after.gpu_pending,
            after.current_buffer
        ),
        (1, 0, 0, 0)
    );
    assert!(
        driver
            .video_acknowledge_frame(&mut memory, &frame, |_| panic!(
                "a stale receipt must not sample time"
            ))
            .is_err()
    );
    assert_eq!(kernel.usage().attempts, 0);
    presenter.cancel();
    println!(
        "agc_memory_sync_gpu_available=true agc_memory_sync_gpu_draws={} agc_memory_sync_gpu_barriers={} agc_memory_sync_gpu_presentations={} agc_memory_sync_gpu_acknowledgements={}",
        driver.usage().draws,
        driver.usage().memory_commands - 1,
        receipt.sequence(),
        after.count
    );
}
