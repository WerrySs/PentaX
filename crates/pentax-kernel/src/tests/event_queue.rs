// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::Kernel;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use std::time::{Duration, Instant};

#[test]
fn graphics_interrupt_subscriptions_coalesce_saturate_and_reset_without_false_notifications() {
    let (mut owner, mut guest, first) = setup();
    call(
        &mut owner,
        &mut guest,
        Service::CreateEqueue,
        &[ARG + 8, ARG + 0x80],
    )
    .unwrap();
    let second = guest.u64(8);
    let handles = owner.usage().handles_live;
    assert_eq!(owner.add_graphics_event(first, 0x20, 17), Ok(0));
    assert_eq!(owner.add_graphics_event(second, 0x20, 27), Ok(0));
    assert_eq!(owner.add_graphics_event(first, 0x21, 37), Ok(0));
    assert_eq!(owner.add_graphics_event(first, 0x20, 47), Ok(0));
    assert_eq!(owner.usage().handles_live, handles + 3);
    drop(owner.prepare_graphics_interrupt(0x20, 1).unwrap());
    assert_eq!(poll(&mut owner, &mut guest, first, 8), Ok(0x8002_003c));
    owner
        .prepare_graphics_interrupt(0x20, 7)
        .unwrap()
        .complete();
    owner
        .prepare_graphics_interrupt(0x20, 9)
        .unwrap()
        .complete();
    assert_eq!(poll(&mut owner, &mut guest, first, 8), Ok(0));
    assert_eq!(event(&guest, 0x200), (0x20, -14, 0, 2, 9, 47));
    assert_eq!(guest.get(ARG + 0x20, 4), 1_u32.to_le_bytes());
    assert_eq!(poll(&mut owner, &mut guest, second, 8), Ok(0));
    assert_eq!(event(&guest, 0x200), (0x20, -14, 0, 2, 9, 27));
    assert_eq!(poll(&mut owner, &mut guest, first, 8), Ok(0x8002_003c));
    let at = owner.event_queues.index(first, &mut owner.budget).unwrap();
    owner.event_queues.queues[at].events[0].fflags = u32::MAX;
    owner
        .prepare_graphics_interrupt(0x20, 11)
        .unwrap()
        .complete();
    assert_eq!(poll(&mut owner, &mut guest, first, 8), Ok(0));
    assert_eq!(event(&guest, 0x200).3, u32::MAX);
    assert_eq!(owner.delete_graphics_event(first, 0x20), Ok(0));
    assert_eq!(
        owner.delete_graphics_event(first, 0x20),
        Err(Failure::Errno(Errno::NoEntry))
    );
    assert_eq!(
        owner.delete_graphics_event(0, 0x20),
        Err(Failure::Errno(Errno::BadFd))
    );
    owner.budget.limits.max_work = owner.usage().work;
    assert!(owner.prepare_graphics_interrupt(0x21, 99).is_err());
}

fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn setup() -> (Kernel, TestGuest, u64) {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0x80, b"original-event-queue\0");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::CreateEqueue,
            &[ARG, ARG + 0x80]
        ),
        Ok(0)
    );
    let id = guest.u64(0);
    (owner, guest, id)
}
fn poll(owner: &mut Kernel, guest: &mut TestGuest, id: u64, maximum: u64) -> Result<u64, Failure> {
    guest.put(0x40, &0_u32.to_le_bytes());
    call(
        owner,
        guest,
        Service::WaitEqueue,
        &[id, ARG + 0x200, maximum, ARG + 0x20, ARG + 0x40],
    )
}
fn event(guest: &TestGuest, offset: u64) -> (u64, i16, u16, u32, u64, u64) {
    let b = guest.get(ARG + offset, 32);
    (
        u64::from_le_bytes(b[..8].try_into().unwrap()),
        i16::from_le_bytes(b[8..10].try_into().unwrap()),
        u16::from_le_bytes(b[10..12].try_into().unwrap()),
        u32::from_le_bytes(b[12..16].try_into().unwrap()),
        u64::from_le_bytes(b[16..24].try_into().unwrap()),
        u64::from_le_bytes(b[24..32].try_into().unwrap()),
    )
}

