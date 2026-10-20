// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked, per-thread, actual-return-driven cleanup.
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/kernel/pthread.cpp (a7fa3003f534af01a119f117954ffe899c982351),
// CleanupThread and DestructPthreadSpecific. See NOTICE and upstream provenance.
//! ThreadDtors followed by live pthread-key destruction. Guest addresses are
//! data only: the native owner must attest each actual return before completion.
//! The unavailable CxaThreadAtexit registration service admits no records here;
//! this does not flush the separate process-wide cxa_atexit registrations.

use crate::pthread_keys::{KEYS_MAX, KeyState, Specific};
use crate::{Failure, GuestAccess, Kernel};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

const PASSES: u8 = 4;
const MASK_WORDS: usize = KEYS_MAX.div_ceil(64);
// One advancement can scan all four passes and commit at most256 cleared
// slots. Charge the entire fixed bound before changing keys or cleanup state.
const ADVANCE_WORK: u64 = (KEYS_MAX * (PASSES as usize + 1) + 32) as u64;
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallbackKind {
    General,
    Key,
}

/// A single callback admission, uniquely bound to its kernel/thread cleanup.
/// Holding it neither executes the entry nor proves that it has returned.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedThreadExitCallback {
    owner: u64,
    thread: u32,
    context: u64,
    serial: u64,
    entry: u64,
    arguments: [u64; 3],
    kind: CallbackKind,
}
impl AdmittedThreadExitCallback {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    /// General ThreadDtors has no arguments; key destructors receive their
    /// opaque saved value. Extra register arguments are explicitly zero.
    pub const fn args(&self) -> [u64; 3] {
        self.arguments
    }
}
impl fmt::Debug for AdmittedThreadExitCallback {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedThreadExitCallback")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadExitAction {
    Callback(AdmittedThreadExitCallback),
    Finished,
}

/// Fixed storage already owned by the active ThreadContext; no cleanup step
/// allocates, publishes a guest pointer or creates another live resource.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ThreadExitState {
    owner: u64,
    serial: u64,
    started: bool,
    finished: bool,
    general_done: bool,
    pass: u8,
    index: u16,
    called_in_pass: bool,
    pending: Option<AdmittedThreadExitCallback>,
}
impl ThreadExitState {
    pub(crate) const fn in_progress(&self) -> bool {
        self.started && !self.finished
    }
    pub(crate) const fn finished(&self) -> bool {
        self.finished
    }
}

struct Plan {
    state: ThreadExitState,
    clear: [u64; MASK_WORDS],
    drop_values: bool,
    action: ThreadExitAction,
}
impl Plan {
    fn callback(
        mut state: ThreadExitState,
        clear: [u64; MASK_WORDS],
        thread: u32,
        context: u64,
        entry: u64,
        value: u64,
        kind: CallbackKind,
    ) -> Result<Self, Failure> {
        let serial = state
            .serial
            .checked_add(1)
            .ok_or(Failure::Limit("thread exit callback serial"))?;
        let request = AdmittedThreadExitCallback {
            owner: state.owner,
            thread,
            context,
            serial,
            entry,
            arguments: [value, 0, 0],
            kind,
        };
        state.serial = serial;
        state.pending = Some(request);
        Ok(Self {
            state,
            clear,
            drop_values: false,
            action: ThreadExitAction::Callback(request),
        })
    }

    fn finished(mut state: ThreadExitState, clear: [u64; MASK_WORDS]) -> Self {
        state.finished = true;
        state.pending = None;
        Self {
            state,
            clear,
            drop_values: true,
            action: ThreadExitAction::Finished,
        }
    }
}

