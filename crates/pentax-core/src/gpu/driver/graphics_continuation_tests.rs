// SPDX-License-Identifier: GPL-2.0-only
//! Original owned queues, actual VideoOut Frames and source-ordered prefixes.
use super::*;
use crate::gpu::video::{FlipSource, FlipSubmission};
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn owner() -> (NativeAddressSpace, u64) {
    for index in 0..16 {
        let base = 0x57_0000_0000 + index * 0x100_0000;
        let mut owner = NativeAddressSpace::new(Default::default()).unwrap();
        if owner
            .map_at(
                base,
                0x30000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .is_ok()
        {
            return (owner, base);
        }
    }
    panic!("original bounded video address candidates unavailable");
}
fn setup(memory: &mut PausedNativeMemory<'_>, base: u64) -> Driver {
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            memory,
            super::super::Function::VideoOpen,
            [255, 0, 0, 0, 0, 0]
        ),
        Ok(1)
    );
    let mut attribute = [0; 80];
    attribute[12..16].copy_from_slice(&2u32.to_le_bytes());
    attribute[16..20].copy_from_slice(&2u32.to_le_bytes());
    attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000u64.to_le_bytes());
    memory.write(base + 0x80, &attribute).unwrap();
    let mut buffers = [0; 64];
    buffers[..8].copy_from_slice(&(base + 0x10000).to_le_bytes());
    buffers[32..40].copy_from_slice(&(base + 0x20000).to_le_bytes());
    memory.write(base + 0x200, &buffers).unwrap();
    memory.write(base + 8, &[0; 16]).unwrap();
    assert_eq!(
        driver.submit_with_stack(
            memory,
            super::super::Function::VideoRegisterBuffers2,
            [1, 0, 0, base + 0x200, 2, base + 0x80],
            Some(base)
        ),
        Ok(0)
    );
    driver
}
fn write_words(address: u64, value: u32) -> [u32; 5] {
    [
        0xc003_3700,
        0x100,
        address as u32,
        (address >> 32) as u32,
        value,
    ]
}
fn stream(memory: &mut PausedNativeMemory<'_>, base: u64, buffer: i32) -> Vec<NativeRange> {
    let mut words = write_words(base + 0x5000, 0x1122_3344).to_vec();
    words.extend([0xc005_1018, 1, buffer as u32, 0, 0, 0, 0]);
    words.extend(write_words(base + 0x8000, 0x5566_7788));
    memory
        .write(
            base + 0x1000,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    vec![NativeRange {
        address: base + 0x1000,
        size: 17 * 4,
    }]
}
fn value(memory: &PausedNativeMemory<'_>, address: u64) -> u32 {
    let mut bytes = [0; 4];
    memory.read(address, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
fn waiting(progress: GraphicsProgress) -> GraphicsTicket {
    match progress {
        GraphicsProgress::Waiting(ticket) => ticket,
        _ => panic!("actual selected wait was not parked"),
    }
}
fn cpu_flip(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>, buffer: i32) {
    assert_eq!(
        driver.video_submit_flip(memory, [1, buffer as u64, 1, 0, 0, 0], |_| Ok(100)),
        Ok(0)
    );
}
fn ack(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>) {
    let tick = driver.video_flip_status(memory, 1).unwrap().unwrap().count + 1;
    let frame = driver.video_take_frame(memory, tick).unwrap().unwrap();
    driver
        .video_acknowledge_frame(memory, &frame, |_| Ok((10, 200)))
        .unwrap();
}

#[test]
fn selected_cpu_ack_resumes_prefix_once_and_serializes_submission_and_suspend_fifo() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = setup(&mut memory, base);
    cpu_flip(&mut driver, &mut memory, 0);
    let ranges = stream(&mut memory, base, 0);
    let ticket = waiting(
        driver
            .begin_graphics_execution(&mut memory, &ranges, None, None)
            .unwrap(),
    );
    assert_eq!(
        (value(&memory, base + 0x5000), value(&memory, base + 0x8000)),
        (0x1122_3344, 0)
    );
    let prefix_work = driver.usage.memory_commands;
    assert!(!driver.poisoned);
    assert!(!driver.graphics_ticket_ready(&memory, &ticket).unwrap());
    // Taking actual pixels does not retire a selected pending flip.
    let frame = driver.video_take_frame(&memory, 1).unwrap().unwrap();
    assert!(!driver.graphics_ticket_ready(&memory, &ticket).unwrap());
    assert_eq!(
        driver
            .video_flip_status(&memory, 1)
            .unwrap()
            .unwrap()
            .pending,
        1
    );
    let boundary = waiting(
        driver
            .queue_graphics_suspend(&mut memory, None, None)
            .unwrap(),
    );
    let second = waiting(
        driver
            .begin_graphics_execution(&mut memory, &[], None, None)
            .unwrap(),
    );
    assert!(!driver.graphics_ticket_ready(&memory, &boundary).unwrap());
    assert!(!driver.graphics_ticket_ready(&memory, &second).unwrap());
    assert_eq!(
        driver.submit(&mut memory, super::super::Function::SuspendPoint, [0; 6]),
        Err(Failure::Waiting)
    );
    assert_eq!(driver.frame.boundaries, 0);
    driver
        .video_acknowledge_frame(&mut memory, &frame, |_| Ok((10, 200)))
        .unwrap();
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, ticket, None, None)
            .unwrap(),
        GraphicsProgress::Complete(0)
    ));
    assert_eq!(value(&memory, base + 0x8000), 0x5566_7788);
    assert_eq!(driver.usage.memory_commands, prefix_work + 1);
    assert!(matches!(
        driver.resume_graphics_execution(&mut memory, ticket, None, None),
        Err(Failure::UnsupportedQueue)
    ));
    assert!(!driver.poisoned); // Completed/stale receipt has no failure authority.
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, second, None, None)
            .unwrap(),
        GraphicsProgress::Waiting(_)
    ));
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, boundary, None, None)
            .unwrap(),
        GraphicsProgress::Complete(0)
    ));
    assert_eq!(driver.frame.boundaries, 1);
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, second, None, None)
            .unwrap(),
        GraphicsProgress::Complete(0)
    ));
}

