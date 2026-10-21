// SPDX-License-Identifier: GPL-2.0-only

//! Caller-declared materialization for pure planning, never TLS admission.
//! Numeric values and staged snapshots can be supplied by safe callers. They
//! are consistency-checked against exact source provenance, not certified as
//! actually applied or semantically correct by this crate.

use crate::source::{ADDRESS_LIMIT, materialized_window, preflight_metadata};
use crate::*;
use pentax_linker::{ModuleId, RelocationSource, SessionDeferredRelocation, SymbolicLinkPlan};
use std::fmt;

/// A declared eight-byte little-endian write for one exact deferred record.
/// These public request fields are not preparation or execution authority.
/// Values must come from a separately verified materializer before native use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeclaredAppliedWrite {
    pub deferred_index: usize,
    pub consumer: ModuleId,
    pub source: RelocationSource,
    pub address: u64,
    pub value: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializedSourceAssurance {
    /// No live backing, TLS resolution, applied-write proof or executor exists.
    CallerDeclared,
}

/// Declaration-phase work and reported buffer capacities, including temporary
/// duplicate tracking. Per-block planning has its own independent usage/bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializedSourceUsage {
    pub declared_writes: u64,
    pub allocation_bytes: u64,
    pub work_units: u64,
}

/// Reusable immutable declaration bound to exactly one borrowed linker plan.
/// Construction validates full record coverage but never resolves those records.
/// The original deferred list remains visible, with its original meaning.
pub struct MaterializedSource<'plan, 'image> {
    source: &'plan SymbolicLinkPlan<'image>,
    writes: Vec<DeclaredAppliedWrite>,
    usage: MaterializedSourceUsage,
}

impl fmt::Debug for MaterializedSource<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterializedSource")
            .field("assurance", &self.assurance())
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

impl<'plan, 'image> MaterializedSource<'plan, 'image> {
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub fn declared_writes(&self) -> &[DeclaredAppliedWrite] {
        &self.writes
    }
    pub fn deferred_relocations(&self) -> &[SessionDeferredRelocation] {
        self.source.deferred()
    }
    pub const fn assurance(&self) -> MaterializedSourceAssurance {
        MaterializedSourceAssurance::CallerDeclared
    }
    pub const fn declaration_usage(&self) -> MaterializedSourceUsage {
        self.usage
    }
}

