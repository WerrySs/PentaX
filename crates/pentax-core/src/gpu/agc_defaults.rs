// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: checked guest register-default materialization.
// Pinned agc.cpp/agcRegisterDefaults.inc; see NOTICE.

mod data;
use pentax_kernel::Failure;

pub(super) const SIZE: usize = data::SIZE;
pub(super) const WORK: u64 = SIZE as u64 * 3 + data::POINTERS.len() as u64 * 16 + 64;

pub(super) fn descriptor(internal: bool, version: u64) -> u64 {
    let version = version as u32;
    data::DESCRIPTORS[usize::from(internal)][if version <= 13 { version as usize } else { 13 }]
        as u64
}

pub(super) fn initialize(base: u64, output: &mut [u8]) -> Result<(), Failure> {
    let invalid = Failure::Unsupported("register-default data invariant");
    if output.len() != SIZE
        || data::WORDS.len() * 8 != SIZE
        || base.checked_add(SIZE as u64).is_none()
    {
        return Err(invalid);
    }
    for (word, bytes) in data::WORDS.iter().zip(output.chunks_exact_mut(8)) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    for &at in data::POINTERS {
        let end = at.checked_add(8).ok_or(invalid)?;
        let slot = output.get_mut(at..end).ok_or(invalid)?;
        let offset = u64::from_le_bytes(slot.try_into().map_err(|_| invalid)?);
        if offset == 0 || offset >= SIZE as u64 {
            return Err(invalid);
        }
        slot.copy_from_slice(&base.checked_add(offset).ok_or(invalid)?.to_le_bytes());
    }
    Ok(())
}