#[test]
fn selected_wait_uses_cpu_and_gpu_request_phases_and_rejects_changed_port_generation() {
    for source in [FlipSource::Cpu, FlipSource::GpuEop] {
        for recording in [false, true] {
            let (mut owner, base) = owner();
            let mut memory = owner.pause().unwrap();
            let mut driver = setup(&mut memory, base);
            // A real reservation/recording is pending without fabricated GPU completion.
            let FlipSubmission::Accepted(flip) = driver
                .video
                .reserve_flip(1, 0, 1, 0, source, 100, memory.owner_id())
                .unwrap()
            else {
                panic!("actual bounded reservation refused")
            };
            if recording {
                driver
                    .video
                    .prepare_flip(flip, &memory, &Default::default())
                    .unwrap();
            }
            let ranges = stream(&mut memory, base, 0);
            let ticket = waiting(
                driver
                    .begin_graphics_execution(&mut memory, &ranges, None, None)
                    .unwrap(),
            );
            assert!(!driver.graphics_ticket_ready(&memory, &ticket).unwrap());
            assert!(driver.video_take_frame(&memory, 1).unwrap().is_none());
            // Closing cancels the real request and changes the selected generation;
            // reopening the numeric port must not release the old execution.
            assert_eq!(
                driver.submit(
                    &mut memory,
                    super::super::Function::VideoClose,
                    [1, 0, 0, 0, 0, 0]
                ),
                Ok(0)
            );
            assert_eq!(
                driver.submit(
                    &mut memory,
                    super::super::Function::VideoOpen,
                    [255, 0, 0, 0, 0, 0]
                ),
                Ok(1)
            );
            assert!(matches!(
                driver.resume_graphics_execution(&mut memory, ticket, None, None),
                Err(Failure::UnsupportedVideo)
            ));
            assert!(driver.poisoned);
            assert_eq!(value(&memory, base + 0x8000), 0);
        }
    }
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = setup(&mut memory, base);
    cpu_flip(&mut driver, &mut memory, 1);
    for selected in [-2, -1, 0, 15] {
        let ranges = stream(&mut memory, base, selected);
        assert!(matches!(
            driver
                .begin_graphics_execution(&mut memory, &ranges, None, None)
                .unwrap(),
            GraphicsProgress::Complete(0)
        ));
        assert!(driver.video_buffer_pending(&memory, 1, 1).unwrap());
    }
}