#[test]
fn event_queues_retain_order_coalesce_user_updates_and_preserve_readd_flags() {
    let (mut owner, mut guest, id) = setup();
    let handles = owner.usage().handles_live;
    for (service, ident) in [
        (Service::AddUserEvent, 1),
        (Service::AddUserEventEdge, 2),
        (Service::AddUserEvent, 3),
    ] {
        assert_eq!(call(&mut owner, &mut guest, service, &[id, ident]), Ok(0));
    }
    // Trigger order differs from insertion order; repeated triggers preserve
    // the original active position but publish the most recent data.
    for (ident, data) in [(3, 30), (1, 10), (3, 31), (2, 20)] {
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::TriggerUserEvent,
                &[id, ident, data]
            ),
            Ok(0)
        );
    }
    assert_eq!(
        call(&mut owner, &mut guest, Service::AddUserEventEdge, &[id, 1]),
        Ok(0)
    );
    assert_eq!(poll(&mut owner, &mut guest, id, 2), Ok(0));
    assert_eq!(event(&guest, 0x200), (3, -11, 1, 0, 31, 31));
    // Re-add updates udata to0, retaining pending data and original level flags.
    assert_eq!(event(&guest, 0x220), (1, -11, 1, 0, 10, 0));
    assert_eq!(poll(&mut owner, &mut guest, id, 9), Ok(0));
    assert_eq!(event(&guest, 0x200), (2, -11, 0x21, 0, 20, 20));
    assert_eq!(event(&guest, 0x220).0, 3);
    assert_eq!(event(&guest, 0x240).0, 1);
    assert_eq!(guest.get(ARG + 0x20, 4), 3_u32.to_le_bytes());
    assert_eq!(poll(&mut owner, &mut guest, id, 9), Ok(0));
    assert_eq!(guest.get(ARG + 0x20, 4), 2_u32.to_le_bytes());
    assert_eq!(owner.usage().handles_live, handles + 3);
    assert_eq!(
        call(&mut owner, &mut guest, Service::DeleteUserEvent, &[id, 3]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::DeleteEqueue, &[id]),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, handles - 1);
    assert_eq!(
        call(&mut owner, &mut guest, Service::DeleteEqueue, &[id]),
        Ok(0x8002_0009)
    );
}

#[test]
fn event_queues_publish_exact_fields_and_order_overlapping_count_without_losing_faulted_events() {
    let (mut owner, mut guest, id) = setup();
    call(
        &mut owner,
        &mut guest,
        Service::AddUserEventEdge,
        &[id, u64::MAX],
    )
    .unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::TriggerUserEvent,
        &[id, u64::MAX, 0xf123_4567_89ab_cdef],
    )
    .unwrap();
    guest.put(0x200, &[0xa5; 40]);
    guest.put(0x40, &[0; 4]);
    guest.refuse = true;
    assert!(
        call(
            &mut owner,
            &mut guest,
            Service::WaitEqueue,
            &[id, ARG + 0x200, 1, ARG + 0x20, ARG + 0x40]
        )
        .is_err()
    );
    guest.refuse = false;
    assert_eq!(guest.get(ARG + 0x200, 40), [0xa5; 40]);
    assert_eq!(poll(&mut owner, &mut guest, id, 1), Ok(0));
    assert_eq!(
        event(&guest, 0x200),
        (
            u64::MAX,
            -11,
            0x21,
            0,
            0xf123_4567_89ab_cdef,
            0xf123_4567_89ab_cdef
        )
    );
    for (service, expected) in [
        (Service::GetEventId, u64::MAX),
        (Service::GetEventFilter, 0xffff_fff5),
        (Service::GetEventData, 0xf123_4567_89ab_cdef),
        (Service::GetEventUserData, 0xf123_4567_89ab_cdef),
        (Service::GetEventFflags, 0),
        (Service::GetEventError, 0),
    ] {
        assert_eq!(
            call(&mut owner, &mut guest, service, &[ARG + 0x200]),
            Ok(expected)
        );
        assert_eq!(call(&mut owner, &mut guest, service, &[0]), Ok(0));
    }
    guest.put(0x20a, &0x4000_u16.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::GetEventError,
            &[ARG + 0x200]
        ),
        Ok(0x89ab_cdef)
    );
    call(
        &mut owner,
        &mut guest,
        Service::TriggerUserEvent,
        &[id, u64::MAX, 0x1234],
    )
    .unwrap();
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::WaitEqueue,
            &[id, ARG + 0x200, 1, ARG + 0x212, ARG + 0x40]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x212, 4), 1_u32.to_le_bytes());
    assert_eq!(guest.get(ARG + 0x220, 8), [0xa5; 8]);
    assert_eq!(poll(&mut owner, &mut guest, id, 1), Ok(0x8002_003c));
    assert_eq!(guest.get(ARG + 0x20, 4), [0; 4]);
    // GetError reads data only when ERROR is set; getters read exact widths.
    guest.put(PAGE - 2, &[0; 2]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::GetEventError,
            &[ARG + PAGE - 12]
        ),
        Ok(0)
    );
}

