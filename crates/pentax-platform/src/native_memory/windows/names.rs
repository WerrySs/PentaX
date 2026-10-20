// SPDX-License-Identifier: GPL-2.0-only
//! Byte-granular semantic names bound to actual mapping lifetimes. Backing
//! mappings/pages are never split or modified by a name change.

use super::*;

#[derive(Clone, Copy)]
pub(super) struct NameEntry {
    start: u64,
    end: u64,
    lifetime: u64,
    value: NativeRangeName,
}
impl std::fmt::Debug for NameEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameEntry")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("lifetime", &self.lifetime)
            .field("name_bytes", &self.value.length)
            .finish()
    }
}

fn value(name: &[u8]) -> Result<NativeRangeName, NativeMemoryError> {
    if name.len() >= 32 || name.contains(&0) {
        return Err(NativeMemoryError::InvalidRange);
    }
    let mut bytes = [0; 32];
    bytes[..name.len()].copy_from_slice(name);
    Ok(NativeRangeName {
        range: NativeRange {
            address: 0,
            size: 0,
        },
        bytes,
        length: name.len() as u8,
    })
}

fn admit(
    owner: &mut NativeAddressSpace,
    limits: &NativeNameLimits,
    work: u64,
) -> Result<(), NativeMemoryError> {
    owner.live()?;
    if limits.max_records > 65_536 || owner.names.len() > limits.max_records {
        return Err(NativeMemoryError::BudgetExceeded);
    }
    let next = owner
        .name_work
        .checked_add(work)
        .filter(|total| *total <= limits.max_work)
        .ok_or(NativeMemoryError::BudgetExceeded)?;
    owner.name_work = next;
    Ok(())
}

fn pages(owner: &NativeAddressSpace, start: u64, end: u64) -> std::ops::Range<usize> {
    let first = start & !(NATIVE_GUEST_PAGE - 1);
    owner.pages.partition_point(|page| page.address < first)
        ..owner.pages.partition_point(|page| page.address < end)
}

