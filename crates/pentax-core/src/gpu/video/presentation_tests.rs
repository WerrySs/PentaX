// SPDX-License-Identifier: GPL-2.0-only
//! Original pixels and actual owned memory; no GPU, window or game execution.

use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn owner() -> (NativeAddressSpace, u64) {
    // Explicit original low guest addresses satisfy the registered40-bit
    // surface profile; bounded alternatives avoid unrelated test reservations.
    for index in 0..16 {
        let base = 0x50_0000_0000 + index * 0x100_0000;
        let mut owner = NativeAddressSpace::new(Default::default()).unwrap();
        if owner
            .map_at(base, 16384, NativePermissions::READ_WRITE)
            .is_err()
        {
            continue;
        }
        if owner
            .map_at(
                base + 65536,
                131072,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .is_err()
        {
            continue;
        }
        return (owner, base);
    }
    panic!("no original bounded address candidate available");
}
fn invoke(
    state: &mut State,
    function: Function,
    args: [u64; 6],
    base: u64,
    memory: &mut PausedNativeMemory<'_>,
) -> Result<u64, Failure> {
    state.invoke(function, args, 60, Default::default(), Some(base), memory)
}
fn register(state: &mut State, base: u64, memory: &mut PausedNativeMemory<'_>, format: u64) {
    let mut attribute = [0; 80];
    attribute[12..16].copy_from_slice(&2_u32.to_le_bytes());
    attribute[16..20].copy_from_slice(&2_u32.to_le_bytes());
    attribute[32..40].copy_from_slice(&format.to_le_bytes());
    memory.write(base + 128, &attribute).unwrap();
    let mut descriptor = [0; 32];
    descriptor[..8].copy_from_slice(&(base + 65536).to_le_bytes());
    memory.write(base + 256, &descriptor).unwrap();
    memory.write(base + 8, &[0; 16]).unwrap();
    assert_eq!(
        invoke(
            state,
            Function::VideoRegisterBuffers2,
            [1, 0, 0, base + 256, 1, base + 128],
            base,
            memory
        ),
        Ok(0)
    );
}
fn setup(state: &mut State, base: u64, memory: &mut PausedNativeMemory<'_>) {
    assert_eq!(
        invoke(
            state,
            Function::VideoOpen,
            [255, 0, 0, u64::MAX, 0, 0],
            base,
            memory
        ),
        Ok(1)
    );
    register(state, base, memory, 0x8000_0000_2200_0000);
}
fn reserve(
    state: &mut State,
    owner: pentax_platform::native_memory::NativeOwnerId,
    argument: i64,
    source: FlipSource,
) -> FlipTicket {
    match state
        .reserve_flip(1, 0, 1, argument, source, argument as u64, owner)
        .unwrap()
    {
        FlipSubmission::Accepted(ticket) => ticket,
        FlipSubmission::Rejected(code) => panic!("unexpected original flip error {code:x}"),
    }
}
fn pixels(memory: &mut PausedNativeMemory<'_>, base: u64, bias: u8) -> Vec<u8> {
    let mut tight = Vec::new();
    for (index, offset) in [0, 4, 16, 20].into_iter().enumerate() {
        let pixel = [bias + index as u8, 20, 30, 255];
        memory.write(base + 65536 + offset, &pixel).unwrap();
        tight.extend(pixel);
    }
    tight
}

#[test]
fn presentation_tiling_known_vectors_cross_blocks_and_roundtrip_have_exact_bounds() {
    let limits = PresentationLimits::default();
    let layout = ImageLayout {
        kind: SurfaceLayout::RenderTarget64Kb,
        width: 129,
        height: 129,
        pitch: 256,
    };
    let mut tiled = vec![0; 262144];
    let mut expected = vec![0; 129 * 129 * 4];
    // Independent literal source-equation vectors, including XOR contributions
    // and horizontal/vertical/corner64KiB block boundaries.
    for (index, (x, y, offset)) in [
        (0, 0, 0),
        (1, 0, 4),
        (4, 0, 0x80),
        (16, 0, 0x2200),
        (64, 0, 0x8400),
        (0, 8, 0x1100),
        (0, 64, 0x4800),
        (127, 127, 0xf0fc),
        (128, 0, 65536),
        (0, 128, 131072),
        (128, 128, 196608),
    ]
    .into_iter()
    .enumerate()
    {
        let value = (0x1020_3040_u32 + index as u32).to_le_bytes();
        tiled[offset..offset + 4].copy_from_slice(&value);
        let to = (y * 129 + x) * 4;
        expected[to..to + 4].copy_from_slice(&value);
    }
    let mut work = 0;
    let actual = detile_32(&tiled, layout, tiled.capacity(), &limits, &mut work).unwrap();
    assert_eq!(actual, expected);
    let encoded = retile_32(
        &actual,
        layout,
        tiled.capacity() + actual.capacity(),
        &limits,
        &mut work,
    )
    .unwrap();
    assert_eq!(encoded, tiled);
    assert!(
        detile_32(
            &tiled[..tiled.len() - 1],
            layout,
            tiled.capacity(),
            &limits,
            &mut work
        )
        .is_err()
    );
    assert!(detile_32(&tiled, layout, limits.max_held_bytes, &limits, &mut work).is_err());
    assert!(
        retile_32(
            &actual,
            ImageLayout {
                pitch: 129,
                ..layout
            },
            actual.capacity(),
            &limits,
            &mut work
        )
        .is_err()
    );
    let exhausted = PresentationLimits {
        max_work: work,
        ..limits
    };
    assert!(detile_32(&tiled, layout, tiled.capacity(), &exhausted, &mut work).is_err());
}

#[test]
fn presentation_cpu_frames_capture_real_pixels_fifo_pacing_and_actual_acknowledgements() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut state = State::default();
    setup(&mut state, base, &mut memory);
    assert_eq!(state.flip_observations(), FlipObservations::default());
    assert_eq!(
        invoke(
            &mut state,
            Function::VideoSetFlipRate,
            [1, 1, 0, 0, 0, 0],
            base,
            &mut memory
        ),
        Ok(0)
    );
    let first_pixels = pixels(&mut memory, base, 1);
    let first = reserve(&mut state, memory.owner_id(), 11, FlipSource::Cpu);
    let second = reserve(&mut state, memory.owner_id(), 22, FlipSource::Cpu);
    assert_eq!(state.flip_observations().reserved, 2);
    let limits = PresentationLimits::default();
    assert_eq!(state.complete_flip(first), Err(Failure::UnsupportedVideo));
    state.prepare_flip(first, &memory, &limits).unwrap();
    assert_eq!(state.flip_observations().recording_with_frame, 1);
    assert_eq!(state.flip_observations().reserved, 1);
    let second_pixels = pixels(&mut memory, base, 80);
    state.prepare_flip(second, &memory, &limits).unwrap();
    state.complete_flip(second).unwrap();
    assert_eq!(state.flip_observations().ready_with_frame, 1);
    assert_eq!(state.flip_observations().recording_with_frame, 1);
    assert!(state.take_frame(2).unwrap().is_none()); // Oldest is not ready.
    assert_eq!(state.flip_observations().ready_with_frame, 1); // Include FIFO-blocked READY.
    state.complete_flip(first).unwrap();
    assert_eq!(state.flip_observations().ready_with_frame, 2);
    assert!(state.take_frame(0).unwrap().is_none());
    assert!(state.take_frame(1).unwrap().is_none());
    let frame = state.take_frame(2).unwrap().unwrap();
    assert_eq!(state.flip_observations().presenting, 1);
    assert_eq!(state.flip_observations().ready_with_frame, 1);
    assert_eq!((frame.width(), frame.height()), (2, 2));
    assert_eq!(frame.rgba8(), first_pixels);
    assert_eq!(state.flip_status(1).unwrap().count, 0);
    assert_eq!(state.flip_status(1).unwrap().pending, 2);
    let completion = state.acknowledge_frame(&frame, 100, 1000).unwrap();
    assert_eq!(state.flip_observations().acknowledged, 1);
    assert_eq!(state.flip_observations().presenting, 0);
    assert_eq!(
        (
            completion.flip_arg(),
            completion.count(),
            completion.buffer()
        ),
        (11, 1, 0)
    );
    assert!(state.acknowledge_frame(&frame, 101, 1001).is_err());
    assert!(state.take_frame(2).unwrap().is_none());
    assert!(state.take_frame(3).unwrap().is_none());
    let frame = state.take_frame(4).unwrap().unwrap();
    assert_eq!(frame.rgba8(), second_pixels);
    state.acknowledge_frame(&frame, 200, 2000).unwrap();
    let status = state.flip_status(1).unwrap();
    assert_eq!(
        (
            status.count,
            status.pending,
            status.gpu_pending,
            status.flip_arg
        ),
        (2, 0, 0, 22)
    );
    assert_eq!(
        (
            status.process_time,
            status.process_counter,
            status.submit_counter
        ),
        (200, 2000, 22)
    );
    let encoded = status.bytes();
    assert_eq!(encoded.len(), 128);
    assert_eq!(&encoded[16..24], &[0; 8]);
    assert_eq!(&encoded[72..], &[0; 56]);
    assert_eq!(state.presentation.held, 0);
    assert!(!state.buffer_pending(1, 0));
    assert_eq!(
        state.flip_observations(),
        FlipObservations {
            acknowledged: 2,
            ..Default::default()
        }
    );
    assert_eq!(
        invoke(
            &mut state,
            Function::VideoClose,
            [1, 0, 0, 0, 0, 0],
            base,
            &mut memory
        ),
        Ok(0)
    );
    assert_eq!(state.flip_observations(), FlipObservations::default());
}

