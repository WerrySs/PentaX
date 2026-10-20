// SPDX-License-Identifier: GPL-2.0-only
//! Data-only parser continuations; native execution/return proof belongs to core.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_PENDING: usize = 32;
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;

/// One exact pending parser request. It carries guest words, not execution
/// authority or evidence of a callback return. Only the native owner may attest
/// a return before invoking Kernel::complete_font_parser.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedFontParser {
    identity: u64,
    entry: u64,
    args: [u64; 3],
    stack_pointer: u64,
}
impl std::fmt::Debug for AdmittedFontParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedFontParser").finish_non_exhaustive()
    }
}
impl AdmittedFontParser {
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn args(&self) -> [u64; 3] {
        self.args
    }
    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontParserAction {
    Returned(u64),
    Call(AdmittedFontParser),
}

pub(super) struct PendingParser {
    setup: Setup,
    pub(super) chars: Vec<Character>,
    request: Option<AdmittedFontParser>,
    stack_pointer: u64,
    iterations: usize,
}

fn next_request(
    setup: Setup,
    stack_pointer: u64,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<AdmittedFontParser, Failure> {
    let scratch = stack_pointer
        .checked_sub(64)
        .ok_or(GuestError::AddressOverflow)?;
    let zeros = [0; 40];
    budget.work(8192)?;
    guest.preflight_write(setup.output, 8)?;
    guest.preflight_write(scratch, 40)?;
    // The source tests parser once before its loop, then rereads it AFTER
    // initializing this iteration's scratch. Preserve that ordering and refuse
    // ambiguous distinct physical aliases before publishing any scratch byte.
    let mut bytes = [0; 8];
    budget.read_staged(
        guest,
        at(setup.source, 32)?,
        &mut bytes,
        &[GuestWrite {
            address: scratch,
            bytes: &zeros,
        }],
    )?;
    let entry = u64::from_le_bytes(bytes);
    if entry == 0 {
        return Err(Failure::Unsupported("removed font parser target"));
    }
    let identity = NEXT_REQUEST
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| Failure::Limit("font parser request identities"))?;
    let request = AdmittedFontParser {
        identity,
        entry,
        args: [setup.source, scratch, scratch + 8],
        stack_pointer,
    };
    write(scratch, &zeros, guest, budget)?;
    Ok(request)
}

impl TextState {
    fn begin_parser(
        &mut self,
        args: CallArgs,
        stack_pointer: Option<u64>,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<FontParserAction, Failure> {
        if args.0[3] == 0 {
            return Ok(FontParserAction::Returned(FAILURE));
        }
        let setup = setup(args.0, guest, budget)?;
        if setup.parser == 0 {
            let mut chars = Vec::new();
            let current = raw_text(setup.source, setup.font, &mut chars, guest, held, budget)?;
            return self
                .publish(setup, chars, 0, 0, current, memory, guest, budget, held, 0)
                .map(FontParserAction::Returned);
        }
        let stack_pointer =
            stack_pointer.ok_or(Failure::Unsupported("captured font parser stack"))?;
        // Admit enough handles for the eventual string+mapping before callbacks;
        // the live pending record itself consumes only one logical handle.
        budget.replacement_handles(0, 2)?;
        let prepared = growth(&self.pending, 1, MAX_PENDING, held, budget)?;
        let held = add(held, prepared.as_ref().map_or(0, capacity))?;
        let expected = MAX_CHARACTERS * core::mem::size_of::<Character>();
        admit(add(held, expected)?, budget)?;
        budget.work(expected as u64)?;
        let mut chars = Vec::new();
        chars
            .try_reserve_exact(MAX_CHARACTERS)
            .map_err(|_| Failure::Allocation)?;
        if chars.capacity() > MAX_CHARACTERS {
            return Err(Failure::Limit("font parser character capacity"));
        }
        admit(add(held, capacity(&chars))?, budget)?;
        let request = next_request(setup, stack_pointer, guest, budget)?;
        insert(
            &mut self.pending,
            prepared,
            PendingParser {
                setup,
                chars,
                request: Some(request),
                stack_pointer,
                iterations: 0,
            },
        );
        budget.added_handle();
        Ok(FontParserAction::Call(request))
    }

    // The callback result and all live publication owners cross one boundary.
    #[allow(clippy::too_many_arguments)]
    fn compact_and_publish(
        &mut self,
        pending: &PendingParser,
        code: u32,
        order: u64,
        current: Option<u64>,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<FontParserAction, Failure> {
        // The maximum-sized admission buffer is temporary. Retain only actual
        // character capacity in the final string; allocator failure cannot undo
        // callback-side writes and never causes a fabricated successful result.
        let expected = pending.chars.len() * core::mem::size_of::<Character>();
        admit(add(held, expected)?, budget)?;
        budget.work(expected as u64 + 1)?;
        let mut chars = Vec::new();
        chars
            .try_reserve_exact(pending.chars.len())
            .map_err(|_| Failure::Allocation)?;
        if chars.capacity() > MAX_CHARACTERS {
            return Err(Failure::Limit("font character capacity"));
        }
        admit(add(held, capacity(&chars))?, budget)?;
        chars.extend_from_slice(&pending.chars);
        self.publish(
            pending.setup,
            chars,
            code,
            order,
            current,
            memory,
            guest,
            budget,
            held,
            1,
        )
        .map(FontParserAction::Returned)
    }

    fn advance_parser(
        &mut self,
        pending: &mut PendingParser,
        value: u64,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<FontParserAction, Failure> {
        let scratch = pending.stack_pointer - 64;
        let status = value as u32 as i32;
        pending.iterations += 1;
        if status == 1 {
            let font = word(guest, scratch + 8, budget)?;
            let code = budget.u32(guest, scratch + 16)?;
            let order = word(guest, scratch, budget)?;
            let base = held
                .checked_sub(capacity(&pending.chars))
                .ok_or(Failure::Limit("font pending capacity"))?;
            append(
                &mut pending.chars,
                if font == 0 { pending.setup.font } else { font },
                code,
                order,
                base,
                budget,
            )?;
            if pending.iterations == MAX_CHARACTERS {
                // The pinned source explicitly succeeds after its finite cap,
                // retaining the actual accumulated characters and default term.
                return self.compact_and_publish(pending, 0, 0, None, memory, guest, budget, held);
            }
            let request = next_request(pending.setup, pending.stack_pointer, guest, budget)?;
            pending.request = Some(request);
            return Ok(FontParserAction::Call(request));
        }
        if status == 0 {
            let code = budget.u32(guest, scratch + 16)?;
            let order = word(guest, scratch, budget)?;
            let order = if order == 0 {
                word(guest, scratch + 8, budget)?
            } else {
                order
            };
            let current = if pending.chars.is_empty()
                && word(guest, at(pending.setup.source, 8)?, budget)?
                    != word(guest, at(pending.setup.source, 16)?, budget)?
            {
                let base = held
                    .checked_sub(capacity(&pending.chars))
                    .ok_or(Failure::Limit("font pending capacity"))?;
                raw_text(
                    pending.setup.source,
                    pending.setup.font,
                    &mut pending.chars,
                    guest,
                    base,
                    budget,
                )?
            } else {
                None
            };
            return self
                .compact_and_publish(pending, code, order, current, memory, guest, budget, held);
        }
        let error = if status == -1 {
            let code = budget.u32(guest, scratch + 16)?;
            if code == 0 { FAILURE } else { u64::from(code) }
        } else {
            FAILURE
        };
        write(pending.setup.output, &0u64.to_le_bytes(), guest, budget)?;
        budget.removed_handle();
        Ok(FontParserAction::Returned(error))
    }

    fn finish_parser(
        &mut self,
        request: &AdmittedFontParser,
        value: u64,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<FontParserAction, Failure> {
        budget.work(self.pending.len() as u64 + 1)?;
        let index = self
            .pending
            .iter()
            .position(|p| p.request == Some(*request))
            .ok_or(Failure::Unsupported(
                "foreign or consumed font parser request",
            ))?;
        let mut pending = self.pending.remove(index);
        // Consume before every fallible read/output. Failed work remains owned
        // and counted, but cannot be replayed as a second callback return.
        pending.request = None;
        let action = self.advance_parser(&mut pending, value, memory, guest, budget, held);
        if !matches!(action, Ok(FontParserAction::Returned(_))) {
            self.pending.insert(index, pending);
        }
        action
    }
}

impl crate::Kernel {
    /// Admit the complete FontCreateString call once. A Call result is data,
    /// not authority to execute a guest address or evidence that it ran.
    pub fn admit_font_string(
        &mut self,
        args: CallArgs,
        stack_pointer: Option<u64>,
        guest: &mut impl GuestAccess,
    ) -> Result<FontParserAction, Failure> {
        self.budget.attempt()?;
        self.budget
            .work(self.fonts.retained_handle_count() as u64 * 32 + 1)?;
        let held = self.fonts.metadata_bytes();
        admit(held, &mut self.budget)?;
        self.fonts.text.begin_parser(
            args,
            stack_pointer,
            &mut self.memory,
            guest,
            &mut self.budget,
            held,
        )
    }

    /// Consume one exact callback result AFTER the native owner validates an
    /// actual return. Preserve real callback writes on all later failures.
    pub fn complete_font_parser(
        &mut self,
        request: &AdmittedFontParser,
        value: u64,
        guest: &mut impl GuestAccess,
    ) -> Result<FontParserAction, Failure> {
        self.budget
            .work(self.fonts.retained_handle_count() as u64 * 32 + 1)?;
        let held = self.fonts.metadata_bytes();
        admit(held, &mut self.budget)?;
        self.fonts.text.finish_parser(
            request,
            value,
            &mut self.memory,
            guest,
            &mut self.budget,
            held,
        )
    }
}
