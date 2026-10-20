// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/libs/libPad.cpp, blob ab9a861fed955b3a30e12275c9b60f312c6478f3.
// See NOTICE and upstream provenance for source profile and attribution.
//! Stateless disconnected raw Mouse/Keyboard profile. The source has neither
//! device discovery nor host capture; returned 1 is not an owned live resource.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service};

const MOUSE_INVALID_ARG: u64 = 0x80df_0001;
const MOUSE_INVALID_HANDLE: u64 = 0x80df_0003;
const KEYBOARD_INVALID_ARG: u64 = 0x80da_0001;
const KEYBOARD_INVALID_HANDLE: u64 = 0x80da_0003;
const MOUSE_RECORD: usize = 40;
const KEYBOARD_RECORD: usize = 96;
const CHARACTER_RECORD: usize = 20;
// The source's largest finite keyboard batch needs no heap allocation. This
// immutable byte table is shared; no old guest payload is read or replayed.
static DISCONNECTED: [u8; 16 * KEYBOARD_RECORD] = [0; 16 * KEYBOARD_RECORD];

fn clear(
    address: u64,
    length: usize,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    if length > budget.limits.max_io_bytes {
        return Err(Failure::Limit("raw input output bytes"));
    }
    if length <= DISCONNECTED.len() {
        budget.write(
            guest,
            &[GuestWrite {
                address,
                bytes: &DISCONNECTED[..length],
            }],
        )?;
    } else {
        let zeros = budget.buffer_held(length, 0)?;
        budget.write(
            guest,
            &[GuestWrite {
                address,
                bytes: &zeros,
            }],
        )?;
    }
    // Records are cleared as in the pinned source, but no sample was received.
    Ok(0)
}

pub(crate) fn invoke(
    service: Service,
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    use Service::*;
    let a = args.0;
    budget.work(1)?;
    let mouse = matches!(service, MouseInit | MouseOpen | MouseClose | MouseRead);
    let invalid_arg = if mouse {
        MOUSE_INVALID_ARG
    } else {
        KEYBOARD_INVALID_ARG
    };
    let invalid_handle = if mouse {
        MOUSE_INVALID_HANDLE
    } else {
        KEYBOARD_INVALID_HANDLE
    };
    match service {
        MouseInit | KeyboardInit => Ok(0),
        MouseOpen | KeyboardOpen => {
            // User ID and param are opaque words. Open neither reads param nor
            // remembers Init/Close, and accepts both source device indices.
            let kind = a[1] as u32 as i32;
            let index = a[2] as u32 as i32;
            Ok(if kind == 0 && (0..2).contains(&index) {
                1
            } else {
                invalid_arg
            })
        }
        MouseClose | KeyboardClose => Ok(if a[0] as u32 as i32 > 0 {
            0
        } else {
            invalid_handle
        }),
        MouseRead | KeyboardRead => {
            if a[0] as u32 as i32 <= 0 {
                return Ok(invalid_handle);
            }
            let count = a[2] as u32 as i32;
            if a[1] == 0 || count <= 0 || (!mouse && count > 16) {
                return Ok(invalid_arg);
            }
            let stride = if mouse { MOUSE_RECORD } else { KEYBOARD_RECORD };
            let length = (count as usize)
                .checked_mul(stride)
                .ok_or(Failure::Limit("raw input record bytes"))?;
            clear(a[1], length, guest, budget)
        }
        KeyboardReadState => {
            if a[0] as u32 as i32 <= 0 {
                return Ok(invalid_handle);
            }
            if a[1] == 0 {
                return Ok(invalid_arg);
            }
            clear(a[1], KEYBOARD_RECORD, guest, budget)
        }
        KeyboardGetKey2Char => {
            if a[0] as u32 as i32 <= 0 {
                return Ok(invalid_handle);
            }
            if a[5] == 0 || !matches!(a[1] as u32 as i32, 0 | 1) {
                return Ok(invalid_arg);
            }
            // LED/modifiers/keycode are ignored by this disconnected source
            // profile. Zero output means unprocessed; no translation is claimed.
            clear(a[5], CHARACTER_RECORD, guest, budget)
        }
        _ => Err(Failure::Unsupported("raw input service")),
    }
}

#[cfg(test)]
mod tests;
