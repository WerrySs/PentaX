// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use std::time::{Duration, Instant};

fn library(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn setup(fifo: bool, initial: u64, max: u64) -> (Kernel, TestGuest, u64) {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0x80, b"original-semaphore\0");
    owner.activate_thread_context(1).unwrap();
    owner
        .register_thread_context(2, ARG + 0x200, 100, 0x7f, &mut guest)
        .unwrap();
    owner
        .register_thread_context(3, ARG + 0x208, 700, 0x7f, &mut guest)
        .unwrap();
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            Service::CreateSema,
            &[ARG, ARG + 0x80, if fifo { 1 } else { 2 }, initial, max, 0]
        ),
        Ok(0)
    );
    let handle = guest.u64(0);
    (owner, guest, handle)
}
fn wait(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    handle: u64,
    need: u64,
    timeout: u64,
    now: Instant,
) -> SemaphoreWait {
    let SemaphoreWaitAction::Waiting(ticket) = owner
        .begin_semaphore_wait_at(CallArgs([handle, need, timeout, 0, 0, 0]), guest, now)
        .unwrap()
    else {
        panic!("expected pending semaphore");
    };
    ticket
}

#[test]
fn semaphore_grants_reserve_counts_once_and_output_failure_retains_the_exact_wait() {
    let (mut owner, mut guest, handle) = setup(true, 0, 4);
    let now = Instant::now();
    guest.put(0x100, &10_000_u32.to_le_bytes());
    let ticket = wait(&mut owner, &mut guest, handle, 2, ARG + 0x100, now);
    assert!(!owner.semaphore_wait_ready_at(&ticket, now).unwrap());
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        library(&mut owner, &mut guest, Service::SignalSema, &[handle, 3]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, Service::PollSema, &[handle, 1]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, Service::PollSema, &[handle, 1]),
        Ok(0x8002_0010)
    );
    assert!(
        owner
            .complete_semaphore_wait_at(&ticket, &mut guest, now)
            .is_err()
    );
    owner.activate_thread_context(1).unwrap();
    guest.refuse = true;
    assert_eq!(
        owner.complete_semaphore_wait_at(&ticket, &mut guest, now + Duration::from_millis(2)),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    owner.budget.limits.max_work = owner.usage().work + 2;
    assert!(matches!(
        owner.complete_semaphore_wait_at(&ticket, &mut guest, now),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_work = Limits::default().max_work;
    assert!(owner.semaphore_wait_ready_at(&ticket, now).unwrap());
    assert_eq!(
        owner.complete_semaphore_wait_at(&ticket, &mut guest, now + Duration::from_millis(2)),
        Ok(Some(0))
    );
    assert_eq!(guest.get(ARG + 0x100, 4), 8_000_u32.to_le_bytes());
    assert!(
        owner
            .complete_semaphore_wait_at(&ticket, &mut guest, now)
            .is_err()
    );
    assert!(!owner.threads.active().waiting);
}

#[test]
fn semaphore_wait_order_uses_fifo_or_real_registered_guest_priority() {
    for fifo in [true, false] {
        let (mut owner, mut guest, handle) = setup(fifo, 0, 4);
        let now = Instant::now();
        let first = wait(&mut owner, &mut guest, handle, 1, 0, now);
        owner.activate_thread_context(2).unwrap();
        let second = wait(&mut owner, &mut guest, handle, 1, 0, now);
        owner.activate_thread_context(3).unwrap();
        assert_eq!(
            library(&mut owner, &mut guest, Service::SignalSema, &[handle, 1]),
            Ok(0)
        );
        assert_eq!(owner.semaphore_wait_ready_at(&first, now).unwrap(), fifo);
        assert_eq!(owner.semaphore_wait_ready_at(&second, now).unwrap(), !fifo);
        assert_eq!(
            library(&mut owner, &mut guest, Service::SignalSema, &[handle, 1]),
            Ok(0)
        );
        owner.activate_thread_context(1).unwrap();
        assert_eq!(
            owner.complete_semaphore_wait_at(&first, &mut guest, now),
            Ok(Some(0))
        );
        owner.activate_thread_context(2).unwrap();
        assert_eq!(
            owner.complete_semaphore_wait_at(&second, &mut guest, now),
            Ok(Some(0))
        );
    }
}

#[test]
fn semaphore_scan_can_grant_past_large_head_and_deletion_preserves_reserved_grants() {
    let (mut owner, mut guest, handle) = setup(true, 0, 4);
    let now = Instant::now();
    let large = wait(&mut owner, &mut guest, handle, 3, 0, now);
    owner.activate_thread_context(2).unwrap();
    let small = wait(&mut owner, &mut guest, handle, 1, 0, now);
    owner.activate_thread_context(3).unwrap();
    assert_eq!(
        library(&mut owner, &mut guest, Service::SignalSema, &[handle, 1]),
        Ok(0)
    );
    assert!(!owner.semaphore_wait_ready_at(&large, now).unwrap());
    assert!(owner.semaphore_wait_ready_at(&small, now).unwrap());
    assert_eq!(
        library(&mut owner, &mut guest, Service::DeleteSema, &[handle]),
        Ok(0)
    );
    assert!(owner.semaphore_wait_ready_at(&large, now).unwrap());
    assert_eq!(
        library(&mut owner, &mut guest, Service::PollSema, &[handle, 1]),
        Ok(0x8002_0003)
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_semaphore_wait_at(&large, &mut guest, now),
        Ok(Some(0x8002_000d))
    );
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        owner.complete_semaphore_wait_at(&small, &mut guest, now),
        Ok(Some(0))
    );
}

#[test]
fn semaphore_cancel_reports_unready_count_atomically_and_retains_real_results() {
    let (mut owner, mut guest, handle) = setup(true, 1, 4);
    let now = Instant::now();
    let first = wait(&mut owner, &mut guest, handle, 2, 0, now);
    owner.activate_thread_context(2).unwrap();
    let second = wait(&mut owner, &mut guest, handle, 2, 0, now);
    owner.activate_thread_context(3).unwrap();
    guest.put(0x100, &99_u32.to_le_bytes());
    guest.refuse = true;
    assert!(
        library(
            &mut owner,
            &mut guest,
            Service::CancelSema,
            &[handle, u64::MAX, ARG + 0x100]
        )
        .is_err()
    );
    guest.refuse = false;
    assert!(!owner.semaphore_wait_ready_at(&first, now).unwrap());
    assert_eq!(guest.get(ARG + 0x100, 4), 99_u32.to_le_bytes());
    assert_eq!(
        library(
            &mut owner,
            &mut guest,
            Service::CancelSema,
            &[handle, u64::MAX, ARG + 0x100]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x100, 4), 2_u32.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, Service::PollSema, &[handle, 1]),
        Ok(0)
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_semaphore_wait_at(&first, &mut guest, now),
        Ok(Some(0x8002_0055))
    );
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        owner.complete_semaphore_wait_at(&second, &mut guest, now),
        Ok(Some(0x8002_0055))
    );
}

