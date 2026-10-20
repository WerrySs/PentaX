// SPDX-License-Identifier: GPL-2.0-only
// Adapted from pinned libFont.cpp; see NOTICE and upstream provenance.
//! Owned text/character views and source-ordered caller structures.

use super::fixed::FixedWrites;
use super::writing::WritingState;
use super::*;

mod parser;
pub use parser::{AdmittedFontParser, FontParserAction};

#[cfg(test)]
mod tests;

pub(super) const MAX_CHARACTERS: usize = 65_536;
const CHARACTER_BYTES: usize = 40;
const CHARACTER_START: u64 = 64;

#[derive(Clone, Copy)]
pub(super) struct Character {
    pub(super) font: u64,
    pub(super) code: u32,
    pub(super) order: u64,
    pub(super) index: u32,
}
pub(super) struct FontString {
    pub(super) runtime: RuntimeObject,
    pub(super) chars: Vec<Character>,
    pub(super) form: i32,
    pub(super) terminate_code: u32,
    pub(super) terminate_order: u64,
}
impl FontString {
    pub(super) fn character_address(&self, index: usize) -> u64 {
        self.runtime.address + CHARACTER_START + index as u64 * CHARACTER_BYTES as u64
    }
}

pub(super) fn character(
    strings: &[FontString],
    address: u64,
    memory: &MemoryState,
    budget: &mut Budget,
) -> Result<Option<(usize, usize)>, Failure> {
    if address == 0 {
        return Ok(None);
    }
    budget.work(strings.len() as u64 + 1)?;
    for (owner, string) in strings.iter().enumerate() {
        let Some(offset) = address.checked_sub(string.runtime.address + CHARACTER_START) else {
            continue;
        };
        if offset % CHARACTER_BYTES as u64 == 0 {
            let index = offset / CHARACTER_BYTES as u64;
            if index < string.chars.len() as u64 {
                memory.check_runtime_object(string.runtime, budget)?;
                return Ok(Some((owner, index as usize)));
            }
        }
    }
    Err(Failure::Unsupported("foreign or released font character"))
}

#[derive(Default)]
pub(crate) struct TextState {
    pub(super) strings: Vec<FontString>,
    writing: WritingState,
    pending: Vec<parser::PendingParser>,
}

fn at(address: u64, offset: u64) -> Result<u64, Failure> {
    address
        .checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}
fn word(guest: &impl GuestAccess, address: u64, budget: &mut Budget) -> Result<u64, Failure> {
    budget.u64(guest, address)
}
fn write(
    address: u64,
    bytes: &[u8],
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    budget.write(guest, &[GuestWrite { address, bytes }])
}

#[derive(Clone, Copy)]
struct Setup {
    memory: u64,
    source: u64,
    output: u64,
    font: u64,
    form: i32,
    parser: u64,
}
fn setup(a: [u64; 6], guest: &impl GuestAccess, budget: &mut Budget) -> Result<Setup, Failure> {
    let form = if a[1] == 0 {
        0x10
    } else {
        let value = word(guest, at(a[1], 56)?, budget)?;
        if value == 0 {
            0x10
        } else {
            value as u32 as i32
        }
    };
    let mut font = if a[2] == 0 {
        0
    } else {
        word(guest, at(a[2], 8)?, budget)?
    };
    if font == 0 && a[1] != 0 {
        font = word(guest, at(a[1], 48)?, budget)?;
    }
    let parser = if a[1] == 0 {
        0
    } else {
        word(guest, at(a[1], 32)?, budget)?
    };
    Ok(Setup {
        memory: a[0],
        source: a[1],
        output: a[3],
        font,
        form,
        parser,
    })
}

fn append(
    chars: &mut Vec<Character>,
    font: u64,
    code: u32,
    order: u64,
    held: usize,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let prepared = growth(
        chars,
        1,
        MAX_CHARACTERS,
        add(held, capacity(chars))?,
        budget,
    )?;
    let index = chars.len() as u32;
    insert(
        chars,
        prepared,
        Character {
            font,
            code,
            order,
            index,
        },
    );
    Ok(())
}