fn next(
    mut state: ThreadExitState,
    thread: u32,
    context: u64,
    general: Option<u64>,
    keys: &KeyState,
    values: &Option<Vec<Specific>>,
) -> Result<Plan, Failure> {
    let mut clear = [0u64; MASK_WORDS];
    if !state.general_done {
        state.general_done = true;
        if let Some(entry) = general {
            return Plan::callback(
                state,
                clear,
                thread,
                context,
                entry,
                0,
                CallbackKind::General,
            );
        }
    }
    if values.is_none() {
        return Ok(Plan::finished(state, clear));
    }
    loop {
        if state.index as usize == KEYS_MAX {
            if !state.called_in_pass || state.pass + 1 == PASSES {
                return Ok(Plan::finished(state, clear));
            }
            state.pass += 1;
            state.index = 0;
            state.called_in_pass = false;
        }
        let index = state.index as usize;
        let bit = 1u64 << (index % 64);
        let already_cleared = clear[index / 64] & bit != 0;
        clear[index / 64] |= bit;
        state.index += 1;
        // Model the exchange before the eligibility check. If this planning
        // step wrapped a pass without calling guest code, an already visited
        // slot is logically zero even though the commit has not happened yet.
        if !already_cleared && let Some((entry, value)) = keys.exit_callback(index, values) {
            return Plan::callback(
                state,
                clear,
                thread,
                context,
                entry,
                value,
                CallbackKind::Key,
            );
        }
    }
}

impl Kernel {
    /// Begin cleanup of the active admitted thread, exactly once. A callback
    /// request remains pending if the native owner later refuses code/frame
    /// admission; do not complete it without attesting an actual guest return.
    pub fn begin_thread_exit(
        &mut self,
        guest: &impl GuestAccess,
    ) -> Result<ThreadExitAction, Failure> {
        guest.preflight_pthread()?;
        self.budget.attempt()?;
        self.budget.work(ADVANCE_WORK)?;
        let current = self.threads.active();
        if current.exit_cleanup.started {
            return Err(Failure::Unsupported("duplicate thread exit cleanup"));
        }
        // A non-recycled activation owner is stronger than a process ID or TLS
        // address: even two kernels with identical contexts cannot share tickets.
        let owner = NEXT_OWNER
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |owner| {
                owner.checked_add(1)
            })
            .map_err(|_| Failure::Limit("thread exit cleanup owners"))?;
        let mut state = current.exit_cleanup;
        state.owner = owner;
        state.started = true;
        let plan = next(
            state,
            current.id,
            current.context(),
            self.thread_lifecycle.destructor,
            &self.pthread_keys,
            &current.values,
        )?;
        Ok(self.commit_thread_exit(plan))
    }

    /// Consume the exact pending callback only after the native owner verified
    /// its actual return in this same active thread/source/stack. Void callback
    /// RAX is intentionally absent: it cannot replace the thread entry result.
    pub fn complete_thread_exit_callback(
        &mut self,
        request: &AdmittedThreadExitCallback,
        guest: &impl GuestAccess,
    ) -> Result<ThreadExitAction, Failure> {
        guest.preflight_pthread()?;
        self.budget.attempt()?;
        self.budget.work(ADVANCE_WORK)?;
        let current = self.threads.active();
        let mut state = current.exit_cleanup;
        if state.pending.as_ref() != Some(request)
            || request.thread != current.id
            || request.context != current.context()
            || state.finished
        {
            return Err(Failure::Unsupported("stale thread exit callback admission"));
        }
        state.pending = None;
        if request.kind == CallbackKind::Key {
            state.called_in_pass = true;
        }
        // All successor planning, including checked serial admission, precedes
        // consumption. A recoverable refusal preserves the existing pending
        // ticket, cursor and live key values for the already-attested callback.
        let plan = next(
            state,
            current.id,
            current.context(),
            self.thread_lifecycle.destructor,
            &self.pthread_keys,
            &current.values,
        )?;
        Ok(self.commit_thread_exit(plan))
    }

    fn commit_thread_exit(&mut self, plan: Plan) -> ThreadExitAction {
        let current = self.threads.active_mut();
        if plan.drop_values {
            // Includes values republished by the fourth pass. Source discards
            // the entire thread-specific array after the finite iteration cap.
            current.values = None;
        } else if let Some(values) = &mut current.values {
            for (index, value) in values.iter_mut().enumerate().take(KEYS_MAX) {
                if plan.clear[index / 64] & (1u64 << (index % 64)) != 0 {
                    *value = Specific::default();
                }
            }
        }
        current.exit_cleanup = plan.state;
        plan.action
    }
}

