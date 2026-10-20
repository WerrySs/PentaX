// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned bounded offline NpWebApi2 state.
// Adapted from the public libNet.cpp blob
// 1667ec6e6e16f412b71cd2746afc1bcee7f7c106; see upstream-provenance.md.
// No PSN authentication, transport, pending events or callback execution.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};

mod storage;
use storage::{growth, insert, text};

const INVALID_ARGUMENT: u64 = 0x8055_3402;
const INVALID_CONTEXT: u64 = 0x8055_3403;
const CONTEXT_NOT_FOUND: u64 = 0x8055_3404;
const REQUEST_NOT_FOUND: u64 = 0x8055_3406;
const NOT_SIGNED_IN: u64 = 0x8055_3407;
const CONTENT_TYPE: &[u8] = b"application/json; charset=utf-8";

struct Context {
    id: i32,
    _http_context: i32,
    pool_size: u64,
}

struct PushHandle {
    id: i32,
    _context: i32,
}

struct User {
    id: i32,
    _context: i32,
    _user: i32,
}

struct Filter {
    _id: i32,
    _context: i32,
    _handle: i32,
    _service_name: u64,
    _service_label: u32,
    _parameters: u64,
    _parameter_count: u64,
}

struct Callback {
    _id: i32,
    _user: i32,
    _filter: i32,
    _callback: u64,
    _argument: u64,
}

struct Request {
    id: i64,
    _user: i32,
    _content_parameter: u64,
    api_group: Vec<u8>,
    path: Vec<u8>,
    method: Vec<u8>,
    // Source currently never fills response storage or queues network work.
    response: Vec<u8>,
    read_offset: usize,
}

impl Request {
    fn text_bytes(&self) -> usize {
        self.api_group.capacity()
            + self.path.capacity()
            + self.method.capacity()
            + self.response.capacity()
    }
}

pub(crate) struct NpWebApi2State {
    contexts: Vec<Context>,
    push_handles: Vec<PushHandle>,
    users: Vec<User>,
    requests: Vec<Request>,
    filters: Vec<Filter>,
    callbacks: Vec<Callback>,
    next_context: u64,
    next_push_handle: u64,
    next_user: u64,
    next_request: u64,
    next_filter: u64,
    next_callback: u64,
    text_bytes: usize,
}

impl Default for NpWebApi2State {
    fn default() -> Self {
        Self {
            contexts: Vec::new(),
            push_handles: Vec::new(),
            users: Vec::new(),
            requests: Vec::new(),
            filters: Vec::new(),
            callbacks: Vec::new(),
            next_context: 1,
            next_push_handle: 1,
            next_user: 1001,
            next_request: 1000,
            next_filter: 1,
            next_callback: 1,
            text_bytes: 0,
        }
    }
}

fn int_id(value: u64) -> Result<(i32, u64), Failure> {
    if value == 0 || value > i32::MAX as u64 {
        return Err(Failure::Limit("NpWebApi2 identifiers"));
    }
    Ok((value as i32, value + 1))
}

fn offset(address: u64, offset: u64) -> Result<u64, Failure> {
    address
        .checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}

