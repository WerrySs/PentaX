// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/libs/ime.cpp, ime.h and imeCommon.cpp. See NOTICE and upstream provenance.
//! Ordinary text IME calls; host input and filter continuations are separate.

use super::{QueuedEvent, add, metadata_capacity, ordered_small, reserve};
use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};

const BUSY: u64 = 0x80bc_0001;
const NOT_OPENED: u64 = 0x80bc_0002;
const INVALID_TEXT: u64 = 0x80bc_0006;
const INVALID_USER: u64 = 0x80bc_0010;
const INVALID_TYPE: u64 = 0x80bc_0011;
const INVALID_LANGUAGES: u64 = 0x80bc_0012;
const INVALID_ENTER: u64 = 0x80bc_0013;
const INVALID_METHOD: u64 = 0x80bc_0014;
const INVALID_OPTION: u64 = 0x80bc_0015;
const INVALID_MAXIMUM: u64 = 0x80bc_0016;
const INVALID_BUFFER: u64 = 0x80bc_0017;
const INVALID_X: u64 = 0x80bc_0018;
const INVALID_Y: u64 = 0x80bc_0019;
const INVALID_HORIZONTAL: u64 = 0x80bc_001a;
const INVALID_VERTICAL: u64 = 0x80bc_001b;
const INVALID_EXTENDED: u64 = 0x80bc_001c;
const INVALID_WORK: u64 = 0x80bc_0020;
const INVALID_HANDLER: u64 = 0x80bc_0022;
const INVALID_PARAM: u64 = 0x80bc_0030;
const INVALID_ADDRESS: u64 = 0x80bc_0031;
const INVALID_RESERVED: u64 = 0x80bc_0032;

const VALID_OPTIONS: u32 = 0x7bff;
const MULTILINE: u32 = 1;
const PASSWORD: u32 = 4;
const EXPANDED_PREEDIT: u32 = 0x1000;
const OVER_2K: u32 = 0x4000;
const MAX_TEXT: usize = 2048;
const WORK_BYTES: usize = 20 * 1024;

#[derive(Default)]
pub(super) struct TextState {
    pub(super) session: Option<TextSession>,
    pub(super) generation: u64,
    pub(super) revision: u64,
}

pub(super) struct TextSession {
    pub(super) arg: u64,
    pub(super) handler: u64,
    pub(super) work: u64,
    pub(super) input: u64,
    pub(super) max_length: u32,
    pub(super) option: u32,
    // Retain the source editor type for the future host input path. Direct
    // SetText deliberately does not impose the host editor's type filter.
    pub(super) kind: u32,
    pub(super) generation: u64,
    pub(super) text: Vec<u16>,
    pub(super) cursor: u32,
    pub(super) events: Vec<QueuedEvent>,
    pub(super) event_overflow: bool,
}

impl TextState {
    pub(super) fn metadata_bytes(&self) -> usize {
        self.session.as_ref().map_or(0, |session| {
            metadata_capacity(&session.text)
                + metadata_capacity(&session.events)
                + session
                    .events
                    .iter()
                    .filter_map(|event| event.text.as_ref())
                    .map(metadata_capacity)
                    .sum::<usize>()
        })
    }

    pub(super) fn retained_handle_count(&self) -> usize {
        usize::from(self.session.is_some())
    }