#[test]
fn event_queues_creation_is_atomic_alias_aware_bounded_and_owner_local() {
    let (mut owner, mut guest, id) = setup();
    let handles = owner.usage().handles_live;
    guest.refuse = true;
    assert!(
        call(
            &mut owner,
            &mut guest,
            Service::CreateEqueue,
            &[ARG + 8, ARG + 0x80]
        )
        .is_err()
    );
    assert_eq!(owner.usage().handles_live, handles);
    guest.refuse = false;
    // Handle2 is visible to the source name copy before output commit.
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::CreateEqueue,
            &[ARG + 8, ARG + 8]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(8), id + 1);
    assert_eq!(owner.event_queues.queues.last().unwrap()._name, [2]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::CreateEqueue,
            &[0, ARG + 0x80]
        ),
        Ok(0x8002_0016)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::WaitEqueue,
            &[0, 0, 0, 0, 0]
        ),
        Ok(0x8002_0009)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::WaitEqueue,
            &[id, 0, 0, 0, 0]
        ),
        Ok(0x8002_000e)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::WaitEqueue,
            &[id, ARG, 0, 0, 0]
        ),
        Ok(0x8002_0016)
    );
    let mut other = kernel(Default::default());
    assert_eq!(
        call(&mut other, &mut guest, Service::DeleteEqueue, &[id]),
        Ok(0x8002_0009)
    );
    owner.budget.limits.max_handles = owner.usage().handles_live;
    assert!(matches!(
        call(&mut owner, &mut guest, Service::AddUserEvent, &[id, 1]),
        Err(Failure::Limit(_))
    ));
}

#[test]
fn event_queues_timer_deadlines_coalesce_periods_and_consume_high_resolution_once() {
    let (mut owner, mut guest, id) = setup();
    // Deterministic clock injection; no sleeping or timing-dependent oracle.
    let base = Instant::now();
    let start = owner.event_queues.nanos(base);
    let at = owner.event_queues.index(id, &mut owner.budget).unwrap();
    let make = |ident, filter, flags, deadline, interval| crate::event_queue::Event {
        ident,
        filter,
        flags,
        fflags: 0,
        data: 0,
        udata: ident + 100,
        deadline,
        interval,
        active: 0,
    };
    owner
        .event_queues
        .add(
            at,
            make(1, -7, 0x21, start + 1000, 1000),
            &mut owner.budget,
            start,
        )
        .unwrap();
    owner
        .event_queues
        .add(
            at,
            make(2, -15, 0x11, start + 500, 0),
            &mut owner.budget,
            start,
        )
        .unwrap();
    let args = CallArgs([id, ARG + 0x200, 9, ARG + 0x20, 0, 0]);
    assert_eq!(
        owner.event_queues.poll(
            at,
            args,
            &mut guest,
            &mut owner.budget,
            base + Duration::from_nanos(3400)
        ),
        Ok(Some(0))
    );
    assert_eq!(event(&guest, 0x200), (2, -15, 0x11, 0, 0, 102));
    assert_eq!(event(&guest, 0x220), (1, -7, 0x21, 0, 3, 101));
    assert_eq!(owner.event_queues.queues[at].events.len(), 1);
    assert_eq!(
        owner.event_queues.poll(
            at,
            args,
            &mut guest,
            &mut owner.budget,
            base + Duration::from_nanos(4500)
        ),
        Ok(Some(0))
    );
    assert_eq!(event(&guest, 0x200).4, 1);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::AddHRTimerEvent,
            &[id, 3, 0, 0]
        ),
        Ok(0x8002_000e)
    );
    guest.put(0x100, &(-1_i64).to_le_bytes());
    guest.put(0x108, &[0; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::AddHRTimerEvent,
            &[id, 3, ARG + 0x100, 0]
        ),
        Ok(0x8002_0016)
    );
}

