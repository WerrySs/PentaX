// SPDX-License-Identifier: GPL-2.0-only

//! Admit loader-owned pages for retained kernel protection tracking. No payload
//! reads, new storage, generated provider authority or host pointers are used.
use super::*;
use crate::memory::PAGE_SIZE;

fn source<'a, 'i>(
    guest: &'a KernelMemory<'_, '_, 'i>,
) -> Result<Option<&'a SymbolicLinkPlan<'i>>, Failure> {
    let Some(source) = guest.source else {
        return Ok(None);
    };
    if source.modules().len() > 128 || guest.supplied_modules > source.modules().len() {
        return Err(Failure::Limit("image protection metadata"));
    }
    if source.modules()[..guest.supplied_modules]
        .iter()
        .any(|module| module.runtime_plan().segments.len() > 256)
    {
        return Err(Failure::Limit("image protection metadata"));
    }
    Ok(Some(source))
}

pub(super) fn work(guest: &KernelMemory<'_, '_, '_>, length: u64) -> Result<u64, Failure> {
    let Some(source) = source(guest)? else {
        return Ok(1);
    };
    let visits = source.modules()[..guest.supplied_modules]
        .iter()
        .map(|module| module.runtime_plan().segments.len() as u64 + 1)
        .sum::<u64>()
        + 1;
    length
        .div_ceil(PAGE_SIZE)
        .checked_mul(visits + 2)
        .and_then(|n| n.checked_add(129))
        .ok_or(Failure::Limit("image protection work"))
}

pub(super) fn preflight(
    guest: &KernelMemory<'_, '_, '_>,
    address: u64,
    length: u64,
) -> Result<bool, Failure> {
    let Some(source) = source(guest)? else {
        return Ok(false);
    };
    if length == 0 || !address.is_multiple_of(PAGE_SIZE) || !length.is_multiple_of(PAGE_SIZE) {
        return Err(GuestError::AddressOverflow.into());
    }
    let end = address
        .checked_add(length)
        .ok_or(GuestError::AddressOverflow)?;
    for page in (address..end).step_by(PAGE_SIZE as usize) {
        let admitted = source.modules()[..guest.supplied_modules]
            .iter()
            .any(|module| {
                module.runtime_plan().segments.iter().any(|segment| {
                    if segment.mem_size == 0 {
                        return false;
                    }
                    let Some(start) = segment.vaddr.checked_add(module.load_bias()) else {
                        return false;
                    };
                    let Some(end) = start
                        .checked_add(segment.mem_size)
                        .and_then(|n| n.checked_add(PAGE_SIZE - 1))
                    else {
                        return false;
                    };
                    start & !(PAGE_SIZE - 1) <= page && page < end & !(PAGE_SIZE - 1)
                })
            });
        if !admitted {
            return Ok(false);
        }
    }
    guest
        .paused
        .preflight_mapped(address, length)
        .map_err(|e| Failure::Guest(error(e, false)))?;
    Ok(true)
}
