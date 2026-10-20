// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded content notifications and atomic event output.
// Adapted from pinned libAppContent.cpp/libSystemService.cpp; see NOTICE.

use crate::budget::{Budget, Limits};
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service};
use std::collections::VecDeque;

const PARAMETER: u64 = 0x80a1_0003;
const NO_EVENT: u64 = 0x80a1_0004;
const ENTITLEMENT_UPDATE: i32 = 0x1000_0003;
pub(crate) const EVENT_BYTES: usize = 4 + 8192;
pub(crate) const STATUS_BYTES: usize = 136;

pub(crate) struct SystemEvents {
    events: VecDeque<i32>,
    maximum: usize,
    content_initialized: bool,
}
impl SystemEvents {
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut events = VecDeque::new();
        events
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            events,
            maximum: limits.max_handles,
            content_initialized: false,
        })
    }
    pub(crate) fn initialized(&self) -> bool {
        self.content_initialized
    }
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        overlaid: bool,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        match service {
            Service::AppContentAppParamGetInt => {
                budget.work(1)?;
                if args.0[1] == 0 {
                    // The pinned source aborts for null instead of returning a
                    // library errno. Keep that branch explicitly unsupported.
                    return Err(Failure::Unsupported(
                        "null application-content integer output",
                    ));
                }
                let parameter = args.0[0] as u32;
                // The explicit-image profile has no supplied SFO metadata.
                // Source initializes missing USER_DEFINED_PARAM_1..4 to zero;
                // its SKU query always uses the source FULL constant (3).
                let value = if parameter == 0 { 3_i32 } else { 0 };
                budget.scratch(4)?;
                budget.record_scratch(4);
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: args.0[1],
                        bytes: &value.to_le_bytes(),
                    }],
                )?;
                if parameter > 4 {
                    // Source writes its initial zero before the unknown-ID
                    // abort. Do not invent a successful unsupported query.
                    return Err(Failure::Unsupported(
                        "application-content integer parameter",
                    ));
                }
                Ok(0)
            }
            Service::AppContentInitialize => {
                budget.work(1)?;
                if self.events.len() >= self.maximum {
                    return Err(Failure::Limit("system events"));
                }
                // The source ignores init_param, accepts a null boot_param and
                // clears the complete 40-byte boot object when one is supplied.
                if args.0[1] != 0 {
                    budget.scratch(40)?;
                    budget.work(40)?;
                    budget.record_scratch(40);
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: args.0[1],
                            bytes: &[0; 40],
                        }],
                    )?;
                }
                // Optional title metadata is absent in this selected-input
                // profile. Do not read extra files or invent an entitlement.
                // Publication is infallible after queue/output admission.
                self.content_initialized = true;
                self.events.push_back(ENTITLEMENT_UPDATE);
                Ok(0)
            }
            Service::SystemReceiveEvent => {
                if args.0[0] == 0 {
                    return Ok(PARAMETER);
                }
                budget.work(1)?;
                budget.scratch(EVENT_BYTES)?;
                budget.work(EVENT_BYTES as u64)?;
                budget.record_scratch(EVENT_BYTES);
                let event = self.events.front().copied();
                let mut bytes = [0; EVENT_BYTES];
                bytes[..4].copy_from_slice(&event.unwrap_or(-1).to_le_bytes());
                // Empty reception still publishes the source's -1/zero object.
                // A failed transaction never consumes a pending notification.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: args.0[0],
                        bytes: &bytes,
                    }],
                )?;
                if event.is_some() {
                    self.events.pop_front();
                    Ok(0)
                } else {
                    Ok(NO_EVENT)
                }
            }
            Service::SystemGetStatus => {
                if args.0[0] == 0 {
                    return Ok(PARAMETER);
                }
                budget.scratch(STATUS_BYTES)?;
                budget.work(STATUS_BYTES as u64)?;
                budget.record_scratch(STATUS_BYTES);
                // Source int32 count, three bools,127 reserved bytes and two
                // tail-padding bytes. No background/sign-in or VR UI is admitted.
                let mut bytes = [0; STATUS_BYTES];
                bytes[..4].copy_from_slice(&(self.events.len() as i32).to_le_bytes());
                bytes[4] = u8::from(overlaid);
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: args.0[0],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            _ => Err(Failure::Unsupported("system event service")),
        }
    }
}

impl crate::Kernel {
    /// Retained initialization receipt, not proof of an owned entitlement or DLC.
    #[must_use]
    pub fn application_content_initialized(&self) -> bool {
        self.system_events.initialized()
    }
}