#[test]
fn parked_execution_rechecks_live_rights_and_retains_terminal_prefix_without_replay() {
    let (mut owner, base) = owner();
    let (mut driver, ticket, prefix) = {
        let mut memory = owner.pause().unwrap();
        let mut driver = setup(&mut memory, base);
        cpu_flip(&mut driver, &mut memory, 0);
        let ranges = stream(&mut memory, base, 0);
        let ticket = waiting(
            driver
                .begin_graphics_execution(&mut memory, &ranges, None, None)
                .unwrap(),
        );
        ack(&mut driver, &mut memory);
        let prefix = driver.usage.memory_commands;
        (driver, ticket, prefix)
    };
    owner
        .protect(base + 0x8000, 0x4000, NativePermissions::READ)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    assert!(
        driver
            .resume_graphics_execution(&mut memory, ticket, None, None)
            .is_err()
    );
    assert_eq!(
        (value(&memory, base + 0x5000), value(&memory, base + 0x8000)),
        (0x1122_3344, 0)
    );
    assert_eq!(driver.usage.memory_commands, prefix); // Failed write never replays earlier actions.
    assert!(driver.graphics_execution.is_some());
    assert!(driver.poisoned);
    assert!(matches!(
        driver.resume_graphics_execution(&mut memory, ticket, None, None),
        Err(Failure::Poisoned)
    ));
    assert!(matches!(
        driver.video_take_frame(&memory, 1),
        Err(Failure::Poisoned)
    ));
    assert!(driver.revoke_graphics_owner(memory.owner_id()).is_ok());
    assert!(driver.graphics_execution.is_none());
}

#[test]
fn opaque_tickets_budget_retention_and_peer_failure_preserve_exact_owner_authority() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = setup(&mut memory, base);
    cpu_flip(&mut driver, &mut memory, 0);
    let ranges = stream(&mut memory, base, 0);
    let ticket = waiting(
        driver
            .begin_graphics_execution(&mut memory, &ranges, None, None)
            .unwrap(),
    );
    let retained = driver.graphics_retained_bytes().unwrap();
    assert!(retained > 0);
    let frame_bytes = driver.video.retained_frame_bytes();
    let census = driver
        .video_consumer_admit(&memory, 4096, retained + frame_bytes, 0)
        .unwrap();
    assert_eq!(census, retained + frame_bytes);
    let before = driver.graphics_admissions.len();
    assert!(matches!(
        driver.begin_graphics_execution(&mut memory, &ranges, None, None),
        Err(Failure::Limit)
    ));
    assert_eq!(driver.graphics_admissions.len(), before);
    assert!(!driver.poisoned);
    let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 0x4000).unwrap();
    assert!(matches!(
        driver.resume_graphics_execution(&mut foreign.pause().unwrap(), ticket, None, None),
        Err(Failure::Memory)
    ));
    assert!(!driver.poisoned);
    // A second driver with the SAME memory owner cannot consume this identity.
    let mut other = setup(&mut memory, base);
    cpu_flip(&mut other, &mut memory, 0);
    let other_ticket = waiting(
        other
            .begin_graphics_execution(&mut memory, &ranges, None, None)
            .unwrap(),
    );
    assert!(matches!(
        other.resume_graphics_execution(&mut memory, ticket, None, None),
        Err(Failure::UnsupportedQueue)
    ));
    assert!(!other.poisoned);
    assert!(!other.graphics_ticket_ready(&memory, &other_ticket).unwrap());
    // A real post-reservation capture-quota failure from a peer operation
    // is terminal and wins over the saved prefix. A pre-reservation clock
    // refusal would correctly have no failure authority.
    assert_eq!(
        driver.video_submit_flip(&mut memory, [1, 1, 1, 0, 0, 0], |_| Ok(100)),
        Err(Failure::Limit)
    );
    assert!(matches!(
        driver.resume_graphics_execution(&mut memory, ticket, None, None),
        Err(Failure::Poisoned)
    ));
    driver.limits.max_preparation_work = 0;
    assert!(driver.revoke_graphics_owner(memory.owner_id()).is_ok()); // Budget-independent teardown.
}

#[test]
fn successive_selected_waits_issue_new_receipts_without_replaying_the_outer_submission() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = setup(&mut memory, base);
    cpu_flip(&mut driver, &mut memory, 0);
    cpu_flip(&mut driver, &mut memory, 1);
    let mut words = write_words(base + 0x5000, 11).to_vec();
    words.extend([0xc005_1018, 1, 0, 0, 0, 0, 0]);
    words.extend(write_words(base + 0x5004, 22));
    words.extend([0xc005_1018, 1, 1, 0, 0, 0, 0]);
    words.extend(write_words(base + 0x5008, 33));
    let size = words.len() as u64 * 4;
    memory
        .write(
            base + 0x1000,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let first = waiting(
        driver
            .begin_graphics_execution(
                &mut memory,
                &[NativeRange {
                    address: base + 0x1000,
                    size,
                }],
                None,
                None,
            )
            .unwrap(),
    );
    assert_eq!(
        (value(&memory, base + 0x5000), value(&memory, base + 0x5004)),
        (11, 0)
    );
    ack(&mut driver, &mut memory);
    let second = waiting(
        driver
            .resume_graphics_execution(&mut memory, first, None, None)
            .unwrap(),
    );
    assert_eq!(
        (value(&memory, base + 0x5004), value(&memory, base + 0x5008)),
        (22, 0)
    );
    assert!(matches!(
        driver.resume_graphics_execution(&mut memory, first, None, None),
        Err(Failure::UnsupportedQueue)
    ));
    assert!(!driver.poisoned);
    ack(&mut driver, &mut memory);
    assert!(matches!(
        driver
            .resume_graphics_execution(&mut memory, second, None, None)
            .unwrap(),
        GraphicsProgress::Complete(0)
    ));
    assert_eq!(value(&memory, base + 0x5008), 33);
    assert_eq!(driver.usage.memory_commands, 5); // Three writes, each wait flush only once.
}