    pub(super) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        use Service::*;
        match service {
            ImeParamInit => {
                if args.0[0] != 0 {
                    let mut bytes = [0; 96];
                    bytes[..4].copy_from_slice(&u32::MAX.to_le_bytes());
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: args.0[0],
                            bytes: &bytes,
                        }],
                    )?;
                }
                // The source is void; the bridge returns a deterministic word.
                Ok(0)
            }
            ImeGetPanelSize => panel_size(args, guest, budget),
            ImeOpen => self.open(args.0[0], args.0[1], guest, budget, held),
            ImeClose => {
                if self.session.is_none() {
                    return Ok(NOT_OPENED);
                }
                let revision = next(self.revision)?;
                self.session = None;
                self.revision = revision;
                budget.removed_handle();
                Ok(0)
            }
            ImeSetText => self.set_text(args.0[0], args.0[1] as u32, guest, budget, held),
            ImeSetCaret => {
                let Some(session) = self.session.as_mut() else {
                    return Ok(NOT_OPENED);
                };
                if args.0[0] == 0 {
                    return Ok(INVALID_ADDRESS);
                }
                let index = u32_at(guest, args.0[0], 12, budget)?;
                if index as usize > session.text.len() {
                    return Ok(INVALID_PARAM);
                }
                let revision = next(self.revision)?;
                session.cursor = normalize_cursor(&session.text, index);
                self.revision = revision;
                Ok(0)
            }
            ImeSetTextGeometry => {
                let Some(session) = self.session.as_ref() else {
                    return Ok(NOT_OPENED);
                };
                if args.0[1] == 0 {
                    return Ok(INVALID_ADDRESS);
                }
                let x = f32::from_bits(u32_at(guest, args.0[1], 0, budget)?);
                if !x.is_finite() {
                    return Ok(INVALID_PARAM);
                }
                let y = f32::from_bits(u32_at(guest, args.0[1], 4, budget)?);
                let (max_x, max_y) = screen(session.option);
                if !y.is_finite()
                    || x < 0.0
                    || x >= max_x
                    || y < 0.0
                    || y >= max_y
                    || !matches!(args.0[0] as u32, 2 | 3)
                {
                    return Ok(INVALID_PARAM);
                }
                // Width and height are deliberately neither read nor retained.
                Ok(0)
            }
            _ => Err(Failure::Unsupported("ordinary text IME service")),
        }
    }

    fn open(
        &mut self,
        param: u64,
        extended: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        if self.session.is_some() {
            return Ok(BUSY);
        }
        if param == 0 {
            return Ok(INVALID_ADDRESS);
        }
        let user = u32_at(guest, param, 0, budget)? as i32;
        if user < 0 || user == 255 {
            return Ok(INVALID_USER);
        }
        let kind = u32_at(guest, param, 4, budget)?;
        if kind > 4 {
            return Ok(INVALID_TYPE);
        }
        if u64_at(guest, param, 8, budget)? & !0x0000_0001_ff1f_ffff != 0 {
            return Ok(INVALID_LANGUAGES);
        }
        if u32_at(guest, param, 16, budget)? > 3 {
            return Ok(INVALID_ENTER);
        }
        if u32_at(guest, param, 20, budget)? != 0 {
            return Ok(INVALID_METHOD);
        }
        let option = u32_at(guest, param, 32, budget)?;
        if option & !VALID_OPTIONS != 0 {
            return Ok(INVALID_OPTION);
        }
        let multiline = option & MULTILINE != 0;
        let password = option & PASSWORD != 0;
        if (multiline && password)
            || (multiline && !matches!(kind, 0 | 1))
            || (password && !matches!(kind, 1 | 4))
        {
            return Ok(INVALID_PARAM);
        }
        let max_length = u32_at(guest, param, 36, budget)?;
        if max_length == 0 || max_length as usize > MAX_TEXT {
            return Ok(INVALID_MAXIMUM);
        }
        let input = u64_at(guest, param, 40, budget)?;
        if input == 0 {
            return Ok(INVALID_BUFFER);
        }
        let x = f32::from_bits(u32_at(guest, param, 48, budget)?);
        let (max_x, max_y) = screen(option);
        if !x.is_finite() || x < 0.0 || x >= max_x {
            return Ok(INVALID_X);
        }
        let y = f32::from_bits(u32_at(guest, param, 52, budget)?);
        if !y.is_finite() || y < 0.0 || y >= max_y {
            return Ok(INVALID_Y);
        }
        if u32_at(guest, param, 56, budget)? > 2 {
            return Ok(INVALID_HORIZONTAL);
        }
        if u32_at(guest, param, 60, budget)? > 2 {
            return Ok(INVALID_VERTICAL);
        }
        if !valid_extended(extended, guest, budget)? {
            return Ok(INVALID_EXTENDED);
        }
        let work = u64_at(guest, param, 64, budget)?;
        if work == 0 || work & 3 != 0 {
            return Ok(INVALID_WORK);
        }
        let handler = u64_at(guest, param, 80, budget)?;
        if handler == 0 {
            return Ok(INVALID_HANDLER);
        }
        if !all_zero(guest, param, 88, 8, budget)? {
            return Ok(INVALID_RESERVED);
        }
        let text_span = (u64::from(max_length)
            + if option & EXPANDED_PREEDIT != 0 {
                0x79
            } else {
                0x1f
            })
            * 2;
        if overlap(input, text_span, work, WORK_BYTES as u64) {
            return Ok(INVALID_PARAM);
        }

        let mut text = reserve::<u16>(max_length as usize, held, budget)?;
        let mut terminated = false;
        for index in 0..=max_length {
            let unit = u16_at(guest, input, u64::from(index) * 2, budget)?;
            if unit == 0 {
                terminated = true;
                break;
            }
            if index == max_length {
                break;
            }
            text.push(unit);
        }
        if !terminated || !valid_text(&text, multiline, budget)? {
            return Ok(INVALID_TEXT);
        }
        let arg = u64_at(guest, param, 72, budget)?;
        let generation = next(self.generation)?;
        let revision = next(self.revision)?;
        budget.replacement_handles(0, 1)?;
        let text_held = add(held, metadata_capacity(&text))?;
        let mut events = reserve::<QueuedEvent>(1, text_held, budget)?;
        let staged_held = add(text_held, metadata_capacity(&events))?;
        let mut work_bytes = bytes(WORK_BYTES, staged_held, budget)?;
        let input_bytes = encode(
            &text,
            add(staged_held, metadata_capacity(&work_bytes))?,
            budget,
        )?;
        work_bytes[..input_bytes.len()].copy_from_slice(&input_bytes);
        let writes = [
            GuestWrite {
                address: work,
                bytes: &work_bytes,
            },
            GuestWrite {
                address: input,
                bytes: &input_bytes,
            },
        ];
        // The original re-reads these caller fields after clearing/syncing work.
        // Param is allowed to overlap those stores, so use source-order overlays.
        let event_x = current_u32(guest, field(param, 48)?, &writes, budget)?;
        let event_y = current_u32(guest, field(param, 52)?, &writes, budget)?;
        let event_kind = current_u32(guest, field(param, 4)?, &writes, budget)?;
        let event_option = current_u32(guest, field(param, 32)?, &writes, budget)?;
        let mut event = [0; 72];
        event[8..12].copy_from_slice(&event_x.to_le_bytes());
        event[12..16].copy_from_slice(&event_y.to_le_bytes());
        let (mut width, mut height) = panel(event_kind);
        if event_option & OVER_2K != 0 {
            width *= 2;
            height *= 2;
        }
        event[16..20].copy_from_slice(&width.to_le_bytes());
        event[20..24].copy_from_slice(&height.to_le_bytes());
        events.push(QueuedEvent {
            bytes: event,
            text: None,
        });
        let cursor = text.len() as u32;
        budget.write(guest, &writes)?;
        self.session = Some(TextSession {
            arg,
            handler,
            work,
            input,
            max_length,
            option,
            kind,
            generation,
            text,
            cursor,
            events,
            event_overflow: false,
        });
        self.generation = generation;
        self.revision = revision;
        budget.added_handle();
        Ok(0)
    }

    fn set_text(
        &mut self,
        source: u64,
        length: u32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        let Some(session) = self.session.as_mut() else {
            return Ok(NOT_OPENED);
        };
        if source == 0 {
            return Ok(INVALID_ADDRESS);
        }
        let length = length.min(session.max_length);
        let mut text = reserve::<u16>(length as usize, held, budget)?;
        let mut index = 0;
        while index < length {
            let unit = u16_at(guest, source, u64::from(index) * 2, budget)?;
            if unit == 0 {
                break;
            }
            if session.option & MULTILINE == 0 && matches!(unit, 10 | 13) {
                return Ok(INVALID_TEXT);
            }
            if high(unit) {
                index += 1;
                if index == length {
                    return Ok(INVALID_TEXT);
                }
                let second = u16_at(guest, source, u64::from(index) * 2, budget)?;
                if !low(second) {
                    return Ok(INVALID_TEXT);
                }
                text.push(unit);
                text.push(second);
                index += 1;
                continue;
            }
            if low(unit) {
                return Ok(INVALID_TEXT);
            }
            text.push(unit);
            index += 1;
        }
        let revision = next(self.revision)?;
        let cursor = normalize_cursor(&text, session.cursor);
        let encoded = encode(&text, add(held, metadata_capacity(&text))?, budget)?;
        budget.write(
            guest,
            &[
                GuestWrite {
                    address: session.work,
                    bytes: &encoded,
                },
                GuestWrite {
                    address: session.input,
                    bytes: &encoded,
                },
            ],
        )?;
        session.text = text;
        session.cursor = cursor;
        self.revision = revision;
        Ok(0)
    }
}