#[test]
fn presentation_gpu_rights_owner_and_quota_failures_keep_pending_state_truthful() {
    let (mut owner, base) = owner();
    let mut state = State::default();
    let ticket;
    {
        let mut memory = owner.pause().unwrap();
        setup(&mut state, base, &mut memory);
        pixels(&mut memory, base, 3);
        ticket = reserve(&mut state, memory.owner_id(), 7, FlipSource::Cpu);
    }
    let limits = PresentationLimits::default();
    let (mut foreign, _) = self::owner();
    assert_eq!(
        state.prepare_flip(ticket, &foreign.pause().unwrap(), &limits),
        Err(Failure::Memory)
    );
    owner
        .protect(base + 65536, 65536, NativePermissions::READ_WRITE)
        .unwrap();
    assert_eq!(
        state.prepare_flip(ticket, &owner.pause().unwrap(), &limits),
        Err(Failure::Memory)
    );
    owner
        .protect(
            base + 65536,
            65536,
            NativePermissions::new(false, false, false).with_gpu(true, true),
        )
        .unwrap();
    let tiny = PresentationLimits {
        max_frame_bytes: 15,
        ..limits
    };
    assert_eq!(
        state.prepare_flip(ticket, &owner.pause().unwrap(), &tiny),
        Err(Failure::Limit)
    );
    assert_eq!(state.presentation.held, 0);
    assert_eq!(state.flip_status(1).unwrap().pending, 1);
    assert_eq!(state.flip_status(1).unwrap().count, 0);
    assert_eq!(
        state.flip_observations(),
        FlipObservations {
            reserved: 1,
            ..Default::default()
        }
    );
    state
        .prepare_flip(ticket, &owner.pause().unwrap(), &limits)
        .unwrap(); // GPU-only backing is valid.
    state.complete_flip(ticket).unwrap();
    let frame = state.take_frame(1).unwrap().unwrap();
    assert_eq!(frame.rgba8()[0], 3);
    {
        let mut memory = owner.pause().unwrap();
        assert_eq!(
            invoke(
                &mut state,
                Function::VideoUnregisterBuffers,
                [1, 0, 0, 0, 0, 0],
                base,
                &mut memory
            ),
            Err(Failure::Waiting)
        );
        assert_eq!(
            invoke(
                &mut state,
                Function::VideoClose,
                [1, 0, 0, 0, 0, 0],
                base,
                &mut memory
            ),
            Ok(0)
        );
        setup(&mut state, base, &mut memory);
    }
    assert!(state.acknowledge_frame(&frame, 1, 1).is_err());
    assert_eq!(state.flip_status(1).unwrap().count, 0);
    assert_eq!(state.flip_observations(), FlipObservations::default());
}

