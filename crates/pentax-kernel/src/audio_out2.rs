// SPDX-License-Identifier: GPL-2.0-only
// Adapted from public KytyPS5@f186b35706ddbc3faf5513f326e2335d224802f7,
// src/libs/libAudio2.cpp, blob91fb83671bec4e4c17a31a28476dbec9dadd43cc.
// Modified for PentaX: exact checked footprints and atomic guest outputs.
//! Stateless parameter/speaker helpers. No host device, context or DSP exists.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};

pub(crate) fn invoke(
    service: Service,
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    budget.work(1)?;
    match service {
        // Upstream only logs and returns OK, including repeated calls.
        Service::AudioOut2Initialize => Ok(0),
        Service::AudioOut2ContextResetParam => {
            if args.0[0] == 0 {
                return Err(Failure::Unsupported("null AudioOut2 context parameter"));
            }
            let mut bytes = [0; 64];
            for (index, value) in [256u32, 256, 0, 4, 512, 1].into_iter().enumerate() {
                bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
            }
            budget.write(
                guest,
                &[GuestWrite {
                    address: args.0[0],
                    bytes: &bytes,
                }],
            )?;
            Ok(0)
        }
        Service::AudioOut2ContextQueryMemory => {
            if args.0[0] == 0 {
                return Err(Failure::Unsupported("null AudioOut2 context parameter"));
            }
            if args.0[1] == 0 {
                return Err(Failure::Unsupported("null AudioOut2 memory output"));
            }
            let field = args.0[0]
                .checked_add(12)
                .ok_or(GuestError::AddressOverflow)?;
            // Only queue_depth is inspected; snapshot before the output so
            // ordinary aliases observe the original four input bytes.
            let depth = budget.u32(guest, field)?;
            let depth = if depth == 0 { 4 } else { depth };
            // Every u32 depth fits this source size_t expression in u64.
            let size = 0x1_0000u64 + u64::from(depth) * 0x590;
            budget.write(
                guest,
                &[GuestWrite {
                    address: args.0[1],
                    bytes: &size.to_le_bytes(),
                }],
            )?;
            Ok(0)
        }
        Service::AudioOut2GetSpeakerInfo => {
            if args.0[0] == 0 {
                return Err(Failure::Unsupported("null AudioOut2 speaker info"));
            }
            let mut bytes = [0; 80];
            bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
            bytes[16..18].copy_from_slice(&(-30i16).to_le_bytes());
            bytes[20..22].copy_from_slice(&30i16.to_le_bytes());
            // Source flags is ignored; all padding and other angles are zero.
            budget.write(
                guest,
                &[GuestWrite {
                    address: args.0[0],
                    bytes: &bytes,
                }],
            )?;
            Ok(0)
        }
        _ => Err(Failure::Unsupported("AudioOut2 helper service")),
    }
}

#[cfg(test)]
#[path = "audio_out2/tests.rs"]
mod tests;
