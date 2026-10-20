// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned bounded HTTP2 metadata and checked guest reads.
// Adapted from libNet.cpp at209dffe9032d72749257ce18680d506b891284de;
// see NOTICE. No HTTP transport or callback execution is implemented here.

//! Source HTTP2 metadata semantics. PentaX additionally bounds all live text
//! capacities by max_io_bytes and all live header records by max_handles.
//! These are resource policies, not source HTTP2 error codes. Context, template
//! and request identities each increase independently and never wrap or reset.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, Service};

const INVALID_ID: u64 = 0x817b_1100;
const NULL_POINTER: u64 = 0x817b_1225;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Options {
    auth_enabled: bool,
    auto_redirect: bool,
    inflate_gzip: bool,
    ssl_options: u32,
    resolve_timeout_us: u32,
    connect_timeout_us: u32,
    connection_wait_timeout_us: u32,
    send_timeout_us: u32,
    recv_timeout_us: u32,
    timeout_us: u32,
    ssl_callback: u64,
    ssl_callback_arg: u64,
    redirect_callback: u64,
    redirect_user_arg: u64,
}

struct Context {
    id: i32,
    _libnet_mem_id: i32,
    _libssl_ctx_id: i32,
    _pool_size: u64,
    _max_concurrent_requests: i32,
}

struct Template {
    id: i32,
    context_id: i32,
    user_agent: Vec<u8>,
    _http_ver: i32,
    _is_auto_proxy_conf: i32,
    options: Options,
}

struct Request {
    id: i32,
    template_id: i32,
    // Derived from the owned template once, keeping context cascades linear.
    context_id: i32,
    method: Vec<u8>,
    url: Vec<u8>,
    content_length: u64,
    headers: Vec<(Vec<u8>, Vec<u8>)>,
    options: Options,
}

impl Request {
    fn text_bytes(&self) -> usize {
        self.method.capacity()
            + self.url.capacity()
            + self
                .headers
                .iter()
                .map(|(name, value)| name.capacity() + value.capacity())
                .sum::<usize>()
    }
}

pub(crate) struct Http2State {
    contexts: Vec<Context>,
    templates: Vec<Template>,
    requests: Vec<Request>,
    next_context: u64,
    next_template: u64,
    next_request: u64,
    text_bytes: usize,
    header_count: usize,
}

impl Default for Http2State {
    fn default() -> Self {
        Self {
            contexts: Vec::new(),
            templates: Vec::new(),
            requests: Vec::new(),
            next_context: 1,
            next_template: 1,
            next_request: 1,
            text_bytes: 0,
            header_count: 0,
        }
    }
}

fn next_id(next: u64) -> Result<(i32, u64), Failure> {
    if next == 0 || next > i32::MAX as u64 {
        return Err(Failure::Limit("HTTP2 identifiers"));
    }
    Ok((next as i32, next + 1))
}

/// Read an opaque NUL-terminated value into private staging, without UTF-8 or
/// host-path interpretation. Copy into exact-length fallible retained storage.
/// A second staged string includes the first one's capacity in scratch usage.
fn text(
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
    allowance: usize,
    held_scratch: usize,
) -> Result<Vec<u8>, Failure> {
    let length = budget.limits.max_name_bytes;
    budget.scratch(
        held_scratch
            .checked_add(length)
            .ok_or(Failure::Limit("scratch"))?,
    )?;
    budget.work(length as u64 + 1)?;
    let mut scratch = Vec::new();
    scratch
        .try_reserve_exact(length)
        .map_err(|_| Failure::Allocation)?;
    let scratch_bytes = held_scratch
        .checked_add(scratch.capacity())
        .ok_or(Failure::Limit("scratch"))?;
    budget.scratch(scratch_bytes)?;
    budget.record_scratch(scratch_bytes);
    scratch.resize(length, 0);
    for index in 0..length {
        budget.read(
            guest,
            address
                .checked_add(index as u64)
                .ok_or(GuestError::AddressOverflow)?,
            &mut scratch[index..index + 1],
        )?;
        if scratch[index] != 0 {
            continue;
        }
        if index == 0 {
            return Ok(Vec::new());
        }
        if index > allowance {
            return Err(Failure::Limit("HTTP2 retained text bytes"));
        }
        budget.scratch(
            scratch_bytes
                .checked_add(index)
                .ok_or(Failure::Limit("scratch"))?,
        )?;
        budget.work(index as u64 + 1)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(index)
            .map_err(|_| Failure::Allocation)?;
        if bytes.capacity() > allowance {
            return Err(Failure::Limit("HTTP2 retained text bytes"));
        }
        let peak = scratch_bytes
            .checked_add(bytes.capacity())
            .ok_or(Failure::Limit("scratch"))?;
        budget.scratch(peak)?;
        budget.record_scratch(peak);
        bytes.extend_from_slice(&scratch[..index]);
        return Ok(bytes);
    }
    Err(Failure::Limit("name bytes"))
}