#[test]
fn presentation_gpu_tags_cannot_complete_and_bounded_queue_errors_preserve_lifetimes() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut state = State::default();
    setup(&mut state, base, &mut memory);
    pixels(&mut memory, base, 5);
    let gpu = reserve(&mut state, memory.owner_id(), 1, FlipSource::GpuEop);
    state
        .prepare_flip(gpu, &memory, &Default::default())
        .unwrap();
    assert_eq!(state.complete_flip(gpu), Err(Failure::UnsupportedVideo));
    assert!(state.take_frame(100).unwrap().is_none());
    for argument in 2..=16 {
        reserve(&mut state, memory.owner_id(), argument, FlipSource::Cpu);
    }
    assert_eq!(
        state.reserve_flip(1, 0, 1, 17, FlipSource::Cpu, 17, memory.owner_id()),
        Ok(FlipSubmission::Rejected(0x8029_0012))
    );
    assert_eq!(state.flip_status(1).unwrap().pending, 16);
    assert_eq!(state.flip_status(1).unwrap().gpu_pending, 1);
    assert_eq!(
        state.reserve_flip(99, 99, 99, 0, FlipSource::Cpu, 0, memory.owner_id()),
        Ok(FlipSubmission::Rejected(INVALID_HANDLE))
    );
    assert_eq!(
        state.reserve_flip(1, 99, 99, 0, FlipSource::Cpu, 0, memory.owner_id()),
        Ok(FlipSubmission::Rejected(0x8029_0006))
    );
    assert_eq!(
        state.reserve_flip(1, 99, 1, 0, FlipSource::Cpu, 0, memory.owner_id()),
        Ok(FlipSubmission::Rejected(0x8029_000a))
    );
    assert_eq!(
        invoke(
            &mut state,
            Function::VideoClose,
            [1, 0, 0, 0, 0, 0],
            base,
            &mut memory
        ),
        Ok(0)
    );
    assert_eq!(state.presentation.held, 0);
    setup(&mut state, base, &mut memory);
    assert!(
        state
            .prepare_flip(gpu, &memory, &Default::default())
            .is_err()
    );
    assert_eq!(state.flip_status(1).unwrap().pending, 0);
}