/// Validate a full bijection to the exact source's deferred records. Writes
/// must be ordered by biased address, nonoverlapping, disjoint from ordinary
/// patches, and cover only the known eight-byte encodings 16/17/18. Each target
/// must be wholly covered by its consumer's logical mappings.
///
/// This admits a **declaration**, not a resolved TLS plan. In particular this
/// function cannot verify the numeric values, actual application, template
/// copying, storage ownership, live permissions, liveness or the PS5 ABI.
/// `plan_block` still refuses every deferred record. A future unsafe native
/// boundary must bind this exact source and every declared value to the actual
/// prepared owner and verified TLS plan, then attest live source bytes.
pub fn declare_materialized_source<'plan, 'image>(
    source: &'plan SymbolicLinkPlan<'image>,
    writes: &[DeclaredAppliedWrite],
    limits: &BlockLimits,
) -> Result<MaterializedSource<'plan, 'image>, CodegenError> {
    let mut account = Accounting {
        work: 0,
        allocated: 0,
        limits: *limits,
    };
    let count = source.deferred_count();
    let segments = preflight_metadata(source, Some(count.max(writes.len())), &mut account)?;
    if writes.len() != count {
        return Err(CodegenError::MaterializedDeclarationCount {
            expected: count,
            actual: writes.len(),
        });
    }
    // Charge all record validation, bounded logical-coverage walks and the
    // monotonic ordinary-patch merge before traversal or full table copies.
    let record_visits = (count as u64)
        .checked_mul(4)
        .ok_or(CodegenError::InvalidAddress)?;
    let visits = (count as u64)
        .checked_mul(segments)
        .and_then(|v| v.checked_mul(8))
        .and_then(|v| v.checked_add(record_visits))
        .and_then(|v| v.checked_add(source.patch_count() as u64))
        .ok_or(CodegenError::InvalidAddress)?;
    account.charge(visits)?;
    for (deferred_index, deferred) in source.deferred().iter().enumerate() {
        let kind = deferred.source().record.kind();
        if !matches!(kind, 16..=18) {
            return Err(CodegenError::UnsupportedMaterializedRelocation {
                deferred_index,
                kind,
            });
        }
    }
    let mut seen = account.buffer::<u8>(count)?;
    seen.resize(count, 0);
    let mut retained = account.buffer::<DeclaredAppliedWrite>(count)?;
    let mut previous_end = 0;
    let mut ordinary_at = 0;
    for (index, write) in writes.iter().enumerate() {
        let bad = |reason| CodegenError::InvalidMaterializedWrite { index, reason };
        let deferred = source
            .deferred()
            .get(write.deferred_index)
            .ok_or_else(|| bad(MaterializedWriteError::DeferredIndex))?;
        if seen[write.deferred_index] != 0 {
            return Err(CodegenError::DuplicateMaterializedWrite {
                deferred_index: write.deferred_index,
            });
        }
        seen[write.deferred_index] = 1;
        if write.consumer != deferred.module() {
            return Err(bad(MaterializedWriteError::Consumer));
        }
        if write.source != deferred.source() {
            return Err(bad(MaterializedWriteError::SourceRecord));
        }
        let module = source
            .modules()
            .get(write.consumer.index())
            .filter(|module| module.id() == write.consumer)
            .ok_or_else(|| bad(MaterializedWriteError::Consumer))?;
        let expected = module
            .load_bias()
            .checked_add(write.source.record.offset)
            .ok_or_else(|| bad(MaterializedWriteError::Address))?;
        let end = write
            .address
            .checked_add(8)
            .filter(|v| *v <= ADDRESS_LIMIT)
            .ok_or_else(|| bad(MaterializedWriteError::Address))?;
        if write.address != expected {
            return Err(bad(MaterializedWriteError::Address));
        }
        if index != 0 && write.address < previous_end {
            return Err(bad(MaterializedWriteError::OrderingOrOverlap));
        }
        previous_end = end;
        let mut cursor = write.address;
        // At most eight iterations: every successful segment covers at least
        // one of this fixed-width write's eight bytes. No BSS bytes become
        // initialized executable source merely because their target is valid.
        while cursor < end {
            let mut covered = None;
            for segment in &module.runtime_plan().segments {
                let start = segment
                    .vaddr
                    .checked_add(module.load_bias())
                    .ok_or_else(|| bad(MaterializedWriteError::LogicalTarget))?;
                let stop = start
                    .checked_add(segment.mem_size)
                    .ok_or_else(|| bad(MaterializedWriteError::LogicalTarget))?;
                if start <= cursor && cursor < stop {
                    covered = Some(stop.min(end));
                    break;
                }
            }
            cursor = covered.ok_or_else(|| bad(MaterializedWriteError::LogicalTarget))?;
        }
        while let Some(patch) = source.patches().get(ordinary_at) {
            let stop = patch
                .address()
                .checked_add(8)
                .ok_or_else(|| bad(MaterializedWriteError::OrdinaryPatchOverlap))?;
            if stop <= write.address {
                ordinary_at += 1;
            } else {
                break;
            }
        }
        if source
            .patches()
            .get(ordinary_at)
            .is_some_and(|patch| patch.address() < end)
        {
            return Err(bad(MaterializedWriteError::OrdinaryPatchOverlap));
        }
        retained.push(*write);
    }
    Ok(MaterializedSource {
        source,
        writes: retained,
        usage: MaterializedSourceUsage {
            declared_writes: count as u64,
            allocation_bytes: account.allocated,
            work_units: account.work,
        },
    })
}

/// A distinct pure result: it deliberately does not dereference or convert to
/// `BlockPlan`. Existing native consumers accepting ordinary plans cannot use
/// this caller-declared result as implicit TLS or execution admission.
///
/// ```compile_fail
/// use pentax_codegen::{BlockPlan, MaterializedBlockPlan};
/// fn ordinary_consumer(_: &BlockPlan<'_, '_>) {}
/// fn cannot_substitute(plan: &MaterializedBlockPlan<'_, '_, '_>) {
///     ordinary_consumer(plan);
/// }
/// ```
pub struct MaterializedBlockPlan<'view, 'plan, 'image> {
    declaration: &'view MaterializedSource<'plan, 'image>,
    inner: BlockPlan<'plan, 'image>,
}

