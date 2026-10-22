// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded scheduler admission for source-void yield.
// Adapted from KytyPS5@209dffe kernel/pthread.cpp PthreadYield; see NOTICE.
use crate::{Failure, Kernel};

impl Kernel {
    /// Admit the source-void call only for an execution adapter that will offer
    /// the next runnable owned guest pthread a cooperative scheduling turn.
    /// There are no arguments, guest outputs, errno writes or integer result.
    pub fn admit_pthread_yield(&mut self) -> Result<(), Failure> {
        self.budget.attempt()?;
        self.budget.work(1)
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{TestGuest, kernel};
    use crate::{CallArgs, Failure, Limits, Service};

    #[test]
    fn yield_requires_scheduler_and_charges_each_admission_without_guest_io() {
        let mut owner = kernel(Limits {
            max_attempts: 2,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        let before = owner.usage();
        assert_eq!(
            owner.invoke_library(Service::PthreadYield, CallArgs([u64::MAX; 6]), &mut guest),
            Err(Failure::Unsupported("cooperative guest-thread yield"))
        );
        assert_eq!(owner.admit_pthread_yield(), Ok(()));
        let after = owner.usage();
        assert_eq!(after.attempts, before.attempts + 2);
        assert_eq!(after.bytes_read, before.bytes_read);
        assert_eq!(after.bytes_written, before.bytes_written);
        assert_eq!(after.handles_live, before.handles_live);
        assert!(matches!(
            owner.admit_pthread_yield(),
            Err(Failure::Limit(_))
        ));
    }

    #[test]
    fn refused_yield_keeps_the_existing_work_limit_typed() {
        let mut owner = kernel(Default::default());
        owner.budget.limits.max_work = owner.usage().work;
        let before = owner.usage();
        assert!(matches!(
            owner.admit_pthread_yield(),
            Err(Failure::Limit("work"))
        ));
        assert_eq!(owner.usage().attempts, before.attempts + 1);
        assert_eq!(owner.usage().work, before.work);
    }
}