fn panel_size(
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    let [param, width, height, ..] = args.0;
    if param == 0 || width == 0 || height == 0 {
        return Ok(INVALID_ADDRESS);
    }
    let kind = u32_at(guest, param, 4, budget)?;
    if kind > 4 {
        return Ok(INVALID_TYPE);
    }
    let option = u32_at(guest, param, 32, budget)?;
    if option & !VALID_OPTIONS != 0 {
        return Ok(INVALID_OPTION);
    }
    let mut payloads = [[0; 4]; 4];
    let addresses = [width, height, width, height];
    payloads[0] = panel(kind).0.to_le_bytes();
    let first = [GuestWrite {
        address: width,
        bytes: &payloads[0],
    }];
    let next_kind = current_u32(guest, field(param, 4)?, &first, budget)?;
    payloads[1] = panel(next_kind).1.to_le_bytes();
    let first_two = [
        GuestWrite {
            address: width,
            bytes: &payloads[0],
        },
        GuestWrite {
            address: height,
            bytes: &payloads[1],
        },
    ];
    let option = current_u32(guest, field(param, 32)?, &first_two, budget)?;
    let count = if option & OVER_2K != 0 {
        payloads[2] = current_u32(guest, width, &first_two, budget)?
            .wrapping_mul(2)
            .to_le_bytes();
        let first_three = [
            GuestWrite {
                address: width,
                bytes: &payloads[0],
            },
            GuestWrite {
                address: height,
                bytes: &payloads[1],
            },
            GuestWrite {
                address: width,
                bytes: &payloads[2],
            },
        ];
        payloads[3] = current_u32(guest, height, &first_three, budget)?
            .wrapping_mul(2)
            .to_le_bytes();
        4
    } else {
        2
    };
    let writes: [(u64, &[u8]); 4] =
        std::array::from_fn(|index| (addresses[index], payloads[index].as_slice()));
    ordered_small(&writes[..count], guest, budget)?;
    Ok(0)
}

