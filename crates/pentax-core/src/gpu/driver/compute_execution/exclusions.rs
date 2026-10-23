// SPDX-License-Identifier: GPL-2.0-only
//! One reusable, charged command exclusion image shared by both engines.
use super::*;

impl Driver {
    pub(in crate::gpu::driver) fn with_command_exclusions<T>(
        &mut self,
        primary: &[compute::CommandRange],
        local_held: usize,
        action: impl FnOnce(&mut Self, &[compute::CommandRange]) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let ceiling = if local_held == 0 {
            self.graphics_host_ceiling()?
                .min(self.limits.compute.max_host_bytes)
        } else {
            self.graphics_host_ceiling()?
        };
        let mut scratch = std::mem::take(&mut self.command_exclusions);
        let result = (|| {
            scratch.clear();
            let needed = primary
                .len()
                .checked_add(self.compute_queues.ranges.len())
                .and_then(|n| n.checked_add(self.graphics_exclusion_count()?))
                .ok_or(Failure::Limit)?;
            if needed > scratch.capacity() {
                // Reallocation can coexist with the old backing. Admit that
                // peak before requesting it; retain actual capacity on error.
                let bytes = needed
                    .checked_mul(size_of::<compute::CommandRange>())
                    .ok_or(Failure::Limit)?;
                self.graphics_held_bytes()?
                    .checked_add(local_held)
                    .and_then(|n| n.checked_add(bytes))
                    .filter(|n| *n <= ceiling)
                    .ok_or(Failure::Limit)?;
                scratch
                    .try_reserve_exact(needed)
                    .map_err(|_| Failure::Allocation)?;
                let bytes = scratch
                    .capacity()
                    .checked_mul(size_of::<compute::CommandRange>())
                    .ok_or(Failure::Limit)?;
                let growth = bytes
                    .checked_sub(self.command_exclusion_bytes)
                    .ok_or(Failure::Limit)?;
                self.command_exclusion_bytes = bytes;
                self.compute_charge(growth as u64)?;
            }
            self.graphics_held_bytes()?
                .checked_add(local_held)
                .filter(|n| *n <= ceiling)
                .ok_or(Failure::Limit)?;
            self.compute_charge(needed as u64)?;
            scratch.extend_from_slice(primary);
            scratch.extend_from_slice(&self.compute_queues.ranges);
            self.copy_graphics_exclusions(&mut scratch);
            action(self, &scratch)
        })();
        self.command_exclusions = scratch;
        result
    }
}
