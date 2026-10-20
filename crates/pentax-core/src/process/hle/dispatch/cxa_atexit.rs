// SPDX-License-Identifier: GPL-2.0-only

//! Original process-owned registration storage. Exact API provenance is in
//! docs/upstream-provenance.md.
//! Registration retains opaque callback/argument/DSO words in source order.
//! A future finalize implementation needs actual guest-call/return admission;
//! dropping this host container never calls guest or host function pointers.

use super::*;

const RECORD_BYTES: u64 = std::mem::size_of::<[u64; 3]>() as u64;
const REGISTER_WORK: u64 = RECORD_BYTES + 1;

pub(super) struct ExitCallbacks {
    entries: Vec<[u64; 3]>,
}

impl ExitCallbacks {
    pub(super) const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl fmt::Debug for ExitCallbacks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Callback and DSO values are opaque guest data, not diagnostics.
        f.debug_struct("ExitCallbacks")
            .field("count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl ServiceBudget {
    pub(super) fn cxa_atexit(
        &mut self,
        args: AbiArgs,
    ) -> Result<ServiceReturn, MemoryServiceError> {
        self.work = total(
            self.work,
            REGISTER_WORK,
            self.limits.max_work_units,
            ServiceLimit::WorkUnits,
        )?;
        let requested = total(
            self.exit_callbacks.entries.len() as u64,
            1,
            self.limits.max_exit_callbacks,
            ServiceLimit::ExitCallbacks,
        )?;
        // A configured slot limit must also describe a finite retained byte
        // bound. This cap is independent of guest span/scratch/byte allowances.
        self.limits
            .max_exit_callbacks
            .checked_mul(RECORD_BYTES)
            .ok_or(MemoryServiceError::CounterOverflow {
                kind: ServiceLimit::ExitCallbacks,
            })?;
        let words = [args.rdi, args.rsi, args.rdx];
        if self.exit_callbacks.entries.len() == self.exit_callbacks.entries.capacity() {
            let maximum = usize::try_from(self.limits.max_exit_callbacks)
                .map_err(|_| MemoryServiceError::Allocation)?;
            let target = self
                .exit_callbacks
                .entries
                .capacity()
                .checked_mul(2)
                .unwrap_or(maximum)
                .max(1)
                .min(maximum);
            debug_assert!(target as u64 >= requested);
            // Stage growth separately: allocator rounding, allocation or work
            // refusal cannot publish an entry or change retained capacity.
            let growth = (target as u64)
                .checked_add(self.exit_callbacks.entries.len() as u64)
                .and_then(|slots| slots.checked_mul(RECORD_BYTES))
                .ok_or(MemoryServiceError::CounterOverflow {
                    kind: ServiceLimit::WorkUnits,
                })?;
            self.work = total(
                self.work,
                growth,
                self.limits.max_work_units,
                ServiceLimit::WorkUnits,
            )?;
            #[cfg(test)]
            {
                let index = self.reservations;
                self.reservations += 1;
                if self.refuse_reservation == Some(index) {
                    return Err(MemoryServiceError::Allocation);
                }
            }
            let mut staged = Vec::new();
            staged
                .try_reserve_exact(target)
                .map_err(|_| MemoryServiceError::Allocation)?;
            within(
                staged.capacity() as u64,
                self.limits.max_exit_callbacks,
                ServiceLimit::ExitCallbacks,
            )?;
            (staged.capacity() as u64)
                .checked_add(self.exit_callbacks.entries.capacity() as u64)
                .and_then(|slots| slots.checked_mul(RECORD_BYTES))
                .ok_or(MemoryServiceError::CounterOverflow {
                    kind: ServiceLimit::ExitCallbacks,
                })?;
            staged.extend_from_slice(&self.exit_callbacks.entries);
            staged.push(words);
            self.exit_callbacks.entries = staged;
        } else {
            self.exit_callbacks.entries.push(words);
        }
        Ok(self.complete(KnownFunction::CxaAtExit, 0))
    }
}

#[cfg(test)]
mod tests;
