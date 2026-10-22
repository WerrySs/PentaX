// SPDX-License-Identifier: GPL-2.0-only
//! Original data-only admission/publication proofs, not native execution.
use super::*;
use crate::{AdmittedFiberTransfer, FiberAction, FiberTransferKind};
fn initialized() -> (Kernel, TestGuest) {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0x300, b"fiber\0");
    extra(&mut guest, [0, 0x0350_0000, 0]);
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0)
    );
    (owner, guest)
}
fn transfer(
    owner: &mut Kernel,
    guest: &TestGuest,
    service: Service,
    values: [u64; 6],
) -> AdmittedFiberTransfer {
    let FiberAction::Transfer(request) = owner
        .admit_fiber_transfer(service, CallArgs(values), guest)
        .unwrap()
    else {
        panic!("expected admission")
    };
    request
}
#[test]
fn run_yield_resume_and_normal_return_publish_actual_source_fields_and_reject_replay() {
    let (mut owner, mut guest) = initialized();
    let run = transfer(&mut owner, &guest, FiberRun, [ARG, 0x1111, INFO, 0, 0, 0]);
    let key = run.target().unwrap();
    assert_eq!(run.kind(), FiberTransferKind::Run);
    assert!(owner.active_fiber().is_none());
    assert_eq!(guest.u64(0), 0x0000_0002_def1_649c);
    assert_eq!(owner.commit_fiber_transfer(run, [0; 10], &mut guest), Ok(0));
    assert_eq!(owner.active_fiber(), Some(key));
    assert_eq!(guest.u64(184), 0x1111);
    let yielded = transfer(
        &mut owner,
        &guest,
        FiberReturnToThread,
        [0x2222, INFO + 8, 0, 0, 0, 0],
    );
    let saved = std::array::from_fn(|i| 0x1234_0000 + i as u64);
    assert_eq!(
        owner.commit_fiber_transfer(yielded, saved, &mut guest),
        Ok(0)
    );
    assert!(owner.active_fiber().is_none());
    assert_eq!(guest.u64(0x500), 0x2222);
    assert_eq!(guest.u64(72), ARG + 104);
    assert_eq!(guest.get(ARG + 200, 1), [1]);
    for (i, value) in saved.iter().enumerate() {
        assert_eq!(guest.u64(104 + i as u64 * 8), *value);
    }
    assert!(
        owner
            .commit_fiber_transfer(yielded, saved, &mut guest)
            .is_err()
    );
    let again = transfer(
        &mut owner,
        &guest,
        FiberRun,
        [ARG, 0x3333, INFO + 16, 0, 0, 0],
    );
    assert_eq!(again.target(), Some(key));
    assert_eq!(
        owner.commit_fiber_transfer(again, [0; 10], &mut guest),
        Ok(0)
    );
    assert_eq!(guest.u64(0x508), 0x3333);
    let finished = owner.admit_fiber_entry_return().unwrap();
    assert_eq!(
        owner.commit_fiber_transfer(finished, [u64::MAX; 10], &mut guest),
        Ok(0x8059_0006)
    );
    assert_eq!(guest.u64(0), 0x0000_0003_def1_649c);
    assert_eq!(guest.u64(72), 0);
    assert_eq!(guest.get(ARG + 200, 1), [0]);
    assert_eq!(guest.u64(0x510), 0);
    // Completion invalidates the context but does not fabricate saved registers.
    for (i, value) in saved.iter().enumerate() {
        assert_eq!(guest.u64(104 + i as u64 * 8), *value);
    }
}
#[test]
fn failed_resume_output_transaction_preserves_both_objects_and_active_root() {
    let (mut owner, mut guest) = initialized();
    let run = transfer(&mut owner, &guest, FiberRun, [ARG, 1, INFO, 0, 0, 0]);
    owner
        .commit_fiber_transfer(run, [0; 10], &mut guest)
        .unwrap();
    let yielded = transfer(
        &mut owner,
        &guest,
        FiberReturnToThread,
        [2, ARG + PAGE, 0, 0, 0, 0],
    );
    owner
        .commit_fiber_transfer(yielded, [3; 10], &mut guest)
        .unwrap();
    let before = guest.get(ARG, 208);
    let output = guest.get(INFO, 16);
    let resume = transfer(&mut owner, &guest, FiberRun, [ARG, 4, INFO + 8, 0, 0, 0]);
    assert!(
        owner
            .commit_fiber_transfer(resume, [0; 10], &mut guest)
            .is_err()
    );
    assert_eq!(guest.get(ARG, 208), before);
    assert_eq!(guest.get(INFO, 16), output);
    assert!(owner.active_fiber().is_none());
    // The prepared native owner treats this as terminal, never a completed Run.
}
#[test]
fn idle_generation_migrates_to_new_thread_and_reinitialization_invalidates_key() {
    let (mut owner, mut guest) = initialized();
    owner.activate_thread_context(0x1000).unwrap();
    let run = transfer(&mut owner, &guest, FiberRun, [ARG, 1, INFO, 0, 0, 0]);
    let old = run.target().unwrap();
    owner
        .commit_fiber_transfer(run, [0; 10], &mut guest)
        .unwrap();
    let yielded = transfer(
        &mut owner,
        &guest,
        FiberReturnToThread,
        [2, INFO + 8, 0, 0, 0, 0],
    );
    owner
        .commit_fiber_transfer(yielded, [0; 10], &mut guest)
        .unwrap();
    owner.activate_thread_context(0x2000).unwrap();
    let migrated = transfer(&mut owner, &guest, FiberRun, [ARG, 3, INFO + 16, 0, 0, 0]);
    assert_eq!(migrated.target(), Some(old));
    owner
        .commit_fiber_transfer(migrated, [0; 10], &mut guest)
        .unwrap();
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberGetSelf, &[INFO + 24]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x518), ARG);
    owner.activate_thread_context(0x1000).unwrap();
    assert!(matches!(
        owner.admit_fiber_transfer(FiberRun, CallArgs([ARG, 0, 0, 0, 0, 0]), &guest),
        Ok(FiberAction::Returned(0x8059_0006))
    ));
    owner.activate_thread_context(0x2000).unwrap();
    let yielded = transfer(&mut owner, &guest, FiberReturnToThread, [4, 0, 0, 0, 0, 0]);
    owner
        .commit_fiber_transfer(yielded, [0; 10], &mut guest)
        .unwrap();
    assert_eq!(invoke(&mut owner, &mut guest, FiberFinalize, &[ARG]), Ok(0));
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0)
    );
    let fresh = transfer(&mut owner, &guest, FiberRun, [ARG, 5, 0, 0, 0, 0]);
    assert_ne!(fresh.target(), Some(old));
    assert_eq!(fresh.target().unwrap().address(), old.address());
    assert!(
        owner
            .commit_fiber_transfer(run, [0; 10], &mut guest)
            .is_err()
    );
    assert_eq!(owner.usage().handles_live, 1);
}
#[test]
fn error_priority_and_exact_execution_scopes_do_not_expand_alias_authority() {
    let (mut owner, mut guest) = initialized();
    for (service, values, code) in [
        (FiberRun, [0; 6], 0x8059_0004),
        (FiberSwitch, [0; 6], 0x8059_0004),
        (FiberSwitch, [ARG, 0, 0, 0, 0, 0], 0x8059_0005),
        (FiberReturnToThread, [0; 6], 0x8059_0005),
    ] {
        assert!(
            matches!(owner.admit_fiber_transfer(service,CallArgs(values),&guest),Ok(FiberAction::Returned(value)) if value==code)
        );
    }
    for service in [FiberGetSelf, FiberGetThreadFramePointerAddress] {
        assert_eq!(
            invoke(&mut owner, &mut guest, service, &[0]),
            Ok(0x8059_0001)
        );
        guest.put(0x500, &[0xa5; 8]);
        assert_eq!(invoke(&mut owner, &mut guest, service, &[INFO]), Ok(0));
        assert_eq!(guest.u64(0x500), 0);
    }
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            FiberStartContextSizeCheck,
            &[u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberStopContextSizeCheck, &[]),
        Ok(0)
    );
    for (nid, service) in [
        ("a0LLrZWac0M", FiberRun),
        ("PFT2S-tJ7Uk", FiberSwitch),
        ("B0ZX2hx9DMw", FiberReturnToThread),
    ] {
        assert_eq!(resolve("Fiber", (1, 1), "Fiber", 1, nid), Some(service));
        assert_eq!(
            resolve("libSceFiber", (1, 1), "libSceFiber", 1, nid),
            Some(service)
        );
        assert_eq!(resolve("libSceFiber", (1, 1), "Fiber", 1, nid), None);
        assert_eq!(resolve("Fiber", (1, 2), "Fiber", 1, nid), None);
    }
    assert_eq!(
        resolve("libSceFiber", (1, 1), "libSceFiber", 1, "p+zLIOg27zU"),
        None
    );
    assert_eq!(
        resolve("Fiber", (1, 1), "Fiber", 1, "avfGJ94g36Q"),
        Some(FiberRun)
    );
}
