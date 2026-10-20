// SPDX-License-Identifier: GPL-2.0-or-later

//! Typed TLS relocation values derived from the image-bound static layout.
//!
//! Generic AMD64 arithmetic is applied under an initial PentaX profile: DTPMOD64
//! writes the provider's layout runtime ID with a zero addend, DTPOFF64 writes
//! S+A and TPOFF64 writes S+A-O, where S is the module-relative TLS offset and O
//! the provider's positive distance below TP. S+A must lie in [0, p_memsz).
//! No load bias is added to these scalars, and no runtime ID is derived from a
//! ModuleId, SCE selector or fixed constant. No TCB, DTV or execution follows.

use super::plan::plan_layout;
use super::storage::{Preparation, prepare};
use super::{
    PreparedTlsSession, StaticTlsLimits, StaticTlsPlan, StaticTlsPolicy, StaticTlsStorage,
    TlsBudget, TlsError, TlsTemplateLayout, Work, merge_ranges, page_range,
};
use crate::process::SessionLoadLimits;
use pentax_linker::{
    ModuleId, RelocationSource, TlsBinding, TlsBindingPlan, TlsDefinition, TlsRelocationKind,
};
use std::fmt;

#[cfg(test)]
mod tests;

/// One checked eight-byte TLS scalar write, never accepted as preparation input.
#[derive(Debug, Clone, Copy)]
pub struct TlsRelocationPatch {
    binding_index: usize,
    consumer: ModuleId,
    source: RelocationSource,
    address: u64,
    value: u64,
    kind: TlsRelocationKind,
    definition: TlsDefinition,
}

impl TlsRelocationPatch {
    /// Index of the originating entry in the retained plan's `bindings()`.
    pub const fn binding_index(&self) -> usize {
        self.binding_index
    }
    pub const fn consumer(&self) -> ModuleId {
        self.consumer
    }
    pub const fn source(&self) -> RelocationSource {
        self.source
    }
    /// Biased guest address of the consumer-owned eight-byte target.
    pub const fn address(&self) -> u64 {
        self.address
    }
    /// Little-endian scalar bits; TPOFF64 values are two's-complement i64.
    pub const fn value(&self) -> u64 {
        self.value
    }
    pub const fn kind(&self) -> TlsRelocationKind {
        self.kind
    }
    pub const fn definition(&self) -> TlsDefinition {
        self.definition
    }
}

/// A static layout derived from the binding plan's exact base, with private
/// typed patches. No base, layout, memory, template or patch can be paired.
pub struct RelocatedStaticTlsPlan<'binding, 'plan, 'image> {
    bindings: &'binding TlsBindingPlan<'plan, 'image>,
    pub(super) layout: StaticTlsPlan<'plan, 'image>,
    patches: Vec<TlsRelocationPatch>,
}

impl fmt::Debug for RelocatedStaticTlsPlan<'_, '_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RelocatedStaticTlsPlan")
            .field("layout", &self.layout)
            .field("tls_patch_count", &self.patches.len())
            .finish_non_exhaustive()
    }
}

impl<'binding, 'plan, 'image> RelocatedStaticTlsPlan<'binding, 'plan, 'image> {
    pub const fn bindings(&self) -> &'binding TlsBindingPlan<'plan, 'image> {
        self.bindings
    }
    pub const fn policy(&self) -> StaticTlsPolicy {
        self.layout.policy()
    }
    pub fn templates(&self) -> &[TlsTemplateLayout] {
        &self.layout.templates
    }
    pub const fn static_extent(&self) -> u64 {
        self.layout.static_extent
    }
    pub const fn maximum_alignment(&self) -> u64 {
        self.layout.maximum_alignment
    }
    /// One patch per binding, in the bindings' biased target-address order.
    pub fn tls_patches(&self) -> &[TlsRelocationPatch] {
        &self.patches
    }
}