fn valid_extended(
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<bool, Failure> {
    if address == 0 {
        return Ok(true);
    }
    let option = u32_at(guest, address, 0, budget)?;
    if option & !0x4fde != 0
        || (option & 0x4000 != 0 && option & 0x80 == 0)
        || u32_at(guest, address, 40, budget)? > 3
        || u32_at(guest, address, 64, budget)? > 7
        || u32_at(guest, address, 68, budget)? & !0x1c00_0003 != 0
    {
        return Ok(false);
    }
    all_zero(guest, address, 72, 60, budget)
}

fn valid_text(text: &[u16], multiline: bool, budget: &mut Budget) -> Result<bool, Failure> {
    budget.work(text.len() as u64)?;
    let mut index = 0;
    while index < text.len() {
        let unit = text[index];
        if !multiline && matches!(unit, 10 | 13) {
            return Ok(false);
        }
        if high(unit) {
            index += 1;
            if index == text.len() || !low(text[index]) {
                return Ok(false);
            }
        } else if low(unit) {
            return Ok(false);
        }
        index += 1;
    }
    Ok(true)
}

fn normalize_cursor(text: &[u16], cursor: u32) -> u32 {
    let mut cursor = (cursor as usize).min(text.len());
    if cursor > 0 && cursor < text.len() && high(text[cursor - 1]) && low(text[cursor]) {
        cursor += 1;
    }
    cursor as u32
}

fn high(unit: u16) -> bool {
    (0xd800..=0xdbff).contains(&unit)
}
fn low(unit: u16) -> bool {
    (0xdc00..=0xdfff).contains(&unit)
}
fn screen(option: u32) -> (f32, f32) {
    if option & OVER_2K != 0 {
        (3840.0, 2160.0)
    } else {
        (1920.0, 1080.0)
    }
}
fn panel(kind: u32) -> (u32, u32) {
    if kind == 4 { (370, 402) } else { (793, 408) }
}
fn next(value: u64) -> Result<u64, Failure> {
    value
        .checked_add(1)
        .ok_or(Failure::Limit("IME generation/revision"))
}
fn field(base: u64, offset: u64) -> Result<u64, Failure> {
    base.checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}

fn bytes(count: usize, held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    let mut output = reserve::<u8>(count, held, budget)?;
    budget.work(count as u64)?;
    output.resize(count, 0);
    Ok(output)
}

fn encode(text: &[u16], held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    let length = text
        .len()
        .checked_add(1)
        .and_then(|n| n.checked_mul(2))
        .ok_or(Failure::Limit("IME UTF16 bytes"))?;
    let mut output = bytes(length, held, budget)?;
    for (target, unit) in output.chunks_exact_mut(2).zip(text) {
        target.copy_from_slice(&unit.to_le_bytes());
    }
    Ok(output)
}

fn all_zero(
    guest: &impl GuestAccess,
    base: u64,
    offset: u64,
    length: u64,
    budget: &mut Budget,
) -> Result<bool, Failure> {
    for index in 0..length {
        let mut byte = [0];
        budget.read(guest, field(base, offset + index)?, &mut byte)?;
        if byte[0] != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

fn u16_at(
    guest: &impl GuestAccess,
    base: u64,
    offset: u64,
    budget: &mut Budget,
) -> Result<u16, Failure> {
    let mut bytes = [0; 2];
    budget.read(guest, field(base, offset)?, &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}
fn u32_at(
    guest: &impl GuestAccess,
    base: u64,
    offset: u64,
    budget: &mut Budget,
) -> Result<u32, Failure> {
    budget.u32(guest, field(base, offset)?)
}
fn u64_at(
    guest: &impl GuestAccess,
    base: u64,
    offset: u64,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    budget.u64(guest, field(base, offset)?)
}

fn overlap(first: u64, first_length: u64, second: u64, second_length: u64) -> bool {
    match (
        first.checked_add(first_length),
        second.checked_add(second_length),
    ) {
        (Some(first_end), Some(second_end)) => first < second_end && second < first_end,
        _ => true,
    }
}

fn current_u32(
    guest: &impl GuestAccess,
    address: u64,
    writes: &[GuestWrite<'_>],
    budget: &mut Budget,
) -> Result<u32, Failure> {
    let mut bytes = [0; 4];
    // Distinct virtual addresses may still alias physical guest backing. The
    // adapter must prove/refuse those aliases even when no virtual span meets.
    budget.read_staged(guest, address, &mut bytes, writes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests;
