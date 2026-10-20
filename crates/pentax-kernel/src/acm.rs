// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7,
// src/libs/acm.cpp (303c2afe5fb515456edc64519e5731938358d78b) and
// src/libs/acm.h (f561b2cdf08560fb4d7022f5d22a26f3036cc7f7).
// Modified for PentaX: process ownership, finite admission and atomic output.

use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Kernel};

/// Creation-only owned contexts. No host pointers, batch receipts or DSP work
/// are represented by these identifiers. The next value can represent the
/// exhausted state after issuing u32::MAX once, without wrapping or reuse.
pub(crate) struct AcmState {
    next: u64,
    contexts: usize,
}

impl Default for AcmState {
    fn default() -> Self {
        Self {
            next: 1,
            contexts: 0,
        }
    }
}

impl AcmState {
    pub(crate) const fn retained_handle_count(&self) -> usize {
        self.contexts
    }
}

impl Kernel {
    pub(crate) fn acm_context_create(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        if args.0[0] == 0 {
            // Upstream takes an unsupported path, not a documented Sony errno.
            return Err(Failure::Unsupported("null ACM context output"));
        }
        self.budget.work(1)?;
        let id =
            u32::try_from(self.acm.next).map_err(|_| Failure::Limit("ACM context identifiers"))?;
        self.budget.can_add_handle()?;
        let contexts = self
            .acm
            .contexts
            .checked_add(1)
            .ok_or(Failure::Limit("ACM contexts"))?;
        let next = self.acm.next + 1; // Proven <= u32::MAX + 1 above.
        self.budget.write(
            guest,
            &[GuestWrite {
                address: args.0[0],
                bytes: &id.to_le_bytes(),
            }],
        )?;
        // The guest adapter preflights and commits the complete live output.
        // Only infallible scalar publication follows that transaction.
        self.acm.next = next;
        self.acm.contexts = contexts;
        self.budget.added_handle();
        Ok(0)
    }
}

#[cfg(test)]
#[path = "acm/tests.rs"]
mod tests;
