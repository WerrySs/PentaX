// SPDX-License-Identifier: GPL-2.0-only
// KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// libFont.cpp:1105-1548 and libFontFt.cpp; see NOTICE for attribution.

use super::*;

const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
const FONT_IMAGE_OFFSET: usize = 64;
const FONT_RUNTIME_BYTES: usize = FONT_IMAGE_OFFSET + engine::IMAGE_BYTES;

pub(super) struct Font {
    pub(super) runtime: RuntimeObject,
    pub(super) data: Vec<u8>,
    pub(super) source_address: u64,
    pub(super) source_size: u32,
    pub(super) library: u64,
    pub(super) font_set_type: u32,
    pub(super) open_mode: u32,
    pub(super) attribute: i32,
    pub(super) renderer: u64,
    pub(super) scale_w: f32,
    pub(super) scale_h: f32,
    pub(super) effect_weight_x: f32,
    pub(super) effect_weight_y: f32,
    pub(super) effect_weight_mode: u32,
    pub(super) effect_slant: f32,
    pub(super) render_scale_w: f32,
    pub(super) render_scale_h: f32,
    pub(super) render_effect_weight_x: f32,
    pub(super) render_effect_weight_y: f32,
    pub(super) render_effect_weight_mode: u32,
    pub(super) render_effect_slant: f32,
}

struct OpenFont {
    library: u64,
    source_address: u64,
    source_size: u32,
    font_set_type: u32,
    open_mode: u32,
    output: u64,
    data: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Objects {
    libraries: Vec<RuntimeObject>,
    renderers: Vec<RuntimeObject>,
    fonts: Vec<Font>,
    library_selection: Option<RuntimeObject>,
    renderer_selection: Option<RuntimeObject>,
}

impl Objects {
    pub(super) fn metadata_bytes(&self) -> usize {
        capacity(&self.libraries)
            + capacity(&self.renderers)
            + capacity(&self.fonts)
            + self
                .fonts
                .iter()
                .map(|font| font.data.capacity())
                .sum::<usize>()
    }

    pub(super) fn retained_handle_count(&self) -> usize {
        self.libraries.len()
            + self.renderers.len()
            + self.fonts.len()
            + usize::from(self.library_selection.is_some())
            + usize::from(self.renderer_selection.is_some())
    }

    fn font_index(
        &self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        budget.work(self.fonts.len() as u64 + 1)?;
        let index = self
            .fonts
            .iter()
            .position(|font| font.runtime.address == address)
            .ok_or(Failure::Unsupported("foreign or released font"))?;
        memory.check_runtime_object(self.fonts[index].runtime, budget)?;
        Ok(index)
    }