#[cfg(test)]
mod tests {
    // These are modeled return attestations at the kernel boundary. They do
    // not establish native callback execution; the core/worker owns that proof.
    use super::*;
    use crate::tests::{ARG, TestGuest, kernel};
    use crate::{CallArgs, GuestError, GuestWrite, Limits, MemoryEffect, Service};

    const CONTEXT: u64 = 0x1111_0000;

    struct Fixture {
        owner: Kernel,
        guest: TestGuest,
        output: u64,
    }
    impl Fixture {
        fn new() -> Self {
            let mut owner = kernel(Limits::default());
            owner.activate_thread_context(CONTEXT).unwrap();
            Self {
                owner,
                guest: TestGuest::new(),
                output: ARG + 0x100,
            }
        }
        fn call(&mut self, service: Service, values: &[u64]) -> Result<u64, Failure> {
            let mut args = [0; 6];
            args[..values.len()].copy_from_slice(values);
            self.owner
                .invoke_library(service, CallArgs(args), &mut self.guest)
        }
        fn key(&mut self, entry: u64) -> u64 {
            let output = self.output;
            self.output += 4;
            assert_eq!(self.call(Service::KeyCreate, &[output, entry]), Ok(0));
            u64::from(u32::from_le_bytes(
                self.guest.get(output, 4).try_into().unwrap(),
            ))
        }
        fn set(&mut self, key: u64, value: u64) {
            assert_eq!(self.call(Service::SetSpecific, &[key, value]), Ok(0));
        }
        fn get(&mut self, key: u64) -> u64 {
            self.call(Service::GetSpecific, &[key]).unwrap()
        }
        fn begin(&mut self) -> Result<ThreadExitAction, Failure> {
            self.owner.begin_thread_exit(&self.guest)
        }
        fn complete(
            &mut self,
            request: &AdmittedThreadExitCallback,
        ) -> Result<ThreadExitAction, Failure> {
            self.owner
                .complete_thread_exit_callback(request, &self.guest)
        }
    }
    fn callback(action: ThreadExitAction) -> AdmittedThreadExitCallback {
        match action {
            ThreadExitAction::Callback(request) => request,
            ThreadExitAction::Finished => panic!("expected original modeled callback"),
        }
    }

    #[test]
    fn module_unload_hooks_do_not_become_thread_exit_callbacks_and_finish_is_single_use() {
        let mut f = Fixture::new();
        assert_eq!(f.call(Service::SetThreadAtexitCount, &[0x1111]), Ok(0));
        assert_eq!(f.call(Service::SetThreadAtexitReport, &[0x2222]), Ok(0));
        let registrations = f.owner.thread_lifecycle_callbacks();
        let census = f.owner.handle_usage();
        let before = f.owner.usage();
        assert!(!f.owner.active_thread_requires_cleanup());
        assert_eq!(f.begin(), Ok(ThreadExitAction::Finished));
        let finished = f.owner.threads.active().exit_cleanup;
        assert!(!f.owner.active_thread_requires_cleanup());
        assert!(matches!(f.begin(), Err(Failure::Unsupported(_))));
        assert!(f.owner.threads.active().exit_cleanup == finished);
        assert!(f.owner.thread_lifecycle_callbacks() == registrations);
        assert_eq!(f.owner.handle_usage(), census);
        assert_eq!(f.owner.usage().bytes_read, before.bytes_read);
        assert_eq!(f.owner.usage().bytes_written, before.bytes_written);
    }

