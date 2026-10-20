// SPDX-License-Identifier: GPL-2.0-only
// Adapted from pinned ime.cpp/ime.h; see NOTICE and upstream provenance.
//! Owned IME sessions and source-ordered, actual-return-driven event delivery.

use crate::budget::Budget;
use crate::memory::{MemoryState, RuntimeObject};
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, RuntimeDataOutputs, Service};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[path = "ime/tests.rs"]
mod event_tests;
mod text;

const BUSY: u64 = 0x80bc0001;
const NOT_OPENED: u64 = 0x80bc0002;
const CONNECTION_FAILED: u64 = 0x80bc0004;
const EVENT_OVERFLOW: u64 = 0x80bc0007;
const INVALID_USER: u64 = 0x80bc0010;
const INVALID_OPTION: u64 = 0x80bc0015;
const INVALID_HANDLER: u64 = 0x80bc0022;
const NO_RESOURCE: u64 = 0x80bc0023;
const INVALID_MODE: u64 = 0x80bc0024;
const INVALID_ADDRESS: u64 = 0x80bc0031;
const INVALID_RESERVED: u64 = 0x80bc0032;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const MAX_PENDING: usize = 32;
const MAX_DISPATCH: usize = 256;
const EVENT_BYTES: usize = 72;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

fn add(left: usize, right: usize) -> Result<usize, Failure> {
    left.checked_add(right)
        .filter(|n| *n <= MAX_METADATA)
        .ok_or(Failure::Limit("IME retained and staged metadata"))
}
fn metadata_capacity<T>(values: &Vec<T>) -> usize {
    values.capacity() * core::mem::size_of::<T>()
}
fn reserve<T>(count: usize, held: usize, budget: &mut Budget) -> Result<Vec<T>, Failure> {
    if count > 65_536 {
        return Err(Failure::Limit("IME bounded allocation"));
    }
    let expected = count
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("IME allocation bytes"))?;
    budget.scratch(add(held, expected)?)?;
    budget.work(expected as u64 + count as u64 + 1)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| Failure::Allocation)?;
    if values.capacity() > count {
        return Err(Failure::Limit("IME allocation capacity"));
    }
    budget.scratch(add(held, metadata_capacity(&values))?)?;
    budget.record_scratch(add(held, metadata_capacity(&values))?);
    Ok(values)
}
fn ordered_small(
    writes: &[(u64, &[u8])],
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let mut plan = crate::font::fixed::FixedWrites::new();
    for (address, bytes) in writes {
        plan.push(*address, bytes)?;
    }
    plan.apply(guest, budget)
}
fn at(base: u64, offset: u64) -> Result<u64, Failure> {
    base.checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}
fn identity() -> Result<u64, Failure> {
    NEXT_REQUEST
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| Failure::Limit("IME event request identities"))
}

pub(super) struct QueuedEvent {
    pub(super) bytes: [u8; EVENT_BYTES],
    pub(super) text: Option<Vec<u16>>,
}
impl QueuedEvent {
    fn metadata_bytes(&self) -> usize {
        self.text.as_ref().map_or(0, metadata_capacity)
    }
}
struct Keyboard {
    argument: u64,
    handler: u64,
    events: Vec<QueuedEvent>,
}
struct Dispatch {
    event: QueuedEvent,
    argument: u64,
    generation: u64,
}
struct Pending {
    handler: u64,
    runtime: RuntimeObject,
    events: Vec<Dispatch>,
    current: usize,
    request: Option<AdmittedImeCallback>,
}

/// Data for one exact event callback. It grants no guest execution authority;
/// the native owner must attest its current actual return before completion.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedImeCallback {
    identity: u64,
    entry: u64,
    args: [u64; 3],
}
impl AdmittedImeCallback {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn args(&self) -> [u64; 3] {
        self.args
    }
}
impl core::fmt::Debug for AdmittedImeCallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AdmittedImeCallback")
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImeCallbackAction {
    Returned(u64),
    Call(AdmittedImeCallback),
}

