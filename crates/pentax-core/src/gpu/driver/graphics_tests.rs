// SPDX-License-Identifier: GPL-2.0-only
//! Original command streams; these refusals and zero draws never enter Vulkan.
use super::*;
use crate::gpu::driver::{Function, Limits};
use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryError, NativePermissions};

fn original_video_owner() -> (NativeAddressSpace, u64) {
    // VideoOut's source tracker admits only the low 40-bit range. An
    // OS-selected native arena may validly be higher, independently of GPU
    // or clock support, so choose one bounded original low-address mapping.
    for index in 0..16 {
        let base = 0x55_0000_0000 + index * 0x100_0000;
        let mut owner = NativeAddressSpace::new(Default::default()).unwrap();
        match owner.map_at(
            base,
            0x20000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        ) {
            Ok(()) => {
                assert!(base + 0x20000 <= 1 << 40);
                return (owner, base);
            }
            Err(NativeMemoryError::OperatingSystem {
                operation: "VirtualAlloc2(placeholder)",
                code: 487,
                ..
            }) => continue, // Actual no-overwrite reservation collision only.
            Err(_) => panic!("original video backing admission failed"),
        }
    }
    panic!("original bounded video address candidates unavailable");
}

fn words(memory: &mut PausedNativeMemory<'_>, address: u64, input: &[u32]) {
    let bytes: Vec<_> = input.iter().flat_map(|word| word.to_le_bytes()).collect();
    memory.write(address, &bytes).unwrap();
}
fn packet(memory: &mut PausedNativeMemory<'_>, address: u64, commands: u64, dwords: u32) {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&commands.to_le_bytes());
    bytes[8..12].copy_from_slice(&dwords.to_le_bytes());
    memory.write(address, &bytes).unwrap();
}

#[test]
fn context_graphics_retains_one_queue_state_but_explicit_acb_and_done_do_not_invent_it() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    memory.write(base + 4, &0_u32.to_le_bytes()).unwrap();
    words(&mut memory, base + 0x100, &[0xc000_2f00, 1]); // NUM_INSTANCES
    packet(&mut memory, base + 0x20, base + 0x100, 2);
    let mut driver = Driver::new(Limits::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert!(!driver.frame.reset_before_graphics);
    assert_eq!(driver.graphics_owner, Some(memory.owner_id()));
    assert_eq!(driver.usage.calls, 1);
    assert_eq!(driver.usage.command_buffers, 1);
    assert_eq!(driver.usage.draws, 0);
    assert_eq!(driver.usage.dispatches, 0);
    assert_eq!(driver.usage.written_bytes, 0);
    // A different non-ACB selector is the same source graphics processor.
    memory
        .write(base + 4, &0xffff_ffff_u32.to_le_bytes())
        .unwrap();
    words(&mut memory, base + 0x100, &[0xc001_2d00, 0, 2]);
    packet(&mut memory, base + 0x20, base + 0x100, 3);
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(driver.usage.draws, 0);
    assert_eq!(driver.usage.calls, 2);
    assert_eq!(driver.usage.command_buffers, 2);
    assert_eq!(
        driver.submit(&mut memory, Function::Acb, [0, base + 0x20, 0, 0, 0, 0]),
        Err(Failure::UnsupportedQueue)
    );
    assert_eq!(
        driver.submit(&mut memory, Function::SuspendPoint, [0; 6]),
        Ok(0)
    );
    assert!(matches!(
        driver.submit(
            &mut memory,
            Function::CommandBuffer,
            [base, base + 0x20, 0, 0, 0, 0]
        ),
        Err(Failure::GraphicsPreparation(draw::Error::Unsupported(
            "unknown NUM_INSTANCES"
        )))
    ));
    assert!(driver.frame.reset_before_graphics);
    assert!(!driver.poisoned);
    assert_eq!(driver.usage.written_bytes, 0);
    // Retained register state is never accepted on a different actual owner.
    let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let mut foreign_memory = foreign.pause().unwrap();
    assert_eq!(
        driver.submit_graphics(
            &mut foreign_memory,
            &[NativeRange {
                address: base + 0x100,
                size: 12
            }]
        ),
        Err(Failure::Memory)
    );
}