impl NpWebApi2State {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.contexts.len()
            + self.push_handles.len()
            + self.users.len()
            + self.requests.len()
            + self.filters.len()
            + self.callbacks.len()
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // Covers all table searches, shifts and pre-admitted replacement moves.
        budget.work((self.retained_handle_count() as u64 + 1) * 4)?;
        let a = args.0;
        let id = a[0] as u32 as i32;
        use Service::*;
        match service {
            NpWebApi2Initialize => {
                let Some(pool_size) = a[1].checked_add(0x3fff).map(|size| size & !0x3fff) else {
                    return Ok(INVALID_ARGUMENT);
                };
                budget.can_add_handle()?;
                let (id, next) = int_id(self.next_context)?;
                let prepared = growth(&self.contexts, 0, budget)?;
                insert(
                    &mut self.contexts,
                    prepared,
                    Context {
                        id,
                        _http_context: a[0] as u32 as i32,
                        pool_size,
                    },
                );
                self.next_context = next;
                budget.added_handle();
                Ok(id as u64)
            }
            NpWebApi2GetMemoryPoolStats => {
                if a[1] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if id <= 0 {
                    return Ok(INVALID_CONTEXT);
                }
                let Some(context) = self.contexts.iter().find(|context| context.id == id) else {
                    return Ok(CONTEXT_NOT_FOUND);
                };
                // Only the 28 defined bytes are assigned. Tail padding survives.
                let mut stats = [0; 28];
                stats[..8].copy_from_slice(&context.pool_size.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &stats,
                    }],
                )?;
                Ok(0)
            }
            NpWebApi2PushEventCreateHandle => {
                budget.can_add_handle()?;
                let (id, next) = int_id(self.next_push_handle)?;
                let prepared = growth(&self.push_handles, 0, budget)?;
                insert(
                    &mut self.push_handles,
                    prepared,
                    PushHandle {
                        id,
                        _context: a[0] as u32 as i32,
                    },
                );
                self.next_push_handle = next;
                budget.added_handle();
                Ok(id as u64)
            }
            NpWebApi2CreateUserContext => {
                budget.can_add_handle()?;
                let (id, next) = int_id(self.next_user)?;
                let prepared = growth(&self.users, 0, budget)?;
                insert(
                    &mut self.users,
                    prepared,
                    User {
                        id,
                        _context: a[0] as u32 as i32,
                        _user: a[1] as u32 as i32,
                    },
                );
                self.next_user = next;
                budget.added_handle();
                Ok(id as u64)
            }
            NpWebApi2DeleteUserContext => {
                // Source logs arbitrary IDs and always succeeds, without cascade.
                if let Some(index) = self.users.iter().position(|user| user.id == id) {
                    self.users.remove(index);
                    budget.removed_handle();
                }
                Ok(0)
            }
            NpWebApi2CreateRequest => self.create_request(a, guest, budget),
            NpWebApi2SendRequest => self.send(a, guest, budget),
            NpWebApi2ReadData => {
                if a[1] == 0 || a[2] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let Some(request) = self
                    .requests
                    .iter_mut()
                    .find(|request| request.id == a[0] as i64)
                else {
                    return Ok(REQUEST_NOT_FOUND);
                };
                let remaining = request
                    .response
                    .len()
                    .checked_sub(request.read_offset)
                    .ok_or(Failure::Unsupported("NpWebApi2 response cursor"))?;
                let count = remaining.min(usize::try_from(a[2]).unwrap_or(usize::MAX));
                if count > i32::MAX as usize {
                    return Err(Failure::Limit("NpWebApi2 response size"));
                }
                if count != 0 {
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: a[1],
                            bytes: &request.response
                                [request.read_offset..request.read_offset + count],
                        }],
                    )?;
                    request.read_offset += count;
                }
                Ok(count as u64)
            }
            NpWebApi2AbortRequest => Ok(
                if self
                    .requests
                    .iter()
                    .any(|request| request.id == a[0] as i64)
                {
                    0
                } else {
                    REQUEST_NOT_FOUND
                },
            ),
            NpWebApi2DeleteRequest => {
                if let Some(index) = self
                    .requests
                    .iter()
                    .position(|request| request.id == a[0] as i64)
                {
                    let removed = self.requests.remove(index);
                    self.text_bytes -= removed.text_bytes();
                    budget.removed_handle();
                }
                Ok(0)
            }
            // All three arguments are logging-only in this public source.
            NpWebApi2AddHttpRequestHeader => Ok(0),
            NpWebApi2GetHttpResponseHeaderValueLength => {
                if a[1] == 0 || a[2] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if !self
                    .requests
                    .iter()
                    .any(|request| request.id == a[0] as i64)
                {
                    return Ok(REQUEST_NOT_FOUND);
                }
                let length = if content_type(a[1], guest, budget)? {
                    CONTENT_TYPE.len() as u64 + 1
                } else {
                    0
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[2],
                        bytes: &length.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            NpWebApi2GetHttpResponseHeaderValue => {
                if a[1] == 0 || a[2] == 0 || a[3] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if !self
                    .requests
                    .iter()
                    .any(|request| request.id == a[0] as i64)
                {
                    return Ok(REQUEST_NOT_FOUND);
                }
                let header = if content_type(a[1], guest, budget)? {
                    CONTENT_TYPE
                } else {
                    b""
                };
                let length = (a[3] - 1).min(header.len() as u64) as usize;
                let mut output = [0; CONTENT_TYPE.len() + 1];
                output[..length].copy_from_slice(&header[..length]);
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[2],
                        bytes: &output[..length + 1],
                    }],
                )?;
                Ok(0)
            }
            NpWebApi2PushEventDeleteHandle => {
                let handle_id = a[1] as u32 as i32;
                if let Some(index) = self
                    .push_handles
                    .iter()
                    .position(|handle| handle.id == handle_id)
                {
                    self.push_handles.remove(index);
                    budget.removed_handle();
                }
                Ok(0)
            }
            // The opaque push-context word is only logged in the source.
            NpWebApi2PushEventDeletePushContext => Ok(0),
            NpWebApi2PushEventCreateFilter => {
                budget.can_add_handle()?;
                let (id, next) = int_id(self.next_filter)?;
                let prepared = growth(&self.filters, 0, budget)?;
                insert(
                    &mut self.filters,
                    prepared,
                    Filter {
                        _id: id,
                        _context: a[0] as u32 as i32,
                        _handle: a[1] as u32 as i32,
                        _service_name: a[2],
                        _service_label: a[3] as u32,
                        _parameters: a[4],
                        _parameter_count: a[5],
                    },
                );
                self.next_filter = next;
                budget.added_handle();
                Ok(id as u64)
            }
            NpWebApi2PushEventRegisterCallback => {
                budget.can_add_handle()?;
                let (id, next) = int_id(self.next_callback)?;
                let prepared = growth(&self.callbacks, 0, budget)?;
                insert(
                    &mut self.callbacks,
                    prepared,
                    Callback {
                        _id: id,
                        _user: a[0] as u32 as i32,
                        _filter: a[1] as u32 as i32,
                        _callback: a[2],
                        _argument: a[3],
                    },
                );
                self.next_callback = next;
                budget.added_handle();
                Ok(id as u64)
            }
            NpWebApi2CheckTimeout => Ok(0),
            NpWebApi2Terminate => {
                if id <= 0 {
                    return Ok(INVALID_CONTEXT);
                }
                let Some(index) = self.contexts.iter().position(|context| context.id == id) else {
                    return Ok(CONTEXT_NOT_FOUND);
                };
                self.contexts.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            _ => Err(Failure::Unsupported("NpWebApi2 offline service")),
        }
    }

    fn create_request(
        &mut self,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if a[5] == 0 {
            return Ok(0);
        }
        budget.can_add_handle()?;
        if self.next_request > i64::MAX as u64 {
            return Err(Failure::Limit("NpWebApi2 identifiers"));
        }
        let id = self.next_request as i64;
        let allowance = budget
            .limits
            .max_io_bytes
            .checked_sub(self.text_bytes)
            .ok_or(Failure::Limit("NpWebApi2 retained text bytes"))?;
        let overlay = Some((a[5], id as u64));
        let api_group = text(a[1], guest, budget, allowance, 0, overlay)?;
        let path = text(
            a[2],
            guest,
            budget,
            allowance - api_group.capacity(),
            api_group.capacity(),
            overlay,
        )?;
        let held = api_group.capacity() + path.capacity();
        let method = text(a[3], guest, budget, allowance - held, held, overlay)?;
        let retained = held + method.capacity();
        let prepared = growth(&self.requests, retained, budget)?;
        budget.write(
            guest,
            &[GuestWrite {
                address: a[5],
                bytes: &id.to_le_bytes(),
            }],
        )?;
        insert(
            &mut self.requests,
            prepared,
            Request {
                id,
                _user: a[0] as u32 as i32,
                _content_parameter: a[4],
                api_group,
                path,
                method,
                response: Vec::new(),
                read_offset: 0,
            },
        );
        self.text_bytes += retained;
        self.next_request += 1;
        budget.added_handle();
        Ok(0)
    }

    fn send(
        &mut self,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let Some(index) = self
            .requests
            .iter()
            .position(|request| request.id == a[0] as i64)
        else {
            return Ok(REQUEST_NOT_FOUND);
        };
        if a[3] != 0 {
            send_outputs(a[3], guest, budget)?;
        }
        // No data/data_size reads: the pinned implementation is offline.
        self.requests[index].response.clear();
        self.requests[index].read_offset = 0;
        Ok(NOT_SIGNED_IN)
    }
}

