// SPDX-License-Identifier: GPL-2.0-only
//! Original encoder -> ordered actual draw -> GPU-ready frame, without fake display.
use super::*;
use crate::gpu::{
    driver::{Driver, Function, Limits as DriverLimits},
    video::{ImageLayout, PresentationLimits, SurfaceLayout, detile_32, retile_32_into},
};

fn owner() -> (NativeAddressSpace, u64) {
    for index in 0..16 {
        let base = 0x70_0000_0000 + index * 0x100_0000;
        let mut owner = NativeAddressSpace::new(Default::default()).unwrap();
        if owner
            .map_at(
                base,
                0x40000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .is_ok()
        {
            return (owner, base);
        }
    }
    panic!("original bounded surface address candidates unavailable");
}
struct Clock;
impl pentax_kernel::GuestAccess for Clock {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), pentax_kernel::GuestError> {
        Err(pentax_kernel::GuestError::UnsupportedMapping)
    }
    fn apply(
        &mut self,
        _: pentax_kernel::MemoryEffect,
        _: &[pentax_kernel::GuestWrite<'_>],
    ) -> Result<(), pentax_kernel::GuestError> {
        Err(pentax_kernel::GuestError::UnsupportedMapping)
    }
    fn time_counter(&self) -> Result<pentax_kernel::TimeCounterSample, pentax_kernel::Failure> {
        let sample = pentax_platform::time_counter::sample()
            .map_err(|_| pentax_kernel::Failure::Unsupported("native process clock"))?;
        Ok(pentax_kernel::TimeCounterSample {
            ticks: sample.ticks,
            frequency: sample.frequency,
        })
    }
}
fn counter(
    kernel: &mut pentax_kernel::Kernel,
    _: &mut PausedNativeMemory<'_>,
) -> Result<u64, crate::gpu::driver::Failure> {
    kernel
        .process_time_counter(&Clock)
        .map_err(crate::gpu::driver::Failure::Kernel)
}

#[test]
#[ignore = "requires a local Vulkan device and native process clock"]
fn actual_encoded_draw_flip_cycles_capture_current_pixels_and_keep_display_status_pending() {
    encoded_draw_flip_cycles(false);
}

#[test]
#[ignore = "requires a local Vulkan device and native process clock"]
fn actual_indexed_draw_flip_cycles_capture_current_pixels_and_keep_display_status_pending() {
    encoded_draw_flip_cycles(true);
}

fn encoded_draw_flip_cycles(indexed: bool) {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(DriverLimits::default());
    let target = base + 0x10000;
    assert_eq!(
        driver.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0]),
        Ok(1)
    );
    let mut attribute = [0; 80];
    attribute[12..16].copy_from_slice(&64u32.to_le_bytes());
    attribute[16..20].copy_from_slice(&64u32.to_le_bytes());
    attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000u64.to_le_bytes());
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
    let mut original = commands_with_layout(base + 0x8000, base + 0x9000, target, 0x0106_c000);
    if indexed {
        // Original resources and shaders are unchanged: only the two AUTO
        // packets become explicit source-indexed triangles in this variant.
        memory.write(base + 0x6000, &[0, 0, 1, 0, 2, 0]).unwrap();
        let mut replacement = Vec::new();
        set(&mut replacement, 0x79, 0x24b, &[0]);
        replacement.extend([0xc000_2a00, 0]);
        let mut at = 0;
        let mut draws = 0;
        while at < original.len() {
            let header = original[at];
            let length = ((header >> 16) & 0x3fff) as usize + 2;
            assert!(at + length <= original.len());
            if header == 0xc001_2d00 {
                assert_eq!(&original[at + 1..at + length], &[3, 2]);
                replacement.extend([
                    0xc004_2700,
                    3,
                    (base + 0x6000) as u32,
                    ((base + 0x6000) >> 32) as u32,
                    3,
                    0,
                ]);
                draws += 1;
            } else {
                replacement.extend_from_slice(&original[at..at + length]);
            }
            at += length;
        }
        assert_eq!(draws, 2);
        original = replacement;
    }
    let split = original
        .iter()
        .position(|word| *word == if indexed { 0xc004_2700 } else { 0xc001_2d00 })
        .unwrap()
        + if indexed { 6 } else { 3 };
    let start = base + 0x1000;
    let mut cursor = start;
    for (index, words) in [&original[..split], &original[split..]]
        .into_iter()
        .enumerate()
    {
        let encoded = bytes(words);
        memory.write(cursor, &encoded).unwrap();
        cursor += encoded.len() as u64;
        let mut header = [0; 56];
        header[16..24].copy_from_slice(&cursor.to_le_bytes());
        header[24..32].copy_from_slice(&(base + 0x7000).to_le_bytes());
        memory.write(base + 0x300, &header).unwrap();
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::AgcDcbSetFlip,
                [base + 0x300, 1, 0, 1, (-7i64 + index as i64) as u64, 0]
            ),
            Ok(cursor)
        );
        cursor += 24;
    }
    let mut descriptor = [0; 16];
    descriptor[..8].copy_from_slice(&start.to_le_bytes());
    descriptor[8..12].copy_from_slice(&((cursor - start) as u32 / 4).to_le_bytes());
    memory.write(base + 0x200, &descriptor).unwrap();
    memory.write(base + 0x104, &[0; 4]).unwrap();
    let mut kernel = pentax_kernel::Kernel::new(
        Default::default(),
        pentax_kernel::ProcessIdentity {
            process_id: 7,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap();
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
    assert_eq!(driver.usage().draws, 2);
    assert_eq!(driver.usage().memory_written_bytes, 64);
    assert_eq!(driver.usage().written_bytes, 131072);
    if indexed {
        assert_eq!(read(&memory, base + 0x6000, 6), [0, 0, 1, 0, 2, 0]);
    }
    assert_eq!(kernel.usage().attempts, 0);
    let status = driver.video_flip_status(&memory, 1).unwrap().unwrap();
    assert_eq!(
        (
            status.count,
            status.pending,
            status.gpu_pending,
            status.current_buffer
        ),
        (0, 2, 2, -1)
    );
    let frame = driver.video_take_frame(&memory, 1).unwrap().unwrap();
    assert_eq!((frame.width(), frame.height()), (64, 64));
    assert_eq!(pixel(frame.rgba8(), 36, 20), RED); // First capture precedes second green draw.
    assert_eq!(pixel(frame.rgba8(), 52, 20), SENTINEL);
    let now = read(&memory, target, 65536);
    let visible = detile_32(
        &now,
        layout,
        now.capacity(),
        &PresentationLimits::default(),
        &mut work,
    )
    .unwrap();
    assert_eq!(pixel(&visible, 36, 20), GREEN);
    assert_eq!(pixel(&visible, 52, 20), GREEN);
    assert!(driver.video_take_frame(&memory, 2).unwrap().is_none()); // Oldest frame still Presenting.
    let pending = driver.video_flip_status(&memory, 1).unwrap().unwrap();
    assert_eq!(
        (pending.count, pending.pending, pending.gpu_pending),
        (0, 2, 2)
    );
    // The supplied tick is original test input, not an observed display tick.
    // No consumer presentation/acknowledgement/event is manufactured here.
    fn refused_clock(
        _: &mut pentax_kernel::Kernel,
        _: &mut PausedNativeMemory<'_>,
    ) -> Result<u64, crate::gpu::driver::Failure> {
        Err(crate::gpu::driver::Failure::Kernel(
            pentax_kernel::Failure::Unsupported("original clock refusal"),
        ))
    }
    assert!(matches!(
        driver.submit_with_kernel_clock(
            &mut memory,
            Function::CommandBuffer,
            [base + 0x100, base + 0x200, 0, 0, 0, 0],
            None,
            &mut kernel,
            refused_clock
        ),
        Err(crate::gpu::driver::Failure::Kernel(
            pentax_kernel::Failure::Unsupported("original clock refusal")
        ))
    ));
    assert_eq!(driver.usage().draws, 3); // One irreversible draw preceded the refused flip.
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
        Err(crate::gpu::driver::Failure::Poisoned)
    );
    assert_eq!(driver.usage(), before); // The retired prefix cannot be replayed.
}

