// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned configured timezone and atomic checked outputs.
// Adapted from pinned kernel/pthread.cpp and pthread.h; see NOTICE.

use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Kernel, Service};

impl Kernel {
    pub(crate) fn calendar(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        let east = i32::from(self.system.timezone_offset_minutes) * 60;
        let dst = if self.system.daylight_saving {
            i32::from(self.system.daylight_saving_offset_minutes) * 60
        } else {
            0
        };
        let delta = i64::from(east) + i64::from(dst);
        let mut timezone = [0; 8];
        timezone[..4]
            .copy_from_slice(&(-i32::from(self.system.timezone_offset_minutes)).to_le_bytes());
        timezone[4..].copy_from_slice(
            &(if self.system.daylight_saving {
                4_i32
            } else {
                0
            })
            .to_le_bytes(),
        );
        let local_or_utc;
        let mut timesec = [0; 16];
        let dst64 = (i64::from(dst) as u64).to_le_bytes();
        let dst32 = dst.to_le_bytes();
        let mut writes = [GuestWrite {
            address: 0,
            bytes: &[],
        }; 3];
        let mut count = 0;
        let mut output = |address, bytes| {
            if address != 0 {
                writes[count] = GuestWrite { address, bytes };
                count += 1;
            }
        };
        match service {
            Service::Gettimezone => {
                if args.0[0] == 0 {
                    return Err(Errno::Fault.into());
                }
                output(args.0[0], &timezone);
            }
            Service::ConvertUtcToLocaltime => {
                let [utc, local, state, daylight, ..] = args.0;
                if local != 0 {
                    let value = (utc as i64).checked_add(delta).ok_or(Errno::Invalid)?;
                    local_or_utc = value.to_le_bytes();
                    output(local, &local_or_utc);
                }
                timesec[..8].copy_from_slice(&utc.to_le_bytes());
                timesec[8..12].copy_from_slice(&(east as u32).to_le_bytes());
                timesec[12..].copy_from_slice(&(dst as u32).to_le_bytes());
                output(state, &timesec);
                output(daylight, &dst64);
            }
            Service::ConvertLocaltimeToUtc => {
                // The second argument is reserved and ignored by the source.
                let [local, _, utc, zone, daylight, ..] = args.0;
                if zone == 0 {
                    return Err(Errno::Invalid.into());
                }
                if utc != 0 {
                    let value = (local as i64).checked_sub(delta).ok_or(Errno::Invalid)?;
                    local_or_utc = value.to_le_bytes();
                    output(utc, &local_or_utc);
                }
                output(zone, &timezone);
                output(daylight, &dst32);
            }
            _ => return Err(Failure::Unsupported("calendar service")),
        }
        if count != 0 {
            self.budget.write(guest, &writes[..count])?;
        }
        Ok(0)
    }
}