    pub(super) fn font(
        &self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<&Font, Failure> {
        let index = self.font_index(address, memory, budget)?;
        Ok(&self.fonts[index])
    }

    pub(super) fn font_mut(
        &mut self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<&mut Font, Failure> {
        let index = self.font_index(address, memory, budget)?;
        Ok(&mut self.fonts[index])
    }

    pub(super) fn invoke(
        &mut self,
        call: FontCall,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        let a = call.args.0;
        use Service::*;
        match call.service {
            FontMemoryInit => memory_init(call, guest, budget),
            FontMemoryTerm => {
                if a[0] != 0 {
                    write(a[0], &0u16.to_le_bytes(), guest, budget)?;
                }
                Ok(0)
            }
            FontCreateLibrary | FontCreateLibraryWithEdition | FontCreateRendererWithEdition => {
                let renderer = call.service == FontCreateRendererWithEdition;
                let with_edition = call.service != FontCreateLibrary;
                let output = a[if with_edition { 3 } else { 2 }];
                if output == 0 {
                    return Ok(FAILURE);
                }
                let table = if renderer {
                    &mut self.renderers
                } else {
                    &mut self.libraries
                };
                budget.replacement_handles(0, 2)?;
                let prepared = growth(table, 1, MAX_RECORDS, held, budget)?;
                let held = add(held, prepared.as_ref().map_or(0, capacity))?;
                let size = if renderer { 24 } else { 16 };
                let object =
                    memory.runtime_object(size, held, guest, budget, |address, bytes| {
                        bytes[..8].copy_from_slice(&a[0].to_le_bytes());
                        if renderer {
                            bytes[8..16].copy_from_slice(&a[1].to_le_bytes());
                            bytes[16..24].copy_from_slice(&a[2].to_le_bytes());
                        } else {
                            let edition = if with_edition { a[2] } else { 0 };
                            bytes[8..16].copy_from_slice(&edition.to_le_bytes());
                        }
                        let mut outputs = RuntimeDataOutputs::new();
                        outputs.push(output, &address.to_le_bytes())?;
                        Ok(outputs)
                    })?;
                insert(table, prepared, object);
                budget.added_handle();
                Ok(0)
            }
            FontDestroyLibrary | FontDestroyRenderer => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let address = budget.u64(guest, a[0])?;
                if address == 0 {
                    write(a[0], &0u64.to_le_bytes(), guest, budget)?;
                    return Ok(0);
                }
                let table = if call.service == FontDestroyLibrary {
                    &mut self.libraries
                } else {
                    &mut self.renderers
                };
                budget.work(table.len() as u64 + 1)?;
                let index = table
                    .iter()
                    .position(|object| object.address == address)
                    .ok_or(Failure::Unsupported(
                        "foreign or released font library or renderer",
                    ))?;
                let zero = 0u64.to_le_bytes();
                memory.release_runtime_object_with_outputs(
                    table[index],
                    guest,
                    budget,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &zero,
                    }],
                )?;
                table.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            FontSelectLibraryFt | FontSelectRendererFt => {
                let renderer = call.service == FontSelectRendererFt;
                let selected = if renderer {
                    &mut self.renderer_selection
                } else {
                    &mut self.library_selection
                };
                let mut bytes = [0; 8];
                bytes[..4].copy_from_slice(&(a[0] as u32).to_le_bytes());
                bytes[4..].copy_from_slice(&u32::from(renderer).to_le_bytes());
                if let Some(object) = *selected {
                    memory.check_runtime_object(object, budget)?;
                    write(object.address, &bytes, guest, budget)?;
                    return Ok(object.address);
                }
                budget.replacement_handles(0, 2)?;
                let object = memory.runtime_object(8, held, guest, budget, |_, output| {
                    output.copy_from_slice(&bytes);
                    Ok(RuntimeDataOutputs::new())
                })?;
                *selected = Some(object);
                budget.added_handle();
                Ok(object.address)
            }
            FontOpenFontSet | FontOpenFontMemory | FontOpenFontInstance => {
                let mut open = if call.service == FontOpenFontInstance {
                    if a[2] == 0 {
                        return Ok(FAILURE);
                    }
                    if a[0] == 0 {
                        OpenFont {
                            library: 0,
                            source_address: a[1],
                            source_size: 0,
                            font_set_type: 0,
                            open_mode: 0,
                            output: a[2],
                            data: Vec::new(),
                        }
                    } else {
                        let font = self.font(a[0], memory, budget)?;
                        let mut data = budget.buffer_held(font.data.len(), held)?;
                        // Private immutable data, not an arbitrary host-font search.
                        data.copy_from_slice(&font.data);
                        admit(add(held, data.capacity())?, budget)?;
                        OpenFont {
                            library: font.library,
                            source_address: font.source_address,
                            source_size: font.source_size,
                            font_set_type: font.font_set_type,
                            open_mode: font.open_mode,
                            output: a[2],
                            data,
                        }
                    }
                } else {
                    if a[4] == 0 {
                        return Ok(FAILURE);
                    }
                    let is_set = call.service == FontOpenFontSet;
                    OpenFont {
                        library: a[0],
                        source_address: if is_set { a[3] } else { a[1] },
                        source_size: if is_set { 0 } else { a[2] as u32 },
                        font_set_type: if is_set { a[1] as u32 } else { 0 },
                        open_mode: if is_set { a[2] as u32 } else { 0 },
                        output: a[4],
                        data: Vec::new(),
                    }
                };
                if call.service == FontOpenFontMemory {
                    open.data =
                        read_font_data(open.source_address, open.source_size, held, guest, budget)?;
                }
                self.open(open, memory, guest, budget, held)
            }
            FontCloseFont => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let index = self.font_index(a[0], memory, budget)?;
                memory.release_runtime_object(self.fonts[index].runtime, guest, budget)?;
                self.fonts.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            FontDefineAttribute => {
                let index = if a[0] == 0 {
                    None
                } else {
                    Some(self.font_index(a[0], memory, budget)?)
                };
                let old = index.map_or(0, |i| self.fonts[i].attribute);
                if a[2] != 0 {
                    write(a[2], &old.to_le_bytes(), guest, budget)?;
                }
                if let Some(index) = index {
                    self.fonts[index].attribute = a[1] as i32;
                }
                Ok(0)
            }
            FontBindRenderer | FontUnbindRenderer | FontRebindRenderer => {
                if a[0] == 0 {
                    return Ok(FAILURE);
                }
                let font = self.font_mut(a[0], memory, budget)?;
                if call.service == FontBindRenderer {
                    font.renderer = a[1];
                }
                if call.service == FontUnbindRenderer {
                    font.renderer = 0;
                }
                Ok(0)
            }
            FontSetScalePixel
            | FontSetEffectSlant
            | FontSetEffectWeight
            | FontSetupRenderScalePixel
            | FontSetupRenderEffectSlant
            | FontSetupRenderEffectWeight => {
                if a[0] == 0 {
                    return Ok(FAILURE);
                }
                let font = self.font_mut(a[0], memory, budget)?;
                let vectors = call
                    .vector_args
                    .ok_or(Failure::Unsupported("missing font vector arguments"))?;
                let x = f32::from_le_bytes(vectors[0][..4].try_into().unwrap());
                let y = f32::from_le_bytes(vectors[1][..4].try_into().unwrap());
                match call.service {
                    FontSetScalePixel => {
                        font.scale_w = x;
                        font.scale_h = y;
                    }
                    FontSetEffectSlant => font.effect_slant = x,
                    FontSetEffectWeight => {
                        font.effect_weight_x = x;
                        font.effect_weight_y = y;
                        font.effect_weight_mode = a[1] as u32;
                    }
                    FontSetupRenderScalePixel => {
                        font.render_scale_w = x;
                        font.render_scale_h = y;
                    }
                    FontSetupRenderEffectSlant => font.render_effect_slant = x,
                    FontSetupRenderEffectWeight => {
                        font.render_effect_weight_x = x;
                        font.render_effect_weight_y = y;
                        font.render_effect_weight_mode = a[1] as u32;
                    }
                    _ => unreachable!(),
                }
                Ok(0)
            }
            // These three pinned implementations only log their opaque inputs.
            FontAttachDeviceCacheBuffer | FontSupportExternalFonts | FontSupportSystemFonts => {
                Ok(0)
            }
            _ => Err(Failure::Unsupported("font object service")),
        }
    }

