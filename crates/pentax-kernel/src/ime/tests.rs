// SPDX-License-Identifier: GPL-2.0-only
//! Original owned-kernel fixtures; numeric completions here model callbacks.
//! Separate worker fixtures prove actual guest callback execution and returns.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{Kernel, Limits, MemoryEffect};

const HANDLER: u64 = 0x1234_5000;
const TEXT_PARAM: u64 = ARG + 0x100;
const KEY_PARAM: u64 = ARG + 0x200;
const WORK: u64 = ARG + PAGE;

fn args(values: &[u64]) -> CallArgs {
    let mut words = [0; 6];
    words[..values.len()].copy_from_slice(values);
    CallArgs(words)
}
fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    owner.invoke_library(service, args(values), guest)
}
fn fixture(limits: Limits) -> (Kernel, TestGuest) {
    let mut guest = TestGuest::new();
    guest
        .apply(
            MemoryEffect::MapZeroed {
                address: WORK,
                length: 2 * PAGE,
                protection: 3,
            },
            &[],
        )
        .unwrap();
    let mut param = [0; 96];
    param[..4].copy_from_slice(&7u32.to_le_bytes());
    param[36..40].copy_from_slice(&16u32.to_le_bytes());
    param[40..48].copy_from_slice(&(ARG + 0x400).to_le_bytes());
    param[64..72].copy_from_slice(&WORK.to_le_bytes());
    param[72..80].copy_from_slice(&0x1111u64.to_le_bytes());
    param[80..88].copy_from_slice(&HANDLER.to_le_bytes());
    guest.put(TEXT_PARAM - ARG, &param);
    guest.put(0x400, &[65, 0, 0, 0]);
    let mut keyboard = [0; 32];
    keyboard[8..16].copy_from_slice(&0x2222u64.to_le_bytes());
    keyboard[16..24].copy_from_slice(&HANDLER.to_le_bytes());
    guest.put(KEY_PARAM - ARG, &keyboard);
    (kernel(limits), guest)
}
fn begin(owner: &mut Kernel, guest: &mut TestGuest) -> AdmittedImeCallback {
    let ImeCallbackAction::Call(request) = owner.admit_ime_update(args(&[HANDLER]), guest).unwrap()
    else {
        panic!("queued event was replaced with numeric completion");
    };
    request
}
fn event(guest: &TestGuest, request: AdmittedImeCallback) -> Vec<u8> {
    guest.get(request.args()[1], 72)
}

#[test]
fn keyboard_validation_precedence_low32_parameters_and_absent_device_outputs_are_exact() {
    let (mut owner, mut guest) = fixture(Limits::default());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[u64::MAX, 0]
        ),
        Ok(INVALID_ADDRESS)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[u64::MAX, u64::MAX]
        ),
        Ok(INVALID_USER)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardClose,
            &[u64::MAX]
        ),
        Ok(NOT_OPENED)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardSetMode,
            &[u64::MAX, u64::MAX]
        ),
        Ok(INVALID_USER)
    );
    guest.put(0x210, &0u64.to_le_bytes());
    guest.put(0x200, &0x40u32.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(INVALID_HANDLER)
    );
    guest.put(0x210, &HANDLER.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(INVALID_OPTION)
    );
    guest.put(0x200, &0u32.to_le_bytes());
    guest.put(0x204, &[1]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(INVALID_RESERVED)
    );
    guest.put(0x204, &[0]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[u64::MAX - 1, KEY_PARAM]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(BUSY)
    );
    guest.put(0x210, &0u64.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(INVALID_HANDLER)
    );
    guest.put(0x500, &[0xa5; 32]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardGetResourceId,
            &[0x1234_0000_0007, ARG + 0x500]
        ),
        Ok(CONNECTION_FAILED)
    );
    let mut expected = [0; 32];
    expected[..4].copy_from_slice(&7u32.to_le_bytes());
    expected[24..].fill(0xa5);
    assert_eq!(guest.get(ARG + 0x500, 32), expected);
    let reads = owner.usage().bytes_read;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardGetInfo,
            &[u64::MAX, u64::MAX]
        ),
        Ok(NO_RESOURCE)
    );
    assert_eq!(owner.usage().bytes_read, reads);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardSetMode,
            &[7, 0xffff_ffff_0000_007f]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardSetMode,
            &[7, 0x80]
        ),
        Ok(INVALID_MODE)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::ImeKeyboardClose, &[1]),
        Ok(0)
    );
    assert_eq!(owner.handle_usage().ime, 0);
    assert_eq!(owner.handle_usage().live, 0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardGetInfo,
            &[0, u64::MAX]
        ),
        Ok(NOT_OPENED)
    );
}

