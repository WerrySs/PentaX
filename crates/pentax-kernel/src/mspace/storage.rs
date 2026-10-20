// SPDX-License-Identifier: GPL-2.0-only
// Original bounded fallible storage for the PentaX Mspace adaptation.

use super::*;

pub(super) fn scratch(held: usize, extra: usize, budget: &mut Budget) -> Result<usize, Failure> {
    let total = held
        .checked_add(extra)
        .ok_or(Failure::Limit("Mspace scratch"))?;
    budget.scratch(total)?;
    budget.record_scratch(total);
    Ok(total)
}

pub(super) fn reserve<T>(
    count: usize,
    held: usize,
    budget: &mut Budget,
) -> Result<Vec<T>, Failure> {
    let bytes = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("Mspace metadata"))?;
    scratch(held, bytes, budget)?;
    budget.work(
        u64::try_from(bytes)
            .ok()
            .and_then(|bytes| bytes.checked_add(1))
            .ok_or(Failure::Limit("Mspace storage work"))?,
    )?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| Failure::Allocation)?;
    let actual = values
        .capacity()
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("Mspace metadata"))?;
    scratch(held, actual, budget)?;
    Ok(values)
}

pub(super) fn buffer(length: u64, held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    let length = usize::try_from(length).map_err(|_| Failure::Limit("Mspace transfer bytes"))?;
    if length > budget.limits.max_io_bytes {
        return Err(Failure::Limit("Mspace transfer bytes"));
    }
    let mut bytes = reserve(length, held, budget)?;
    bytes.resize(length, 0);
    Ok(bytes)
}

pub(super) fn metadata(current: usize, staged: usize) -> Result<(), Failure> {
    if current
        .checked_add(staged)
        .is_none_or(|bytes| bytes > MAX_METADATA_BYTES)
    {
        Err(Failure::Limit("Mspace metadata"))
    } else {
        Ok(())
    }
}

pub(super) fn prove(
    guest: &impl GuestAccess,
    address: u64,
    length: u64,
    budget: &mut Budget,
) -> Result<(), Failure> {
    address
        .checked_add(length)
        .ok_or(GuestError::AddressOverflow)?;
    // Alignment-independent upper bound for all three RW/NX metadata walks.
    let pages = length
        .checked_add(2 * PAGE - 2)
        .and_then(|bytes| bytes.checked_div(PAGE))
        .ok_or(Failure::Limit("Mspace backing work"))?;
    let work = pages
        .checked_mul(3)
        .and_then(|work| work.checked_add(128))
        .ok_or(Failure::Limit("Mspace backing work"))?;
    budget.work(work)?;
    guest.preflight_heap_arena(address, length)?;
    Ok(())
}

pub(super) fn chunk_bytes(capacity: usize) -> Result<usize, Failure> {
    capacity
        .checked_mul(core::mem::size_of::<Chunk>())
        .ok_or(Failure::Limit("Mspace metadata"))
}
