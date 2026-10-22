// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owner-bound module unwind metadata; see NOTICE.

use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, budget::Budget};

/// Exact source ModuleInfoForUnwind ABI, encoded without native pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleUnwindInfo {
    pub name: [u8; 256],
    pub eh_frame_header: u64,
    pub eh_frame: u64,
    pub eh_frame_size: u64,
    pub image_address: u64,
    pub image_size: u64,
}

impl ModuleUnwindInfo {
    fn encode(&self) -> [u8; 304] {
        let mut bytes = [0; 304];
        bytes[..8].copy_from_slice(&304u64.to_le_bytes());
        bytes[8..264].copy_from_slice(&self.name);
        for (index, value) in [
            self.eh_frame_header,
            self.eh_frame,
            self.eh_frame_size,
            self.image_address,
            self.image_size,
        ]
        .into_iter()
        .enumerate()
        {
            bytes[264 + index * 8..272 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

pub(crate) fn invoke(
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    let output = args.0[2];
    // A closed allowance covers the ABI copies, adapter/header chunk and
    // loader metadata simultaneously, without allocating from guest lengths.
    budget.scratch(4096)?;
    budget.record_scratch(4096);
    if args.0[1] as i32 >= 3 {
        if output != 0 {
            budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &[0; 304],
                }],
            )?;
        }
        return Err(Errno::Invalid.into());
    }
    if output == 0 {
        return Err(Errno::Fault.into());
    }
    if budget.u64(guest, output)? < 304 {
        return Err(Errno::Invalid.into());
    }
    let info = guest
        .module_unwind_info(args.0[0])?
        .ok_or(Errno::NoSuchResource)?;
    let bytes = info.encode();
    budget.write(
        guest,
        &[GuestWrite {
            address: output,
            bytes: &bytes,
        }],
    )?;
    Ok(0)
}

#[cfg(test)]
mod tests;
