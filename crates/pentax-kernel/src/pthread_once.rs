// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked initial-thread once admission and guest
// state writes; callback execution belongs to the actual prepared owner.
// Adapted from pinned kernel/pthread.cpp, Posix::pthread_once; see NOTICE.

use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Kernel};
use std::fmt;

/// Request data, never proof that an initializer executed or returned.
/// The native owner must independently admit the target and observe its return.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedPthreadOnce {
    control: u64,
    entry: u64,
}
impl AdmittedPthreadOnce {
    pub const fn control(&self) -> u64 {
        self.control
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
}
impl fmt::Debug for AdmittedPthreadOnce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedPthreadOnce")
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum PthreadOnceAction {
    Returned(u64),
    Initialize(AdmittedPthreadOnce),
}

impl Kernel {
    /// Admit one POSIX once call, consuming exactly one service attempt.
    /// This neither changes the control word nor executes the guest callback.
    pub fn admit_pthread_once(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<PthreadOnceAction, Failure> {
        self.budget.attempt()?;
        self.pthread_once_request(args, guest)
    }
    fn pthread_once_request(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<PthreadOnceAction, Failure> {
        guest.preflight_pthread()?;
        self.budget.work(12)?;
        let [control, entry, ..] = args.0;
        // Source validates both pointers even for an already completed once.
        if control == 0 || entry == 0 {
            return Ok(PthreadOnceAction::Returned(22));
        }
        match self.budget.u32(guest, control)? {
            0 => Ok(PthreadOnceAction::Initialize(AdmittedPthreadOnce {
                control,
                entry,
            })),
            1 => Ok(PthreadOnceAction::Returned(0)),
            2 | 3 => Err(Failure::Unsupported("contended pthread once wait")),
            _ => Ok(PthreadOnceAction::Returned(22)),
        }
    }
    pub(crate) fn invoke_pthread_once(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<u64, Failure> {
        match self.pthread_once_request(args, guest)? {
            PthreadOnceAction::Returned(value) => Ok(value),
            PthreadOnceAction::Initialize(_) => {
                Err(Failure::Unsupported("prepared pthread once callback"))
            }
        }
    }
    /// Write IN_PROGRESS only after the trusted owner has prepared the exact
    /// call frame and target. This is a memory transition, not execution proof.
    pub fn begin_pthread_once(
        &mut self,
        request: &AdmittedPthreadOnce,
        guest: &mut impl GuestAccess,
    ) -> Result<(), Failure> {
        guest.preflight_pthread()?;
        self.budget.work(8)?;
        if self.budget.u32(guest, request.control)? != 0 {
            return Err(Failure::Unsupported("stale pthread once admission"));
        }
        self.budget.write(
            guest,
            &[GuestWrite {
                address: request.control,
                bytes: &2_u32.to_le_bytes(),
            }],
        )
    }
    /// Write DONE only after the trusted owner independently verifies an actual
    /// initializer return. Neither request possession nor this pure backing
    /// operation is native execution evidence. The initializer has void result.
    pub fn complete_pthread_once(
        &mut self,
        request: &AdmittedPthreadOnce,
        guest: &mut impl GuestAccess,
    ) -> Result<(), Failure> {
        guest.preflight_pthread()?;
        self.budget.work(8)?;
        self.budget.write(
            guest,
            &[GuestWrite {
                address: request.control,
                bytes: &1_u32.to_le_bytes(),
            }],
        )
    }
}
