// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from pinned KytyPS5-Performance libNet.cpp; see NOTICE/provenance.
// Modified 2026 for PentaX: checked atomic guest output and bounded staging.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};

const INVALID_ARGUMENT: u64 = 0x8055_3804;
const INTENT_NOT_FOUND: u64 = 0x8055_3806;
const VALUE_NOT_FOUND: u64 = 0x8055_3807;
const PREFIX_BYTES: usize = 4 + 33;
const DATA_BYTES: usize = 16_392;
const DATA_OFFSET: u64 = 308;

pub(crate) fn invoke(
    service: Service,
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    budget.work(1)?;
    let a = args.0;
    match service {
        // The pinned offline implementation has no initialization state or
        // semantic parameter read. Its pointer logging is intentionally absent.
        Service::GameIntentInitialize | Service::GameIntentTerminate => Ok(0),
        Service::GameIntentReceiveIntent if a[0] == 0 => Ok(INVALID_ARGUMENT),
        Service::GameIntentReceiveIntent => {
            let prefix = a[0].checked_add(8).ok_or(GuestError::AddressOverflow)?;
            let data = a[0]
                .checked_add(DATA_OFFSET)
                .ok_or(GuestError::AddressOverflow)?;
            data.checked_add(DATA_BYTES as u64)
                .ok_or(GuestError::AddressOverflow)?;
            // One zeroed allocation contains both exact source output spans;
            // no input size, reserved byte or arbitrary guest payload is read.
            let mut output = budget.buffer(PREFIX_BYTES + DATA_BYTES)?;
            budget.scratch(output.capacity())?;
            budget.record_scratch(output.capacity());
            output[..4].copy_from_slice(&u32::MAX.to_le_bytes());
            budget.write(
                guest,
                &[
                    GuestWrite {
                        address: prefix,
                        bytes: &output[..PREFIX_BYTES],
                    },
                    GuestWrite {
                        address: data,
                        bytes: &output[PREFIX_BYTES..],
                    },
                ],
            )?;
            Ok(INTENT_NOT_FOUND)
        }
        Service::GameIntentGetPropertyValueString => {
            if a[..4].contains(&0) {
                return Ok(INVALID_ARGUMENT);
            }
            // Data/key are opaque nonnull inputs in this source fallback.
            // Only one output byte is accessed, irrespective of buffer size.
            budget.write(
                guest,
                &[GuestWrite {
                    address: a[2],
                    bytes: &[0],
                }],
            )?;
            Ok(VALUE_NOT_FOUND)
        }
        _ => Err(Failure::Unsupported("Game Intent service")),
    }
}

#[cfg(test)]
mod tests;
