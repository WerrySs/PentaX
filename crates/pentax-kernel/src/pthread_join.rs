// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked publication of owned cooperative joins.
// Adapted from pinned kernel/pthread.cpp PthreadJoin; see NOTICE.
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Kernel};
impl Kernel {
    /// Consume one call attempt. Actual completion and handle lifecycle belong
    /// to the prepared owner; no numeric value alone proves a thread return.
    pub fn admit_pthread_join(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        self.budget.attempt()?;
        self.budget.work(12)?;
        if args.0[0] == 0 {
            return Ok(Some(Errno::Invalid.kernel_code() as u32 as u64));
        }
        guest.preflight_pthread()?;
        if self.threads.active().self_pointer == Some(args.0[0]) {
            return Ok(Some(Errno::Deadlock.kernel_code() as u32 as u64));
        }
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("nested pthread join wait"));
        }
        Ok(None)
    }
    pub fn poll_pthread_join(&mut self) -> Result<(), Failure> {
        self.budget.work(8)
    }
    pub fn park_pthread_join(&mut self) -> Result<(), Failure> {
        self.budget.work(1)?;
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("nested pthread join wait"));
        }
        self.threads.active_mut().waiting = true;
        Ok(())
    }
    /// Atomic optional result publication. The core must independently retain
    /// the exact owned handle, sole join claim and authenticated completed value.
    /// A failed output write does not clear the waiting state or reap anything.
    pub fn publish_pthread_join(
        &mut self,
        context: u64,
        pointer: u64,
        output: u64,
        value: u64,
        waited: bool,
        guest: &mut impl GuestAccess,
    ) -> Result<(), Failure> {
        guest.preflight_pthread()?;
        self.threads
            .require_registered(context, pointer, &mut self.budget)?;
        if self.threads.active().waiting != waited {
            return Err(Failure::Unsupported("pthread join wait state"));
        }
        if output != 0 {
            self.budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &value.to_le_bytes(),
                }],
            )?;
        }
        self.threads.active_mut().waiting = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
