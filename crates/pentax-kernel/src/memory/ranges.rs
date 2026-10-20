// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX bounded metadata staging for the pinned range operations.

//! Prepare complete split/coalesced metadata before any backing mutation.
//! A mapping identity survives splitting; separate allocations do not merge.

use super::*;

impl MemoryState {
    pub(super) fn revised_mappings(
        &self,
        address: u64,
        length: u64,
        protection: Option<u32>,
        budget: &mut Budget,
    ) -> Result<(Vec<Mapping>, usize), Failure> {
        self.revised_with_imports(address, length, protection, &[], 0, budget)
    }

    pub(super) fn revised_with_imports(
        &self,
        address: u64,
        length: u64,
        protection: Option<u32>,
        imported: &[Mapping],
        import_scratch: usize,
        budget: &mut Budget,
    ) -> Result<(Vec<Mapping>, usize), Failure> {
        let end = address
            .checked_add(length)
            .ok_or(Failure::Limit("mapping range"))?;
        // A contiguous change creates at most two additional boundaries.
        // Keep at least max_handles capacity so a future admitted map's push
        // remains infallible AFTER its actual backing/output publication.
        let capacity = self
            .limits
            .max_handles
            .checked_add(2)
            .ok_or(Failure::Limit("mapping metadata"))?;
        let reserved = capacity
            .checked_mul(std::mem::size_of::<Mapping>() + 32)
            .and_then(|n| n.checked_add(import_scratch))
            .ok_or(Failure::Limit("mapping metadata"))?;
        budget.scratch(reserved)?;
        let visits = ((self.mappings.len() + imported.len()) as u64)
            .checked_add(1)
            .and_then(|n| n.checked_mul(n))
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(reserved as u64))
            .ok_or(Failure::Limit("mapping range work"))?;
        budget.work(visits)?;

        // Check the WHOLE range, including late holes/internal pages, before
        // staging or touching the actual backing. Existing order is irrelevant.
        let mut cursor = address;
        while cursor < end {
            let mapping = self
                .mappings
                .iter()
                .chain(imported)
                .find(|m| m.start <= cursor && cursor < m.start + m.length)
                .ok_or(if protection.is_none() {
                    Failure::Errno(Errno::Access)
                } else {
                    Failure::Unsupported("source-aborting unmapped protection range")
                })?;
            if mapping.internal {
                return Err(Failure::Unsupported("runtime-owned kernel mapping"));
            }
            if mapping.physical.is_some() && protection.is_some_and(|p| p & 4 != 0) {
                return Err(Errno::Access.into());
            }
            cursor = end.min(mapping.start + mapping.length);
        }