#[test]
#[ignore = "requires a local Vulkan device"]
fn actual_current_draw_refuses_surface_clock_and_capture_failures_without_leaking_reservations() {
    use crate::gpu::driver::Failure;
    use crate::gpu::{
        graphics::execute_draw_current,
        video::{DisplayPolicy, State},
    };
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let target = base + 0x10000;
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
    commands.extend([0xc004_105c, 1, 0, 1, 7, 0]);
    let plan = plan_commands(
        &[&bytes(&commands)],
        GraphicsState::empty(),
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.draws().len(), 1);
    let pentax_gpu::graphics::Action::Flip(flip) = plan.actions()[1] else {
        panic!("original flip missing");
    };
    for case in 0..3 {
        let mut state = State::default();
        assert_eq!(
            state.invoke(
                Function::VideoOpen,
                [255, 0, 0, 0, 0, 0],
                60,
                DisplayPolicy::default(),
                None,
                &mut memory
            ),
            Ok(1)
        );
        let mut attribute = [0; 80];
        attribute[12..16].copy_from_slice(&64u32.to_le_bytes());
        attribute[16..20].copy_from_slice(&64u32.to_le_bytes());
        attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000u64.to_le_bytes());
        memory.write(base + 0x80, &attribute).unwrap();
        let registered = if case == 0 { target + 0x10000 } else { target };
        let mut registration = [0; 32];
        registration[..8].copy_from_slice(&registered.to_le_bytes());
        memory.write(base + 0x180, &registration).unwrap();
        memory.write(base + 8, &[0; 16]).unwrap();
        assert_eq!(
            state.invoke(
                Function::VideoRegisterBuffers2,
                [1, 0, 0, base + 0x180, 1, base + 0x80],
                60,
                DisplayPolicy::default(),
                Some(base),
                &mut memory
            ),
            Ok(0)
        );
        let current = execute_draw_current(
            &mut memory,
            &plan.draws()[0],
            &Limits::default(),
            &GpuLimits::default(),
            &[],
            |_, _, _| Ok(()),
        )
        .unwrap();
        let mut clock_calls = 0;
        let limits = PresentationLimits {
            max_frame_bytes: if case == 2 { 1 } else { 16 * 1024 * 1024 },
            ..Default::default()
        };
        let (_, result) = state.submit_gpu_flip(current, flip, &limits, |_| {
            clock_calls += 1;
            if case == 1 {
                Err(Failure::Kernel(pentax_kernel::Failure::Unsupported(
                    "original clock refusal",
                )))
            } else {
                Ok(0)
            }
        });
        assert_eq!(
            result,
            Err(match case {
                0 => Failure::UnsupportedVideo,
                1 => Failure::Kernel(pentax_kernel::Failure::Unsupported(
                    "original clock refusal"
                )),
                _ => Failure::Limit,
            })
        );
        assert_eq!(clock_calls, usize::from(case != 0));
        assert_eq!(state.flip_status(1), Some(Default::default()));
        assert_eq!(state.retained_frame_bytes(), 0);
        assert!(state.take_frame(1).unwrap().is_none());
        // CurrentDraw was consumed even on refusal; no API can replay it or
        // reconstruct it from the ordinary public completion report.
    }
}

#[path = "drain_flip_tests.rs"]
mod drain_flip_tests;

#[path = "memory_sync_tests.rs"]
mod memory_sync_tests;

#[path = "wait_flip_tests.rs"]
mod wait_flip_tests;