#[test]
fn exact_admitted_poll_budget_failure_keeps_parked_storage_until_owner_cleanup() {
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = setup(&mut memory, base);
    cpu_flip(&mut driver, &mut memory, 0);
    let ranges = stream(&mut memory, base, 0);
    let ticket = waiting(
        driver
            .begin_graphics_execution(&mut memory, &ranges, None, None)
            .unwrap(),
    );
    let retained = driver.graphics_retained_bytes().unwrap();
    driver.limits.max_preparation_work = driver.usage.preparation_work;
    assert!(matches!(
        driver.graphics_ticket_ready(&memory, &ticket),
        Err(Failure::Limit)
    ));
    assert!(driver.poisoned);
    assert_eq!(driver.graphics_retained_bytes().unwrap(), retained);
    assert_eq!(value(&memory, base + 0x8000), 0);
    assert!(matches!(
        driver.begin_graphics_execution(&mut memory, &[], None, None),
        Err(Failure::Poisoned)
    ));
    driver.revoke_graphics_owner(memory.owner_id()).unwrap();
    assert_eq!(driver.graphics_retained_bytes().unwrap(), 0);
}

#[test]
fn full_flip_queue_is_admitted_at_the_real_turn_after_actual_selected_requests_retire() {
    fn forbidden_clock(
        _: &mut pentax_kernel::Kernel,
        _: &mut PausedNativeMemory<'_>,
    ) -> Result<u64, Failure> {
        panic!("no future flip may sample its clock before the actual draw")
    }
    let (mut owner, base) = owner();
    let mut memory = owner.pause().unwrap();
    let mut driver = setup(&mut memory, base);
    for _ in 0..16 {
        cpu_flip(&mut driver, &mut memory, 0);
    }
    let words = [
        0xc005_1018,
        1,
        0,
        0,
        0,
        0,
        0,
        0xc000_2f00,
        1,
        0xc001_2d00,
        3,
        2,
        0xc004_105c,
        1,
        0,
        1,
        77,
        0,
    ];
    memory
        .write(
            base + 0x1000,
            &words
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let mut kernel = pentax_kernel::Kernel::new(
        Default::default(),
        pentax_kernel::ProcessIdentity {
            process_id: 7,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap();
    let ticket = waiting(
        driver
            .begin_graphics_execution(
                &mut memory,
                &[NativeRange {
                    address: base + 0x1000,
                    size: words.len() as u64 * 4,
                }],
                Some(&mut kernel),
                Some(forbidden_clock),
            )
            .unwrap(),
    );
    assert_eq!(
        driver
            .video_flip_status(&memory, 1)
            .unwrap()
            .unwrap()
            .pending,
        16
    );
    let run = driver.graphics_execution.as_ref().unwrap();
    let flip = run.plan.flips().next().unwrap();
    let (retained, maximum, presentation) = (run.retained, run.max_host_bytes, run.presentation);
    assert!(matches!(
        driver.admit_graphics_flip_turn(flip, retained, maximum, &presentation),
        Err(Failure::Waiting)
    ));
    for _ in 0..16 {
        ack(&mut driver, &mut memory);
    }
    assert!(driver.graphics_ticket_ready(&memory, &ticket).unwrap());
    assert_eq!(
        driver.admit_graphics_flip_turn(flip, retained, maximum, &presentation),
        Ok(())
    );
    assert_eq!(
        driver
            .video_flip_status(&memory, 1)
            .unwrap()
            .unwrap()
            .pending,
        0
    );
    assert_eq!(driver.usage.draws, 0); // Admission is not a draw or fabricated completion.
    assert_eq!(kernel.usage().attempts, 0);
    driver.revoke_graphics_owner(memory.owner_id()).unwrap();
}
