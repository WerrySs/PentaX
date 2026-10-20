// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked source-scoped controller entry contracts.
// Adapted from pinned controller.cpp/libPad.cpp; see NOTICE.

//! Source entry/identity contracts only. No device connection, input snapshot,
//! motion sensor, vibration or host controller handle is invented.
use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service};

const INVALID_ARGUMENT: u64 = 0x8092_0001;
const INVALID_HANDLE: u64 = 0x8092_0003;
const NO_HANDLE: u64 = 0x8092_0008;

pub(crate) fn invoke(
    service: Service,
    args: CallArgs,
    configured_user: i32,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    let [first, second, third, ..] = args.0;
    match service {
        // The pinned source does no initialization or teardown here. These do
        // not imply a connected device, nor create/remove an OS capability.
        Service::PadInit | Service::PadClose => Ok(0),
        Service::PadOpen | Service::PadGetHandle => {
            let user = first as u32 as i32;
            let port = second as u32 as i32;
            let index = third as u32 as i32;
            let personal = user == configured_user && matches!(port, 0 | 2);
            let system_remote = user == 0xff && port == 16;
            if index == 0 && (personal || system_remote) {
                Ok(1)
            } else {
                Ok(if service == Service::PadOpen {
                    INVALID_ARGUMENT
                } else {
                    NO_HANDLE
                })
            }
        }
        Service::PadIsRemote => {
            if first as u32 as i32 != 1 {
                return Ok(INVALID_HANDLE);
            }
            if second == 0 {
                return Ok(INVALID_ARGUMENT);
            }
            // Exact source bool output. All unrelated caller bytes survive.
            budget.write(
                guest,
                &[GuestWrite {
                    address: second,
                    bytes: &[0],
                }],
            )?;
            Ok(0)
        }
        _ => Err(Failure::Unsupported("controller entry service")),
    }
}
