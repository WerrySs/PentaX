// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from public KytyPS5 7b7f9a70 libConvertKeycode.cpp; see NOTICE.
//! Source-defined local keyboard-type query with checked four-byte output.

use crate::{CallArgs, Failure, GuestAccess, GuestWrite, budget::Budget};

pub(crate) fn invoke(
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    budget.work(1)?;
    let output = args.0[1];
    // Preserve source validation order and signed low-32-bit user identity.
    if output == 0 {
        return Ok(0x80bc_0031);
    }
    if (args.0[0] as i32) < 0 {
        return Ok(0x80bc_0010);
    }
    budget.write(
        guest,
        &[GuestWrite {
            address: output,
            bytes: &0u32.to_le_bytes(),
        }],
    )?;
    Ok(0)
}