fn raw_text(
    source: u64,
    font: u64,
    chars: &mut Vec<Character>,
    guest: &impl GuestAccess,
    held: usize,
    budget: &mut Budget,
) -> Result<Option<u64>, Failure> {
    if source == 0 {
        return Ok(None);
    }
    let mut current = word(guest, at(source, 8)?, budget)?;
    let end = word(guest, at(source, 16)?, budget)?;
    if current == 0 || end == 0 {
        return Ok(None);
    }
    while current < end {
        let start = current;
        let mut byte = [0];
        budget.read(guest, current, &mut byte)?;
        current = at(current, 1)?;
        let first = u32::from(byte[0]);
        let available = end - current;
        let extra = if first & 0xe0 == 0xc0 && available >= 1 {
            1
        } else if first & 0xf0 == 0xe0 && available >= 2 {
            2
        } else if first & 0xf8 == 0xf0 && available >= 3 {
            3
        } else {
            0
        };
        let mut code = match extra {
            1 => first & 0x1f,
            2 => first & 0x0f,
            3 => first & 7,
            _ => first,
        };
        for _ in 0..extra {
            budget.read(guest, current, &mut byte)?;
            current = at(current, 1)?;
            code = (code << 6) | u32::from(byte[0] & 0x3f);
        }
        append(chars, font, code, start, held, budget)?;
    }
    Ok(Some(end))
}

impl TextState {
    pub(super) fn metadata_bytes(&self) -> usize {
        capacity(&self.strings)
            + self
                .strings
                .iter()
                .map(|s| capacity(&s.chars))
                .sum::<usize>()
            + self.writing.metadata_bytes()
            + capacity(&self.pending)
            + self
                .pending
                .iter()
                .map(|p| capacity(&p.chars))
                .sum::<usize>()
    }
    pub(super) fn retained_handle_count(&self) -> usize {
        self.strings.len() + self.writing.retained_handle_count() + self.pending.len()
    }
    pub(super) fn depends_on_font(&self, font: u64) -> bool {
        self.strings
            .iter()
            .any(|s| s.chars.iter().any(|c| c.font == font))
            || self.writing.depends_on_font(font)
    }
    pub(super) fn work_items(&self) -> usize {
        self.strings.len() + self.strings.iter().map(|s| s.chars.len()).sum::<usize>()
    }
    fn string(
        &self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<Option<usize>, Failure> {
        if address == 0 {
            return Ok(None);
        }
        budget.work(self.strings.len() as u64 + 1)?;
        let index = self
            .strings
            .iter()
            .position(|s| s.runtime.address == address)
            .ok_or(Failure::Unsupported("foreign or released font string"))?;
        memory.check_runtime_object(self.strings[index].runtime, budget)?;
        Ok(Some(index))
    }
    // Keep the admitted text result and transaction owners explicit at commit.
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        setup: Setup,
        chars: Vec<Character>,
        terminate_code: u32,
        terminate_order: u64,
        current: Option<u64>,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
        retiring: usize,
    ) -> Result<u64, Failure> {
        let handles = budget.replacement_handles(retiring, 2)?;
        let held = add(held, capacity(&chars))?;
        let prepared = growth(&self.strings, 1, MAX_RECORDS, held, budget)?;
        let held = add(held, prepared.as_ref().map_or(0, capacity))?;
        let length = (CHARACTER_START as usize)
            .checked_add(
                chars
                    .len()
                    .checked_mul(CHARACTER_BYTES)
                    .ok_or(Failure::Limit("font character storage"))?,
            )
            .ok_or(Failure::Limit("font character storage"))?;
        let runtime = memory.runtime_object(length, held, guest, budget, |address, bytes| {
            bytes[..8].copy_from_slice(&setup.memory.to_le_bytes());
            bytes[8..12].copy_from_slice(&setup.form.to_le_bytes());
            bytes[12..16].copy_from_slice(&terminate_code.to_le_bytes());
            bytes[16..24].copy_from_slice(&terminate_order.to_le_bytes());
            for (index, c) in chars.iter().enumerate() {
                let at = CHARACTER_START as usize + index * CHARACTER_BYTES;
                bytes[at..at + 8].copy_from_slice(&address.to_le_bytes());
                bytes[at + 8..at + 12].copy_from_slice(&c.index.to_le_bytes());
                bytes[at + 16..at + 24].copy_from_slice(&c.font.to_le_bytes());
                bytes[at + 24..at + 28].copy_from_slice(&c.code.to_le_bytes());
                bytes[at + 32..at + 40].copy_from_slice(&c.order.to_le_bytes());
            }
            let mut outputs = FixedWrites::new();
            if let Some(current) = current {
                outputs.push(at(setup.source, 24)?, &current.to_le_bytes())?;
            }
            outputs.push(setup.output, &address.to_le_bytes())?;
            outputs.outputs()
        })?;
        insert(
            &mut self.strings,
            prepared,
            FontString {
                runtime,
                chars,
                form: setup.form,
                terminate_code,
                terminate_order,
            },
        );
        budget.replaced_handles(handles);
        Ok(0)
    }