impl Http2State {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.contexts.len() + self.templates.len() + self.requests.len()
    }

    fn allowance(&self, budget: &Budget) -> Result<usize, Failure> {
        budget
            .limits
            .max_io_bytes
            .checked_sub(self.text_bytes)
            .ok_or(Failure::Limit("HTTP2 retained text bytes"))
    }

    fn options(&mut self, id: i32) -> Option<&mut Options> {
        // Source deliberately prefers requests when independent ID spaces
        // collide. Template options are never copied into a fresh request.
        if let Some(request) = self.requests.iter_mut().find(|request| request.id == id) {
            return Some(&mut request.options);
        }
        self.templates
            .iter_mut()
            .find(|template| template.id == id)
            .map(|template| &mut template.options)
    }

    fn remove_requests(&mut self, mut remove: impl FnMut(&Request) -> bool, budget: &mut Budget) {
        let mut count = 0;
        let mut released_text = 0;
        let mut released_headers = 0;
        self.requests.retain(|request| {
            if !remove(request) {
                return true;
            }
            count += 1;
            released_text += request.text_bytes();
            released_headers += request.headers.len();
            false
        });
        self.text_bytes -= released_text;
        self.header_count -= released_headers;
        for _ in 0..count {
            budget.removed_handle();
        }
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // A conservative bound covers lookups, vector shifts and cascades,
        // including every retained header inspected while releasing strings.
        // Admission occurs before any metadata mutation or table reservation.
        let records = self
            .retained_handle_count()
            .checked_add(self.header_count)
            .ok_or(Failure::Limit("work"))?;
        let work = (records as u64)
            .checked_add(1)
            .and_then(|n| n.checked_mul(4))
            .ok_or(Failure::Limit("work"))?;
        budget.work(work)?;
        let a = args.0;
        let id = a[0] as u32 as i32;
        use Service::*;
        match service {
            Http2Init => {
                budget.can_add_handle()?;
                let (id, next) = next_id(self.next_context)?;
                self.contexts
                    .try_reserve_exact(1)
                    .map_err(|_| Failure::Allocation)?;
                self.contexts.push(Context {
                    id,
                    _libnet_mem_id: a[0] as u32 as i32,
                    _libssl_ctx_id: a[1] as u32 as i32,
                    _pool_size: a[2],
                    _max_concurrent_requests: a[3] as u32 as i32,
                });
                self.next_context = next;
                budget.added_handle();
                Ok(id as u64)
            }
            Http2CreateTemplate => {
                if !self.contexts.iter().any(|context| context.id == id) {
                    return Ok(INVALID_ID);
                }
                budget.can_add_handle()?;
                let (template_id, next) = next_id(self.next_template)?;
                let allowance = self.allowance(budget)?;
                let user_agent = if a[1] == 0 {
                    Vec::new()
                } else {
                    text(a[1], guest, budget, allowance, 0)?
                };
                self.templates
                    .try_reserve_exact(1)
                    .map_err(|_| Failure::Allocation)?;
                self.text_bytes += user_agent.capacity();
                self.templates.push(Template {
                    id: template_id,
                    context_id: id,
                    user_agent,
                    _http_ver: a[2] as u32 as i32,
                    _is_auto_proxy_conf: a[3] as u32 as i32,
                    options: Options::default(),
                });
                self.next_template = next;
                budget.added_handle();
                Ok(template_id as u64)
            }
            Http2CreateRequestWithURL => {
                if a[1] == 0 || a[2] == 0 {
                    return Ok(NULL_POINTER);
                }
                let Some(template) = self.templates.iter().find(|template| template.id == id)
                else {
                    return Ok(INVALID_ID);
                };
                let context_id = template.context_id;
                budget.can_add_handle()?;
                let (request_id, next) = next_id(self.next_request)?;
                let allowance = self.allowance(budget)?;
                let method = text(a[1], guest, budget, allowance, 0)?;
                let url = text(
                    a[2],
                    guest,
                    budget,
                    allowance - method.capacity(),
                    method.capacity(),
                )?;
                self.requests
                    .try_reserve_exact(1)
                    .map_err(|_| Failure::Allocation)?;
                self.text_bytes += method.capacity() + url.capacity();
                self.requests.push(Request {
                    id: request_id,
                    template_id: id,
                    context_id,
                    method,
                    url,
                    content_length: a[3],
                    headers: Vec::new(),
                    options: Options::default(),
                });
                self.next_request = next;
                budget.added_handle();
                Ok(request_id as u64)
            }
            Http2AddRequestHeader => {
                if a[1] == 0 || a[2] == 0 {
                    return Ok(NULL_POINTER);
                }
                let Some(index) = self.requests.iter().position(|request| request.id == id) else {
                    return Ok(INVALID_ID);
                };
                if self.header_count >= budget.limits.max_handles {
                    return Err(Failure::Limit("HTTP2 headers"));
                }
                let allowance = self.allowance(budget)?;
                let name = text(a[1], guest, budget, allowance, 0)?;
                let value = text(
                    a[2],
                    guest,
                    budget,
                    allowance - name.capacity(),
                    name.capacity(),
                )?;
                self.requests[index]
                    .headers
                    .try_reserve_exact(1)
                    .map_err(|_| Failure::Allocation)?;
                // Source mode is ignored, including unknown values: always append.
                self.text_bytes += name.capacity() + value.capacity();
                self.requests[index].headers.push((name, value));
                self.header_count += 1;
                Ok(0)
            }
            Http2SetRequestContentLength => {
                let Some(request) = self.requests.iter_mut().find(|request| request.id == id)
                else {
                    return Ok(INVALID_ID);
                };
                request.content_length = a[1];
                Ok(0)
            }
            Http2DeleteRequest => {
                if !self.requests.iter().any(|request| request.id == id) {
                    return Ok(INVALID_ID);
                }
                self.remove_requests(|request| request.id == id, budget);
                Ok(0)
            }
            Http2DeleteTemplate => {
                let Some(index) = self.templates.iter().position(|template| template.id == id)
                else {
                    return Ok(INVALID_ID);
                };
                self.remove_requests(|request| request.template_id == id, budget);
                let template = self.templates.remove(index);
                self.text_bytes -= template.user_agent.capacity();
                budget.removed_handle();
                Ok(0)
            }
            Http2Term => {
                let Some(index) = self.contexts.iter().position(|context| context.id == id) else {
                    return Ok(INVALID_ID);
                };
                self.remove_requests(|request| request.context_id == id, budget);
                let mut count = 0;
                let mut released = 0;
                self.templates.retain(|template| {
                    if template.context_id != id {
                        return true;
                    }
                    count += 1;
                    released += template.user_agent.capacity();
                    false
                });
                self.text_bytes -= released;
                for _ in 0..count {
                    budget.removed_handle();
                }
                self.contexts.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            Http2SetAuthEnabled
            | Http2SetAutoRedirect
            | Http2SetInflateGZIPEnabled
            | Http2SslDisableOption
            | Http2SslEnableOption
            | Http2SetRedirectCallback
            | Http2SetSslCallback
            | Http2SetRecvTimeOut
            | Http2SetSendTimeOut
            | Http2SetConnectTimeOut
            | Http2SetConnectionWaitTimeOut
            | Http2SetResolveTimeOut
            | Http2SetTimeOut => {
                let Some(options) = self.options(id) else {
                    return Ok(INVALID_ID);
                };
                match service {
                    Http2SetAuthEnabled => options.auth_enabled = a[1] as u32 != 0,
                    Http2SetAutoRedirect => options.auto_redirect = a[1] as u32 != 0,
                    Http2SetInflateGZIPEnabled => options.inflate_gzip = a[1] as u32 != 0,
                    Http2SslDisableOption => options.ssl_options &= !(a[1] as u32),
                    Http2SslEnableOption => options.ssl_options |= a[1] as u32,
                    Http2SetRedirectCallback => {
                        options.redirect_callback = a[1];
                        options.redirect_user_arg = a[2];
                    }
                    Http2SetSslCallback => {
                        options.ssl_callback = a[1];
                        options.ssl_callback_arg = a[2];
                    }
                    Http2SetRecvTimeOut => options.recv_timeout_us = a[1] as u32,
                    Http2SetSendTimeOut => options.send_timeout_us = a[1] as u32,
                    Http2SetConnectTimeOut => options.connect_timeout_us = a[1] as u32,
                    Http2SetConnectionWaitTimeOut => {
                        options.connection_wait_timeout_us = a[1] as u32
                    }
                    Http2SetResolveTimeOut => options.resolve_timeout_us = a[1] as u32,
                    Http2SetTimeOut => options.timeout_us = a[1] as u32,
                    _ => unreachable!("classified HTTP2 option service"),
                }
                Ok(0)
            }
            _ => Err(Failure::Unsupported("HTTP2 metadata service")),
        }
    }
}

#[cfg(test)]
mod tests;