#[test]
fn modeled_update_detaches_both_queues_preserves_keyboard_snapshot_and_fresh_reopened_sessions() {
    let (mut owner, mut guest) = fixture(Limits::default());
    assert_eq!(
        call(&mut owner, &mut guest, Service::ImeOpen, &[TEXT_PARAM, 0]),
        Ok(0)
    );
    assert!(owner.ime_visual_state().active);
    // Seed a legitimate second queued IME event to model a host accept. This
    // fixture does not claim that a UI or a host-input transport produced it.
    let held = owner.ime.metadata_bytes();
    let mut events = reserve::<QueuedEvent>(2, held, &mut owner.budget).unwrap();
    events.append(&mut owner.ime.text.session.as_mut().unwrap().events);
    let mut accepted = [0; 72];
    accepted[..4].copy_from_slice(&5u32.to_le_bytes());
    events.push(QueuedEvent {
        bytes: accepted,
        text: None,
    });
    owner.ime.text.session.as_mut().unwrap().events = events;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(0)
    );
    assert!(call(&mut owner, &mut guest, Service::ImeUpdate, &[HANDLER]).is_err());
    let first = begin(&mut owner, &mut guest);
    assert_eq!(first.args()[0], 0x1111);
    assert_eq!(&event(&guest, first)[..8], &[0; 8]);
    let usage = owner.handle_usage();
    assert_eq!(
        (usage.ime, usage.memory, usage.live, usage.accounted()),
        (3, 1, 4, 4)
    );
    assert_eq!(
        owner.admit_ime_update(args(&[HANDLER]), &mut guest),
        Ok(ImeCallbackAction::Returned(0))
    );
    assert_eq!(call(&mut owner, &mut guest, Service::ImeClose, &[]), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, Service::ImeOpen, &[TEXT_PARAM, 0]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::ImeKeyboardClose, &[7]),
        Ok(0)
    );
    guest.put(0x208, &0x3333u64.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[42, KEY_PARAM]
        ),
        Ok(0)
    );
    let ImeCallbackAction::Call(second) = owner.complete_ime_callback(&first, &mut guest).unwrap()
    else {
        panic!("detached keyboard event was discarded");
    };
    assert_ne!(first, second);
    assert_eq!(second.args()[0], 0x2222);
    let bytes = event(&guest, second);
    assert_eq!(&bytes[..4], &256u32.to_le_bytes());
    assert_eq!(&bytes[8..12], &7u32.to_le_bytes());
    assert!(bytes[12..].iter().all(|byte| *byte == 0));
    assert!(owner.complete_ime_callback(&first, &mut guest).is_err());
    assert_eq!(
        owner.complete_ime_callback(&second, &mut guest),
        Ok(ImeCallbackAction::Returned(0))
    );
    assert!(guest.read(second.args()[1], &mut [0; 1]).is_err());
    let third = begin(&mut owner, &mut guest);
    assert_eq!(&event(&guest, third)[..4], &0u32.to_le_bytes());
    let ImeCallbackAction::Call(fourth) = owner.complete_ime_callback(&third, &mut guest).unwrap()
    else {
        panic!("new keyboard event missing");
    };
    assert_eq!(fourth.args()[0], 0x3333);
    assert_eq!(&event(&guest, fourth)[8..12], &42u32.to_le_bytes());
    assert_eq!(
        owner.complete_ime_callback(&fourth, &mut guest),
        Ok(ImeCallbackAction::Returned(0))
    );
    assert_eq!(owner.handle_usage().ime, 2);
    assert_eq!(owner.handle_usage().live, 2);
    assert_eq!(call(&mut owner, &mut guest, Service::ImeClose, &[]), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, Service::ImeKeyboardClose, &[0]),
        Ok(0)
    );
    assert_eq!(owner.handle_usage().accounted(), 0);
}

