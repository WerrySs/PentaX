// SPDX-License-Identifier: GPL-2.0-only
//! Original pure state/backing tests. Completion calls model a trusted owner's
//! return observation; no guest callback, socket or host network query executes.

use super::*;

const CALLBACK_MAX: u64 = 0x8041_2103;
const ID_NOT_FOUND: u64 = 0x8041_2104;
const INVALID_ID: u64 = 0x8041_2105;
const INVALID_ADDR: u64 = 0x8041_2107;
const NOT_CONNECTED: u64 = 0x8041_2108;
const INVALID_TYPE: u64 = 0x8041_210f;

fn invoke(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}

fn register(owner: &mut Kernel, guest: &mut TestGuest, entry: u64, argument: u64) -> u32 {
    assert_eq!(
        invoke(
            owner,
            guest,
            Service::NetCtlRegisterCallback,
            &[entry, argument, ARG]
        ),
        Ok(0)
    );
    u32::from_le_bytes(guest.get(ARG, 4).try_into().unwrap())
}

fn admitted(owner: &mut Kernel) -> AdmittedNetCtlCallback {
    match owner.admit_netctl_callbacks().unwrap() {
        NetCtlCallbackAction::Call(request) => request,
        other => panic!("expected original callback admission, got {other:?}"),
    }
}

#[test]
fn forced_offline_queries_work_before_init_and_preserve_exact_output_spans() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for service in [Service::NetCtlGetState, Service::NetCtlGetStateV6] {
        guest.put(0x40, &[0xa5; 6]);
        assert_eq!(
            invoke(&mut owner, &mut guest, service, &[ARG + 0x41]),
            Ok(0)
        );
        assert_eq!(guest.get(ARG + 0x40, 6), [0xa5, 0, 0, 0, 0, 0xa5]);
        assert_eq!(
            invoke(&mut owner, &mut guest, service, &[0]),
            Ok(INVALID_ADDR)
        );
    }
    guest.put(0x80, &[0x5a; 18]);
    guest.put(0x81, &16_u32.to_le_bytes());
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlGetNatInfo,
            &[ARG + 0x81]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x80, 1), [0x5a]);
    assert_eq!(guest.get(ARG + 0x81, 4), 16_u32.to_le_bytes());
    assert_eq!(guest.get(ARG + 0x85, 12), [0; 12]);
    assert_eq!(guest.get(ARG + 0x91, 1), [0x5a]);
    for code in [0, 1, 21, u64::MAX] {
        guest.put(0x100, &[0x37; 258]);
        assert_eq!(
            invoke(
                &mut owner,
                &mut guest,
                Service::NetCtlGetInfo,
                &[code, ARG + 0x101]
            ),
            Ok(NOT_CONNECTED)
        );
        assert_eq!(guest.get(ARG + 0x100, 1), [0x37]);
        assert_eq!(guest.get(ARG + 0x101, 256), [0; 256]);
        assert_eq!(guest.get(ARG + 0x201, 1), [0x37]);
    }
    for event in [1, 2, 3, (9_u64 << 32) | 1] {
        guest.put(0x40, &[0xa5; 6]);
        assert_eq!(
            invoke(
                &mut owner,
                &mut guest,
                Service::NetCtlGetResult,
                &[event, ARG + 0x41]
            ),
            Ok(0)
        );
        assert_eq!(guest.get(ARG + 0x40, 6), [0xa5, 0, 0, 0, 0, 0xa5]);
    }
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlGetResult, &[99, 0]),
        Ok(INVALID_ADDR)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlGetResult, &[99, ARG]),
        Ok(INVALID_TYPE)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlTerm, &[]),
        Ok(0)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlGetState, &[ARG]),
        Ok(0)
    );
    assert_eq!(guest.get(ARG, 4), [0; 4]);
}

