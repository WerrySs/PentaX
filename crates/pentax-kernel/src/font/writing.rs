// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// libFont.cpp:641-699,1882-2135; see NOTICE for exact source attribution.
// Modified 2026: owned guest views and atomic bounded state publication.

use super::*;

const WRITING_BYTES: usize = 224;
const STEP: u64 = 64;
const LETTER: u64 = 136;
const METRICS: u64 = 200;
const LINE_HEADER: usize = 64;
pub(super) const LINE_MAX_STEPS: usize = 4096;
const LINE_BYTES: usize = LINE_HEADER + LINE_MAX_STEPS * 64;

struct Writing {
    owner: u64,
    runtime: RuntimeObject,
    string: u64,
    character_string: u64,
    character: u64,
    font: u64,
}

struct Line {
    runtime: RuntimeObject,
    count: usize,
    cursor: usize,
    metrics: [f32; 6],
}

#[derive(Default)]
pub(super) struct WritingState {
    writings: Vec<Writing>,
    lines: Vec<Line>,
}

fn fixed_metrics(count: u32) -> [f32; 6] {
    let advance = count as f32 * 8.0;
    [advance, 0.0, -12.0, 4.0, 0.0, advance]
}

fn floats<const N: usize>(values: &[f32], bytes: &mut [u8; N]) {
    for (value, output) in values.iter().zip(bytes.chunks_exact_mut(4)) {
        output.copy_from_slice(&value.to_le_bytes());
    }
}

fn line_fields(count: usize, cursor: usize, metrics: &[f32; 6]) -> [u8; 36] {
    let mut output = [0; 36];
    output[..4].copy_from_slice(&(count as u32).to_le_bytes());
    output[4..12].copy_from_slice(&(cursor as u64).to_le_bytes());
    let mut encoded = [0; 24];
    floats(metrics, &mut encoded);
    output[12..].copy_from_slice(&encoded);
    output
}

fn writing_view(address: u64, character: u64) -> [u8; 256] {
    let mut output = [0; 256];
    for (index, value) in [
        address,
        address + STEP,
        character,
        address + LETTER,
        address + METRICS,
    ]
    .into_iter()
    .enumerate()
    {
        output[index * 8..index * 8 + 8].copy_from_slice(&value.to_le_bytes());
    }
    output
}

fn writing_payload(
    string: u64,
    character: u64,
    selected: Option<text::Character>,
    metrics: engine::Metrics,
) -> [u8; WRITING_BYTES] {
    let mut output = [0; WRITING_BYTES];
    output[..8].copy_from_slice(&string.to_le_bytes());
    output[8..16].copy_from_slice(&character.to_le_bytes());
    output[STEP as usize + 8..STEP as usize + 12].copy_from_slice(&8.0_f32.to_le_bytes());
    if let Some(selected) = selected {
        output[STEP as usize + 16..STEP as usize + 24]
            .copy_from_slice(&selected.font.to_le_bytes());
        output[STEP as usize + 24..STEP as usize + 28].copy_from_slice(&1_u32.to_le_bytes());
        output[STEP as usize + 28..STEP as usize + 32]
            .copy_from_slice(&selected.code.to_le_bytes());
        let letter = LETTER as usize;
        output[letter + 16..letter + 20].copy_from_slice(&1_u32.to_le_bytes());
        output[letter + 20..letter + 24].copy_from_slice(&selected.index.to_le_bytes());
        output[letter + 24..letter + 28].copy_from_slice(&1_u32.to_le_bytes());
        for offset in [32, 34, 37, 38] {
            output[letter + offset] = 1;
        }
    }
    output[STEP as usize + 40..STEP as usize + 72].copy_from_slice(&metrics.bytes());
    output[LETTER as usize + 8..LETTER as usize + 12].copy_from_slice(&8.0_f32.to_le_bytes());
    let mut encoded = [0; 24];
    floats(&fixed_metrics(u32::from(selected.is_some())), &mut encoded);
    output[METRICS as usize..METRICS as usize + 24].copy_from_slice(&encoded);
    output
}

fn ordering_space(
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
    destinations: &[u64],
    advance: f32,
) -> Result<(), Failure> {
    let mut writes = fixed::FixedWrites::new();
    for (address, value) in destinations
        .iter()
        .copied()
        .zip([advance, 0.0, advance, 0.0])
    {
        if address != 0 {
            writes.push(address, &value.to_le_bytes())?;
        }
    }
    writes.apply(guest, budget)
}