#[test]
fn update_quota_and_mapping_refusal_preserve_queued_events_and_do_not_publish_capacity() {
    for quota in [true, false] {
        let (mut owner, mut guest) = fixture(Limits {
            max_handles: if quota { 2 } else { 256 },
            ..Limits::default()
        });
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::ImeKeyboardOpen,
                &[7, KEY_PARAM]
            ),
            Ok(0)
        );
        let before = owner.ime.metadata_bytes();
        let usage = owner.handle_usage();
        guest.refuse = !quota;
        assert!(
            owner
                .admit_ime_update(args(&[HANDLER]), &mut guest)
                .is_err()
        );
        assert_eq!(owner.ime.metadata_bytes(), before);
        assert_eq!(owner.handle_usage(), usage);
        assert_eq!(owner.ime.keyboard.as_ref().unwrap().events.len(), 1);
        assert!(owner.ime.pending.is_empty());
    }
}

#[test]
fn modeled_completion_keeps_real_callback_writes_and_consumes_ticket_after_failed_release() {
    let (mut owner, mut guest) = fixture(Limits::default());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::ImeKeyboardOpen,
            &[7, KEY_PARAM]
        ),
        Ok(0)
    );
    let request = begin(&mut owner, &mut guest);
    let (mut other, mut other_guest) = fixture(Limits::default());
    assert!(
        other
            .complete_ime_callback(&request, &mut other_guest)
            .is_err()
    );
    guest.put(0x500, &[0x37; 8]);
    guest.refuse = true;
    assert!(owner.complete_ime_callback(&request, &mut guest).is_err());
    assert_eq!(guest.get(ARG + 0x500, 8), [0x37; 8]);
    assert_eq!(owner.handle_usage().live, owner.handle_usage().accounted());
    assert_eq!(owner.handle_usage().live, 3);
    guest.refuse = false;
    assert!(owner.complete_ime_callback(&request, &mut guest).is_err());
    assert!(owner.ime.pending[0].request.is_none());
    assert_eq!(owner.ime.keyboard.as_ref().unwrap().events.len(), 0);
    assert_eq!(
        owner.admit_ime_update(args(&[HANDLER]), &mut guest),
        Ok(ImeCallbackAction::Returned(0))
    );
}

#[test]
fn unsupported_host_text_payload_refuses_before_queue_drain_or_callback_mapping() {
    let (mut owner, mut guest) = fixture(Limits::default());
    assert_eq!(
        call(&mut owner, &mut guest, Service::ImeOpen, &[TEXT_PARAM, 0]),
        Ok(0)
    );
    // Model an edit event supplied by a future host transport. It must not be
    // delivered as successful input until its buffer/filter pipeline exists.
    owner.ime.text.session.as_mut().unwrap().events[0].text = Some(vec![65]);
    let held = owner.ime.metadata_bytes();
    let usage = owner.handle_usage();
    let written = owner.usage().bytes_written;
    assert!(matches!(
        owner.admit_ime_update(args(&[HANDLER]), &mut guest),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.ime.metadata_bytes(), held);
    assert_eq!(owner.handle_usage(), usage);
    assert_eq!(owner.usage().bytes_written, written);
    assert!(owner.ime.pending.is_empty());
    assert_eq!(owner.ime.text.session.as_ref().unwrap().events.len(), 1);
    owner.ime.text.session.as_mut().unwrap().events[0].text = None;
    let request = begin(&mut owner, &mut guest);
    assert_eq!(
        owner.complete_ime_callback(&request, &mut guest),
        Ok(ImeCallbackAction::Returned(0))
    );
    assert_eq!(call(&mut owner, &mut guest, Service::ImeClose, &[]), Ok(0));
    assert_eq!(owner.handle_usage().accounted(), 0);
}