#[test]
fn source_abort_branches_and_checked_output_failures_remain_typed_and_atomic() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlGetNatInfo, &[0]),
        Err(Failure::Unsupported("source-null NetCtl NAT info"))
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlGetInfo, &[1, 0]),
        Err(Failure::Unsupported("source-null NetCtl info"))
    );
    guest.put(0, &[0x5a; 16]);
    guest.put(0, &15_u32.to_le_bytes());
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlGetNatInfo, &[ARG]),
        Err(Failure::Unsupported("NetCtl NAT info size"))
    );
    assert_eq!(guest.get(ARG + 4, 12), [0x5a; 12]);
    guest.put(PAGE - 4, &[0xa5; 4]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlGetInfo,
            &[999, ARG + PAGE - 4]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + PAGE - 4, 4), [0xa5; 4]);
    // NAT reads its caller-owned size word and writes only the twelve result
    // bytes, so that word can be read-only in the preceding guest page.
    guest.put(PAGE - 4, &16_u32.to_le_bytes());
    guest
        .apply(
            MemoryEffect::MapZeroed {
                address: ARG + PAGE,
                length: PAGE,
                protection: 3,
            },
            &[],
        )
        .unwrap();
    guest
        .apply(
            MemoryEffect::Protect {
                address: ARG,
                length: PAGE,
                protection: 1,
            },
            &[],
        )
        .unwrap();
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlGetNatInfo,
            &[ARG + PAGE - 4]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + PAGE - 4, 4), 16_u32.to_le_bytes());
    assert_eq!(guest.get(ARG + PAGE, 12), [0; 12]);
}

#[test]
fn eight_slots_reuse_first_free_and_publish_id_before_owning_a_registration() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for index in 0..8 {
        assert_eq!(
            register(&mut owner, &mut guest, u64::MAX - index, index),
            index as u32
        );
    }
    guest.refuse = true;
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlRegisterCallback,
            &[1, 0, ARG]
        ),
        Ok(CALLBACK_MAX)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlRegisterCallback,
            &[1, 0, 0]
        ),
        Ok(INVALID_ADDR)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlRegisterCallback,
            &[0, 0, ARG]
        ),
        Ok(INVALID_ADDR)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlUnregisterCallback,
            &[u64::MAX]
        ),
        Ok(INVALID_ID)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlUnregisterCallback,
            &[8]
        ),
        Ok(INVALID_ID)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlUnregisterCallback,
            &[(17_u64 << 32) | 3]
        ),
        Ok(0)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlRegisterCallback,
            &[1, 0, ARG]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(register(&mut owner, &mut guest, u64::MAX, 0), 3);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlInit, &[]),
        Ok(0)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlRegisterCallback,
            &[1, 0, ARG]
        ),
        Ok(CALLBACK_MAX)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlTerm, &[]),
        Ok(0)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::NetCtlUnregisterCallback,
            &[0]
        ),
        Ok(ID_NOT_FOUND)
    );
    assert_eq!(register(&mut owner, &mut guest, 1, 0), 0);
}

#[test]
fn admission_does_not_mark_delivery_and_completion_is_ordered_once_per_event() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(register(&mut owner, &mut guest, ARG + 0x1000, 37), 0);
    assert_eq!(register(&mut owner, &mut guest, ARG + 0x1100, 51), 1);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlCheckCallback, &[]),
        Err(Failure::Unsupported("prepared NetCtl callback"))
    );
    let attempts = owner.usage().attempts;
    let first = admitted(&mut owner);
    assert_eq!(owner.usage().attempts, attempts + 1);
    assert_eq!((first.entry(), first.args()), (ARG + 0x1000, [1, 37, 0]));
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlCheckCallback, &[]),
        Err(Failure::Unsupported("recursive NetCtl callback check"))
    );
    assert_eq!(
        owner.admit_netctl_callbacks(),
        Err(Failure::Unsupported("recursive NetCtl callback check"))
    );
    let attempts = owner.usage().attempts;
    let NetCtlCallbackAction::Call(second) = owner.complete_netctl_callback(&first).unwrap() else {
        panic!("missing second callback");
    };
    assert_eq!((second.entry(), second.args()), (ARG + 0x1100, [1, 51, 0]));
    assert_eq!(owner.usage().attempts, attempts);
    assert_eq!(
        owner.complete_netctl_callback(&first),
        Err(Failure::Unsupported("stale NetCtl callback admission"))
    );
    assert_eq!(
        owner.complete_netctl_callback(&second),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    assert_eq!(
        owner.complete_netctl_callback(&second),
        Err(Failure::Unsupported("stale NetCtl callback admission"))
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlInit, &[]),
        Ok(0)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::NetCtlCheckCallback, &[]),
        Ok(0)
    );
    assert_eq!(
        owner.admit_netctl_callbacks(),
        Ok(NetCtlCallbackAction::Returned(0))
    );
}

