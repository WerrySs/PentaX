// SPDX-License-Identifier: GPL-2.0-only
// Modified 2026 for PentaX: owned glyph mappings and transactional Rust output.
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de
// src/libs/libFont.cpp, blob c3b58f805f0781105a9a7cf216bf4750b3e57d5e.
// See NOTICE for the retained source attribution and deliberate rasterizer change.

use super::{
    FontCall, Objects, add, capacity,
    engine::{self, GlyphImage, Metrics},
    growth, insert,
};
use crate::{
    Failure, GuestAccess, GuestError, GuestWrite, RuntimeDataOutputs, Service,
    budget::Budget,
    memory::{MemoryState, RuntimeObject},
};

const IMAGE_OFFSET: u64 = 64;
const OBJECT_BYTES: usize = 64 + engine::IMAGE_BYTES;
const MAX_SPANS: usize = 16_512;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

fn scratch_buffer(length: usize, held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    super::admit(add(held, length)?, budget)?;
    let bytes = budget.buffer_held(length, held)?;
    super::admit(add(held, bytes.capacity())?, budget)?;
    Ok(bytes)
}

struct Glyph {
    runtime: RuntimeObject,
    attribute: i32,
}
#[derive(Default)]
pub(super) struct RenderState {
    glyphs: Vec<Glyph>,
}

struct Span {
    address: u64,
    bytes: Vec<u8>,
}
struct Plan {
    spans: Vec<Span>,
    external: usize,
}
impl Plan {
    fn new(external: usize, budget: &mut Budget) -> Result<Self, Failure> {
        let expected = MAX_SPANS * std::mem::size_of::<Span>();
        budget.scratch(add(external, expected)?)?;
        let mut spans = Vec::new();
        spans
            .try_reserve_exact(MAX_SPANS)
            .map_err(|_| Failure::Allocation)?;
        budget.scratch(add(
            external,
            spans.capacity() * std::mem::size_of::<Span>(),
        )?)?;
        Ok(Self { spans, external })
    }
    fn held(&self) -> Result<usize, Failure> {
        let mut result = add(self.external, capacity(&self.spans))?;
        for span in &self.spans {
            result = add(result, span.bytes.capacity())?;
        }
        Ok(result)
    }
    // Normalize each source-ordered store. Connected intervals have no holes,
    // so preservation never requires reading unwritten destination bytes.
    fn store(&mut self, address: u64, bytes: &[u8], budget: &mut Budget) -> Result<(), Failure> {
        if bytes.is_empty() {
            return Ok(());
        }
        let end = address
            .checked_add(bytes.len() as u64)
            .ok_or(GuestError::AddressOverflow)?;
        budget.work(self.spans.len() as u64 * 4 + bytes.len() as u64 + 1)?;
        let begin = self
            .spans
            .partition_point(|span| span.address + (span.bytes.len() as u64) < address);
        let mut last = begin;
        let mut merged_start = address;
        let mut merged_end = end;
        while last < self.spans.len() && self.spans[last].address <= merged_end {
            merged_start = merged_start.min(self.spans[last].address);
            merged_end =
                merged_end.max(self.spans[last].address + self.spans[last].bytes.len() as u64);
            last += 1;
        }
        let length = usize::try_from(merged_end - merged_start)
            .map_err(|_| Failure::Limit("font output bytes"))?;
        if length > MAX_OUTPUT || self.spans.len() - (last - begin) >= MAX_SPANS {
            return Err(Failure::Limit("font output spans"));
        }
        let mut next = scratch_buffer(length, self.held()?, budget)?;
        for span in &self.spans[begin..last] {
            let at = (span.address - merged_start) as usize;
            next[at..at + span.bytes.len()].copy_from_slice(&span.bytes);
        }
        let at = (address - merged_start) as usize;
        next[at..at + bytes.len()].copy_from_slice(bytes);
        drop(self.spans.drain(begin..last));
        self.spans.insert(
            begin,
            Span {
                address: merged_start,
                bytes: next,
            },
        );
        if self
            .spans
            .iter()
            .map(|span| span.bytes.len())
            .sum::<usize>()
            > MAX_OUTPUT
        {
            return Err(Failure::Limit("font output bytes"));
        }
        Ok(())
    }
    fn read(
        &self,
        guest: &impl GuestAccess,
        address: u64,
        bytes: &mut [u8],
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        if self.spans.len() > 16 {
            return Err(Failure::Limit("font staged header spans"));
        }
        let mut writes = [GuestWrite {
            address: 0,
            bytes: &[],
        }; 16];
        for (write, span) in writes.iter_mut().zip(&self.spans) {
            *write = GuestWrite {
                address: span.address,
                bytes: &span.bytes,
            };
        }
        budget.read_staged(guest, address, bytes, &writes[..self.spans.len()])
    }
    fn known(&self, address: u64, length: usize) -> Result<&[u8], Failure> {
        let end = address
            .checked_add(length as u64)
            .ok_or(GuestError::AddressOverflow)?;
        for span in &self.spans {
            if address >= span.address && end <= span.address + span.bytes.len() as u64 {
                let at = (address - span.address) as usize;
                return Ok(&span.bytes[at..at + length]);
            }
        }
        Err(Failure::Unsupported("font image staging identity"))
    }
    fn publish(self, guest: &mut impl GuestAccess, budget: &mut Budget) -> Result<(), Failure> {
        let expected = self.spans.len() * std::mem::size_of::<GuestWrite<'_>>();
        // The native adapter needs an equally sized descriptor conversion for
        // transactions above its four stack outputs. Admit both live tables.
        let adapter = if self.spans.len() > 4 { expected } else { 0 };
        budget.scratch(add(add(self.held()?, expected)?, adapter)?)?;
        let mut writes = Vec::new();
        writes
            .try_reserve_exact(self.spans.len())
            .map_err(|_| Failure::Allocation)?;
        budget.scratch(add(
            self.held()?,
            add(
                writes.capacity() * std::mem::size_of::<GuestWrite<'_>>(),
                adapter,
            )?,
        )?)?;
        for span in &self.spans {
            writes.push(GuestWrite {
                address: span.address,
                bytes: &span.bytes,
            });
        }
        budget.write(guest, &writes)
    }
}