        let mut revised = Vec::new();
        revised
            .try_reserve_exact(capacity)
            .map_err(|_| Failure::Allocation)?;
        let mut scratch = revised
            .capacity()
            .checked_mul(std::mem::size_of::<Mapping>())
            .and_then(|n| n.checked_add(import_scratch))
            .ok_or(Failure::Limit("mapping metadata"))?;
        budget.scratch(scratch)?;
        budget.record_scratch(scratch);
        for mapping in self.mappings.iter().chain(imported) {
            let map_end = mapping.start + mapping.length;
            if end <= mapping.start || map_end <= address {
                revised.push(mapping.piece(
                    mapping.start,
                    map_end,
                    mapping.protection,
                    &mut scratch,
                    budget,
                )?);
                continue;
            }
            if mapping.start < address {
                revised.push(mapping.piece(
                    mapping.start,
                    address,
                    mapping.protection,
                    &mut scratch,
                    budget,
                )?);
            }
            if let Some(protection) = protection {
                revised.push(mapping.piece(
                    mapping.start.max(address),
                    map_end.min(end),
                    protection,
                    &mut scratch,
                    budget,
                )?);
            }
            if end < map_end {
                revised.push(mapping.piece(
                    end,
                    map_end,
                    mapping.protection,
                    &mut scratch,
                    budget,
                )?);
            }
        }
        debug_assert!(revised.len() <= self.mappings.len() + imported.len() + 2);
        revised.sort_unstable_by_key(|m| m.start);
        let mut used = 0;
        for index in 0..revised.len() {
            if used != 0 && revised[used - 1].joins(&revised[index]) {
                revised[used - 1].length += revised[index].length;
            } else {
                revised.swap(used, index);
                used += 1;
            }
        }
        revised.truncate(used);
        let handles = budget.replacement_handles(self.mappings.len(), revised.len())?;
        Ok((revised, handles))
    }

    /// Stage only previously untracked input-image portions of this request.
    /// Existing runtime-owned mappings are still refused by the range revision.
    /// No metadata, counters or identities publish before the backing effect.
    pub(super) fn protection_imports(
        &self,
        address: u64,
        length: u64,
        protection: u32,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(Vec<Mapping>, u64, u64), Failure> {
        let end = address
            .checked_add(length)
            .ok_or(crate::GuestError::AddressOverflow)?;
        let mut imported = Vec::new();
        let mut cursor = address;
        let mut bytes = 0_u64;
        let mut next_identity = self.next_mapping;
        while cursor < end {
            budget.work(self.mappings.len() as u64 + 1)?;
            if let Some(mapping) = self
                .mappings
                .iter()
                .find(|m| m.start <= cursor && cursor < m.start + m.length)
            {
                cursor = end.min(mapping.start + mapping.length);
                continue;
            }
            let stop = self
                .mappings
                .iter()
                .filter(|m| m.start > cursor)
                .map(|m| m.start)
                .min()
                .unwrap_or(end)
                .min(end);
            let size = stop - cursor;
            budget.work(guest.image_memory_work(size)?)?;
            if !guest.preflight_image_memory(cursor, size)? {
                return Err(Failure::Unsupported(
                    "source-aborting unmapped protection range",
                ));
            }
            if self.mappings.len() + imported.len() >= self.limits.max_handles {
                return Err(Failure::Limit("mapping metadata"));
            }
            // Include the complete range-revision vector in the simultaneous
            // scratch allowance; per-record names are empty for these imports.
            let revision_scratch = (self.limits.max_handles + 2)
                .checked_mul(std::mem::size_of::<Mapping>() + 32)
                .ok_or(Failure::Limit("mapping metadata"))?;
            budget.scratch(
                revision_scratch
                    .checked_add((imported.len() + 1) * std::mem::size_of::<Mapping>())
                    .ok_or(Failure::Limit("mapping metadata"))?,
            )?;
            imported
                .try_reserve_exact(1)
                .map_err(|_| Failure::Allocation)?;
            let scratch = imported
                .capacity()
                .checked_mul(std::mem::size_of::<Mapping>())
                .and_then(|n| n.checked_add(revision_scratch))
                .ok_or(Failure::Limit("mapping metadata"))?;
            budget.scratch(scratch)?;
            budget.record_scratch(scratch);
            let identity = next_identity;
            next_identity = identity
                .checked_add(1)
                .ok_or(Failure::Limit("mapping identities"))?;
            imported.push(Mapping {
                identity,
                start: cursor,
                length: size,
                protection,
                physical: None,
                internal: false,
                pthread: None,
                _name: Vec::new(),
            });
            bytes = bytes
                .checked_add(size)
                .ok_or(Failure::Limit("mapped bytes"))?;
            budget.can_map(bytes)?;
            cursor = stop;
        }
        Ok((imported, bytes, next_identity))
    }
}

impl Mapping {
    pub(super) fn piece(
        &self,
        start: u64,
        end: u64,
        protection: u32,
        scratch: &mut usize,
        budget: &mut Budget,
    ) -> Result<Self, Failure> {
        let length = end
            .checked_sub(start)
            .filter(|n| *n != 0)
            .ok_or(Failure::Limit("mapping metadata"))?;
        let offset = start
            .checked_sub(self.start)
            .ok_or(Failure::Limit("mapping metadata"))?;
        let physical = self
            .physical
            .map(|base| {
                base.checked_add(offset)
                    .ok_or(Failure::Limit("physical mapping offset"))
            })
            .transpose()?;
        let mut name = Vec::new();
        name.try_reserve_exact(self._name.len())
            .map_err(|_| Failure::Allocation)?;
        *scratch = scratch
            .checked_add(name.capacity())
            .ok_or(Failure::Limit("mapping metadata"))?;
        budget.scratch(*scratch)?;
        budget.record_scratch(*scratch);
        name.extend_from_slice(&self._name);
        Ok(Self {
            identity: self.identity,
            start,
            length,
            protection,
            physical,
            internal: self.internal,
            pthread: self.pthread,
            _name: name,
        })
    }
    fn joins(&self, right: &Self) -> bool {
        self.identity == right.identity
            && self.start + self.length == right.start
            && self.protection == right.protection
            && self.internal == right.internal
            && self.pthread == right.pthread
            && self._name == right._name
            && match (self.physical, right.physical) {
                (None, None) => true,
                (Some(left), Some(right)) => left.checked_add(self.length) == Some(right),
                _ => false,
            }
    }
}
