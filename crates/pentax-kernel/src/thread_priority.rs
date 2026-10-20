// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned guest scheduling metadata and cooperative
// classes, with no host priority, affinity or global scheduling-policy change.
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/kernel/pthread.cpp (a7fa3003f534af01a119f117954ffe899c982351).
// See NOTICE and upstream provenance for the explicit host-scheduler divergence.

use crate::font::fixed::FixedWrites;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Kernel, Service};

/// Effective guest execution class for the cooperative runtime. Exact guest
/// priority/policy remain separate metadata. These are not Windows priorities,
/// priority-inheritance promises, or authority to change any host setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadPriorityClass {
    Low,
    Normal,
    High,
}

impl ThreadPriorityClass {
    pub(crate) const fn from_priority(priority: i32, core_bound: bool) -> Self {
        let class = if priority <= 478 {
            Self::High
        } else if priority >= 733 {
            Self::Low
        } else {
            Self::Normal
        };
        if core_bound {
            match class {
                Self::High => Self::Normal,
                Self::Normal | Self::Low => Self::Low,
            }
        } else {
            class
        }
    }
}

impl Kernel {
    /// Allocation-free observation after the runtime selected the same actual
    /// TLS context. Reading this does not consume a library attempt or work.
    pub fn active_thread_priority_class(&self) -> ThreadPriorityClass {
        self.threads.active().priority_class
    }