#[test]
fn event_queue_waits_park_recheck_competing_consumers_retry_faults_and_wake_on_close() {
    let (mut owner, mut guest, id) = setup();
    owner.activate_thread_context(1).unwrap();
    owner
        .register_thread_context(2, ARG + 0x300, 100, 0x7f, &mut guest)
        .unwrap();
    owner
        .register_thread_context(3, ARG + 0x308, 100, 0x7f, &mut guest)
        .unwrap();
    let args = CallArgs([id, ARG + 0x200, 1, ARG + 0x20, 0, 0]);
    let now = Instant::now();
    let EventQueueWaitAction::Waiting(ticket) = owner
        .begin_event_queue_wait_at(args, &mut guest, now)
        .unwrap()
    else {
        panic!("must wait")
    };
    assert!(!owner.event_queue_wait_ready_at(&ticket, now).unwrap());
    let mut other = kernel(Default::default());
    assert!(other.event_queue_wait_ready_at(&ticket, now).is_err());
    assert!(owner.event_queue_wait_delay(&ticket).unwrap().is_none());
    owner.activate_thread_context(2).unwrap();
    call(&mut owner, &mut guest, Service::AddUserEventEdge, &[id, 7]).unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::TriggerUserEvent,
        &[id, 7, 99],
    )
    .unwrap();
    assert!(owner.event_queue_wait_ready_at(&ticket, now).unwrap());
    assert!(
        owner
            .complete_event_queue_wait_at(&ticket, &mut guest, now)
            .is_err()
    );
    // A competing thread may consume the edge before the sleeping one resumes.
    assert_eq!(poll(&mut owner, &mut guest, id, 1), Ok(0));
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_event_queue_wait_at(&ticket, &mut guest, now),
        Ok(None)
    );
    owner.activate_thread_context(2).unwrap();
    call(
        &mut owner,
        &mut guest,
        Service::TriggerUserEvent,
        &[id, 7, 100],
    )
    .unwrap();
    owner.activate_thread_context(1).unwrap();
    guest.refuse = true;
    assert!(
        owner
            .complete_event_queue_wait_at(&ticket, &mut guest, now)
            .is_err()
    );
    guest.refuse = false;
    assert_eq!(
        owner.complete_event_queue_wait_at(&ticket, &mut guest, now),
        Ok(Some(0))
    );
    assert_eq!(event(&guest, 0x200).4, 100);
    let EventQueueWaitAction::Waiting(closed) = owner
        .begin_event_queue_wait_at(args, &mut guest, now)
        .unwrap()
    else {
        panic!("must wait")
    };
    owner.activate_thread_context(2).unwrap();
    call(&mut owner, &mut guest, Service::DeleteEqueue, &[id]).unwrap();
    assert!(owner.event_queue_wait_ready_at(&closed, now).unwrap());
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        owner.complete_event_queue_wait_at(&closed, &mut guest, now),
        Ok(Some(0x8002_0009))
    );
    assert_eq!(guest.get(ARG + 0x20, 4), 0x8002_0009_u32.to_le_bytes());
    assert!(
        owner
            .complete_event_queue_wait_at(&closed, &mut guest, now)
            .is_err()
    );
    // A finite timeout preserves its input and publishes count0 exactly once.
    call(
        &mut owner,
        &mut guest,
        Service::CreateEqueue,
        &[ARG, ARG + 0x80],
    )
    .unwrap();
    let id = guest.u64(0);
    guest.put(0x40, &100_u32.to_le_bytes());
    let EventQueueWaitAction::Waiting(timed) = owner
        .begin_event_queue_wait_at(
            CallArgs([id, ARG + 0x200, 1, ARG + 0x20, ARG + 0x40, 0]),
            &mut guest,
            now,
        )
        .unwrap()
    else {
        panic!("must wait")
    };
    assert!(!owner.event_queue_wait_ready_at(&timed, now).unwrap());
    assert!(
        owner
            .event_queue_wait_ready_at(&timed, now + Duration::from_micros(100))
            .unwrap()
    );
    assert_eq!(
        owner.complete_event_queue_wait_at(&timed, &mut guest, now + Duration::from_micros(100)),
        Ok(Some(0x8002_003c))
    );
    assert_eq!(guest.get(ARG + 0x40, 4), 100_u32.to_le_bytes());
}
