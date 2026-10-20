// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX bounded storage for public offline NpWebApi2 state.

use super::*;

/// A table replacement stays private until every guest effect has committed.
/// Its actual capacity and the staged strings share the scratch admission.
pub(super) fn growth<T>(
    table: &Vec<T>,
    held: usize,
    budget: &mut Budget,
) -> Result<Option<Vec<T>>, Failure> {
    if table.len() >= budget.limits.max_handles {
        return Err(Failure::Limit("NpWebApi2 records"));
    }
    let maximum = budget.limits.max_handles;
    if table.capacity() > maximum {
        return Err(Failure::Limit("NpWebApi2 record capacity"));
    }
    if table.len() < table.capacity() {
        budget.scratch(held)?;
        budget.record_scratch(held);
        return Ok(None);
    }
    let length = table.len() + 1;
    let bytes = length
        .checked_mul(core::mem::size_of::<T>())
        .and_then(|bytes| bytes.checked_add(held))
        .ok_or(Failure::Limit("scratch"))?;
    budget.scratch(bytes)?;
    budget.work(length as u64 + 1)?;
    let mut next = Vec::new();
    next.try_reserve_exact(length)
        .map_err(|_| Failure::Allocation)?;
    if next.capacity() > maximum {
        return Err(Failure::Limit("NpWebApi2 record capacity"));
    }
    let actual = next
        .capacity()
        .checked_mul(core::mem::size_of::<T>())
        .and_then(|bytes| bytes.checked_add(held))
        .ok_or(Failure::Limit("scratch"))?;
    budget.scratch(actual)?;
    budget.record_scratch(actual);
    Ok(Some(next))
}

/// Pre-admission guarantees both append paths cannot allocate or fail.
pub(super) fn insert<T>(table: &mut Vec<T>, growth: Option<Vec<T>>, value: T) {
    if let Some(mut next) = growth {
        next.append(table);
        next.push(value);
        *table = next;
    } else {
        table.push(value);
    }
}

pub(super) fn text(
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
    allowance: usize,
    held: usize,
    overlay: Option<(u64, u64)>,
) -> Result<Vec<u8>, Failure> {
    if address == 0 {
        return Ok(Vec::new());
    }
    let maximum = budget.limits.max_name_bytes;
    budget.scratch(maximum.checked_add(held).ok_or(Failure::Limit("scratch"))?)?;
    budget.work(
        u64::try_from(maximum)
            .ok()
            .and_then(|length| length.checked_add(1))
            .ok_or(Failure::Limit("work"))?,
    )?;
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(maximum)
        .map_err(|_| Failure::Allocation)?;
    let staging = scratch
        .capacity()
        .checked_add(held)
        .ok_or(Failure::Limit("scratch"))?;
    budget.scratch(staging)?;
    budget.record_scratch(staging);
    scratch.resize(maximum, 0);
    for index in 0..maximum {
        let at = address
            .checked_add(index as u64)
            .ok_or(GuestError::AddressOverflow)?;
        if let Some((destination, value)) = overlay {
            // The source publishes the request ID before constructing strings.
            // This query models exact virtual overlap and either handles or
            // refuses distinct physical aliases without changing guest bytes.
            budget.work(8 * 16 * 16)?;
            budget.precharge_read(1)?;
            scratch[index] = guest.read_after_u64_write_byte(at, destination, value)?;
        } else {
            budget.read(guest, at, &mut scratch[index..index + 1])?;
        }
        if scratch[index] != 0 {
            continue;
        }
        if index == 0 {
            return Ok(Vec::new());
        }
        if index > allowance {
            return Err(Failure::Limit("NpWebApi2 retained text bytes"));
        }
        budget.scratch(
            staging
                .checked_add(index)
                .ok_or(Failure::Limit("scratch"))?,
        )?;
        budget.work(index as u64 + 1)?;
        let mut value = Vec::new();
        value
            .try_reserve_exact(index)
            .map_err(|_| Failure::Allocation)?;
        if value.capacity() > allowance {
            return Err(Failure::Limit("NpWebApi2 retained text bytes"));
        }
        let actual = staging
            .checked_add(value.capacity())
            .ok_or(Failure::Limit("scratch"))?;
        budget.scratch(actual)?;
        budget.record_scratch(actual);
        value.extend_from_slice(&scratch[..index]);
        return Ok(value);
    }
    Err(Failure::Limit("name bytes"))
}