    pub(crate) fn invoke_thread_priority(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        use Service::*;
        match service {
            // The source ignores policy, including invalid/negative values.
            // These scalar queries have no thread-backing requirement.
            PosixSchedGetPriorityMax => return Ok(256),
            PosixSchedGetPriorityMin => return Ok(767),
            PthreadGetprio
            | PthreadSetprio
            | PosixThreadSetprio
            | PosixThreadGetSchedParam
            | PosixThreadSetSchedParam => {}
            _ => return Err(Failure::Unsupported("thread priority service")),
        }
        guest.preflight_pthread()?;
        self.budget.work(8)?;
        let a = args.0;
        if matches!(
            service,
            PthreadGetprio | PthreadSetprio | PosixThreadSetprio
        ) {
            let result = if a[0] == 0 {
                Err(Errno::NoSuchResource.into())
            } else if service == PthreadGetprio {
                let entry = self.threads.priority_entry(a[0], &mut self.budget)?;
                if a[1] == 0 {
                    return Err(Failure::Unsupported("source-null pthread priority output"));
                }
                let priority = entry.priority.to_le_bytes();
                self.budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &priority,
                    }],
                )?;
                Ok(0)
            } else {
                let entry = self.threads.priority_entry_mut(a[0], &mut self.budget)?;
                let priority = a[1] as u32 as i32;
                let class = ThreadPriorityClass::from_priority(priority, entry.core_bound);
                // All validation/work precedes this infallible metadata commit.
                entry.priority = priority;
                entry.priority_class = class;
                Ok(0)
            };
            return if service == PosixThreadSetprio {
                crate::posix::pthread_result(result)
            } else {
                result
            };
        }
        if a[0] == 0 || a[2] == 0 || (service == PosixThreadGetSchedParam && a[1] == 0) {
            return Ok(Errno::Invalid as u64);
        }
        if service == PosixThreadGetSchedParam {
            let entry = self.threads.priority_entry(a[0], &mut self.budget)?;
            let policy = entry.policy.to_le_bytes();
            let priority = entry.priority.to_le_bytes();
            // Source policy store precedes the priority store, including exact
            // and partial aliases. Normalize them privately into one atomic
            // transaction without reading/clobbering any intervening bytes.
            let mut outputs = FixedWrites::new();
            outputs.push(a[1], &policy)?;
            outputs.push(a[2], &priority)?;
            outputs.apply(guest, &mut self.budget)?;
            return Ok(0);
        }
        let entry = self.threads.priority_entry_mut(a[0], &mut self.budget)?;
        // Only sched_priority:i32@0 is accessed, never a host sched_param image.
        // A typed bad-backing/policy refusal occurs before either owned field
        // changes. The reference's unavailable host failures are not invented.
        let priority = self.budget.u32(guest, a[2])? as i32;
        entry.policy = a[1] as u32 as i32;
        entry.priority = priority;
        // This source wrapper mutates attributes, not the running thread's
        // effective host priority. A future first start still recomputes class
        // from then-current metadata; started threads retain their old class.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ARG, PAGE, TestGuest, kernel};
    use crate::{
        GuestError, InitialThreadStack, Limits, MemoryEffect, ThreadAttributes, ThreadExitAction,
    };

    const MAIN: u64 = 0x1100_0000;
    const PARAM: u64 = ARG + 0x300;
    const POLICY_OUT: u64 = ARG + 0x180;
    const PRIORITY_OUT: u64 = ARG + 0x188;

    fn library(
        owner: &mut Kernel,
        guest: &mut impl GuestAccess,
        service: Service,
        values: &[u64],
    ) -> Result<u64, Failure> {
        let mut args = [0; 6];
        args[..values.len()].copy_from_slice(values);
        owner.invoke_library(service, CallArgs(args), guest)
    }
    struct Fixture {
        owner: Kernel,
        guest: TestGuest,
        main: u64,
        next_pointer: u64,
    }
    impl Fixture {
        fn new() -> Self {
            let mut owner = kernel(Limits::default());
            let mut guest = TestGuest::new();
            owner.activate_thread_context(MAIN).unwrap();
            let main = library(&mut owner, &mut guest, Service::PthreadSelf, &[]).unwrap();
            Self {
                owner,
                guest,
                main,
                next_pointer: ARG + 0x400,
            }
        }
        fn call(&mut self, service: Service, values: &[u64]) -> Result<u64, Failure> {
            library(&mut self.owner, &mut self.guest, service, values)
        }
        fn child(&mut self, context: u64, priority: i32, policy: i32, affinity: u64) -> u64 {
            let pointer = self.next_pointer;
            self.next_pointer += 16;
            self.owner
                .register_thread_context_with_attributes(
                    context,
                    pointer,
                    ThreadAttributes {
                        priority,
                        policy,
                        affinity,
                        ..ThreadAttributes::default()
                    },
                    &mut self.guest,
                )
                .unwrap();
            pointer
        }
        fn set_metadata(&mut self, handle: u64, policy: i32, priority: i32) {
            self.guest.put(PARAM - ARG, &priority.to_le_bytes());
            assert_eq!(
                self.call(
                    Service::PosixThreadSetSchedParam,
                    &[handle, policy as u32 as u64, PARAM]
                ),
                Ok(0)
            );
        }
        fn query(&mut self, handle: u64) -> (i32, i32) {
            assert_eq!(
                self.call(
                    Service::PosixThreadGetSchedParam,
                    &[handle, POLICY_OUT, PRIORITY_OUT]
                ),
                Ok(0)
            );
            (self.word(POLICY_OUT), self.word(PRIORITY_OUT))
        }
        fn word(&self, address: u64) -> i32 {
            i32::from_le_bytes(self.guest.get(address, 4).try_into().unwrap())
        }
    }

    struct NoGuest;
    impl GuestAccess for NoGuest {
        fn preflight_pthread(&self) -> Result<(), Failure> {
            panic!("scalar priority query required pthread backing");
        }
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("scalar priority payload read");
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("scalar priority write");
        }
    }

    #[test]
    fn scalar_endpoints_ignore_policy_and_class_observation_consumes_no_attempt_or_work() {
        let mut owner = kernel(Limits::default());
        for policy in [0, 1, 3, u64::MAX, 0x8000_0000] {
            assert_eq!(
                library(
                    &mut owner,
                    &mut NoGuest,
                    Service::PosixSchedGetPriorityMax,
                    &[policy]
                ),
                Ok(256)
            );
            assert_eq!(
                library(
                    &mut owner,
                    &mut NoGuest,
                    Service::PosixSchedGetPriorityMin,
                    &[policy]
                ),
                Ok(767)
            );
        }
        let before = owner.usage();
        assert_eq!(
            owner.active_thread_priority_class(),
            ThreadPriorityClass::Normal
        );
        assert_eq!(owner.usage(), before);
        assert_eq!(before.bytes_read + before.bytes_written, 0);
        assert_eq!(before.handles_live + before.mapped_bytes as usize, 0);
    }

    #[test]
    fn source_errors_and_full_width_live_handle_resolution_preserve_errno_and_outputs() {
        let mut f = Fixture::new();
        let errno = f.call(Service::ErrnoLocation, &[]).unwrap();
        f.guest
            .apply(
                MemoryEffect::None,
                &[GuestWrite {
                    address: errno,
                    bytes: &37u32.to_le_bytes(),
                }],
            )
            .unwrap();
        f.guest.put(POLICY_OUT - ARG, &[0xa5; 20]);
        let census = f.owner.handle_usage();
        // Raw service calls retain typed errors; the audited library adapter
        // converts precisely these int32 outcomes into the guest return word.
        for service in [Service::PthreadGetprio, Service::PthreadSetprio] {
            assert_eq!(
                f.owner.invoke(service, CallArgs([0; 6]), &mut f.guest),
                Err(Errno::NoSuchResource.into())
            );
        }
        for (service, args, expected) in [
            (Service::PthreadGetprio, [0, 0, 0], 0x8002_0003),
            (Service::PthreadSetprio, [0, u64::MAX, 0], 0x8002_0003),
            (Service::PosixThreadSetprio, [0, u64::MAX, 0], 3),
            (
                Service::PosixThreadGetSchedParam,
                [0, POLICY_OUT, PRIORITY_OUT],
                22,
            ),
            (
                Service::PosixThreadGetSchedParam,
                [f.main, 0, PRIORITY_OUT],
                22,
            ),
            (
                Service::PosixThreadGetSchedParam,
                [f.main, POLICY_OUT, 0],
                22,
            ),
            (Service::PosixThreadSetSchedParam, [0, 99, PARAM], 22),
            (Service::PosixThreadSetSchedParam, [f.main, 99, 0], 22),
        ] {
            assert_eq!(f.call(service, &args), Ok(expected));
        }
        assert!(matches!(
            f.call(Service::PthreadGetprio, &[f.main, 0]),
            Err(Failure::Unsupported(_))
        ));
        for handle in [f.main + 1, f.main ^ (1u64 << 63)] {
            assert!(matches!(
                f.call(Service::PthreadGetprio, &[handle, PRIORITY_OUT]),
                Err(Failure::Unsupported(_))
            ));
            assert!(matches!(
                f.call(Service::PthreadSetprio, &[handle, 256]),
                Err(Failure::Unsupported(_))
            ));
            assert!(matches!(
                f.call(Service::PosixThreadSetSchedParam, &[handle, 99, PARAM]),
                Err(Failure::Unsupported(_))
            ));
        }
        assert_eq!(f.guest.get(POLICY_OUT, 20), [0xa5; 20]);
        assert_eq!(f.guest.get(errno, 4), 37u32.to_le_bytes());
        assert_eq!(f.owner.handle_usage(), census);
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Normal
        );
        let child = f.child(MAIN + 1, 700, 1, 0x7f);
        f.owner.activate_thread_context(MAIN + 1).unwrap();
        assert_eq!(
            f.owner.begin_thread_exit(&f.guest),
            Ok(ThreadExitAction::Finished)
        );
        f.owner.activate_thread_context(MAIN).unwrap();
        assert!(matches!(
            f.call(Service::PthreadGetprio, &[child, PRIORITY_OUT]),
            Err(Failure::Unsupported(_))
        ));
        assert_eq!(f.guest.get(errno, 4), 37u32.to_le_bytes());
    }

    #[test]
    fn exact_signed_low32_priorities_have_source_bands_and_only_four_output_bytes() {
        let mut f = Fixture::new();
        for (priority, class) in [
            (i32::MIN, ThreadPriorityClass::High),
            (-1, ThreadPriorityClass::High),
            (478, ThreadPriorityClass::High),
            (479, ThreadPriorityClass::Normal),
            (732, ThreadPriorityClass::Normal),
            (733, ThreadPriorityClass::Low),
            (i32::MAX, ThreadPriorityClass::Low),
        ] {
            let word = 0xaabb_ccdd_0000_0000 | u64::from(priority as u32);
            assert_eq!(f.call(Service::PthreadSetprio, &[f.main, word]), Ok(0));
            f.guest.put(PRIORITY_OUT - ARG, &[0xa5; 8]);
            assert_eq!(
                f.call(Service::PthreadGetprio, &[f.main, PRIORITY_OUT]),
                Ok(0)
            );
            assert_eq!(f.word(PRIORITY_OUT), priority);
            assert_eq!(f.guest.get(PRIORITY_OUT + 4, 4), [0xa5; 4]);
            let usage = f.owner.usage();
            assert_eq!(f.owner.active_thread_priority_class(), class);
            assert_eq!(f.owner.usage(), usage);
        }
        assert_eq!(
            f.call(Service::PosixThreadSetprio, &[f.main, u64::MAX]),
            Ok(0)
        );
        assert_eq!(f.query(f.main), (1, -1));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::High
        );
    }

    #[test]
    fn cross_thread_attributes_are_exact_snapshots_and_only_setprio_changes_started_class() {
        let mut f = Fixture::new();
        let high = f.child(MAIN + 1, 478, -77, 0x7f);
        let low = f.child(MAIN + 2, 733, i32::MAX, 0x7f);
        f.owner.activate_thread_context(MAIN + 1).unwrap();
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::High
        );
        f.set_metadata(high, -55, 733);
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::High
        );
        f.owner.activate_thread_context(MAIN + 2).unwrap();
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Low
        );
        f.set_metadata(high, -55, 479);
        assert_eq!(f.query(high), (-55, 479));
        assert_eq!(f.query(low), (i32::MAX, 733));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Low
        );
        f.owner.activate_thread_context(MAIN + 1).unwrap();
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::High
        );
        assert_eq!(f.call(Service::PosixThreadSetprio, &[high, 479]), Ok(0));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Normal
        );
        assert_eq!(f.query(high), (-55, 479));
        assert_eq!(f.query(f.main), (1, 700));
    }

    #[test]
    fn first_child_selection_latches_current_metadata_once_but_main_and_direct_contexts_do_not() {
        let mut f = Fixture::new();
        let child = f.child(MAIN + 1, 700, 13, 0x7f);
        assert_eq!(f.call(Service::PthreadSetprio, &[child, 256]), Ok(0));
        f.set_metadata(child, 14, 478);
        assert_eq!(f.call(Service::PthreadSetaffinity, &[child, 2]), Ok(0));
        f.owner.activate_thread_context(MAIN + 1).unwrap();
        assert_eq!(f.query(child), (14, 478));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Normal
        );
        assert_eq!(f.call(Service::PthreadSetaffinity, &[child, 0x7f]), Ok(0));
        for (priority, class) in [
            (478, ThreadPriorityClass::Normal),
            (479, ThreadPriorityClass::Low),
            (733, ThreadPriorityClass::Low),
        ] {
            assert_eq!(f.call(Service::PthreadSetprio, &[child, priority]), Ok(0));
            assert_eq!(f.owner.active_thread_priority_class(), class);
        }
        f.owner.activate_thread_context(MAIN).unwrap();
        assert_eq!(f.call(Service::PthreadSetaffinity, &[f.main, 1]), Ok(0));
        assert_eq!(f.call(Service::PthreadSetprio, &[f.main, 478]), Ok(0));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::High
        );
        f.owner.activate_thread_context(MAIN + 2).unwrap(); // preexisting direct selection
        let direct = f.call(Service::PthreadSelf, &[]).unwrap();
        assert_eq!(f.call(Service::PthreadSetaffinity, &[direct, 1]), Ok(0));
        assert_eq!(f.call(Service::PthreadSetprio, &[direct, 478]), Ok(0));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::High
        );
    }

    struct NoReadGuest<'a>(&'a mut TestGuest);
    impl GuestAccess for NoReadGuest<'_> {
        fn preflight_pthread(&self) -> Result<(), Failure> {
            self.0.preflight_pthread()
        }
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("priority getter read output payload");
        }
        fn apply(
            &mut self,
            effect: MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.0.apply(effect, writes)
        }
    }

    #[test]
    fn ordered_getter_aliases_and_late_backing_quota_refusals_are_atomic() {
        let mut f = Fixture::new();
        f.set_metadata(f.main, 0x1122_3344, 0x5566_7788);
        for offset in [0, 2] {
            f.guest.put(POLICY_OUT - ARG - 1, &[0xa5; 10]);
            assert_eq!(
                library(
                    &mut f.owner,
                    &mut NoReadGuest(&mut f.guest),
                    Service::PosixThreadGetSchedParam,
                    &[f.main, POLICY_OUT, POLICY_OUT + offset]
                ),
                Ok(0)
            );
            let expected = if offset == 0 {
                vec![0xa5, 0x88, 0x77, 0x66, 0x55, 0xa5, 0xa5, 0xa5]
            } else {
                vec![0xa5, 0x44, 0x33, 0x88, 0x77, 0x66, 0x55, 0xa5]
            };
            assert_eq!(f.guest.get(POLICY_OUT - 1, 8), expected);
        }
        f.guest.put(POLICY_OUT - ARG, &[0xa5; 16]);
        assert_eq!(
            f.call(
                Service::PosixThreadGetSchedParam,
                &[f.main, POLICY_OUT, ARG + PAGE - 2]
            ),
            Err(Failure::Guest(GuestError::WriteDenied))
        );
        assert_eq!(f.guest.get(POLICY_OUT, 16), [0xa5; 16]);
        let maximum = f.owner.budget.limits.max_bytes;
        let usage = f.owner.usage();
        f.owner.budget.limits.max_bytes = usage.bytes_read + usage.bytes_written + 7;
        assert!(matches!(
            f.call(
                Service::PosixThreadGetSchedParam,
                &[f.main, POLICY_OUT, PRIORITY_OUT]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(f.guest.get(POLICY_OUT, 16), [0xa5; 16]);
        f.owner.budget.limits.max_bytes = maximum;
        assert_eq!(f.query(f.main), (0x1122_3344, 0x5566_7788));
        let class = f.owner.active_thread_priority_class();
        let work_limit = f.owner.budget.limits.max_work;
        f.owner.budget.limits.max_work = f.owner.usage().work + 1;
        assert!(matches!(
            f.call(Service::PthreadSetprio, &[f.main, i32::MAX as u64]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(f.owner.active_thread_priority_class(), class);
        f.owner.budget.limits.max_work = work_limit;
        assert_eq!(f.query(f.main), (0x1122_3344, 0x5566_7788));
        assert!(matches!(
            f.call(
                Service::PosixThreadSetSchedParam,
                &[f.main, 99, ARG + PAGE - 2]
            ),
            Err(Failure::Guest(_))
        ));
        assert_eq!(f.query(f.main), (0x1122_3344, 0x5566_7788));
        f.guest.put(PAGE - 4, &(-123i32).to_le_bytes());
        assert_eq!(
            f.call(
                Service::PosixThreadSetSchedParam,
                &[f.main, u64::MAX, ARG + PAGE - 4]
            ),
            Ok(0)
        );
        assert_eq!(f.query(f.main), (-1, -123));
        assert_eq!(f.owner.active_thread_priority_class(), class);
    }

    struct StackGuest<'a>(&'a mut TestGuest);
    impl GuestAccess for StackGuest<'_> {
        fn preflight_pthread(&self) -> Result<(), Failure> {
            self.0.preflight_pthread()
        }
        fn initial_thread_stack(&self) -> Result<InitialThreadStack, Failure> {
            Ok(InitialThreadStack {
                address: ARG,
                size: PAGE,
                guard_size: 0,
            })
        }
        fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), GuestError> {
            self.0.read(address, bytes)
        }
        fn apply(
            &mut self,
            effect: MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.0.apply(effect, writes)
        }
    }

    #[test]
    fn creation_registration_retry_and_active_attribute_query_preserve_live_policy() {
        let mut f = Fixture::new();
        let attributes = ThreadAttributes {
            priority: 256,
            policy: -8,
            affinity: 1,
            ..ThreadAttributes::default()
        };
        let pointer = ARG + 0x400;
        f.guest.put(pointer - ARG, &[0xa5; 8]);
        f.guest.refuse = true;
        assert_eq!(
            f.owner.register_thread_context_with_attributes(
                MAIN + 1,
                pointer,
                attributes,
                &mut f.guest
            ),
            Err(Failure::Guest(GuestError::WriteDenied))
        );
        assert_eq!(f.guest.get(pointer, 8), [0xa5; 8]);
        f.guest.refuse = false;
        f.owner
            .register_thread_context_with_attributes(MAIN + 1, pointer, attributes, &mut f.guest)
            .unwrap();
        assert_eq!(f.guest.get(pointer + 4, 4), [0xa5; 4]);
        f.owner.activate_thread_context(MAIN + 1).unwrap();
        assert_eq!(f.query(pointer), (-8, 256));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Normal
        );
        f.set_metadata(pointer, -9, 733);
        let slot = ARG + 0x100;
        assert_eq!(f.call(Service::ThreadAttrInit, &[slot]), Ok(0));
        assert_eq!(
            library(
                &mut f.owner,
                &mut StackGuest(&mut f.guest),
                Service::ThreadAttrGet,
                &[pointer, slot]
            ),
            Ok(0)
        );
        assert_eq!(
            f.call(Service::ThreadAttrGetSchedPolicy, &[slot, POLICY_OUT]),
            Ok(0)
        );
        assert_eq!(
            f.call(Service::ThreadAttrGetSchedParam, &[slot, PRIORITY_OUT]),
            Ok(0)
        );
        assert_eq!((f.word(POLICY_OUT), f.word(PRIORITY_OUT)), (-9, 733));
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Normal
        );
        f.owner.activate_thread_context(MAIN).unwrap();
        let legacy = ARG + 0x420;
        f.owner
            .register_thread_context(MAIN + 2, legacy, 733, 0x7f, &mut f.guest)
            .unwrap();
        assert_eq!(f.query(legacy), (1, 733));
        f.owner.activate_thread_context(MAIN + 2).unwrap();
        assert_eq!(
            f.owner.active_thread_priority_class(),
            ThreadPriorityClass::Low
        );
    }
}