#[test]
fn semaphore_timeouts_are_monotonic_full_width_and_do_not_synthesize_acquisition() {
    let (mut owner, mut guest, handle) = setup(true, 0, 4);
    let now = Instant::now();
    guest.put(0x100, &2_000_u32.to_le_bytes());
    let ticket = wait(&mut owner, &mut guest, handle, 1, ARG + 0x100, now);
    assert!(
        !owner
            .semaphore_wait_ready_at(&ticket, now + Duration::from_millis(1))
            .unwrap()
    );
    assert_eq!(
        owner.complete_semaphore_wait_at(&ticket, &mut guest, now + Duration::from_millis(1)),
        Ok(None)
    );
    assert!(
        owner
            .semaphore_wait_ready_at(&ticket, now + Duration::from_millis(2))
            .unwrap()
    );
    assert_eq!(
        owner.complete_semaphore_wait_at(&ticket, &mut guest, now + Duration::from_millis(2)),
        Ok(Some(0x8002_003c))
    );
    assert_eq!(guest.get(ARG + 0x100, 4), [0; 4]);
    assert_eq!(
        library(&mut owner, &mut guest, Service::PollSema, &[handle, 1]),
        Ok(0x8002_0010)
    );
    guest.put(0x100, &u32::MAX.to_le_bytes());
    let long = wait(&mut owner, &mut guest, handle, 1, ARG + 0x100, now);
    assert!(
        !owner
            .semaphore_wait_ready_at(&long, now + Duration::from_millis(3))
            .unwrap()
    );
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        library(&mut owner, &mut guest, Service::CancelSema, &[handle, 0, 0]),
        Ok(0)
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_semaphore_wait_at(&long, &mut guest, now + Duration::from_millis(3)),
        Ok(Some(0x8002_0055))
    );
    assert_eq!(guest.get(ARG + 0x100, 4), (u32::MAX - 3000).to_le_bytes());
    assert!(!owner.threads.active().waiting);
}

#[test]
fn semaphore_tickets_are_kernel_bound_and_pending_kinds_cannot_share_one_thread() {
    let (mut owner, mut guest, handle) = setup(true, 0, 4);
    let now = Instant::now();
    let ticket = wait(&mut owner, &mut guest, handle, 1, 0, now);
    let (mut foreign, mut other, foreign_handle) = setup(true, 0, 4);
    let unrelated = wait(&mut foreign, &mut other, foreign_handle, 1, 0, now);
    assert!(foreign.semaphore_wait_ready(&ticket).is_err());
    assert!(owner.semaphore_wait_ready(&unrelated).is_err());
    assert!(matches!(
        owner.begin_semaphore_wait_at(CallArgs([handle, 1, 0, 0, 0, 0]), &mut guest, now),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        owner.begin_condition_wait(
            Service::CondWait,
            CallArgs([ARG + 0x300, ARG + 0x308, 0, 0, 0, 0]),
            &mut guest
        ),
        Err(Failure::Unsupported(_))
    ));
}
