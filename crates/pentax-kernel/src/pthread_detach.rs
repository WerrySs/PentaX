// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: registered owner-bound pthread detachment.
// Adapted from pinned kernel/pthread.cpp PthreadDetach and pthread_detach;
// see NOTICE. Actual execution and Join claims belong to the prepared runtime.
use crate::{CallArgs, Errno, Failure, GuestAccess, Kernel, Service};

/// Return the selected source ABI's error value without modifying errno.
pub fn pthread_detach_error(service: Service, error: Errno) -> Result<u64, Failure> {
    match service {
        Service::PthreadDetach => Ok(error.kernel_code() as u32 as u64),
        Service::PosixPthreadDetach => Ok(error as u64),
        _ => Err(Failure::Unsupported("pthread detach service")),
    }
}

impl Kernel {
    /// Charge one call before the prepared owner checks its actual lifecycle.
    /// The reference validates null before touching the thread object.
    pub fn admit_pthread_detach(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        if !matches!(
            service,
            Service::PthreadDetach | Service::PosixPthreadDetach
        ) {
            return Err(Failure::Unsupported("pthread detach service"));
        }
        self.budget.attempt()?;
        self.budget.work(12)?;
        if args.0[0] == 0 {
            return pthread_detach_error(service, Errno::Invalid).map(Some);
        }
        guest.preflight_pthread()?;
        Ok(None)
    }

    /// Commit only after the prepared runtime has rejected reaped handles and
    /// accepted Join claims. The registered initial thread is also detachable.
    /// No guest memory is read/written and no backing or handle is reclaimed.
    pub fn complete_pthread_detach(
        &mut self,
        service: Service,
        pointer: u64,
        guest: &impl GuestAccess,
    ) -> Result<u64, Failure> {
        if !matches!(
            service,
            Service::PthreadDetach | Service::PosixPthreadDetach
        ) {
            return Err(Failure::Unsupported("pthread detach service"));
        }
        guest.preflight_pthread()?;
        if self.threads.detach(pointer, &mut self.budget)? {
            Ok(0)
        } else {
            pthread_detach_error(service, Errno::NoSuchResource)
        }
    }
}

#[cfg(test)]
mod tests;