    pub(super) fn invoke(
        &mut self,
        call: FontCall,
        objects: &mut Objects,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        use Service::*;
        let a = call.args.0;
        match call.service {
            FontTextSourceInit => {
                if a[0] == 0 {
                    return Ok(FAILURE);
                }
                let zero = [0; 96];
                let overlay = [GuestWrite {
                    address: a[0],
                    bytes: &zero,
                }];
                let mut size = a[2] as u32;
                if a[1] != 0 && size == 0 {
                    while size < 4096 {
                        let mut byte = [0];
                        budget.read_staged(
                            guest,
                            at(a[1], u64::from(size))?,
                            &mut byte,
                            &overlay,
                        )?;
                        if byte[0] == 0 {
                            break;
                        }
                        size += 1;
                    }
                }
                let end = if a[1] == 0 {
                    0
                } else {
                    at(a[1], u64::from(size))?
                };
                let mut bytes = zero;
                for (offset, value) in [
                    (8, a[1]),
                    (16, end),
                    (24, a[1]),
                    (32, a[3]),
                    (40, a[4]),
                    (56, 0x10),
                    (64, u64::from(size)),
                ] {
                    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
                }
                write(a[0], &bytes, guest, budget)?;
                Ok(0)
            }
            FontTextSourceRewind | FontTextSourceSetDefaultFont | FontTextSourceSetWritingForm => {
                if a[0] == 0 {
                    return Ok(FAILURE);
                }
                let (offset, value) = match call.service {
                    FontTextSourceRewind => (24, word(guest, at(a[0], 8)?, budget)?),
                    FontTextSourceSetDefaultFont => (48, a[1]),
                    _ => (56, a[1] as u32 as i32 as i64 as u64),
                };
                write(at(a[0], offset)?, &value.to_le_bytes(), guest, budget)?;
                Ok(0)
            }
            FontCreateString => {
                if a[3] == 0 {
                    return Ok(FAILURE);
                }
                let setup = setup(a, guest, budget)?;
                if setup.parser != 0 {
                    return Err(Failure::Unsupported("native font parser continuation"));
                }
                let mut chars = Vec::new();
                let current = raw_text(setup.source, setup.font, &mut chars, guest, held, budget)?;
                self.publish(setup, chars, 0, 0, current, memory, guest, budget, held, 0)
            }
            FontDestroyString => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let value = word(guest, a[0], budget)?;
                let Some(index) = self.string(value, memory, budget)? else {
                    write(a[0], &0u64.to_le_bytes(), guest, budget)?;
                    return Ok(0);
                };
                if self.writing.depends_on_string(value) {
                    return Err(Failure::Unsupported("font string retained by writing"));
                }
                memory.release_runtime_object_with_outputs(
                    self.strings[index].runtime,
                    guest,
                    budget,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &0u64.to_le_bytes(),
                    }],
                )?;
                self.strings.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            FontStringGetWritingForm | FontStringGetTerminateCode | FontStringGetTerminateOrder => {
                let index = self.string(a[0], memory, budget)?;
                Ok(match call.service {
                    FontStringGetWritingForm => {
                        index.map_or(0x10, |i| self.strings[i].form as u32 as u64)
                    }
                    FontStringGetTerminateCode => {
                        index.map_or(0, |i| u64::from(self.strings[i].terminate_code))
                    }
                    _ => index.map_or(0, |i| self.strings[i].terminate_order),
                })
            }
            FontStringRefersTextCharacters | FontStringRefersRenderCharacters => {
                let render = call.service == FontStringRefersRenderCharacters;
                let count_out = if render { a[3] } else { a[1] };
                // An explicit start makes the string argument opaque. The last
                // pointer is only an exclusive comparison sentinel, never read.
                if render && a[1] != 0 && count_out == 0 {
                    return Ok(a[1]);
                }
                let first = if render && a[1] != 0 {
                    character(&self.strings, a[1], memory, budget)?
                } else {
                    self.string(a[0], memory, budget)?
                        .filter(|i| !self.strings[*i].chars.is_empty())
                        .map(|i| (i, 0))
                };
                let count = first.map_or(0, |(i, start)| {
                    let string = &self.strings[i];
                    if render {
                        a[2].checked_sub(string.character_address(start))
                            .filter(|offset| offset % CHARACTER_BYTES as u64 == 0)
                            .and_then(|offset| {
                                usize::try_from(offset / CHARACTER_BYTES as u64).ok()
                            })
                            .map(|offset| offset.min(string.chars.len() - start))
                            .unwrap_or(string.chars.len() - start)
                    } else {
                        string.chars.len() - start
                    }
                });
                if count_out != 0 {
                    write(count_out, &(count as u32).to_le_bytes(), guest, budget)?;
                }
                Ok(first.map_or(0, |(i, index)| self.strings[i].character_address(index)))
            }
            FontCharacterGetBidiLevel | FontCharacterGetSyllableStringState => {
                if a[0] == 0 || a[1] == 0 {
                    return Ok(FAILURE);
                }
                // The source checks only nonnull words, never reads character.
                write(a[1], &0u32.to_le_bytes(), guest, budget)?;
                Ok(0)
            }
            FontCharacterLooksFormatCharacters => Ok(0),
            FontCharacterLooksWhiteSpace
            | FontCharacterRefersTextNext
            | FontCharacterRefersTextBack
            | FontCharacterGetTextFontCode
            | FontCharacterGetTextOrder => {
                if a[0] == 0 {
                    return Ok(
                        if matches!(
                            call.service,
                            FontCharacterGetTextFontCode | FontCharacterGetTextOrder
                        ) {
                            FAILURE
                        } else {
                            0
                        },
                    );
                }
                if call.service == FontCharacterGetTextOrder && a[1] == 0 {
                    return Ok(FAILURE);
                }
                if call.service == FontCharacterGetTextFontCode && a[1] == 0 && a[2] == 0 {
                    return Ok(0);
                }
                let (i, index) = character(&self.strings, a[0], memory, budget)?
                    .expect("nonnull owned character");
                let c = self.strings[i].chars[index];
                match call.service {
                    FontCharacterLooksWhiteSpace => {
                        Ok(u64::from(matches!(c.code, 9 | 10 | 13 | 32)))
                    }
                    FontCharacterRefersTextNext => Ok(if index + 1 < self.strings[i].chars.len() {
                        self.strings[i].character_address(index + 1)
                    } else {
                        0
                    }),
                    FontCharacterRefersTextBack => Ok(if index > 0 {
                        self.strings[i].character_address(index - 1)
                    } else {
                        0
                    }),
                    FontCharacterGetTextOrder => {
                        write(a[1], &c.order.to_le_bytes(), guest, budget)?;
                        Ok(0)
                    }
                    _ => {
                        let mut outputs = FixedWrites::new();
                        if a[1] != 0 {
                            outputs.push(a[1], &c.font.to_le_bytes())?;
                        }
                        if a[2] != 0 {
                            outputs.push(a[2], &c.code.to_le_bytes())?;
                        }
                        outputs.apply(guest, budget)?;
                        Ok(0)
                    }
                }
            }
            FontCharactersRefersTextCodes | FontTextCodesStepNext | FontTextCodesStepBack => {
                let (current, term, output) = if call.service == FontCharactersRefersTextCodes {
                    (a[0], a[1], a[2])
                } else {
                    if a[0] == 0 {
                        return Ok(0);
                    }
                    (
                        word(guest, at(a[0], 16)?, budget)?,
                        word(guest, at(a[0], 24)?, budget)?,
                        a[0],
                    )
                };
                if output == 0 || current == 0 {
                    return Ok(if call.service == FontCharactersRefersTextCodes {
                        output
                    } else {
                        0
                    });
                }
                let (i, mut index) = character(&self.strings, current, memory, budget)?
                    .expect("nonnull owned character");
                if call.service == FontTextCodesStepNext {
                    index += 1;
                    if index == self.strings[i].chars.len()
                        || self.strings[i].character_address(index) == term
                    {
                        return Ok(0);
                    }
                } else if call.service == FontTextCodesStepBack {
                    if index == 0 {
                        return Ok(0);
                    }
                    index -= 1;
                }
                let c = self.strings[i].chars[index];
                let mut bytes = [0; 64];
                bytes[..8].copy_from_slice(&c.order.to_le_bytes());
                bytes[8..12].copy_from_slice(&c.code.to_le_bytes());
                bytes[16..24]
                    .copy_from_slice(&self.strings[i].character_address(index).to_le_bytes());
                bytes[24..32].copy_from_slice(&term.to_le_bytes());
                write(output, &bytes, guest, budget)?;
                Ok(output)
            }
            _ => self
                .writing
                .invoke(call, &self.strings, objects, memory, guest, budget, held),
        }
    }
}
