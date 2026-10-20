// SPDX-License-Identifier: GPL-2.0-or-later

//! Recoverable reservation failures for one write into existing mappings.
//! All backing is prepared privately before any page, counter or byte changes.
//! This does not make mapping allocations or host process OOM recoverable.

use super::{GuestMemory, MemoryError, PAGE_SIZE, Permissions, page_base};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FallibleWriteError {
    Memory(MemoryError),
    Allocation,
}

impl From<MemoryError> for FallibleWriteError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

struct WritePlan {
    end: u64,
    missing: usize,
    resident_after: u64,
}

impl GuestMemory {
    /// Check a complete readable span without allocating or changing memory.
    /// As with `read`, an empty span never validates its otherwise unused address.
    pub(crate) fn preflight_read(&self, address: u64, size: u64) -> Result<(), MemoryError> {
        if size == 0 {
            return Ok(());
        }
        self.check(address, size, Some(Permissions::READ))
    }

    /// Check the complete writable span and logical resident-page budget.
    /// No backing is allocated; an empty span is an address-independent no-op.
    pub(crate) fn preflight_write(&self, address: u64, size: u64) -> Result<(), MemoryError> {
        self.write_plan(address, size).map(|_| ())
    }

    /// Write once after all missing backing has been reserved fallibly.
    /// Every returned error leaves bytes, page residency, permissions and
    /// counters unchanged. Allocator/process aborts are outside this guarantee.
    pub(crate) fn write_fallible(
        &mut self,
        address: u64,
        bytes: &[u8],
    ) -> Result<(), FallibleWriteError> {
        self.write_with_reservation_gate(address, bytes, || Ok(()))
    }

    fn write_plan(&self, address: u64, size: u64) -> Result<WritePlan, MemoryError> {
        if size == 0 {
            return Ok(WritePlan {
                end: address,
                missing: 0,
                resident_after: self.resident_bytes,
            });
        }
        self.check(address, size, Some(Permissions::WRITE))?;
        let end = address.checked_add(size).ok_or(MemoryError::InvalidRange)?;
        let missing = self
            .pages
            .range(page_base(address)..end)
            .filter(|(_, page)| page.data.is_none())
            .count();
        let needed = u64::try_from(missing)
            .ok()
            .and_then(|count| count.checked_mul(PAGE_SIZE))
            .ok_or(MemoryError::BudgetExceeded)?;
        let resident_after = self
            .resident_bytes
            .checked_add(needed)
            .filter(|&total| total <= self.resident_limit)
            .ok_or(MemoryError::BudgetExceeded)?;
        Ok(WritePlan {
            end,
            missing,
            resident_after,
        })
    }

    // The gate is private and per call: tests may reject the Nth reservation.
    // Production always permits the attempt, which still uses try_reserve_exact.
    fn write_with_reservation_gate(
        &mut self,
        address: u64,
        bytes: &[u8],
        mut before_reserve: impl FnMut() -> Result<(), FallibleWriteError>,
    ) -> Result<(), FallibleWriteError> {
        let length = u64::try_from(bytes.len()).map_err(|_| MemoryError::InvalidRange)?;
        let plan = self.write_plan(address, length)?;
        if bytes.is_empty() {
            return Ok(());
        }

        // Each descriptor owns one prepared buffer for an existing lazy page.
        // The logical resident budget bounds page payload requests; temporary
        // descriptor capacity and allocator overhead are not resident bytes.
        let mut pending = Vec::new();
        reserve_exact(&mut pending, plan.missing, &mut before_reserve)?;
        for (&base, page) in self.pages.range(page_base(address)..plan.end) {
            if page.data.is_none() {
                let mut data = Vec::new();
                reserve_exact(&mut data, PAGE_SIZE as usize, &mut before_reserve)?;
                // Both resize and push remain within successful reservations.
                data.resize(PAGE_SIZE as usize, 0);
                pending.push((base, data));
            }
        }

        // Commit: no insertions, reservations, callbacks or recoverable errors.
        // The exclusive borrow keeps every preflighted mapping unchanged.
        for (base, data) in pending {
            self.pages
                .get_mut(&base)
                .expect("preflight checked mapping")
                .data = Some(data);
        }
        self.resident_bytes = plan.resident_after;
        let mut copied = 0;
        while copied < bytes.len() {
            let cursor = address + copied as u64;
            let offset = (cursor - page_base(cursor)) as usize;
            let count = (PAGE_SIZE as usize - offset).min(bytes.len() - copied);
            let data = self
                .pages
                .get_mut(&page_base(cursor))
                .expect("preflight checked mapping")
                .data
                .as_mut()
                .expect("all backing prepared before commit");
            data[offset..offset + count].copy_from_slice(&bytes[copied..copied + count]);
            copied += count;
        }
        Ok(())
    }
}

fn reserve_exact<T>(
    buffer: &mut Vec<T>,
    additional: usize,
    before_reserve: &mut impl FnMut() -> Result<(), FallibleWriteError>,
) -> Result<(), FallibleWriteError> {
    if additional != 0 {
        before_reserve()?;
        buffer
            .try_reserve_exact(additional)
            .map_err(|_| FallibleWriteError::Allocation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