/// Derive the static layout from `bindings.base_plan()` and calculate every
/// bound TLS value against it before any guest allocation.
///
/// Any remaining unbound deferred record is refused before layout. The bound
/// provider's module, PT_TLS header index and p_memsz must match the layout.
/// Extra writes target checked ordinary image memory and join the resident
/// interval union. Only their intersection with p_filesz reaches TLS copies.
/// One per-call work budget covers layout, binding visits/lookups and interval
/// operations, charged before traversal.
pub fn plan_relocated_static_tls<'binding, 'plan, 'image>(
    bindings: &'binding TlsBindingPlan<'plan, 'image>,
    limits: &StaticTlsLimits,
) -> Result<RelocatedStaticTlsPlan<'binding, 'plan, 'image>, TlsError> {
    let remaining = bindings.remaining_deferred();
    if let Some(first) = remaining.first() {
        return Err(TlsError::RemainingDeferredRelocations {
            count: remaining.len(),
            module: first.module(),
            source: first.source(),
        });
    }
    let mut work = Work {
        used: 0,
        limit: limits.max_work_steps,
    };
    let exhausted = TlsError::Budget {
        kind: TlsBudget::WorkSteps,
        limit: limits.max_work_steps,
    };
    let mut layout = plan_layout(bindings.base_plan(), limits, &mut work)?;
    let records = bindings.bindings();
    let count = records.len() as u64;
    // One visit and one ordered provider-layout lookup per binding.
    work.charge(count.checked_mul(2).ok_or_else(|| exhausted.clone())?)?;
    let mut patches = Vec::with_capacity(records.len());
    for (binding_index, binding) in records.iter().enumerate() {
        patches.push(relocation_patch(binding_index, binding, &layout.templates)?);
    }
    // Union, rather than add, so shared pages are charged once and otherwise
    // untouched BSS target pages are charged before materialization.
    let intervals = (layout.image_resident_pages.len() as u64)
        .checked_add(count)
        .ok_or(exhausted)?;
    work.charge(intervals)?;
    for patch in &patches {
        layout
            .image_resident_pages
            .push(page_range(patch.address, 8)?);
    }
    layout.image_resident_bytes = merge_ranges(&mut layout.image_resident_pages)?;
    Ok(RelocatedStaticTlsPlan {
        bindings,
        layout,
        patches,
    })
}

/// Stage the retained combined plan and copy its relocated initialized bytes.
///
/// After the existing storage, guard and aggregate preflight, every original
/// payload, ordinary write and TLS write is applied before final original/RELRO
/// permissions; only then are fresh guarded RW/NX blocks filled from the staged
/// templates. Failure drops the entire unpublished session.
pub fn load_relocated_static_tls(
    plan: &RelocatedStaticTlsPlan<'_, '_, '_>,
    storage: &StaticTlsStorage<'_>,
    limits: &SessionLoadLimits,
) -> Result<PreparedTlsSession, TlsError> {
    prepare(Preparation::Relocated(plan), storage, limits)
}

fn relocation_patch(
    binding_index: usize,
    binding: &TlsBinding,
    templates: &[TlsTemplateLayout],
) -> Result<TlsRelocationPatch, TlsError> {
    let consumer = binding.consumer();
    let source = binding.source();
    let definition = binding.definition();
    // Templates are in input-module order, so one ordered lookup suffices.
    let provider = templates
        .binary_search_by_key(&definition.module(), |template| template.module)
        .ok()
        .map(|index| &templates[index])
        .filter(|template| {
            template.program_header_index == definition.program_header_index()
                && template.mem_size == definition.template_mem_size()
        })
        .ok_or(TlsError::TlsProviderMismatch {
            consumer,
            source,
            definition: definition.symbol(),
        })?;
    let addend_range = || TlsError::TlsAddendRange {
        consumer,
        source,
        definition: definition.symbol(),
    };
    let addend = i128::from(source.record.addend);
    let value = match binding.kind() {
        TlsRelocationKind::DtpMod64 => {
            if addend != 0 {
                return Err(addend_range());
            }
            provider.runtime_id.get()
        }
        kind @ (TlsRelocationKind::DtpOff64 | TlsRelocationKind::TpOff64) => {
            let block = u64::try_from(i128::from(definition.offset().get()) + addend)
                .ok()
                .filter(|block| *block < provider.mem_size)
                .ok_or_else(addend_range)?;
            if matches!(kind, TlsRelocationKind::DtpOff64) {
                block
            } else {
                let signed = i64::try_from(i128::from(block) - i128::from(provider.tp_offset))
                    .map_err(|_| TlsError::TlsValueRange {
                        consumer,
                        source,
                        definition: definition.symbol(),
                    })?;
                signed as u64
            }
        }
    };
    Ok(TlsRelocationPatch {
        binding_index,
        consumer,
        source,
        address: binding.address(),
        value,
        kind: binding.kind(),
        definition,
    })
}