    #[test]
    fn general_callback_precedes_keys_and_live_later_values_are_cleared_before_admission() {
        let mut f = Fixture::new();
        let first = f.key(0x2222);
        let second = f.key(0x3333);
        f.set(first, 0xa1);
        f.set(second, 0xb1);
        assert_eq!(f.call(Service::SetThreadDtors, &[0x1111]), Ok(0));
        let census = f.owner.handle_usage();
        let general = callback(f.begin().unwrap());
        assert_eq!((general.entry(), general.args()), (0x1111, [0; 3]));
        assert_eq!(f.get(first), 0xa1);
        f.set(first, 0xa2); // modeled general callback writes live thread state
        let first_request = callback(f.complete(&general).unwrap());
        assert_eq!(
            (first_request.entry(), first_request.args()),
            (0x2222, [0xa2, 0, 0])
        );
        assert_eq!(f.get(first), 0);
        assert_eq!(f.get(second), 0xb1);
        assert!(f.owner.active_thread_requires_cleanup());
        let pending = f.owner.threads.active().exit_cleanup;
        assert!(matches!(f.complete(&general), Err(Failure::Unsupported(_))));
        assert!(f.owner.threads.active().exit_cleanup == pending);
        assert_eq!(f.get(second), 0xb1);
        f.set(second, 0xb2); // later slot changes are observed in this same pass
        let second_request = callback(f.complete(&first_request).unwrap());
        assert_eq!(
            (second_request.entry(), second_request.args()),
            (0x3333, [0xb2, 0, 0])
        );
        assert_eq!(f.get(second), 0);
        // There are now no eligible values, but the admitted callback still
        // requires an actual completion before the thread can be finished.
        assert!(f.owner.active_thread_requires_cleanup());
        assert_eq!(f.complete(&second_request), Ok(ThreadExitAction::Finished));
        assert!(f.owner.threads.active().values.is_none());
        assert!(!f.owner.active_thread_requires_cleanup());
        assert_eq!(
            f.owner.thread_lifecycle_callbacks().destructor,
            Some(0x1111)
        );
        assert_eq!(f.owner.handle_usage(), census);
    }

    #[test]
    fn four_pass_limit_uses_distinct_tickets_and_discards_the_final_republished_value() {
        let mut f = Fixture::new();
        let key = f.key(0x4444);
        f.set(key, 0x55);
        let mut request = callback(f.begin().unwrap());
        for pass in 0..4 {
            assert_eq!((request.entry(), request.args()), (0x4444, [0x55, 0, 0]));
            assert_eq!(f.get(key), 0);
            f.set(key, 0x55);
            let action = f.complete(&request).unwrap();
            if pass == 3 {
                assert_eq!(action, ThreadExitAction::Finished);
            } else {
                let next = callback(action);
                assert_ne!(next, request); // same code/value, new consume-once serial
                request = next;
            }
        }
        assert!(f.owner.threads.active().values.is_none());
        assert_eq!(f.get(key), 0);
        assert_eq!(f.owner.handle_usage().keys, 1);
        assert!(!f.owner.active_thread_requires_cleanup());
    }

    #[test]
    fn recreated_later_key_rejects_stale_value_and_earlier_mutations_wait_for_the_next_pass() {
        let mut f = Fixture::new();
        let first = f.key(0x1000);
        let old_second = f.key(0x2000);
        let third = f.key(0x3000);
        f.set(first, 10);
        f.set(old_second, 20);
        f.set(third, 30);
        let first_request = callback(f.begin().unwrap());
        assert_eq!(first_request.entry(), 0x1000);
        assert_eq!(f.call(Service::KeyDelete, &[old_second]), Ok(0));
        let new_second = f.key(0x4000);
        assert_eq!(new_second, old_second);
        f.set(first, 40);
        let third_request = callback(f.complete(&first_request).unwrap());
        // The new generation must receive neither old value20 nor an old
        // snapshotted destructor. The still-live later index2 runs first.
        assert_eq!(
            (third_request.entry(), third_request.args()),
            (0x3000, [30, 0, 0])
        );
        assert_eq!(f.get(new_second), 0);
        f.set(new_second, 50);
        let next_first = callback(f.complete(&third_request).unwrap());
        assert_eq!(
            (next_first.entry(), next_first.args()),
            (0x1000, [40, 0, 0])
        );
        let next_second = callback(f.complete(&next_first).unwrap());
        assert_eq!(
            (next_second.entry(), next_second.args()),
            (0x4000, [50, 0, 0])
        );
        assert_eq!(f.complete(&next_second), Ok(ThreadExitAction::Finished));
        assert!(f.owner.threads.active().values.is_none());
    }