fn at(address: u64, offset: u64) -> Result<u64, Failure> {
    address
        .checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}
fn floats(call: FontCall) -> Result<[f32; 2], Failure> {
    let vectors = call
        .vector_args
        .ok_or(Failure::Unsupported("font floating arguments unavailable"))?;
    Ok([
        f32::from_le_bytes(
            vectors[0][..4]
                .try_into()
                .map_err(|_| Failure::Unsupported("font float capture"))?,
        ),
        f32::from_le_bytes(
            vectors[1][..4]
                .try_into()
                .map_err(|_| Failure::Unsupported("font float capture"))?,
        ),
    ])
}
fn integer(value: f32) -> Result<i32, Failure> {
    if !value.is_finite() || !(i32::MIN as f32..i32::MAX as f32).contains(&value) {
        return Err(Failure::Unsupported("font coordinate conversion"));
    }
    Ok(value as i32)
}
fn u32_read(
    plan: &Plan,
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<u32, Failure> {
    let mut bytes = [0; 4];
    plan.read(guest, address, &mut bytes, budget)?;
    Ok(u32::from_le_bytes(bytes))
}
fn u64_read(
    plan: &Plan,
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    let mut bytes = [0; 8];
    plan.read(guest, address, &mut bytes, budget)?;
    Ok(u64::from_le_bytes(bytes))
}
fn metric(
    plan: &Plan,
    address: u64,
    metrics: Metrics,
    index: usize,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<f32, Failure> {
    if address == 0 {
        return Ok(metrics.values[index]);
    }
    Ok(f32::from_bits(u32_read(
        plan,
        at(address, index as u64 * 4)?,
        guest,
        budget,
    )?))
}
fn trans(image: &GlyphImage, address: u64) -> [u8; 20] {
    let mut bytes = [0; 20];
    bytes[..8].copy_from_slice(&address.to_le_bytes());
    bytes[8..12].copy_from_slice(&(engine::STRIDE as u32).to_le_bytes());
    bytes[12..16].copy_from_slice(&image.width.to_le_bytes());
    bytes[16..20].copy_from_slice(&image.height.to_le_bytes());
    bytes
}

impl RenderState {
    pub(super) fn metadata_bytes(&self) -> usize {
        capacity(&self.glyphs)
    }
    pub(super) fn retained_handle_count(&self) -> usize {
        self.glyphs.len()
    }
    // Generated glyphs retain complete copied data and never dereference the
    // originating font in any source-registered glyph operation.
    pub(super) fn depends_on_font(&self, _font: u64) -> bool {
        false
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
        let a = call.args.0;
        use Service::*;
        match call.service {
            FontGetCharGlyphMetrics | FontGetRenderCharGlyphMetrics => {
                if a[2] == 0 {
                    return Ok(0);
                }
                let value = if a[0] == 0 {
                    engine::metrics(None, 16.0, a[1] as u32, held, budget)?
                } else {
                    let font = objects.font(a[0], memory, budget)?;
                    engine::metrics(Some(&font.data), font.scale_h, a[1] as u32, held, budget)?
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[2],
                        bytes: &value.bytes(),
                    }],
                )?;
                Ok(0)
            }
            FontGetHorizontalLayout | FontGetVerticalLayout => {
                if a[0] == 0 || a[1] == 0 {
                    return Ok(u64::from(u32::MAX));
                }
                let values = if call.service == FontGetVerticalLayout {
                    [0.0, 16.0, 16.0]
                } else {
                    let font = objects.font(a[0], memory, budget)?;
                    engine::horizontal_layout(Some(&font.data), font.scale_h, held, budget)?
                };
                let mut bytes = [0; 12];
                for (index, value) in values.iter().enumerate() {
                    bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            FontRenderSurfaceInit => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let mut bytes = [0; 128];
                bytes[..8].copy_from_slice(&a[1].to_le_bytes());
                bytes[8..12].copy_from_slice(&(a[2] as u32).to_le_bytes());
                bytes[12] = a[3] as u8;
                let width = (a[4] as u32 as i32).max(0) as u32;
                let height = (a[5] as u32 as i32).max(0) as u32;
                bytes[16..20].copy_from_slice(&width.to_le_bytes());
                bytes[20..24].copy_from_slice(&height.to_le_bytes());
                bytes[32..36].copy_from_slice(&width.to_le_bytes());
                bytes[36..40].copy_from_slice(&height.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            FontRenderSurfaceSetScissor => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let mut bytes = [0; 16];
                for (index, value) in a[1..5].iter().enumerate() {
                    bytes[index * 4..index * 4 + 4].copy_from_slice(&(*value as u32).to_le_bytes());
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: at(a[0], 24)?,
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            FontGenerateCharGlyph => {
                if a[0] == 0 || a[3] == 0 {
                    return Ok(u64::from(u32::MAX));
                }
                let font = objects.font(a[0], memory, budget)?;
                let image =
                    engine::rasterize(Some(&font.data), font.scale_h, a[1] as u32, held, budget)?;
                let staged = add(held, image.metadata_bytes())?;
                budget.replacement_handles(0, 2)?;
                let prepared = growth(&self.glyphs, 1, super::MAX_RECORDS, staged, budget)?;
                let staged = add(staged, prepared.as_ref().map_or(0, capacity))?;
                let runtime =
                    memory.runtime_object(OBJECT_BYTES, staged, guest, budget, |base, bytes| {
                        bytes[..20].copy_from_slice(&trans(&image, base + IMAGE_OFFSET));
                        bytes[24..56].copy_from_slice(&image.metrics.bytes());
                        bytes[IMAGE_OFFSET as usize..OBJECT_BYTES].copy_from_slice(&image.pixels);
                        let mut outputs = RuntimeDataOutputs::new();
                        outputs.push(a[3], &base.to_le_bytes())?;
                        Ok(outputs)
                    })?;
                insert(
                    &mut self.glyphs,
                    prepared,
                    Glyph {
                        runtime,
                        attribute: 0,
                    },
                );
                budget.added_handle();
                Ok(0)
            }
            FontDeleteGlyph => {
                if a[1] == 0 {
                    return Ok(0);
                }
                let token = budget.u64(guest, a[1])?;
                budget.work(self.glyphs.len() as u64 + 1)?;
                if let Some(index) = self
                    .glyphs
                    .iter()
                    .position(|glyph| glyph.runtime.address == token)
                {
                    memory.release_runtime_object_with_outputs(
                        self.glyphs[index].runtime,
                        guest,
                        budget,
                        &[GuestWrite {
                            address: a[1],
                            bytes: &0_u64.to_le_bytes(),
                        }],
                    )?;
                    self.glyphs.remove(index);
                    budget.removed_handle();
                } else {
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: a[1],
                            bytes: &0_u64.to_le_bytes(),
                        }],
                    )?;
                }
                Ok(0)
            }
            FontGlyphDefineAttribute => {
                budget.work(self.glyphs.len() as u64 + 1)?;
                let index = self
                    .glyphs
                    .iter()
                    .position(|glyph| glyph.runtime.address == a[0]);
                if let Some(index) = index {
                    memory.check_runtime_object(self.glyphs[index].runtime, budget)?;
                }
                let old = index.map_or(0, |index| self.glyphs[index].attribute);
                if a[2] != 0 {
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: a[2],
                            bytes: &old.to_le_bytes(),
                        }],
                    )?;
                }
                if let Some(index) = index {
                    self.glyphs[index].attribute = a[1] as u32 as i32;
                }
                Ok(0)
            }
            FontRenderCharGlyphImageHorizontal => {
                if a[0] == 0 {
                    return Ok(u64::from(u32::MAX));
                }
                let xy = floats(call)?;
                let font = objects.font(a[0], memory, budget)?;
                let image =
                    engine::rasterize(Some(&font.data), font.scale_h, a[1] as u32, held, budget)?;
                let base = font.runtime.address;
                let mut plan = Plan::new(add(held, image.metadata_bytes())?, budget)?;
                plan.store(base + IMAGE_OFFSET, &image.pixels, budget)?;
                plan.store(base, &trans(&image, base + IMAGE_OFFSET), budget)?;
                if a[3] != 0 {
                    plan.store(a[3], &image.metrics.bytes(), budget)?;
                }
                if a[4] != 0 {
                    result(&mut plan, &image, base, a[2], xy, a[3], a[4], guest, budget)?;
                }
                draw(&mut plan, &image, base, a[2], xy, a[3], guest, budget)?;
                plan.publish(guest, budget)?;
                Ok(0)
            }
            _ => Err(Failure::Unsupported("font render service")),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn result(
    plan: &mut Plan,
    image: &GlyphImage,
    base: u64,
    surface: u64,
    xy: [f32; 2],
    metrics: u64,
    output: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    let x = integer(xy[0] + metric(plan, metrics, image.metrics, 2, guest, budget)?)?.max(0) as u32;
    let y = integer(xy[1] - metric(plan, metrics, image.metrics, 3, guest, budget)?)?.max(0) as u32;
    plan.store(output, &base.to_le_bytes(), budget)?;
    let buffer = if surface == 0 {
        0
    } else {
        u64_read(plan, surface, guest, budget)?
    };
    plan.store(at(output, 8)?, &buffer.to_le_bytes(), budget)?;
    let pitch = if surface == 0 {
        0
    } else {
        (u32_read(plan, at(surface, 8)?, guest, budget)? as i32).max(0) as u32
    };
    plan.store(at(output, 16)?, &pitch.to_le_bytes(), budget)?;
    let mut pixel = [0];
    if surface != 0 {
        plan.read(guest, at(surface, 12)?, &mut pixel, budget)?;
    }
    plan.store(at(output, 20)?, &[(pixel[0] as i8).max(0) as u8, 0], budget)?;
    for (offset, value) in [(24, x), (28, y)] {
        plan.store(at(output, offset)?, &value.to_le_bytes(), budget)?;
    }
    for (offset, source) in [(32, 12), (36, 16)] {
        let value = u32_read(plan, at(base, source)?, guest, budget)?;
        plan.store(at(output, offset)?, &value.to_le_bytes(), budget)?;
    }
    plan.store(at(output, 40)?, &0_f32.to_le_bytes(), budget)?;
    for (offset, index) in [(44, 3), (48, 4), (52, 4)] {
        let value = metric(plan, metrics, image.metrics, index, guest, budget)?;
        plan.store(at(output, offset)?, &value.to_le_bytes(), budget)?;
    }
    for (offset, source) in [(56, 12), (60, 16)] {
        let value = u32_read(plan, at(base, source)?, guest, budget)?;
        plan.store(at(output, offset)?, &value.to_le_bytes(), budget)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw(
    plan: &mut Plan,
    image: &GlyphImage,
    base: u64,
    surface: u64,
    xy: [f32; 2],
    metrics: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    if surface == 0 {
        return Ok(());
    }
    // Read only fields actually consumed, after earlier output overlays.
    let buffer = u64_read(plan, surface, guest, budget)?;
    if buffer == 0 {
        return Ok(());
    }
    let width = u32_read(plan, at(surface, 16)?, guest, budget)? as i32;
    if width <= 0 {
        return Ok(());
    }
    let height = u32_read(plan, at(surface, 20)?, guest, budget)? as i32;
    if height <= 0 {
        return Ok(());
    }
    let pitch = u32_read(plan, at(surface, 8)?, guest, budget)? as i32;
    if pitch <= 0 {
        return Ok(());
    }
    let mut pixel = [0];
    plan.read(guest, at(surface, 12)?, &mut pixel, budget)?;
    let pixel = i32::from(pixel[0] as i8);
    if pixel <= 0 {
        return Ok(());
    }
    let start_x = integer(xy[0] + metric(plan, metrics, image.metrics, 2, guest, budget)?)?.max(0);
    let start_y = integer(xy[1] - metric(plan, metrics, image.metrics, 3, guest, budget)?)?.max(0);
    let mut descriptor = [0; 20];
    plan.read(guest, base, &mut descriptor, budget)?;
    if u64::from_le_bytes(
        descriptor[..8]
            .try_into()
            .map_err(|_| GuestError::UnsupportedMapping)?,
    ) != base + IMAGE_OFFSET
        || u32::from_le_bytes(
            descriptor[8..12]
                .try_into()
                .map_err(|_| GuestError::UnsupportedMapping)?,
        ) != 128
    {
        return Err(Failure::Unsupported("font image descriptor output alias"));
    }
    let image_width = u32::from_le_bytes(
        descriptor[12..16]
            .try_into()
            .map_err(|_| GuestError::UnsupportedMapping)?,
    );
    let image_height = u32::from_le_bytes(
        descriptor[16..20]
            .try_into()
            .map_err(|_| GuestError::UnsupportedMapping)?,
    );
    if image_width > 128 || image_height > 128 {
        return Err(Failure::Unsupported("font image extent output alias"));
    }
    let end_x = start_x
        .checked_add(image_width as i32)
        .ok_or(Failure::Unsupported("font surface arithmetic"))?
        .min(width);
    let end_y = start_y
        .checked_add(image_height as i32)
        .ok_or(Failure::Unsupported("font surface arithmetic"))?
        .min(height);
    let mut scissor = [0; 16];
    plan.read(guest, at(surface, 24)?, &mut scissor, budget)?;
    let mut clip = [0; 4];
    for (index, chunk) in scissor.chunks_exact(4).enumerate() {
        clip[index] = u32::from_le_bytes(
            chunk
                .try_into()
                .map_err(|_| GuestError::UnsupportedMapping)?,
        );
    }
    let left = start_x.max(clip[0].min(width as u32) as i32);
    let top = start_y.max(clip[1].min(height as u32) as i32);
    let right = end_x.min(clip[2].min(width as u32) as i32);
    let bottom = end_y.min(clip[3].min(height as u32) as i32);
    if right <= left || bottom <= top {
        return Ok(());
    }
    budget.work(u64::from(image_width) * u64::from(image_height) * (pixel as u64 + 4))?;
    // Capture source bytes after metadata overlays but before surface stores.
    // Surface/image feedback is explicitly refused; it is not snapshotted into
    // invented independent backing or allowed to change future source samples.
    let mut source = scratch_buffer(engine::IMAGE_BYTES, plan.held()?, budget)?;
    source.copy_from_slice(plan.known(base + IMAGE_OFFSET, engine::IMAGE_BYTES)?);
    let mut row = scratch_buffer(
        (right - left) as usize * pixel as usize,
        add(plan.held()?, source.capacity())?,
        budget,
    )?;
    for y in top..bottom {
        let mut x = left;
        while x < right {
            let alpha = source[(y - start_y) as usize * 128 + (x - start_x) as usize];
            if alpha == 0 {
                x += 1;
                continue;
            }
            let begin = x;
            let mut count = 0;
            while x < right {
                let alpha = source[(y - start_y) as usize * 128 + (x - start_x) as usize];
                if alpha == 0 {
                    break;
                }
                row[count..count + pixel as usize].fill(alpha);
                count += pixel as usize;
                x += 1;
            }
            let offset = y
                .checked_mul(pitch)
                .and_then(|value| {
                    begin
                        .checked_mul(pixel)
                        .and_then(|column| value.checked_add(column))
                })
                .ok_or(Failure::Unsupported("font surface arithmetic"))?;
            let address = at(buffer, offset as u64)?;
            let end = at(address, count as u64)?;
            if address < base + OBJECT_BYTES as u64 && base < end {
                return Err(Failure::Unsupported("font surface image feedback"));
            }
            let prior_external = plan.external;
            plan.external = add(prior_external, add(source.capacity(), row.capacity())?)?;
            let stored = plan.store(address, &row[..count], budget);
            plan.external = prior_external;
            stored?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
