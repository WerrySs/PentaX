// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// libFont.cpp and libFontFt.cpp. See NOTICE and docs/upstream-provenance.md.
//! Owned opaque font objects, real glyph data, and bounded text state.

use crate::budget::Budget;
use crate::memory::{MemoryState, RuntimeObject};
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, RuntimeDataOutputs, Service};

mod engine;
pub(crate) mod fixed;
mod objects;
mod render;
mod text;
pub use text::{AdmittedFontParser, FontParserAction};
mod writing;

use objects::Objects;

const FAILURE: u64 = u32::MAX as u64;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;

#[derive(Clone, Copy)]
pub(super) struct FontCall {
    pub(super) service: Service,
    pub(super) args: CallArgs,
    pub(super) vector_args: Option<[[u8; 16]; 8]>,
    pub(super) stack_pointer: Option<u64>,
}

#[derive(Default)]
pub(crate) struct FontState {
    objects: Objects,
    pub(crate) text: text::TextState,
    render: render::RenderState,
}

fn add(left: usize, right: usize) -> Result<usize, Failure> {
    left.checked_add(right)
        .filter(|total| *total <= MAX_METADATA)
        .ok_or(Failure::Limit("font retained and staged metadata"))
}

fn capacity<T>(table: &Vec<T>) -> usize {
    table.capacity() * core::mem::size_of::<T>()
}

fn admit(bytes: usize, budget: &mut Budget) -> Result<(), Failure> {
    if bytes > MAX_METADATA {
        return Err(Failure::Limit("font retained and staged metadata"));
    }
    budget.scratch(bytes)?;
    budget.record_scratch(bytes);
    Ok(())
}

/// A failed publication must not retain even a larger empty table capacity.
fn growth<T>(
    table: &Vec<T>,
    extra: usize,
    maximum: usize,
    held: usize,
    budget: &mut Budget,
) -> Result<Option<Vec<T>>, Failure> {
    let needed = table
        .len()
        .checked_add(extra)
        .filter(|n| *n <= maximum)
        .ok_or(Failure::Limit("font record count"))?;
    if needed <= table.capacity() {
        return Ok(None);
    }
    let requested = needed.max(table.capacity().saturating_mul(2)).min(maximum);
    let bytes = requested
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("font record storage"))?;
    admit(add(held, bytes)?, budget)?;
    budget.work(bytes as u64 + table.len() as u64 + 1)?;
    let mut next = Vec::new();
    next.try_reserve_exact(requested)
        .map_err(|_| Failure::Allocation)?;
    if next.capacity() > maximum {
        return Err(Failure::Limit("font record capacity"));
    }
    admit(add(held, capacity(&next))?, budget)?;
    Ok(Some(next))
}

fn insert<T>(table: &mut Vec<T>, prepared: Option<Vec<T>>, item: T) {
    if let Some(mut next) = prepared {
        next.append(table);
        next.push(item);
        *table = next;
    } else {
        table.push(item);
    }
}

