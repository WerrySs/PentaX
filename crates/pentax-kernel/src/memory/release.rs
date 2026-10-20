// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: pre-effect physical/alias split staging; see NOTICE.

//! Adapt GetAllocatedSpan/FindMappings/ReleaseDirectMemoryInternal without pointers.
use super::*;

impl MemoryState {
    pub(super) fn physical_coverage(&self, start: u64, end: u64) -> bool {
        let mut cursor = start;
        while cursor < end {
            let Some(a) = self
                .allocations
                .iter()
                .find(|a| a.start <= cursor && cursor < a.start + a.length)
            else {
                return false;
            };
            cursor = end.min(a.start + a.length);
        }
        true
    }

    pub(super) fn release_range(
        &mut self,
        start: u64,
        length: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let end = start.checked_add(length).ok_or(Errno::Invalid)?;
        let a = self.allocations.len() as u64 + 1;
        let m = self.mappings.len() as u64 + 1;
        let capacity = self
            .limits
            .max_handles
            .checked_add(1)
            .ok_or(Failure::Limit("release metadata"))?;
        let map_capacity = self
            .mappings
            .len()
            .checked_mul(2)
            .ok_or(Failure::Limit("release metadata"))?
            .max(self.limits.max_handles);
        let scratch_limit = capacity
            .checked_mul(std::mem::size_of::<Allocation>())
            .and_then(|n| {
                map_capacity
                    .checked_mul(std::mem::size_of::<Mapping>() + 32)
                    .and_then(|v| v.checked_add(n))
            })
            .ok_or(Failure::Limit("release metadata"))?;
        budget.scratch(scratch_limit)?;
        budget.work(
            a.checked_mul(a)
                .and_then(|n| n.checked_add(m * 4))
                .and_then(|n| n.checked_add(scratch_limit as u64))
                .ok_or(Failure::Limit("release work"))?,
        )?;
        if !self.physical_coverage(start, end) {
            // The pinned unchecked wrapper discards this failure. Retain the
            // explicit unavailable no-op policy rather than reporting a free.
            return Err(Failure::Unsupported(
                "source unchecked-release no-op branch",
            ));
        }
        let mut allocations = Vec::new();
        allocations
            .try_reserve_exact(capacity)
            .map_err(|_| Failure::Allocation)?;
        let mut mappings = Vec::new();
        mappings
            .try_reserve_exact(map_capacity)
            .map_err(|_| Failure::Allocation)?;
        let mut scratch = allocations
            .capacity()
            .checked_mul(std::mem::size_of::<Allocation>())
            .and_then(|n| {
                mappings
                    .capacity()
                    .checked_mul(std::mem::size_of::<Mapping>())
                    .and_then(|v| v.checked_add(n))
            })
            .ok_or(Failure::Limit("release metadata"))?;
        budget.scratch(scratch)?;
        budget.record_scratch(scratch);
        for a in &self.allocations {
            let stop = a.start + a.length;
            if stop <= start || end <= a.start {
                allocations.push(Allocation {
                    start: a.start,
                    length: a.length,
                });
            } else {
                if a.start < start {
                    allocations.push(Allocation {
                        start: a.start,
                        length: start - a.start,
                    });
                }
                if end < stop {
                    allocations.push(Allocation {
                        start: end,
                        length: stop - end,
                    });
                }
            }
        }
        let mut unmapped = 0_u64;
        for m in &self.mappings {
            let stop = m.start + m.length;
            if let Some(p) = m.physical.filter(|p| *p < end && start < *p + m.length) {
                let first = start.max(p);
                let last = end.min(p + m.length);
                let left = m.start + first - p;
                let right = m.start + last - p;
                if m.internal {
                    return Err(Failure::Unsupported("runtime-owned kernel mapping"));
                }
                if left > m.start {
                    mappings.push(m.piece(m.start, left, m.protection, &mut scratch, budget)?);
                }
                if right < stop {
                    mappings.push(m.piece(right, stop, m.protection, &mut scratch, budget)?);
                }
                unmapped = unmapped
                    .checked_add(last - first)
                    .ok_or(Failure::Limit("release aliases"))?;
            } else {
                mappings.push(m.piece(m.start, stop, m.protection, &mut scratch, budget)?);
            }
        }
        let handles = budget.replacement_handles(
            self.allocations.len() + self.mappings.len(),
            allocations.len() + mappings.len(),
        )?;
        budget.work(unmapped / PAGE)?;
        budget.apply(
            guest,
            MemoryEffect::ReleaseDirect {
                physical: start,
                length,
            },
            &[],
        )?;
        // No allocation or fallible operation after backing publication. New
        // vectors retain capacity for every future admitted map/allocation push.
        self.allocations = allocations;
        self.mappings = mappings;
        budget.replaced_handles(handles);
        budget.unmapped(unmapped);
        Ok(0)
    }
}