impl PausedNativeMemory<'_> {
    /// Rename only actual mapped intersections, including source/stack pages
    /// with no read/write permissions. Empty ranges and holes have no effect.
    /// Source-style endpoints saturate rather than wrapping. Allocation and
    /// bounded staging precede one metadata publication; no host call occurs.
    pub fn set_virtual_range_name(
        &mut self,
        address: u64,
        size: u64,
        name: &[u8],
        limits: &NativeNameLimits,
    ) -> Result<(), NativeMemoryError> {
        self.owner.live()?;
        let name = value(name)?;
        if size == 0 {
            return Ok(());
        }
        let end = address.saturating_add(size);
        let search = (usize::BITS - self.owner.pages.len().max(1).leading_zeros()) as u64 + 1;
        admit(
            self.owner,
            limits,
            (self.owner.names.len() as u64 + 1) * (2 * search + 4),
        )?;
        let selected = pages(self.owner, address, end);
        let mut visits = selected.len() as u64 * 2 + 1;
        for entry in &self.owner.names {
            visits = visits
                .checked_add(pages(self.owner, entry.start, entry.end).len() as u64 + 2)
                .ok_or(NativeMemoryError::BudgetExceeded)?;
        }
        // Charge every possible liveness/page visit before scanning payload-
        // independent metadata. Name work remains cumulative even on refusal.
        admit(self.owner, limits, visits)?;
        if selected.is_empty() {
            return Ok(());
        }
        let mut groups = 0_usize;
        let mut last = None;
        for page in &self.owner.pages[selected.clone()] {
            if last != Some((page.address, page.lifetime)) {
                groups += 1;
            }
            last = Some((page.address + NATIVE_GUEST_PAGE, page.lifetime));
        }
        let capacity = self
            .owner
            .names
            .len()
            .checked_add(groups)
            .and_then(|n| n.checked_add(2))
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let scratch = capacity
            .checked_mul(std::mem::size_of::<NameEntry>())
            .filter(|bytes| *bytes <= limits.max_scratch_bytes)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        // Bound all comparisons of the subsequent sort as well as record moves.
        let sort_work = (capacity as u64)
            .checked_mul((usize::BITS - capacity.max(1).leading_zeros()) as u64 + 4)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        admit(self.owner, limits, sort_work + scratch as u64)?;
        let mut revised = Vec::new();
        revised
            .try_reserve_exact(capacity)
            .map_err(|_| NativeMemoryError::Allocation)?;
        for entry in &self.owner.names {
            let live = self.owner.pages[pages(self.owner, entry.start, entry.end)]
                .iter()
                .any(|page| page.lifetime == entry.lifetime);
            if !live {
                continue;
            }
            if entry.end <= address || end <= entry.start {
                revised.push(*entry);
            } else {
                if entry.start < address {
                    revised.push(NameEntry {
                        end: address,
                        ..*entry
                    });
                }
                if end < entry.end {
                    revised.push(NameEntry {
                        start: end,
                        ..*entry
                    });
                }
            }
        }
        let mut group: Option<NameEntry> = None;
        for page in &self.owner.pages[selected] {
            let start = address.max(page.address);
            let stop = end.min(page.address + NATIVE_GUEST_PAGE);
            if let Some(previous) = group
                .as_mut()
                .filter(|old| old.end == start && old.lifetime == page.lifetime)
            {
                previous.end = stop;
            } else {
                if let Some(previous) = group.take() {
                    revised.push(previous);
                }
                group = Some(NameEntry {
                    start,
                    end: stop,
                    lifetime: page.lifetime,
                    value: name,
                });
            }
        }
        if let Some(previous) = group {
            revised.push(previous);
        }
        revised.sort_unstable_by_key(|entry| entry.start);
        let mut used = 0;
        for index in 0..revised.len() {
            if used != 0
                && revised[used - 1].end == revised[index].start
                && revised[used - 1].lifetime == revised[index].lifetime
                && revised[used - 1].value.name() == revised[index].value.name()
            {
                revised[used - 1].end = revised[index].end;
            } else {
                revised.swap(used, index);
                used += 1;
            }
        }
        revised.truncate(used);
        if revised.len() > limits.max_records {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        self.owner.names = revised;
        Ok(())
    }

    /// Resolve a semantic name within a caller-retained base mapping. The
    /// caller supplies its original default name and bounds, not access rights.
    /// Only the actual page lifetime at address can select stored metadata;
    /// unmap/remap and aliases never inherit stale labels by address alone.
    pub fn virtual_range_name(
        &mut self,
        address: u64,
        bounds: NativeRange,
        default: &[u8],
        limits: &NativeNameLimits,
    ) -> Result<NativeRangeName, NativeMemoryError> {
        self.owner.live()?;
        let default = value(default)?;
        let end = bounds
            .address
            .checked_add(bounds.size)
            .ok_or(NativeMemoryError::InvalidRange)?;
        if bounds.address > address || address >= end {
            return Err(NativeMemoryError::InvalidRange);
        }
        let search = (usize::BITS - self.owner.pages.len().max(1).leading_zeros()) as u64 + 1;
        admit(
            self.owner,
            limits,
            self.owner.names.len() as u64 * 2 + search,
        )?;
        let page = address & !(NATIVE_GUEST_PAGE - 1);
        let index = self
            .owner
            .pages
            .binary_search_by_key(&page, |p| p.address)
            .map_err(|_| NativeMemoryError::Unmapped(address))?;
        let lifetime = self.owner.pages[index].lifetime;
        let selected = self.owner.names.iter().find(|entry| {
            entry.lifetime == lifetime && entry.start <= address && address < entry.end
        });
        let mut result = selected.map_or(default, |entry| entry.value);
        let mut start = bounds.address;
        let mut stop = end;
        for entry in self
            .owner
            .names
            .iter()
            .filter(|entry| entry.lifetime == lifetime)
        {
            if entry.value.name() == default.name() {
                continue;
            }
            if entry.start <= address && address < entry.end {
                start = start.max(entry.start);
                stop = stop.min(entry.end);
                break;
            }
            if entry.end <= address {
                start = start.max(entry.end);
            }
            if address < entry.start {
                stop = stop.min(entry.start);
            }
        }
        result.range = NativeRange {
            address: start,
            size: stop - start,
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