pub(crate) fn is_service(service: Service) -> bool {
    use Service::*;
    matches!(
        service,
        FontAttachDeviceCacheBuffer
            | FontCloseFont
            | FontCreateRendererWithEdition
            | FontDestroyRenderer
            | FontBindRenderer
            | FontRebindRenderer
            | FontUnbindRenderer
            | FontSetScalePixel
            | FontSetEffectSlant
            | FontSetEffectWeight
            | FontSetupRenderScalePixel
            | FontSetupRenderEffectSlant
            | FontSetupRenderEffectWeight
            | FontGetHorizontalLayout
            | FontGetVerticalLayout
            | FontDefineAttribute
            | FontCreateLibrary
            | FontCreateLibraryWithEdition
            | FontCreateString
            | FontCreateWritingLine
            | FontDestroyString
            | FontDestroyWritingLine
            | FontDeleteGlyph
            | FontDestroyLibrary
            | FontGenerateCharGlyph
            | FontGetCharGlyphMetrics
            | FontGetRenderCharGlyphMetrics
            | FontGlyphDefineAttribute
            | FontMemoryInit
            | FontMemoryTerm
            | FontOpenFontInstance
            | FontOpenFontMemory
            | FontOpenFontSet
            | FontRenderCharGlyphImageHorizontal
            | FontRenderSurfaceInit
            | FontRenderSurfaceSetScissor
            | FontStringGetTerminateCode
            | FontStringGetTerminateOrder
            | FontStringGetWritingForm
            | FontStringRefersRenderCharacters
            | FontStringRefersTextCharacters
            | FontTextSourceInit
            | FontTextSourceRewind
            | FontTextSourceSetDefaultFont
            | FontTextSourceSetWritingForm
            | FontCharacterGetBidiLevel
            | FontCharacterGetSyllableStringState
            | FontCharacterGetTextFontCode
            | FontCharacterGetTextOrder
            | FontCharacterLooksFormatCharacters
            | FontCharacterLooksWhiteSpace
            | FontCharacterRefersTextBack
            | FontCharacterRefersTextNext
            | FontCharactersRefersTextCodes
            | FontTextCodesStepBack
            | FontTextCodesStepNext
            | FontWritingGetRenderMetrics
            | FontWritingInit
            | FontWritingLineClear
            | FontWritingLineGetOrderingSpace
            | FontWritingLineGetRenderMetrics
            | FontWritingLineRefersRenderStep
            | FontWritingLineWritesOrder
            | FontWritingRefersRenderStep
            | FontWritingRefersRenderStepCharacter
            | FontWritingSetMaskInvisible
            | FontSupportExternalFonts
            | FontSupportSystemFonts
            | FontSelectLibraryFt
            | FontSelectRendererFt
    )
}

impl FontState {
    pub(crate) fn metadata_bytes(&self) -> usize {
        self.objects.metadata_bytes() + self.text.metadata_bytes() + self.render.metadata_bytes()
    }

    pub(crate) fn retained_handle_count(&self) -> usize {
        self.objects.retained_handle_count()
            + self.text.retained_handle_count()
            + self.render.retained_handle_count()
    }

    pub(crate) fn invoke(
        &mut self,
        call: FontCall,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // Font input capacities are bounded independently of logical handles.
        budget.work((self.retained_handle_count() as u64 + 1) * 32)?;
        let held = self.metadata_bytes();
        admit(held, budget)?;
        use Service::*;
        if call.service == FontCloseFont && call.args.0[0] != 0 {
            // A string can retain many character records for one logical
            // handle. Charge their bounded capacities before dependency scans.
            budget.work(
                self.text.work_items() as u64 + self.render.retained_handle_count() as u64 + 1,
            )?;
            if self.text.depends_on_font(call.args.0[0])
                || self.render.depends_on_font(call.args.0[0])
            {
                return Err(Failure::Unsupported(
                    "font has retained text or glyph views",
                ));
            }
        }
        match call.service {
            FontAttachDeviceCacheBuffer
            | FontCloseFont
            | FontCreateRendererWithEdition
            | FontDestroyRenderer
            | FontBindRenderer
            | FontRebindRenderer
            | FontUnbindRenderer
            | FontSetScalePixel
            | FontSetEffectSlant
            | FontSetEffectWeight
            | FontSetupRenderScalePixel
            | FontSetupRenderEffectSlant
            | FontSetupRenderEffectWeight
            | FontDefineAttribute
            | FontCreateLibrary
            | FontCreateLibraryWithEdition
            | FontDestroyLibrary
            | FontMemoryInit
            | FontMemoryTerm
            | FontOpenFontInstance
            | FontOpenFontMemory
            | FontOpenFontSet
            | FontSupportExternalFonts
            | FontSupportSystemFonts
            | FontSelectLibraryFt
            | FontSelectRendererFt => self.objects.invoke(call, memory, guest, budget, held),
            FontGetHorizontalLayout
            | FontGetVerticalLayout
            | FontDeleteGlyph
            | FontGenerateCharGlyph
            | FontGetCharGlyphMetrics
            | FontGetRenderCharGlyphMetrics
            | FontGlyphDefineAttribute
            | FontRenderCharGlyphImageHorizontal
            | FontRenderSurfaceInit
            | FontRenderSurfaceSetScissor => {
                self.render
                    .invoke(call, &mut self.objects, memory, guest, budget, held)
            }
            _ if is_service(call.service) => {
                self.text
                    .invoke(call, &mut self.objects, memory, guest, budget, held)
            }
            _ => Err(Failure::Unsupported("font service")),
        }
    }
}
