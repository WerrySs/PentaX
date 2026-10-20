// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded offline NP requests and atomic account outputs.
// Adapted from pinned network.cpp/libNet.cpp; see NOTICE.

use crate::budget::{Budget, Limits};
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};

const INVALID_ARGUMENT: u64 = 0x8055_0003;
const SIGNED_OUT: u64 = 0x8055_0006;
const INVALID_SIZE: u64 = 0x8055_0011;
const ABORTED: u64 = 0x8055_0012;
const REQUEST_MAX: u64 = 0x8055_0013;
const NOT_FOUND: u64 = 0x8055_0014;
const INVALID_ID: u64 = 0x8055_0015;
const SOURCE_MAX: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Free,
    Ready,
    Aborted,
    Complete,
}
struct Request {
    state: State,
    asynchronous: bool,
    result: u64,
}
pub(crate) struct NpState {
    requests: Vec<Request>,
    maximum: usize,
}

impl NpState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.requests
            .iter()
            .filter(|request| request.state != State::Free)
            .count()
    }

    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let maximum = limits.max_handles.min(SOURCE_MAX);
        let mut requests = Vec::new();
        requests
            .try_reserve_exact(maximum)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self { requests, maximum })
    }
    fn create(&mut self, asynchronous: bool, budget: &mut Budget) -> Result<u64, Failure> {
        budget.work(self.requests.len() as u64 + 1)?;
        let free = self
            .requests
            .iter()
            .position(|request| request.state == State::Free);
        if free.is_none() && self.requests.len() == SOURCE_MAX {
            return Ok(REQUEST_MAX);
        }
        if free.is_none() && self.requests.len() >= self.maximum {
            return Err(Failure::Limit("np requests"));
        }
        budget.can_add_handle()?;
        let request = Request {
            state: State::Ready,
            asynchronous,
            result: 0,
        };
        let index = if let Some(index) = free {
            self.requests[index] = request;
            index
        } else {
            self.requests.push(request);
            self.requests.len() - 1
        };
        budget.added_handle();
        // Source deliberately reuses the lowest free ordinal within a process.
        Ok(index as u64 + 1)
    }
    fn index(&self, id: u64) -> Option<usize> {
        let id = id as u32 as i32;
        let index = usize::try_from(id.checked_sub(1)?).ok()?;
        self.requests
            .get(index)
            .filter(|request| request.state != State::Free)
            .map(|_| index)
    }
    fn complete(request: &mut Request) -> u64 {
        match request.state {
            State::Complete => {
                request.result = INVALID_ARGUMENT;
                INVALID_ARGUMENT
            }
            State::Aborted => {
                request.result = ABORTED;
                ABORTED
            }
            State::Ready => {
                request.state = State::Complete;
                request.result = SIGNED_OUT;
                if request.asynchronous { 0 } else { SIGNED_OUT }
            }
            State::Free => unreachable!("owned live NP request"),
        }
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
            NpGetId | NpGetOnlineId | NpGetAccountId | NpGetAccountCountry => {
                if second == 0 {
                    return Err(Failure::Unsupported("source-null NP account output"));
                }
                let length = match service {
                    NpGetId => 36,
                    NpGetOnlineId => 20,
                    NpGetAccountId => 8,
                    _ => 4,
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &[0; 36][..length],
                    }],
                )?;
                return Ok(SIGNED_OUT);
            }
            NpGetState | NpGetReachabilityState | NpHasSignedUp => {
                if second == 0 {
                    return if service == NpGetState {
                        Err(Failure::Unsupported("source-null NP state output"))
                    } else {
                        Ok(INVALID_ARGUMENT)
                    };
                }
                let value = u32::from(service == NpGetState).to_le_bytes();
                let length = if service == NpHasSignedUp { 1 } else { 4 };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &value[..length],
                    }],
                )?;
                return Ok(0);
            }
            NpGetAccountAge => {
                if first as u32 as i32 <= 0 || third == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                // Pinned age query only checks positivity, not request ownership.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: third,
                        bytes: &[0],
                    }],
                )?;
                return Ok(SIGNED_OUT);
            }
            NpGetAccountLanguage => {
                if first as u32 as i32 <= 0 || third == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                return Ok(if self.index(first).is_some() {
                    SIGNED_OUT
                } else {
                    NOT_FOUND
                });
            }
            NpCreateRequest => return self.create(false, budget),
            NpCreateAsyncRequest => {
                if first == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if budget.u64(guest, first)? < 24 {
                    return Ok(INVALID_SIZE);
                }
                // Check the source's actual affinity/priority fields after size
                // validation. They do not change host scheduling in offline mode.
                let address = first.checked_add(8).ok_or(GuestError::AddressOverflow)?;
                budget.read(guest, address, &mut [0; 12])?;
                return self.create(true, budget);
            }
            NpCheckAvailability => {
                if first as u32 as i32 <= 0 || second == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                // The source clears this optional int BEFORE checking ownership
                // or completion state. User text has no offline semantic use.
                if third != 0 {
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: third,
                            bytes: &[0; 4],
                        }],
                    )?;
                }
            }
            NpCheckReachability => {
                if first as u32 as i32 <= 0 {
                    return Ok(INVALID_ARGUMENT);
                }
            }
            NpPollAsync => {
                if second == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
            }
            NpCheckPremium => {
                if second == 0 || third == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if budget.u64(guest, second)? < 56 {
                    return Ok(INVALID_SIZE);
                }
                budget.u32(
                    guest,
                    second.checked_add(8).ok_or(GuestError::AddressOverflow)?,
                )?;
                budget.u64(
                    guest,
                    second.checked_add(16).ok_or(GuestError::AddressOverflow)?,
                )?;
            }
            NpDeleteRequest | NpAbortRequest => {}
            _ => return Err(Failure::Unsupported("NP service")),
        }
        let Some(index) = self.index(first) else {
            return Ok(NOT_FOUND);
        };
        let request = &mut self.requests[index];
        match service {
            NpDeleteRequest => {
                request.state = State::Free;
                request.asynchronous = false;
                request.result = 0;
                budget.removed_handle();
                Ok(0)
            }
            NpAbortRequest => {
                request.state = State::Aborted;
                request.result = ABORTED;
                Ok(0)
            }
            NpCheckAvailability | NpCheckReachability => Ok(Self::complete(request)),
            NpPollAsync => {
                if !request.asynchronous || request.state == State::Ready {
                    return Ok(INVALID_ID);
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &(request.result as u32).to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            NpCheckPremium => {
                if request.state == State::Complete {
                    request.result = INVALID_ARGUMENT;
                    return Ok(INVALID_ARGUMENT);
                }
                if request.state == State::Aborted {
                    return Ok(ABORTED);
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: third,
                        bytes: &[0; 33],
                    }],
                )?;
                Ok(Self::complete(request))
            }
            _ => unreachable!("reviewed request service"),
        }
    }
}
