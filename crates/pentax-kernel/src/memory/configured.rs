// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from KytyPS5 f186b35706ddbc3faf5513f326e2335d224802f7,
// src/kernel/memory.cpp:3507..3518; see NOTICE and Kyty-MIT.txt.
//! Immutable configured capacity and pre-effect admission of actual pool users.

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
        if retained
            .checked_add(length)
            .is_none_or(|total| total > self.limits.flexible_memory_bytes)
        {
            return Err(Errno::NoMemory.into());
        }
        Ok(())
    }
}
