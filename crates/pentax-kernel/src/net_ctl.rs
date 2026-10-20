// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned forced-offline state, checked guest outputs
// and delayed callback completion. Adapted from pinned network.cpp; see NOTICE.

//! The pinned NetCtl profile deliberately forces disconnected status. Its
//! host-query helper and online info switch are unreachable, not emulated here.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Kernel, Service};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

const SLOTS: usize = 8;
const EVENT_DISCONNECTED: u64 = 1;
const CALLBACK_MAX: u64 = 0x8041_2103;
const ID_NOT_FOUND: u64 = 0x8041_2104;
const INVALID_ID: u64 = 0x8041_2105;
const INVALID_ADDR: u64 = 0x8041_2107;
const NOT_CONNECTED: u64 = 0x8041_2108;
const INVALID_TYPE: u64 = 0x8041_210f;
const NAT_SIZE: u32 = 16;
const INFO_SIZE: usize = 256;

// Request identities are process-unique, including across distinct kernels.
// Exhaustion fails before publishing a new request and can never recycle one.
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

/// Request data only. The actual native owner must check the executable target,
/// prepare its own call frame and verify an actual return before completion.
/// No execution or guest pointer dereference occurs in this crate.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedNetCtlCallback {
    slot: u8,
    sequence: u64,
    entry: u64,
    argument: u64,
}
impl AdmittedNetCtlCallback {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    /// Event, callback argument, and an explicitly zero third register argument.
    pub const fn args(&self) -> [u64; 3] {
        [EVENT_DISCONNECTED, self.argument, 0]
    }
}
impl fmt::Debug for AdmittedNetCtlCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdmittedNetCtlCallback")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetCtlCallbackAction {
    Returned(u64),
    Call(AdmittedNetCtlCallback),
}

#[derive(Clone, Copy, Default)]
struct Slot {
    entry: u64,
    argument: u64,
    last_event: u64,
}

/// Fixed source slots; no socket, network query, heap table or host callback.
#[derive(Default)]
pub(crate) struct NetCtlState {
    slots: [Slot; SLOTS],
    status_initialized: bool,
    pending: Option<AdmittedNetCtlCallback>,
    scan_cursor: usize,
}

impl NetCtlState {
    fn ensure_offline(&mut self) {
        if !self.status_initialized {
            self.status_initialized = true;
        }
    }

