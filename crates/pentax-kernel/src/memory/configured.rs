// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from KytyPS5 f186b35706ddbc3faf5513f326e2335d224802f7,
// src/kernel/memory.cpp:3507..3518; see NOTICE and Kyty-MIT.txt.
// Available capacity additionally adapts KytyPS5-Performance 209dffe9032d72749257ce18680d506b891284de,
// src/kernel/memory.cpp:2707..2711,4404..4424; see NOTICE.
//! Configured and remaining capacity, with pre-effect admission of pool users.

use super::*;

impl MemoryState {
    pub(super) fn configured_flexible_size(
        &self,
        output: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.work(1)?;
        if output == 0 {
            return Err(Errno::Invalid.into());
        }
        budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &self.limits.flexible_memory_bytes.to_le_bytes(),
            }],
        )?;
        Ok(0)
    }

    pub(super) fn can_map_flexible(&self, length: u64, budget: &mut Budget) -> Result<(), Failure> {
        if length > self.remaining_flexible_bytes(budget)? {
            return Err(Errno::NoMemory.into());
        }
        Ok(())
    }

    pub(super) fn available_flexible_size(
        &self,
        output: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.work(1)?;
        // Preserve the source's null-before-pool-query order. Guest output is
        // admitted atomically only after the bounded owner-local census.
        if output == 0 {
            return Err(Errno::Invalid.into());
        }
        let available = self.remaining_flexible_bytes(budget)?;
        budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &available.to_le_bytes(),
            }],
        )?;
        Ok(0)
    }

    fn remaining_flexible_bytes(&self, budget: &mut Budget) -> Result<u64, Failure> {
        // The records already carry every current split and successful unmap.
        // Charge the full bounded scan before inspecting them. Imported image
        // protection records, direct aliases and internal pages are not users.
        budget.work(self.mappings.len() as u64)?;
        let retained = self
            .mappings
            .iter()
            .filter(|mapping| mapping.flexible)
            .try_fold(0_u64, |sum, mapping| {
                sum.checked_add(mapping.length)
                    .ok_or(Failure::Limit("flexible pool accounting"))
            })?;
        // Admission prevents an overcommitted pool; a broken internal census
        // remains an explicit failure instead of inventing a free-byte value.
        self.limits
            .flexible_memory_bytes
            .checked_sub(retained)
            .ok_or(Failure::Limit("flexible pool accounting"))
    }
}