impl fmt::Debug for MaterializedBlockPlan<'_, '_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MaterializedBlockPlan")
            .field("assurance", &self.assurance())
            .field("module", &self.module())
            .field("guest_pc", &self.guest_pc())
            .field("usage", &self.usage())
            .finish_non_exhaustive()
    }
}

impl<'view, 'plan, 'image> MaterializedBlockPlan<'view, 'plan, 'image> {
    pub const fn declaration(&self) -> &'view MaterializedSource<'plan, 'image> {
        self.declaration
    }
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.inner.source_plan()
    }
    pub const fn assurance(&self) -> MaterializedSourceAssurance {
        MaterializedSourceAssurance::CallerDeclared
    }
    pub const fn module(&self) -> ModuleId {
        self.inner.module()
    }
    pub const fn guest_pc(&self) -> u64 {
        self.inner.guest_pc()
    }
    pub fn original_bytes(&self) -> &[u8] {
        self.inner.original_bytes()
    }
    /// Exact image/ordinary/declaration overlay, checked against the supplied
    /// immutable staged window. This is not proof of an actual memory owner.
    pub fn relocated_source_bytes(&self) -> &[u8] {
        self.inner.relocated_source_bytes()
    }
    pub fn bytes(&self) -> &[u8] {
        self.inner.bytes()
    }
    pub fn instructions(&self) -> &[InstructionRecord] {
        self.inner.instructions()
    }
    pub const fn exit(&self) -> &BlockExit {
        self.inner.exit()
    }
    pub const fn placement(&self) -> CachePlacement {
        self.inner.placement()
    }
    pub const fn policy(&self) -> TranslationPolicy {
        self.inner.policy()
    }
    /// Same immutable emitted-body fence requirement as the inner pure plan.
    pub const fn emits_non_temporal_store(&self) -> bool {
        self.inner.emits_non_temporal_store()
    }
    pub const fn emits_vector_memory_move(&self) -> bool {
        self.inner.emits_vector_memory_move()
    }
    pub const fn emits_scalar_float_memory(&self) -> bool {
        self.inner.emits_scalar_float_memory()
    }
    pub const fn usage(&self) -> BlockUsage {
        self.inner.usage()
    }
    /// Pure comparison only; the future unsafe consumer must independently
    /// establish the exact prepared owner, typed proof and live source bytes.
    pub fn attest_emission(&self, address: u64, bytes: &[u8]) -> bool {
        self.inner.attest_emission(address, bytes)
    }
    pub fn attest_exit(&self, address: u64, bytes: &[u8]) -> bool {
        self.inner.attest_exit(address, bytes)
    }
}

/// Plan a bounded initialized executable window from this exact declaration.
/// Every staged byte must match the source plus retained ordinary writes and
/// declared extra writes. A caller may choose a shorter initialized window;
/// truncated instructions fail unless the explicit partial-window policy can
/// split after a complete prefix. No executable BSS is synthesized.
/// This function never validates TLS values or returns an execution capability.
pub fn plan_materialized_block<'view, 'plan, 'image>(
    source: &'view MaterializedSource<'plan, 'image>,
    module_index: usize,
    guest_pc: u64,
    staged_window: &[u8],
    placement: CachePlacement,
    policy: &TranslationPolicy,
    limits: &BlockLimits,
) -> Result<MaterializedBlockPlan<'view, 'plan, 'image>, CodegenError> {
    let mut accounting = crate::plan::begin(limits)?;
    let window = materialized_window(
        source,
        module_index,
        guest_pc,
        placement,
        staged_window,
        &mut accounting,
    )?;
    let inner = crate::plan::plan_window(
        source.source_plan(),
        guest_pc,
        placement,
        policy,
        window,
        accounting,
    )?;
    Ok(MaterializedBlockPlan {
        declaration: source,
        inner,
    })
}