/// Closed editor metadata for a future presentation owner. Active state is not
/// evidence that an operating-system keyboard or a visible panel exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImeVisualState {
    pub active: bool,
    pub generation: u64,
    pub revision: u64,
    pub kind: u32,
    pub option: u32,
    pub max_length: u32,
    pub cursor: u32,
}

#[derive(Default)]
pub(crate) struct ImeState {
    keyboard: Option<Keyboard>,
    text: text::TextState,
    pending: Vec<Pending>,
}
impl ImeState {
    fn charge_metadata(&self, budget: &mut Budget) -> Result<(), Failure> {
        budget.work(self.pending.len() as u64 * 4 + 8)?;
        let visits = self.pending.iter().map(|p| p.events.len()).sum::<usize>()
            + self.text.session.as_ref().map_or(0, |s| s.events.len())
            + self.keyboard.as_ref().map_or(0, |s| s.events.len());
        budget.work(visits as u64 + 1)
    }
    pub(crate) fn retained_handle_count(&self) -> usize {
        usize::from(self.keyboard.is_some())
            + self.text.retained_handle_count()
            + self.pending.len()
    }
    fn metadata_bytes(&self) -> usize {
        self.text.metadata_bytes()
            + self
                .keyboard
                .as_ref()
                .map_or(0, |k| metadata_capacity(&k.events))
            + metadata_capacity(&self.pending)
            + self
                .pending
                .iter()
                .map(|p| {
                    metadata_capacity(&p.events)
                        + p.events
                            .iter()
                            .map(|d| d.event.metadata_bytes())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
    fn selection(&self, handler: u64) -> Result<(bool, bool), u64> {
        let text = self
            .text
            .session
            .as_ref()
            .is_some_and(|s| s.handler == handler);
        let keyboard = self.keyboard.as_ref().is_some_and(|s| s.handler == handler);
        if text || keyboard {
            Ok((text, keyboard))
        } else {
            Err(NOT_OPENED)
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
        self.charge_metadata(budget)?;
        let held = self.metadata_bytes();
        budget.scratch(add(held, 0)?)?;
        let a = args.0;
        match service {
            ImeKeyboardOpen => {
                if a[1] == 0 {
                    return Ok(INVALID_ADDRESS);
                }
                if a[0] as u32 == u32::MAX {
                    return Ok(INVALID_USER);
                }
                let handler = budget.u64(guest, at(a[1], 16)?)?;
                if handler == 0 {
                    return Ok(INVALID_HANDLER);
                }
                if budget.u32(guest, a[1])? & !0x3f != 0 {
                    return Ok(INVALID_OPTION);
                }
                let mut reserved = [0; 4];
                budget.read(guest, at(a[1], 4)?, &mut reserved)?;
                if reserved != [0; 4] {
                    return Ok(INVALID_RESERVED);
                }
                let mut reserved = [0; 8];
                budget.read(guest, at(a[1], 24)?, &mut reserved)?;
                if reserved != [0; 8] {
                    return Ok(INVALID_RESERVED);
                }
                if self.keyboard.is_some() {
                    return Ok(BUSY);
                }
                let argument = budget.u64(guest, at(a[1], 8)?)?;
                budget.replacement_handles(0, 1)?;
                let mut events = reserve(1, held, budget)?;
                let mut bytes = [0; EVENT_BYTES];
                bytes[..4].copy_from_slice(&256u32.to_le_bytes());
                bytes[8..12].copy_from_slice(&(a[0] as u32).to_le_bytes());
                events.push(QueuedEvent { bytes, text: None });
                self.keyboard = Some(Keyboard {
                    argument,
                    handler,
                    events,
                });
                budget.added_handle();
                Ok(0)
            }
            ImeKeyboardClose => {
                if self.keyboard.is_none() {
                    return Ok(NOT_OPENED);
                }
                if a[0] as u32 == u32::MAX {
                    return Ok(INVALID_USER);
                }
                self.keyboard = None;
                budget.removed_handle();
                Ok(0)
            }
            ImeKeyboardGetResourceId => {
                if a[1] == 0 {
                    return Ok(INVALID_ADDRESS);
                }
                if a[0] as u32 == u32::MAX {
                    return Ok(INVALID_USER);
                }
                let mut bytes = [0; 24];
                bytes[..4].copy_from_slice(&(a[0] as u32).to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &bytes,
                    }],
                )?;
                Ok(if self.keyboard.is_some() {
                    CONNECTION_FAILED
                } else {
                    NOT_OPENED
                })
            }
            ImeKeyboardGetInfo => {
                if a[1] == 0 {
                    return Ok(INVALID_ADDRESS);
                }
                Ok(if self.keyboard.is_some() {
                    NO_RESOURCE
                } else {
                    NOT_OPENED
                })
            }
            ImeKeyboardSetMode => {
                if a[0] as u32 == u32::MAX {
                    return Ok(INVALID_USER);
                }
                if self.keyboard.is_none() {
                    return Ok(NOT_OPENED);
                }
                Ok(if a[1] as u32 & !0x7f == 0 {
                    0
                } else {
                    INVALID_MODE
                })
            }
            ImeUpdate => {
                let (text, keyboard) = match self.selection(a[0]) {
                    Ok(value) => value,
                    Err(code) => return Ok(code),
                };
                if text && self.text.session.as_ref().is_some_and(|s| s.event_overflow) {
                    self.text.session.as_mut().unwrap().event_overflow = false;
                    return Ok(EVENT_OVERFLOW);
                }
                let count = if text {
                    self.text.session.as_ref().unwrap().events.len()
                } else {
                    0
                } + if keyboard {
                    self.keyboard.as_ref().unwrap().events.len()
                } else {
                    0
                };
                if count == 0 {
                    Ok(0)
                } else {
                    Err(Failure::Unsupported("native IME event continuation"))
                }
            }
            _ => self.text.invoke(service, args, guest, budget, held),
        }
    }

    fn admit(
        &mut self,
        handler: u64,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<ImeCallbackAction, Failure> {
        self.charge_metadata(budget)?;
        let (selected_text, selected_keyboard) = match self.selection(handler) {
            Ok(v) => v,
            Err(code) => return Ok(ImeCallbackAction::Returned(code)),
        };
        if selected_text && self.text.session.as_ref().is_some_and(|s| s.event_overflow) {
            self.text.session.as_mut().unwrap().event_overflow = false;
            return Ok(ImeCallbackAction::Returned(EVENT_OVERFLOW));
        }
        let count = if selected_text {
            self.text.session.as_ref().unwrap().events.len()
        } else {
            0
        } + if selected_keyboard {
            self.keyboard.as_ref().unwrap().events.len()
        } else {
            0
        };
        if count == 0 {
            return Ok(ImeCallbackAction::Returned(0));
        }
        if count > MAX_DISPATCH || self.pending.len() == MAX_PENDING {
            return Err(Failure::Limit("IME pending dispatch"));
        }
        // Host edit/filter/presentation integration is still separate work.
        // Until its text-buffer continuation exists, refuse text payloads
        // before allocating or draining anything, including the first event.
        if selected_text
            && self
                .text
                .session
                .as_ref()
                .unwrap()
                .events
                .iter()
                .any(|e| e.text.is_some())
        {
            return Err(Failure::Unsupported("IME host text payload continuation"));
        }
        budget.replacement_handles(0, 2)?;
        let mut held = self.metadata_bytes();
        let mut replacement = if self.pending.len() == self.pending.capacity() {
            Some(reserve::<Pending>(
                (self.pending.len() + 1)
                    .max(self.pending.capacity() * 2)
                    .min(MAX_PENDING),
                held,
                budget,
            )?)
        } else {
            None
        };
        held = add(held, replacement.as_ref().map_or(0, metadata_capacity))?;
        let mut events = reserve::<Dispatch>(count, held, budget)?;
        held = add(held, metadata_capacity(&events))?;
        if selected_text {
            let session = self.text.session.as_ref().unwrap();
            for queued in &session.events {
                events.push(Dispatch {
                    event: QueuedEvent {
                        bytes: queued.bytes,
                        text: None,
                    },
                    argument: session.arg,
                    generation: session.generation,
                });
            }
        }
        if selected_keyboard {
            let keyboard = self.keyboard.as_ref().unwrap();
            for queued in &keyboard.events {
                events.push(Dispatch {
                    event: QueuedEvent {
                        bytes: queued.bytes,
                        text: None,
                    },
                    argument: keyboard.argument,
                    generation: 0,
                });
            }
        }
        let id = identity()?;
        let runtime = memory.runtime_object(EVENT_BYTES, held, guest, budget, |_, bytes| {
            bytes.copy_from_slice(&events[0].event.bytes);
            Ok(RuntimeDataOutputs::new())
        })?;
        let request = AdmittedImeCallback {
            identity: id,
            entry: handler,
            args: [events[0].argument, runtime.address, 0],
        };
        // All allocations and the initial event publication precede queue drain.
        if selected_text {
            self.text.session.as_mut().unwrap().events.clear();
        }
        if selected_keyboard {
            self.keyboard.as_mut().unwrap().events.clear();
        }
        let pending = Pending {
            handler,
            runtime,
            events,
            current: 0,
            request: Some(request),
        };
        if let Some(mut next) = replacement.take() {
            next.append(&mut self.pending);
            next.push(pending);
            self.pending = next;
        } else {
            self.pending.push(pending);
        }
        budget.added_handle();
        Ok(ImeCallbackAction::Call(request))
    }

    fn advance(
        &mut self,
        pending: &mut Pending,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<ImeCallbackAction, Failure> {
        pending.current += 1;
        while pending.current < pending.events.len() {
            let item = &pending.events[pending.current];
            budget.work(1)?;
            if item.generation != 0
                && self
                    .text
                    .session
                    .as_ref()
                    .is_none_or(|s| s.generation != item.generation)
            {
                pending.current += 1;
                continue;
            }
            memory.check_runtime_object(pending.runtime, budget)?;
            let id = identity()?;
            budget.write(
                guest,
                &[GuestWrite {
                    address: pending.runtime.address,
                    bytes: &item.event.bytes,
                }],
            )?;
            let request = AdmittedImeCallback {
                identity: id,
                entry: pending.handler,
                args: [item.argument, pending.runtime.address, 0],
            };
            pending.request = Some(request);
            return Ok(ImeCallbackAction::Call(request));
        }
        memory.release_runtime_object(pending.runtime, guest, budget)?;
        budget.removed_handle();
        Ok(ImeCallbackAction::Returned(0))
    }
    fn complete(
        &mut self,
        request: &AdmittedImeCallback,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<ImeCallbackAction, Failure> {
        self.charge_metadata(budget)?;
        let index = self
            .pending
            .iter()
            .position(|p| p.request == Some(*request))
            .ok_or(Failure::Unsupported("foreign or consumed IME callback"))?;
        let mut pending = self.pending.remove(index);
        pending.request = None;
        let result = self.advance(&mut pending, memory, guest, budget);
        if !matches!(result, Ok(ImeCallbackAction::Returned(_))) {
            self.pending.insert(index, pending);
        }
        result
    }
}

impl crate::Kernel {
    #[must_use]
    pub fn ime_visual_state(&self) -> ImeVisualState {
        let mut state = ImeVisualState {
            generation: self.ime.text.generation,
            revision: self.ime.text.revision,
            ..ImeVisualState::default()
        };
        if let Some(session) = &self.ime.text.session {
            state.active = true;
            state.kind = session.kind;
            state.option = session.option;
            state.max_length = session.max_length;
            state.cursor = session.cursor;
        }
        state
    }
    /// Admit one complete ImeUpdate. Call requests require actual native body
    /// execution/return attestation before complete_ime_callback is permitted.
    pub fn admit_ime_update(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<ImeCallbackAction, Failure> {
        self.budget.attempt()?;
        self.budget.work(self.ime.pending.len() as u64 + 1)?;
        self.ime
            .admit(args.0[0], &mut self.memory, guest, &mut self.budget)
    }
    /// Consume one exact actual event return. Callback-side changes persist;
    /// an error never replays a consumed event or fabricates successful delivery.
    pub fn complete_ime_callback(
        &mut self,
        request: &AdmittedImeCallback,
        guest: &mut impl GuestAccess,
    ) -> Result<ImeCallbackAction, Failure> {
        self.ime
            .complete(request, &mut self.memory, guest, &mut self.budget)
    }
}