#[test]
fn callback_slot_mutations_observe_later_slots_live_and_defer_new_earlier_slots() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for (entry, argument) in [(100, 10), (200, 20), (300, 30)] {
        register(&mut owner, &mut guest, entry, argument);
    }
    let first = admitted(&mut owner);
    let NetCtlCallbackAction::Call(second) = owner.complete_netctl_callback(&first).unwrap() else {
        panic!("missing second callback");
    };
    for index in 0..3 {
        assert_eq!(
            invoke(
                &mut owner,
                &mut guest,
                Service::NetCtlUnregisterCallback,
                &[index]
            ),
            Ok(0)
        );
    }
    assert_eq!(register(&mut owner, &mut guest, 400, 40), 0);
    assert_eq!(register(&mut owner, &mut guest, 500, 50), 1);
    assert_eq!(register(&mut owner, &mut guest, 600, 60), 2);
    let NetCtlCallbackAction::Call(later) = owner.complete_netctl_callback(&second).unwrap() else {
        panic!("missing live later callback");
    };
    assert_eq!((later.entry(), later.args()), (600, [1, 60, 0]));
    assert_eq!(
        owner.complete_netctl_callback(&later),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    let earlier = admitted(&mut owner);
    assert_eq!((earlier.entry(), earlier.args()), (400, [1, 40, 0]));
    // The replacement in the current slot1 was marked by the OLD slot1
    // callback's post-return assignment, so it is not spuriously delivered.
    assert_eq!(
        owner.complete_netctl_callback(&earlier),
        Ok(NetCtlCallbackAction::Returned(0))
    );
}

#[test]
fn term_inside_callback_retains_pending_identity_and_tokens_are_kernel_bound() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    register(&mut owner, &mut guest, 100, 10);
    let original = admitted(&mut owner);
    invoke(&mut owner, &mut guest, Service::NetCtlTerm, &[]).unwrap();
    invoke(&mut owner, &mut guest, Service::NetCtlInit, &[]).unwrap();
    assert_eq!(register(&mut owner, &mut guest, 200, 20), 0);
    assert_eq!(
        owner.admit_netctl_callbacks(),
        Err(Failure::Unsupported("recursive NetCtl callback check"))
    );
    assert_eq!(
        owner.complete_netctl_callback(&original),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    assert_eq!(
        owner.admit_netctl_callbacks(),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    invoke(&mut owner, &mut guest, Service::NetCtlTerm, &[]).unwrap();
    register(&mut owner, &mut guest, 100, 10);
    let fresh = admitted(&mut owner);
    assert_ne!(fresh, original);
    assert_eq!(
        owner.complete_netctl_callback(&original),
        Err(Failure::Unsupported("stale NetCtl callback admission"))
    );
    let mut other = kernel(Default::default());
    register(&mut other, &mut guest, 100, 10);
    let foreign = admitted(&mut other);
    assert_ne!(fresh, foreign);
    assert_eq!(
        owner.complete_netctl_callback(&foreign),
        Err(Failure::Unsupported("stale NetCtl callback admission"))
    );
    assert_eq!(
        owner.complete_netctl_callback(&fresh),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    assert_eq!(
        other.complete_netctl_callback(&foreign),
        Ok(NetCtlCallbackAction::Returned(0))
    );
}

#[test]
fn callback_budgets_precede_state_changes_and_completion_consumes_no_attempt() {
    let mut empty = kernel(Limits {
        max_attempts: 1,
        ..Default::default()
    });
    assert_eq!(
        empty.admit_netctl_callbacks(),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    assert_eq!(
        empty.admit_netctl_callbacks(),
        Err(Failure::Limit("attempts"))
    );
    assert_eq!(empty.usage().attempts, 1);
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    register(&mut owner, &mut guest, 100, 10);
    register(&mut owner, &mut guest, 200, 20);
    let first = admitted(&mut owner);
    owner.budget.limits.max_work = owner.usage().work;
    let attempts = owner.usage().attempts;
    assert_eq!(
        owner.complete_netctl_callback(&first),
        Err(Failure::Limit("work"))
    );
    assert_eq!(owner.usage().attempts, attempts);
    owner.budget.limits.max_work = Limits::default().max_work;
    let NetCtlCallbackAction::Call(second) = owner.complete_netctl_callback(&first).unwrap() else {
        panic!("pending callback was incorrectly consumed");
    };
    assert_eq!(second.entry(), 200);
    assert_eq!(
        owner.complete_netctl_callback(&second),
        Ok(NetCtlCallbackAction::Returned(0))
    );
    assert_eq!(
        owner.admit_netctl_callbacks(),
        Ok(NetCtlCallbackAction::Returned(0))
    );
}
