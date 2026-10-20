// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned guest profile and transactional session outputs.
// Adapted from pinned libUserService.cpp; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service, UserConfig};

const INVALID_ARGUMENT: u64 = 0x8096_0005;
const NO_EVENT: u64 = 0x8096_0007;
const NOT_LOGGED_IN: u64 = 0x8096_0009;

#[derive(Default)]
pub(crate) struct UserState {
    profile: UserConfig,
    initialized: bool,
    login_delivered: bool,
    observed: bool,
}
impl UserState {
    pub(crate) fn observed_id(&mut self) -> i32 {
        self.observed = true;
        self.profile.id
    }
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.work(1)?;
        let result = self.dispatch(service, args, guest, budget);
        if result.is_ok() {
            self.observed = true;
        }
        result
    }
    fn dispatch(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        let [first, output, size, ..] = args.0;
        match service {
            UserInitialize | UserInitialize2 => {
                // Pinned initialization ignores the optional input, is
                // idempotent and does not reset the one-time login event.
                self.initialized = true;
                return Ok(0);
            }
            UserGetInitial => {
                if first == 0 {
                    return Err(Failure::Unsupported("source-null initial user output"));
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: first,
                        bytes: &self.profile.id.to_le_bytes(),
                    }],
                )?;
                return Ok(0);
            }
            UserGetEvent => {
                if first == 0 {
                    return Err(Failure::Unsupported("source-null user event output"));
                }
                if self.login_delivered {
                    return Ok(NO_EVENT);
                }
                let mut bytes = [0; 8]; // Source event_type Login=0.
                bytes[4..].copy_from_slice(&self.profile.id.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: first,
                        bytes: &bytes,
                    }],
                )?;
                self.login_delivered = true;
                return Ok(0);
            }
            UserGetLoginList => {
                if first == 0 {
                    return Err(Failure::Unsupported("source-null login list output"));
                }
                let mut bytes = [0xff; 16];
                bytes[..4].copy_from_slice(&self.profile.id.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: first,
                        bytes: &bytes,
                    }],
                )?;
                return Ok(0);
            }
            _ => {}
        }
        // Name's pinned source checks identity before its fatal size/output
        // cases. Other reviewed getters validate null output before identity.
        if service != UserGetName && output == 0 {
            return Ok(INVALID_ARGUMENT);
        }
        if first as u32 as i32 != self.profile.id {
            return Ok(NOT_LOGGED_IN);
        }
        if service == UserGetName {
            let name = self.profile.name.as_str().as_bytes();
            if size < 5 || size <= name.len() as u64 {
                return Err(Failure::Unsupported("source-truncated user name"));
            }
            // Retain the full UTF-8 name and terminator; the source's fatal
            // truncation path is refused before publishing a partial string.
            budget.scratch(64)?;
            budget.work(name.len() as u64 + 1)?;
            budget.record_scratch(64);
            let mut bytes = [0; 64];
            bytes[..name.len()].copy_from_slice(name);
            budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &bytes[..name.len() + 1],
                }],
            )?;
        } else if service == UserGetGamePresets {
            let requested = budget.u64(guest, output)?;
            let cleared = if requested == 0 || requested > 40 {
                40
            } else {
                requested as usize
            };
            // Source always writes the full size_t header even when this_size
            // asks to clear fewer than8bytes. Leave every other byte untouched.
            budget.scratch(40)?;
            budget.work(40)?;
            budget.record_scratch(40);
            let mut bytes = [0; 40];
            bytes[..8].copy_from_slice(&40_u64.to_le_bytes());
            budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &bytes[..cleared.max(8)],
                }],
            )?;
        } else {
            // These are the explicit values in the pinned local-session profile,
            // not a claim that host accessibility or parental control is active.
            let value: u32 = match service {
                UserGetNumber | UserGetVibration | UserGetTriggerEffect => 1,
                UserGetAgeLevel => 9,
                UserGetChatTranscription
                | UserGetPressAndHoldDelay
                | UserGetZoomEnabled
                | UserGetZoomFollowFocus => 0,
                _ => return Err(Failure::Unsupported("user service")),
            };
            budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &value.to_le_bytes(),
                }],
            )?;
        }
        Ok(0)
    }
}

impl crate::Kernel {
    /// Select a local guest profile before any observable user-service result.
    /// This never reads a host identity or resets a delivered login notification.
    pub fn configure_user(&mut self, profile: UserConfig) -> Result<(), Failure> {
        profile
            .validate()
            .map_err(|_| Failure::Unsupported("invalid user configuration"))?;
        if self.user.observed && self.user.profile != profile {
            return Err(Failure::Unsupported("active user configuration"));
        }
        self.user.profile = profile;
        Ok(())
    }
    #[must_use]
    pub const fn user_service_initialized(&self) -> bool {
        self.user.initialized
    }
}
