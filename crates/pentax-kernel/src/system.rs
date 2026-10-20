// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c)2021 InoriRus
// Adapted from pinned libSystemService.cpp/libKernel.cpp; see NOTICE.

use crate::{
    CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Kernel, Service, SystemConfig,
};
use pentax_config::{DateFormat, EnterButton, TimeFormat};

const PARAMETER_ERROR: u64 = 0x80a1_0003;

impl Kernel {
    pub(crate) fn system_display_query(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        let output = args.0[0];
        if output == 0 {
            return Ok(PARAMETER_ERROR);
        }
        // Pinned emulated display profile, not a host monitor/HDR probe.
        // The safe-area source writes only ratio, retaining its reserved tail.
        let (values, length) = match service {
            Service::SystemGetHdrToneMapLuminance => ([80.0_f32, 1000.0, 0.0], 12),
            Service::SystemGetDisplaySafeAreaInfo => ([1.0_f32, 0.0, 0.0], 4),
            _ => return Err(Failure::Unsupported("system display query")),
        };
        let mut bytes = [0; 12];
        for (target, value) in bytes.chunks_exact_mut(4).zip(values) {
            target.copy_from_slice(&value.to_le_bytes());
        }
        self.budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &bytes[..length],
            }],
        )?;
        Ok(0)
    }

    pub(crate) fn hardware_mode(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.budget.work(1)?;
        if service == Service::IsTrinityMode {
            // Pinned profile: base PS5, never a host/model capability probe.
            return Ok(0);
        }
        let [mode, submode, ..] = args.0;
        if mode == 0 || submode == 0 {
            return Err(Failure::Unsupported("source-null operation-mode output"));
        }
        let mode_end = mode
            .checked_add(4)
            .ok_or(Failure::Guest(GuestError::AddressOverflow))?;
        let submode_end = submode
            .checked_add(4)
            .ok_or(Failure::Guest(GuestError::AddressOverflow))?;
        let bytes = 2_u32.to_le_bytes();
        // Source writes mode=2 then submode=0, including aliases. Normalize
        // overlapping/adjacent spans without ever touching a disjoint gap.
        if mode <= submode_end && submode <= mode_end {
            let start = mode.min(submode);
            let end = mode_end.max(submode_end);
            let mut union = [0; 8];
            let at = (mode - start) as usize;
            union[at..at + 4].copy_from_slice(&bytes);
            let at = (submode - start) as usize;
            union[at..at + 4].fill(0);
            self.budget.write(
                guest,
                &[GuestWrite {
                    address: start,
                    bytes: &union[..(end - start) as usize],
                }],
            )?;
        } else {
            self.budget.write(
                guest,
                &[
                    GuestWrite {
                        address: mode,
                        bytes: &bytes,
                    },
                    GuestWrite {
                        address: submode,
                        bytes: &[0; 4],
                    },
                ],
            )?;
        }
        Ok(0)
    }
    /// Replace guest preferences only after complete validation. No guest code
    /// or host environment lookup occurs; the owning application selects them.
    pub fn configure_system(&mut self, config: SystemConfig) -> Result<(), Failure> {
        config
            .validate()
            .map_err(|_| Failure::Unsupported("invalid system configuration"))?;
        self.system = config;
        Ok(())
    }

    pub(crate) fn system_param_get_int(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        let id = args.0[0] as u32 as i32;
        let output = args.0[1];
        if output == 0 {
            return Ok(PARAMETER_ERROR);
        }
        let value: i32 = match id {
            1 | 400 => self.system.language_id.into(),
            2 => match self.system.date_format {
                DateFormat::YearMonthDay => 0,
                DateFormat::DayMonthYear => 1,
                DateFormat::MonthDayYear => 2,
            },
            3 => match self.system.time_format {
                TimeFormat::TwelveHour => 0,
                TimeFormat::TwentyFourHour => 1,
            },
            4 => self.system.timezone_offset_minutes.into(),
            5 => self.system.daylight_saving.into(),
            7 => self.system.parental_level.into(),
            100 => self.system.closed_captions.into(),
            208 => self.system.screen_reader.into(),
            1000 => match self.system.enter_button {
                EnterButton::Circle => 0,
                EnterButton::Cross => 1,
            },
            _ => return Err(Failure::Unsupported("system integer parameter")),
        };
        self.budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &value.to_le_bytes(),
            }],
        )?;
        Ok(0)
    }
}