#[test]
fn complete_multibuffer_admission_and_live_refetch_precede_state_publication_or_gpu_effects() {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(
            base,
            0x4000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    let mut memory = owner.pause().unwrap();
    memory.write(base + 4, &0_u32.to_le_bytes()).unwrap();
    memory
        .write(base + 0x40, &(base + 0x100).to_le_bytes())
        .unwrap();
    memory
        .write(base + 0x48, &(base + 0x180).to_le_bytes())
        .unwrap();
    words(&mut memory, base + 0x60, &[5, 1]);
    words(
        &mut memory,
        base + 0x100,
        &[0xc000_2f00, 1, 0xc001_2d00, 0, 2],
    );
    words(&mut memory, base + 0x180, &[0]); // unsupported TYPE0, after valid prefix
    memory.write(base + 0x200, &[0xa5; 32]).unwrap();
    let args = [base, base + 0x40, base + 0x60, 2, 0, 0];
    let mut driver = Driver::new(Limits::default());
    assert!(matches!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Err(Failure::GraphicsPreparation(draw::Error::Packet {
            buffer: 1,
            ..
        }))
    ));
    assert!(driver.frame.reset_before_graphics);
    assert_eq!(driver.graphics_owner, None);
    assert!(!driver.poisoned);
    // Mutating the live second stream must be seen; no retained source replay.
    words(&mut memory, base + 0x180, &[0x8000_0000]);
    assert_eq!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Ok(0)
    );
    assert!(!driver.frame.reset_before_graphics);
    let mut actual = [0; 32];
    memory.read(base + 0x200, &mut actual).unwrap();
    assert_eq!(actual, [0xa5; 32]);
    assert_eq!(driver.usage.draws, 0);
    assert_eq!(driver.usage.written_bytes, 0);
    // Real nonzero draw without required resources refuses before the GPU;
    // publication/reset do not advance. A terminal driver never retries it.
    driver
        .submit(&mut memory, Function::SuspendPoint, [0; 6])
        .unwrap();
    words(
        &mut memory,
        base + 0x100,
        &[0xc000_2f00, 1, 0xc001_2d00, 3, 2],
    );
    assert!(matches!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Err(Failure::GraphicsPreparation(_))
    ));
    assert!(driver.frame.reset_before_graphics);
    assert!(!driver.poisoned);
    driver.poisoned = true;
    let before = driver.usage();
    assert_eq!(
        driver.submit(&mut memory, Function::MultiCommandBuffers, args),
        Err(Failure::Poisoned)
    );
    assert_eq!(driver.usage(), before);
}

#[test]
fn gpu_flip_clock_owner_full_queue_and_capture_quota_refuse_before_any_draw() {
    fn forbidden_clock(
        _: &mut pentax_kernel::Kernel,
        _: &mut PausedNativeMemory<'_>,
    ) -> Result<u64, Failure> {
        panic!("pre-effect admission must not sample the clock");
    }
    let (mut owner, base) = original_video_owner();
    let mut memory = owner.pause().unwrap();
    words(
        &mut memory,
        base + 0x1000,
        &[
            0xc000_2f00,
            1,
            0xc001_2d00,
            3,
            2,
            0xc004_105c,
            1,
            0,
            1,
            0,
            0,
        ],
    );
    let ranges = [NativeRange {
        address: base + 0x1000,
        size: 44,
    }];
    let mut kernel = pentax_kernel::Kernel::new(
        Default::default(),
        pentax_kernel::ProcessIdentity {
            process_id: 7,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap();
    let mut missing = Driver::new(Limits::default());
    assert!(matches!(
        missing.submit_graphics(&mut memory, &ranges),
        Err(Failure::Kernel(pentax_kernel::Failure::Unsupported(
            "bound GPU flip process clock"
        )))
    ));
    assert_eq!(missing.usage.draws, 0);
    assert!(!missing.poisoned);

    for full in [false, true] {
        let mut limits = Limits::default();
        if !full {
            limits.video_presentation.max_frame_bytes = 1;
        }
        let mut driver = Driver::new(limits);
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
        registration[..8].copy_from_slice(&(base + 0x10000).to_le_bytes());
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
        if full {
            for _ in 0..16 {
                assert!(matches!(
                    driver.video.reserve_flip(
                        1,
                        0,
                        1,
                        0,
                        super::super::super::video::FlipSource::Cpu,
                        0,
                        memory.owner_id()
                    ),
                    Ok(super::super::super::video::FlipSubmission::Accepted(_))
                ));
            }
        }
        assert_eq!(
            driver.submit_graphics_with_context(
                &mut memory,
                &ranges,
                Some(&mut kernel),
                Some(forbidden_clock)
            ),
            Err(if full {
                Failure::Waiting
            } else {
                Failure::Limit
            })
        );
        assert_eq!(driver.usage.draws, 0);
        assert_eq!(driver.usage.written_bytes, 0);
        assert!(!driver.poisoned);
        assert!(driver.frame.reset_before_graphics);
        assert_eq!(
            driver.video.flip_status(1).unwrap().pending,
            if full { 16 } else { 0 }
        );
        let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let foreign_memory = foreign.pause().unwrap();
        assert_eq!(
            driver.video.check_owner(foreign_memory.owner_id()),
            Err(Failure::Memory)
        );
    }
    assert_eq!(kernel.usage().attempts, 0);
}
