// SPDX-License-Identifier: GPL-2.0-only
// Adapted from AnyPS5 contributors, revision 80565c59424190cd0f660f8097945443a0017c42,
// core/libs/prx/libSceUserService/Export.cpp:173..179; see NOTICE.
//! The public WS1 getter's stateless four-byte local profile.

use crate::{CallArgs, Failure, GuestAccess, GuestWrite, budget::Budget};

pub(crate) fn invoke(
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    budget.work(1)?;
    // The signed int32 user ID is ignored by the source, independently of
    // initialization, configured identity and login-event delivery.
    let output = args.0[1];
    if output == 0 {
        return Ok(0x8096_0005);
    }
    budget.write(
        guest,
        &[GuestWrite {
            address: output,
            bytes: &0_u32.to_le_bytes(),
        }],
    )?;
    Ok(0)
}
