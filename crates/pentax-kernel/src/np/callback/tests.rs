// SPDX-License-Identifier: GPL-2.0-only
//! Original offline queue consumers; these unit admissions do not claim native execution.
use super::*;
use crate::{GuestAccess, GuestError, GuestWrite, MemoryEffect, ProcessIdentity, UserConfig};
struct NoMemory;
impl GuestAccess for NoMemory {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("NP registration/poll must not read arbitrary guest pointers")
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("NP registration/poll must not write guest outputs")
    }
}
fn kernel() -> Kernel {
    Kernel::new(
        Default::default(),
        ProcessIdentity {
            process_id: 7,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap()
}
fn register(k: &mut Kernel, entry: u64, data: u64) -> Result<u64, Failure> {
    k.invoke_library(
        Service::NpRegisterStateCallbackA,
        CallArgs([entry, data, 0, 0, 0, 0]),
        &mut NoMemory,
    )
}
fn poll(k: &mut Kernel) -> NpCallbackAction {
    k.admit_np_callbacks().unwrap()
}
fn call(k: &mut Kernel) -> AdmittedNpCallback {
    let NpCallbackAction::Call(x) = poll(k) else {
        panic!("pending callback replaced by success")
    };
    x
}
#[test]
fn np_callback_registration_returns_ordinals_and_duplicate_function_does_not_replace_userdata_or_user_profile()
 {
    let mut k = kernel();
    k.configure_user(UserConfig {
        id: 37,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(poll(&mut k), NpCallbackAction::Returned(0));
    assert_eq!(register(&mut k, 0, u64::MAX), Ok(INVALID_ARGUMENT));
    assert_eq!(register(&mut k, 0x1000, 0x1234), Ok(1));
    assert_eq!(register(&mut k, 0x1000, 0x9999), Ok(DUPLICATE));
    assert_eq!(register(&mut k, 0x2000, 0x5678), Ok(2));
    assert_eq!(k.usage().handles_live, 2);
    let first = call(&mut k);
    assert_eq!(first.entry(), 0x1000);
    assert_eq!(first.args(), [37, 1, 0x1234]);
    let NpCallbackAction::Call(second) = k.complete_np_callback(&first).unwrap() else {
        panic!("second row lost")
    };
    assert_eq!(second.args(), [37, 1, 0x5678]);
    assert_eq!(
        k.complete_np_callback(&second),
        Ok(NpCallbackAction::Returned(0))
    );
    assert_eq!(register(&mut k, 0x1000, 0), Ok(DUPLICATE));
    assert_eq!(k.usage().handles_live, 2);
}
#[test]
fn np_callback_outer_snapshot_excludes_new_rows_and_recursive_checks_do_not_clobber_each_other() {
    let mut k = kernel();
    assert_eq!(register(&mut k, 0x1000, 11), Ok(1));
    assert_eq!(register(&mut k, 0x2000, 22), Ok(2));
    let first = call(&mut k);
    assert_eq!(poll(&mut k), NpCallbackAction::Returned(0));
    assert_eq!(register(&mut k, 0x3000, 33), Ok(3));
    let nested = call(&mut k);
    assert_eq!(nested.entry(), 0x3000);
    assert_eq!(poll(&mut k), NpCallbackAction::Returned(0));
    assert_eq!(
        k.complete_np_callback(&nested),
        Ok(NpCallbackAction::Returned(0))
    );
    let NpCallbackAction::Call(second) = k.complete_np_callback(&first).unwrap() else {
        panic!("outer snapshot lost")
    };
    assert_eq!(second.entry(), 0x2000);
    assert_eq!(second.args()[2], 22);
    assert_eq!(
        k.complete_np_callback(&second),
        Ok(NpCallbackAction::Returned(0))
    );
    assert_eq!(poll(&mut k), NpCallbackAction::Returned(0));
    assert!(k.complete_np_callback(&first).is_err());
    assert!(k.complete_np_callback(&nested).is_err());
}
#[test]
fn np_callback_foreign_stale_failed_and_budget_refusals_never_turn_pending_dispatch_into_empty_success()
 {
    let mut a = kernel();
    let mut b = kernel();
    register(&mut a, 0x1000, 1).unwrap();
    register(&mut b, 0x1000, 1).unwrap();
    let first = call(&mut a);
    let foreign = call(&mut b);
    assert!(a.complete_np_callback(&foreign).is_err());
    assert_eq!(
        a.complete_np_callback(&first),
        Ok(NpCallbackAction::Returned(0))
    );
    assert!(a.complete_np_callback(&first).is_err());
    register(&mut a, 0x2000, 2).unwrap();
    let before = a.np.callbacks.snapshot_end;
    let limit = a.budget.limits.max_work;
    a.budget.limits.max_work = a.usage().work;
    assert!(a.admit_np_callbacks().is_err());
    assert_eq!(a.np.callbacks.snapshot_end, before);
    a.budget.limits.max_work = limit;
    let next = call(&mut a);
    assert_eq!(next.entry(), 0x2000);
    a.fail_np_callback(&next);
    assert!(a.admit_np_callbacks().is_err());
    assert!(a.complete_np_callback(&next).is_err());
    assert!(
        a.invoke_library(
            Service::NpCheckCallback,
            CallArgs([u64::MAX; 6]),
            &mut NoMemory
        )
        .is_err()
    );
    assert_eq!(
        b.complete_np_callback(&foreign),
        Ok(NpCallbackAction::Returned(0))
    );
}
#[test]
fn np_callback_rows_share_real_handle_limits_and_snapshot_banks_reuse_only_completed_slots() {
    let mut k = Kernel::new(
        crate::Limits {
            max_handles: 1,
            ..Default::default()
        },
        ProcessIdentity {
            process_id: 7,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap();
    assert_eq!(register(&mut k, 0x1000, 1), Ok(1));
    assert!(register(&mut k, 0x2000, 2).is_err());
    assert_eq!(k.usage().handles_live, 1);
    let mut k = kernel();
    let mut starts = Vec::new();
    for index in 0..32 {
        assert_eq!(register(&mut k, 0x1000 + index, 0), Ok(index + 1));
        starts.push(call(&mut k));
    }
    assert_eq!(register(&mut k, 0x2000, 0), Ok(33));
    let before = k.np.callbacks.snapshot_end;
    assert!(k.admit_np_callbacks().is_err());
    assert_eq!(k.np.callbacks.snapshot_end, before);
    assert_eq!(
        k.complete_np_callback(&starts[0]),
        Ok(NpCallbackAction::Returned(0))
    );
    let reused = call(&mut k);
    assert_eq!(reused.entry(), 0x2000);
    assert!(k.complete_np_callback(&starts[0]).is_err());
    assert_eq!(
        k.complete_np_callback(&reused),
        Ok(NpCallbackAction::Returned(0))
    );
    for token in starts.into_iter().skip(1) {
        assert_eq!(
            k.complete_np_callback(&token),
            Ok(NpCallbackAction::Returned(0))
        );
    }
}

#[test]
fn np_callback_batch_abort_tracks_successors_but_cannot_poison_a_reused_slot() {
    let mut k = kernel();
    register(&mut k, 0x1000, 1).unwrap();
    register(&mut k, 0x2000, 2).unwrap();
    let first = call(&mut k);
    let NpCallbackAction::Call(second) = k.complete_np_callback(&first).unwrap() else {
        panic!("missing successor")
    };
    k.fail_np_callback(&first);
    assert!(k.complete_np_callback(&second).is_err());
    assert!(k.admit_np_callbacks().is_err());
    let mut k = kernel();
    register(&mut k, 0x1000, 1).unwrap();
    let old = call(&mut k);
    assert_eq!(
        k.complete_np_callback(&old),
        Ok(NpCallbackAction::Returned(0))
    );
    register(&mut k, 0x2000, 2).unwrap();
    let new = call(&mut k);
    assert_eq!(new.batch, old.batch);
    assert_ne!(new.batch_sequence, old.batch_sequence);
    k.fail_np_callback(&old);
    assert_eq!(
        k.complete_np_callback(&new),
        Ok(NpCallbackAction::Returned(0))
    );
}