    struct Denied;
    impl GuestAccess for Denied {
        fn preflight_pthread(&self) -> Result<(), Failure> {
            Err(Failure::Unsupported("original denied pthread fixture"))
        }
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("cleanup payload read");
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("cleanup payload write");
        }
    }

    #[test]
    fn pending_admission_survives_cross_owner_context_replay_preflight_and_budget_refusals() {
        let mut f = Fixture::new();
        let mut other = Fixture::new();
        for fixture in [&mut f, &mut other] {
            assert_eq!(fixture.call(Service::SetThreadDtors, &[0x1234]), Ok(0));
        }
        let request = callback(f.begin().unwrap());
        let other_request = callback(other.begin().unwrap());
        assert_eq!(
            (request.entry(), request.args()),
            (other_request.entry(), other_request.args())
        );
        assert_ne!(request, other_request);
        let before = f.owner.threads.active().exit_cleanup;
        let other_before = other.owner.threads.active().exit_cleanup;
        assert!(matches!(f.begin(), Err(Failure::Unsupported(_))));
        assert!(matches!(
            other.complete(&request),
            Err(Failure::Unsupported(_))
        ));
        assert!(other.owner.threads.active().exit_cleanup == other_before);
        assert!(matches!(
            f.owner.complete_thread_exit_callback(&request, &Denied),
            Err(Failure::Unsupported(_))
        ));
        assert!(f.owner.threads.active().exit_cleanup == before);
        let maximum = f.owner.budget.limits.max_work;
        f.owner.budget.limits.max_work = f.owner.usage().work + 1;
        assert!(matches!(f.complete(&request), Err(Failure::Limit(_))));
        assert!(f.owner.threads.active().exit_cleanup == before);
        f.owner.budget.limits.max_work = maximum;
        f.owner.activate_thread_context(CONTEXT + 1).unwrap();
        let foreign = f.owner.threads.active().exit_cleanup;
        assert!(matches!(f.complete(&request), Err(Failure::Unsupported(_))));
        assert!(f.owner.threads.active().exit_cleanup == foreign);
        f.owner.activate_thread_context(CONTEXT).unwrap();
        assert!(f.owner.threads.active().exit_cleanup == before);
        assert_eq!(f.complete(&request), Ok(ThreadExitAction::Finished));
        let finished = f.owner.threads.active().exit_cleanup;
        assert!(matches!(f.complete(&request), Err(Failure::Unsupported(_))));
        assert!(f.owner.threads.active().exit_cleanup == finished);
        assert_eq!(
            other.complete(&other_request),
            Ok(ThreadExitAction::Finished)
        );
    }

    #[test]
    fn no_callback_pass_drops_stale_and_null_destructor_values_without_guest_effects() {
        let mut f = Fixture::new();
        let opaque = f.key(0);
        let empty = f.key(0x1111);
        let stale = f.key(0x2222);
        f.set(opaque, 0xaa);
        f.set(empty, 0);
        f.set(stale, 0xbb);
        assert_eq!(f.call(Service::KeyDelete, &[stale]), Ok(0));
        assert!(!f.owner.active_thread_requires_cleanup());
        let old_capacity = f.owner.threads.active().values.as_ref().unwrap().capacity();
        let before = f.owner.threads.active().exit_cleanup;
        let maximum = f.owner.budget.limits.max_work;
        f.owner.budget.limits.max_work = f.owner.usage().work + 1;
        assert!(matches!(f.begin(), Err(Failure::Limit(_))));
        assert!(f.owner.threads.active().exit_cleanup == before);
        assert_eq!(
            f.owner.threads.active().values.as_ref().unwrap().capacity(),
            old_capacity
        );
        f.owner.budget.limits.max_work = maximum;
        assert_eq!(f.get(opaque), 0xaa);
        let usage = f.owner.usage();
        let census = f.owner.handle_usage();
        assert_eq!(f.begin(), Ok(ThreadExitAction::Finished));
        assert!(f.owner.threads.active().values.is_none());
        assert_eq!(f.owner.handle_usage(), census);
        assert_eq!(f.owner.usage().bytes_read, usage.bytes_read);
        assert_eq!(f.owner.usage().bytes_written, usage.bytes_written);
    }
}