impl WritingState {
    pub(super) fn metadata_bytes(&self) -> usize {
        capacity(&self.writings) + capacity(&self.lines)
    }
    pub(super) fn retained_handle_count(&self) -> usize {
        self.writings.len() + self.lines.len()
    }
    pub(super) fn depends_on_string(&self, address: u64) -> bool {
        address != 0
            && self
                .writings
                .iter()
                .any(|record| record.string == address || record.character_string == address)
    }
    pub(super) fn depends_on_font(&self, address: u64) -> bool {
        address != 0 && self.writings.iter().any(|record| record.font == address)
    }

    fn writing(
        &self,
        address: u64,
        memory: &MemoryState,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<Option<usize>, Failure> {
        if address == 0 {
            return Ok(None);
        }
        let state = budget.u64(guest, address)?;
        budget.work(self.writings.len() as u64 + 1)?;
        let Some(index) = self
            .writings
            .iter()
            .position(|record| record.runtime.address == state)
        else {
            return Ok(None);
        };
        if self.writings[index].owner != address {
            return Err(Failure::Unsupported("foreign font writing view"));
        }
        memory.check_runtime_object(self.writings[index].runtime, budget)?;
        Ok(Some(index))
    }

    fn line(
        &self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        budget.work(self.lines.len() as u64 + 1)?;
        let index = self
            .lines
            .iter()
            .position(|record| record.runtime.address == address)
            .ok_or(Failure::Unsupported("unowned font writing line"))?;
        memory.check_runtime_object(self.lines[index].runtime, budget)?;
        Ok(index)
    }

    #[allow(clippy::too_many_arguments)]
    fn initialize(
        &mut self,
        a: [u64; 6],
        strings: &[text::FontString],
        objects: &Objects,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        if a[0] == 0 {
            return Ok(FAILURE);
        }
        let _ = self.writing(a[0], memory, guest, budget)?;
        let mut selected = text::character(strings, a[2], memory, budget)?;
        if selected.is_none() && a[1] != 0 {
            budget.work(strings.len() as u64 + 1)?;
            let index = strings
                .iter()
                .position(|string| string.runtime.address == a[1])
                .ok_or(Failure::Unsupported("unowned font writing string"))?;
            memory.check_runtime_object(strings[index].runtime, budget)?;
            if !strings[index].chars.is_empty() {
                selected = Some((index, 0));
            }
        }
        let character = selected.map(|(string, index)| strings[string].chars[index]);
        let character_address = selected.map_or(0, |(string, index)| {
            strings[string].character_address(index)
        });
        let character_string = selected.map_or(0, |(string, _)| strings[string].runtime.address);
        let font_address = character.map_or(0, |character| character.font);
        budget.work(self.writings.len() as u64 + 1)?;
        let previous = self.writings.iter().position(|record| record.owner == a[0]);
        let prepared = if previous.is_none() {
            growth(&self.writings, 1, MAX_RECORDS, held, budget)?
        } else {
            None
        };
        let held = add(held, prepared.as_ref().map_or(0, capacity))?;
        let staged = add(held, 512)?;
        admit(staged, budget)?;
        let (data, height) = if font_address == 0 {
            (None, 16.0)
        } else {
            let font = objects.font(font_address, memory, budget)?;
            (
                if font.data.is_empty() {
                    None
                } else {
                    Some(font.data.as_slice())
                },
                font.scale_h,
            )
        };
        let metrics = engine::metrics(
            data,
            height,
            character.map_or(u32::from(b'?'), |character| character.code),
            staged,
            budget,
        )?;
        let payload = writing_payload(a[1], character_address, character, metrics);
        budget.work((WRITING_BYTES + 256) as u64)?;
        if let Some(index) = previous {
            let runtime = self.writings[index].runtime;
            memory.check_runtime_object(runtime, budget)?;
            let view = writing_view(runtime.address, character_address);
            budget.write(
                guest,
                &[
                    GuestWrite {
                        address: runtime.address,
                        bytes: &payload,
                    },
                    GuestWrite {
                        address: a[0],
                        bytes: &view,
                    },
                ],
            )?;
            self.writings[index] = Writing {
                owner: a[0],
                runtime,
                string: a[1],
                character_string,
                character: character_address,
                font: font_address,
            };
        } else {
            let handles = budget.replacement_handles(0, 2)?;
            let runtime =
                memory.runtime_object(WRITING_BYTES, staged, guest, budget, |address, bytes| {
                    bytes.copy_from_slice(&payload);
                    let mut output = RuntimeDataOutputs::new();
                    output.push(a[0], &writing_view(address, character_address))?;
                    Ok(output)
                })?;
            insert(
                &mut self.writings,
                prepared,
                Writing {
                    owner: a[0],
                    runtime,
                    string: a[1],
                    character_string,
                    character: character_address,
                    font: font_address,
                },
            );
            budget.replaced_handles(handles);
        }
        Ok(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn invoke(
        &mut self,
        call: FontCall,
        strings: &[text::FontString],
        objects: &Objects,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        use Service::*;
        let a = call.args.0;
        if call.service == FontWritingInit {
            return self.initialize(a, strings, objects, memory, guest, budget, held);
        }
        match call.service {
            FontWritingSetMaskInvisible
            | FontWritingRefersRenderStep
            | FontWritingRefersRenderStepCharacter
            | FontWritingGetRenderMetrics => {
                let found = self.writing(a[0], memory, guest, budget)?;
                match call.service {
                    FontWritingSetMaskInvisible => {
                        let Some(index) = found else {
                            return Ok(FAILURE);
                        };
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: self.writings[index].runtime.address + 16,
                                bytes: &(a[1] as u32).to_le_bytes(),
                            }],
                        )?;
                        Ok(0)
                    }
                    FontWritingRefersRenderStep => {
                        Ok(found.map_or(0, |index| self.writings[index].runtime.address + STEP))
                    }
                    FontWritingRefersRenderStepCharacter => {
                        if a[1] != 0 {
                            let pointer = found
                                .map_or(0, |index| self.writings[index].runtime.address + LETTER);
                            budget.write(
                                guest,
                                &[GuestWrite {
                                    address: a[1],
                                    bytes: &pointer.to_le_bytes(),
                                }],
                            )?;
                        }
                        Ok(found.map_or(0, |index| self.writings[index].character))
                    }
                    FontWritingGetRenderMetrics => {
                        let Some(index) = found else {
                            return Ok(FAILURE);
                        };
                        if a[1] == 0 {
                            return Ok(FAILURE);
                        }
                        let mut bytes = [0; 24];
                        budget.read(
                            guest,
                            self.writings[index].runtime.address + METRICS,
                            &mut bytes,
                        )?;
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: a[1],
                                bytes: &bytes,
                            }],
                        )?;
                        Ok(0)
                    }
                    _ => unreachable!(),
                }
            }
            FontCreateWritingLine => {
                if a[3] == 0 {
                    return Ok(FAILURE);
                }
                let prepared = growth(&self.lines, 1, MAX_RECORDS, held, budget)?;
                let held = add(held, prepared.as_ref().map_or(0, capacity))?;
                let handles = budget.replacement_handles(0, 2)?;
                let metrics = fixed_metrics(0);
                let runtime =
                    memory.runtime_object(LINE_BYTES, held, guest, budget, |address, bytes| {
                        bytes[..8].copy_from_slice(&a[0].to_le_bytes());
                        bytes[8..12].copy_from_slice(&(a[1] as u32).to_le_bytes());
                        bytes[12..48].copy_from_slice(&line_fields(0, 0, &metrics));
                        let mut output = RuntimeDataOutputs::new();
                        output.push(a[3], &address.to_le_bytes())?;
                        Ok(output)
                    })?;
                insert(
                    &mut self.lines,
                    prepared,
                    Line {
                        runtime,
                        count: 0,
                        cursor: 0,
                        metrics,
                    },
                );
                budget.replaced_handles(handles);
                Ok(0)
            }
            FontDestroyWritingLine => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let pointer = budget.u64(guest, a[0])?;
                if pointer == 0 {
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: a[0],
                            bytes: &[0; 8],
                        }],
                    )?;
                    return Ok(0);
                }
                let index = self.line(pointer, memory, budget)?;
                memory.release_runtime_object_with_outputs(
                    self.lines[index].runtime,
                    guest,
                    budget,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &[0; 8],
                    }],
                )?;
                self.lines.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            FontWritingLineClear
            | FontWritingLineWritesOrder
            | FontWritingLineGetOrderingSpace
            | FontWritingLineRefersRenderStep
            | FontWritingLineGetRenderMetrics => {
                if a[0] == 0 {
                    return Ok(if call.service == FontWritingLineRefersRenderStep {
                        0
                    } else {
                        FAILURE
                    });
                }
                if call.service == FontWritingLineGetRenderMetrics && a[1] == 0 {
                    return Ok(FAILURE);
                }
                let index = self.line(a[0], memory, budget)?;
                match call.service {
                    FontWritingLineClear => {
                        let metrics = fixed_metrics(0);
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: a[0] + 12,
                                bytes: &line_fields(0, 0, &metrics),
                            }],
                        )?;
                        self.lines[index].count = 0;
                        self.lines[index].cursor = 0;
                        self.lines[index].metrics = metrics;
                        Ok(0)
                    }
                    FontWritingLineWritesOrder => self.append(index, a, guest, budget),
                    FontWritingLineGetOrderingSpace => {
                        ordering_space(guest, budget, &a[1..5], self.lines[index].metrics[0])?;
                        Ok(0)
                    }
                    FontWritingLineRefersRenderStep => {
                        let line = &self.lines[index];
                        if line.cursor >= line.count {
                            return Ok(0);
                        }
                        let pointer = a[0] + LINE_HEADER as u64 + line.cursor as u64 * 64;
                        let next = line.cursor + 1;
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: a[0] + 16,
                                bytes: &(next as u64).to_le_bytes(),
                            }],
                        )?;
                        self.lines[index].cursor = next;
                        Ok(pointer)
                    }
                    FontWritingLineGetRenderMetrics => {
                        let mut bytes = [0; 24];
                        floats(&self.lines[index].metrics, &mut bytes);
                        budget.write(
                            guest,
                            &[GuestWrite {
                                address: a[1],
                                bytes: &bytes,
                            }],
                        )?;
                        Ok(0)
                    }
                    _ => unreachable!(),
                }
            }
            _ => Err(Failure::Unsupported("font writing service")),
        }
    }

    fn append(
        &mut self,
        index: usize,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let line = &self.lines[index];
        if line.count >= LINE_MAX_STEPS {
            return Err(Failure::Limit("font writing line steps"));
        }
        let order = if a[2] == 0 {
            fixed_metrics(1)
        } else {
            let mut bytes = [0; 24];
            budget.read(guest, a[2], &mut bytes)?;
            std::array::from_fn(|index| {
                f32::from_le_bytes(
                    bytes[index * 4..index * 4 + 4]
                        .try_into()
                        .expect("float field"),
                )
            })
        };
        budget.work(128)?;
        let old = line.metrics;
        let mut next = [
            old[0] + order[0],
            old[1] + order[1],
            order[2],
            order[3],
            old[0] + order[4],
            old[0] + order[5],
        ];
        if line.count != 0 {
            // std::min/max source comparison order preserves a NaN left value.
            next[2] = if order[2] < old[2] { order[2] } else { old[2] };
            next[3] = if old[3] < order[3] { order[3] } else { old[3] };
            next[4] = if next[4] < old[4] { next[4] } else { old[4] };
            next[5] = if old[5] < next[5] { next[5] } else { old[5] };
        }
        let mut step = [0; 64];
        step[..4].copy_from_slice(&old[0].to_le_bytes());
        step[8..12].copy_from_slice(&order[0].to_le_bytes());
        step[12..16].copy_from_slice(&order[1].to_le_bytes());
        step[24..32].copy_from_slice(&a[3].to_le_bytes());
        let mut bytes = [0; 24];
        floats(&order, &mut bytes);
        step[40..].copy_from_slice(&bytes);
        let address = line.runtime.address + LINE_HEADER as u64 + line.count as u64 * 64;
        budget.write(
            guest,
            &[
                GuestWrite {
                    address,
                    bytes: &step,
                },
                GuestWrite {
                    address: line.runtime.address + 12,
                    bytes: &line_fields(line.count + 1, 0, &next),
                },
            ],
        )?;
        self.lines[index].count += 1;
        self.lines[index].cursor = 0;
        self.lines[index].metrics = next;
        Ok(0)
    }
}

#[cfg(test)]
mod tests;