    fn request_from(&self, start: usize) -> Result<Option<AdmittedNetCtlCallback>, Failure> {
        let Some(index) = (start..SLOTS).find(|index| {
            let slot = self.slots[*index];
            slot.entry != 0 && slot.last_event != EVENT_DISCONNECTED
        }) else {
            return Ok(None);
        };
        let sequence = NEXT_REQUEST
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |sequence| {
                sequence.checked_add(1)
            })
            .map_err(|_| Failure::Limit("NetCtl callback sequence"))?;
        let slot = self.slots[index];
        Ok(Some(AdmittedNetCtlCallback {
            slot: index as u8,
            sequence,
            entry: slot.entry,
            argument: slot.argument,
        }))
    }

    fn set_pending(&mut self, request: Option<AdmittedNetCtlCallback>) -> NetCtlCallbackAction {
        self.pending = request;
        self.scan_cursor = request.map_or(0, |request| request.slot as usize + 1);
        request.map_or(
            NetCtlCallbackAction::Returned(0),
            NetCtlCallbackAction::Call,
        )
    }

    fn admit(&mut self, budget: &mut Budget) -> Result<NetCtlCallbackAction, Failure> {
        budget.work(SLOTS as u64 + 2)?;
        if self.pending.is_some() {
            return Err(Failure::Unsupported("recursive NetCtl callback check"));
        }
        // NetCtlConnected initializes false even when Init was never called.
        self.ensure_offline();
        let first = self.request_from(0)?;
        Ok(self.set_pending(first))
    }

    fn complete(
        &mut self,
        request: &AdmittedNetCtlCallback,
        budget: &mut Budget,
    ) -> Result<NetCtlCallbackAction, Failure> {
        // Preflight bounded verification, marking and the live remaining scan
        // before changing state. A refused completion leaves this pending token
        // and all previously completed event marks intact.
        budget.work(SLOTS as u64 + 2)?;
        if self.pending.as_ref() != Some(request) {
            return Err(Failure::Unsupported("stale NetCtl callback admission"));
        }
        // Reserve a possible successor's sequence before committing the current
        // mark. Reading later slots is independent of marking this earlier slot;
        // no callback/user code runs between this preflight and the commit.
        let next = self.request_from(self.scan_cursor)?;
        // Source retains a reference to the original array element, not the
        // registration identity: clearing/replacing that slot inside a callback
        // does not prevent the post-return event assignment to the CURRENT slot.
        self.slots[request.slot as usize].last_event = EVENT_DISCONNECTED;
        Ok(self.set_pending(next))
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        budget.work(1)?;
        let [first, second, third, ..] = args.0;
        match service {
            NetCtlInit => {
                self.status_initialized = true;
                Ok(0)
            }
            NetCtlTerm => {
                budget.work(SLOTS as u64)?;
                self.slots = [Slot::default(); SLOTS];
                // Offline initialization and the active Check frame survive;
                // eventual completion still marks its current array element
                // after the actual return, even though all slots were cleared.
                Ok(0)
            }
            NetCtlGetState | NetCtlGetStateV6 => {
                if first == 0 {
                    return Ok(INVALID_ADDR);
                }
                self.ensure_offline();
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: first,
                        bytes: &0_u32.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            NetCtlGetNatInfo => {
                if first == 0 {
                    return Err(Failure::Unsupported("source-null NetCtl NAT info"));
                }
                if budget.u32(guest, first)? != NAT_SIZE {
                    return Err(Failure::Unsupported("NetCtl NAT info size"));
                }
                let address = first.checked_add(4).ok_or(GuestError::AddressOverflow)?;
                // The caller's size word is read, not overwritten. Exactly the
                // three four-byte result fields become zero, atomically.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &[0; 12],
                    }],
                )?;
                Ok(0)
            }
            NetCtlRegisterCallback => {
                if first == 0 || third == 0 {
                    return Ok(INVALID_ADDR);
                }
                budget.work(SLOTS as u64)?;
                let Some(index) = self.slots.iter().position(|slot| slot.entry == 0) else {
                    return Ok(CALLBACK_MAX);
                };
                // Target admission belongs to callback execution. Registration
                // only checks source nulls and publishes the output before slot
                // ownership changes, so a guest write failure retains the slot.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: third,
                        bytes: &(index as u32).to_le_bytes(),
                    }],
                )?;
                self.slots[index] = Slot {
                    entry: first,
                    argument: second,
                    last_event: 0,
                };
                Ok(0)
            }
            NetCtlUnregisterCallback => {
                let index = first as u32 as i32;
                if !(0..SLOTS as i32).contains(&index) {
                    return Ok(INVALID_ID);
                }
                let slot = &mut self.slots[index as usize];
                if slot.entry == 0 {
                    return Ok(ID_NOT_FOUND);
                }
                *slot = Slot::default();
                Ok(0)
            }
            NetCtlCheckCallback => {
                budget.work(SLOTS as u64 + 1)?;
                if self.pending.is_some() {
                    return Err(Failure::Unsupported("recursive NetCtl callback check"));
                }
                self.ensure_offline();
                if self
                    .slots
                    .iter()
                    .any(|slot| slot.entry != 0 && slot.last_event != EVENT_DISCONNECTED)
                {
                    return Err(Failure::Unsupported("prepared NetCtl callback"));
                }
                Ok(0)
            }
            NetCtlGetResult => {
                if second == 0 {
                    return Ok(INVALID_ADDR);
                }
                if !matches!(first as u32 as i32, 1..=3) {
                    return Ok(INVALID_TYPE);
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &0_u32.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            NetCtlGetInfo => {
                if second == 0 {
                    return Err(Failure::Unsupported("source-null NetCtl info"));
                }
                // The source's forced-offline return precedes its code switch:
                // every numeric code clears the complete union and returns this
                // error. No online field, invalid-code branch or host query runs.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &[0; INFO_SIZE],
                    }],
                )?;
                Ok(NOT_CONNECTED)
            }
            _ => Err(Failure::Unsupported("NetCtl service")),
        }
    }
}

impl Kernel {
    /// Admit one source Check loop, consuming exactly one guest service attempt.
    /// A Call result is data only; its target/frame/return need the native owner.
    pub fn admit_netctl_callbacks(&mut self) -> Result<NetCtlCallbackAction, Failure> {
        self.budget.attempt()?;
        self.net_ctl.admit(&mut self.budget)
    }

    /// Continue only after the trusted native owner verified this exact pending
    /// callback's actual return. No extra guest attempt is consumed. Completion
    /// marks the original SLOT, even if guest code cleared/replaced it, then
    /// visits later slots live. Possession of a request is not execution proof.
    pub fn complete_netctl_callback(
        &mut self,
        request: &AdmittedNetCtlCallback,
    ) -> Result<NetCtlCallbackAction, Failure> {
        self.net_ctl.complete(request, &mut self.budget)
    }
}