fn content_type(
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<bool, Failure> {
    // Model the source's two strcmp calls, including first-mismatch stopping.
    // A different name needs neither a full string scan nor UTF-8 decoding.
    for expected in [b"Content-Type\0", b"content-type\0"] {
        let mut matched = true;
        for (index, expected) in expected.iter().enumerate() {
            let mut byte = [0];
            budget.read(guest, offset(address, index as u64)?, &mut byte)?;
            if byte[0] != *expected {
                matched = false;
                break;
            }
        }
        if matched {
            return Ok(true);
        }
    }
    Ok(false)
}

fn send_outputs(
    address: u64,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    offset(address, 32)?;
    // This metadata proof excludes physical self-aliases inside the descriptor.
    // Thus staged status/length stores cannot change the pointer/size read next.
    budget.work(32 * 32 + 32)?;
    guest.preflight_distinct_backing(address, 32)?;
    let status = [0; 4];
    let response_size = [0; 8];
    let length_address = offset(address, 24)?;
    let pointer = budget.u64(guest, offset(address, 8)?)?;
    // Source short-circuits the size read when the pointer is null.
    let clear_error = pointer != 0 && budget.u64(guest, offset(address, 16)?)? != 0;
    let nul = [0];
    let mut writes = [GuestWrite {
        address,
        bytes: &status,
    }; 3];
    writes[1] = GuestWrite {
        address: length_address,
        bytes: &response_size,
    };
    let already_zero = (address..offset(address, 4)?).contains(&pointer)
        || (length_address..offset(address, 32)?).contains(&pointer);
    let count = if clear_error && !already_zero {
        writes[2] = GuestWrite {
            address: pointer,
            bytes: &nul,
        };
        3
    } else {
        2
    };
    budget.write(guest, &writes[..count])
}

#[cfg(test)]
mod tests;
