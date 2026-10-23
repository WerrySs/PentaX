// SPDX-License-Identifier: GPL-2.0-only
//! Real indefinite CPU wait plus an actual admitted compute wait, without GPU work.
use super::graphics_wait_tests::{call, image, request, stopped, word, write};
use super::*;
use pentax_codegen::BlockLimits;
use pentax_core::process::hle::{
    NativeImageLimits, NativeServiceAction, with_native_image_session,
};

#[test]
fn all_parked_compute_pending_retries_with_existing_idle_cap_and_empty_queues_keep_deadlock() {
    for (queued, interactive) in [(false, false), (true, false), (true, true)] {
        let primary = image();
        let mut limits = NativeImageLimits::default();
        // A first retry must be refused before sleeping. No new sleep quota,
        // GPU hardware, invented ready predicate or wall-clock race is needed.
        limits.startup.kernel.max_sleep_nanoseconds = 0;
        with_native_image_session(&[&primary], &limits, |owner, layout| {
            let proof = owner
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let scratch = layout.entry.stack.address + 0x2000;
            let stack = layout.entry.return_slot - 64;
            write(owner, stack + 8, &[0; 16]);
            assert_eq!(
                call(
                    owner,
                    &proof,
                    b"IWIBBdTHit4",
                    b"libkernel",
                    [scratch, 0x10000, 0x33, 0, 0, 0],
                    stack
                ),
                0
            );
            let base = word(owner, scratch);
            if queued {
                // Actual false memory predicate retains a real owner-bound
                // submission. A bool supplied by this test cannot create it.
                write(owner, base + 0x3000, &[0; 8]);
                let words = [
                    0xc005_3c00_u32,
                    0x13,
                    (base + 0x3000) as u32,
                    ((base + 0x3000) >> 32) as u32,
                    7,
                    u32::MAX,
                    0,
                ];
                write(
                    owner,
                    base + 0x1000,
                    &words
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>(),
                );
                write(owner, base + 0x100, &(base + 0x1000).to_le_bytes());
                write(owner, base + 0x108, &[7, 0, 0, 0, 0, 0, 0, 0]);
                assert_eq!(
                    call(
                        owner,
                        &proof,
                        b"gSRnr79F8tQ",
                        b"Graphics5Driver",
                        [0x20, base + 0x100, 0, 0, 0, 0],
                        stack
                    ),
                    0
                );
            }
            let semaphore = base + 0x4000;
            assert_eq!(
                call(
                    owner,
                    &proof,
                    b"GEnUkDZoUwY",
                    b"libkernel",
                    [semaphore, 0, 0, 0, 0, 0],
                    stack
                ),
                0
            );
            let (module, request) = request(
                owner,
                &proof,
                b"C36iRE0F5sE",
                b"libkernel",
                [semaphore, 0, 0, 0, 0, 0],
                stack,
            );
            let (mut engine, token) = stopped(owner, &proof, request, layout.entry, module);
            let NativeServiceAction::WaitThread(pending) =
                owner.service_action(&proof, request).unwrap()
            else {
                panic!("actual zero-count indefinite semaphore must park");
            };
            assert_eq!(owner.thread_wait_delay(&pending).unwrap(), None);
            assert!(!owner.thread_wait_ready(&pending).unwrap());
            let mut scheduling = Scheduling::new(None, 1).unwrap();
            scheduling.wait(owner, pending, token).unwrap();
            assert!(scheduling.active_blocked());
            let before = *engine.state();
            // This is the production order: pump all admitted GPU queues before
            // scanning parked CPU readiness; derive pending only from Driver.
            let compute_pending = owner.pump_compute_queues().unwrap();
            assert_eq!(compute_pending, queued);
            let mut frames = Vec::new();
            let mut phase = NativeSessionPhase::MainEntry;
            let result = scheduling.rotate(
                owner,
                &proof,
                &mut engine,
                &mut frames,
                &mut phase,
                None,
                compute_pending,
                interactive,
            );
            if queued {
                assert!(matches!(
                    result,
                    Err(NativeSessionError::Budget("idle waits"))
                ));
            } else {
                assert!(matches!(result, Err(NativeSessionError::ThreadDeadlock)));
            }
            assert_eq!(*engine.state(), before);
            engine.validate_pending_syscall(token).unwrap();
            assert_eq!(scheduling.threads[0].waiting.as_ref().unwrap().1, token);
            assert_eq!(scheduling.idle_nanoseconds, 0);
            assert_eq!(scheduling.external_idle_nanoseconds, 0);
            assert!(frames.is_empty());
            assert!(
                !owner
                    .thread_wait_ready(&scheduling.threads[0].waiting.as_ref().unwrap().0)
                    .unwrap()
            );
            assert_eq!(
                owner.revoke_graphics_continuations_on_teardown().unwrap(),
                queued
            );
        })
        .unwrap();
    }
}