#[test]
fn presentation_channel_conversion_is_real_and_unsupported_formats_never_become_frames() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let packed = (2_u32 << 30) | (512 << 10) | 1023;
    for (format, input, expected) in [
        (0x8000_0000_2200_0000, [10, 20, 30, 40], [10, 20, 30, 40]),
        (0x8000_0000_0000_0000, [30, 20, 10, 40], [10, 20, 30, 40]),
        (
            0x8100_0000_2200_0000,
            packed.to_le_bytes(),
            [255, 128, 0, 170],
        ),
        (
            0x8100_0000_0000_0000,
            packed.to_le_bytes(),
            [0, 128, 255, 170],
        ),
    ] {
        let mut state = State::default();
        assert_eq!(
            invoke(
                &mut state,
                Function::VideoOpen,
                [255, 0, 0, 0, 0, 0],
                base,
                &mut memory
            ),
            Ok(1)
        );
        register(&mut state, base, &mut memory, format);
        for offset in [0, 4, 16, 20] {
            memory.write(base + 65536 + offset, &input).unwrap();
        }
        let ticket = reserve(&mut state, memory.owner_id(), 1, FlipSource::Cpu);
        state
            .prepare_flip(ticket, &memory, &Default::default())
            .unwrap();
        state.complete_flip(ticket).unwrap();
        let frame = state.take_frame(1).unwrap().unwrap();
        assert_eq!(frame.rgba8(), expected.repeat(4));
    }
    for format in [
        0x8100_0704_2200_0000,
        0xc001_0006_2200_0000,
        0xc001_0006_0000_0000,
    ] {
        let mut state = State::default();
        assert_eq!(
            invoke(
                &mut state,
                Function::VideoOpen,
                [255, 0, 0, 0, 0, 0],
                base,
                &mut memory
            ),
            Ok(1)
        );
        register(&mut state, base, &mut memory, format);
        let ticket = reserve(&mut state, memory.owner_id(), 1, FlipSource::Cpu);
        assert_eq!(
            state.prepare_flip(ticket, &memory, &Default::default()),
            Err(Failure::UnsupportedVideo)
        );
        assert!(state.complete_flip(ticket).is_err());
        assert!(state.take_frame(1).unwrap().is_none());
        assert_eq!(state.flip_status(1).unwrap().count, 0);
        assert_eq!(state.presentation.held, 0);
    }
}