    fn open(
        &mut self,
        open: OpenFont,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        held: usize,
    ) -> Result<u64, Failure> {
        budget.replacement_handles(0, 2)?;
        let held = add(held, open.data.capacity())?;
        let prepared = growth(&self.fonts, 1, MAX_RECORDS, held, budget)?;
        let held = add(held, prepared.as_ref().map_or(0, capacity))?;
        let image = engine::rasterize(
            if open.data.is_empty() {
                None
            } else {
                Some(&open.data)
            },
            16.0,
            u32::from(b'?'),
            held,
            budget,
        )?;
        if image.pixels.len() != engine::IMAGE_BYTES
            || image.width > engine::STRIDE as u32
            || image.height > 128
        {
            return Err(Failure::Unsupported("font constructor image extent"));
        }
        let held = add(held, image.metadata_bytes())?;
        budget.work(FONT_RUNTIME_BYTES as u64)?;
        let runtime =
            memory.runtime_object(FONT_RUNTIME_BYTES, held, guest, budget, |address, bytes| {
                let pixels = address
                    .checked_add(FONT_IMAGE_OFFSET as u64)
                    .ok_or(GuestError::AddressOverflow)?;
                bytes[..8].copy_from_slice(&pixels.to_le_bytes());
                bytes[8..12].copy_from_slice(&(engine::STRIDE as u32).to_le_bytes());
                bytes[12..16].copy_from_slice(&image.width.to_le_bytes());
                bytes[16..20].copy_from_slice(&image.height.to_le_bytes());
                bytes[FONT_IMAGE_OFFSET..].copy_from_slice(&image.pixels);
                let mut outputs = RuntimeDataOutputs::new();
                outputs.push(open.output, &address.to_le_bytes())?;
                Ok(outputs)
            })?;
        insert(
            &mut self.fonts,
            prepared,
            Font {
                runtime,
                data: open.data,
                source_address: open.source_address,
                source_size: open.source_size,
                library: open.library,
                font_set_type: open.font_set_type,
                open_mode: open.open_mode,
                attribute: 0,
                renderer: 0,
                scale_w: 8.0,
                scale_h: 16.0,
                effect_weight_x: 0.0,
                effect_weight_y: 0.0,
                effect_weight_mode: 0,
                effect_slant: 0.0,
                render_scale_w: 8.0,
                render_scale_h: 16.0,
                render_effect_weight_x: 0.0,
                render_effect_weight_y: 0.0,
                render_effect_weight_mode: 0,
                render_effect_slant: 0.0,
            },
        );
        budget.added_handle();
        Ok(0)
    }
}

fn memory_init(
    call: FontCall,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    let a = call.args.0;
    if a[0] == 0 {
        return Ok(FAILURE);
    }
    let stack = call
        .stack_pointer
        .ok_or(Failure::Unsupported("missing font stack arguments"))?;
    let argument = stack.checked_add(8).ok_or(GuestError::AddressOverflow)?;
    let destroy_object = budget.u64(guest, argument)?;
    let mut bytes = [0; 64];
    bytes[..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[4..8].copy_from_slice(&(a[2] as u32).to_le_bytes());
    bytes[8..16].copy_from_slice(&a[1].to_le_bytes());
    bytes[16..24].copy_from_slice(&a[4].to_le_bytes());
    bytes[24..32].copy_from_slice(&a[3].to_le_bytes());
    bytes[32..40].copy_from_slice(&a[5].to_le_bytes());
    bytes[40..48].copy_from_slice(&destroy_object.to_le_bytes());
    write(a[0], &bytes, guest, budget)?;
    Ok(0)
}

fn write(
    address: u64,
    bytes: &[u8],
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    budget.write(guest, &[GuestWrite { address, bytes }])
}

fn read_font_data(
    address: u64,
    size: u32,
    held: usize,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<Vec<u8>, Failure> {
    if address == 0 || size < 12 {
        return Ok(Vec::new());
    }
    let mut magic = [0; 4];
    budget.read(guest, address, &mut magic)?;
    if !matches!(&magic, b"\0\x01\0\0" | b"true" | b"OTTO") {
        return Ok(Vec::new());
    }
    let size = usize::try_from(size).map_err(|_| Failure::Limit("font input bytes"))?;
    if size > MAX_FONT_BYTES {
        return Err(Failure::Limit("font input bytes"));
    }
    let mut data = budget.buffer_held(size, held)?;
    admit(add(held, data.capacity())?, budget)?;
    budget.read(guest, address, &mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests;
