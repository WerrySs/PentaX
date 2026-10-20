// SPDX-License-Identifier: GPL-2.0-only

//! Exact retained-plan staging into owned, paused, physically NX native pages.
//! No sparse snapshot, caller-selected host pointer or native executor exists.

use super::{MemoryError, service::convert};
use crate::process::{
    ProcessError, SessionLoadLimits, SessionMetadata, SessionPlan, stage_session_with,
};
use pentax_linker::SymbolicLinkPlan;
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryLimits, NativeOwnerId, PausedNativeMemory,
};

#[derive(Debug, Clone)]
pub struct NativeLoadLimits {
    pub session: SessionLoadLimits,
    pub reserved_limit: u64,
}
impl Default for NativeLoadLimits {
    fn default() -> Self {
        Self {
            session: SessionLoadLimits::default(),
            reserved_limit: 512 * 1024 * 1024,
        }
    }
}

/// Source-bound owner. Only an exclusive pause can read/write its real pages;
/// source EXECUTE remains logical fetch permission with physical NX backing.
#[derive(Debug)]
pub struct NativeGuestMemory<'plan, 'image> {
    source: &'plan SymbolicLinkPlan<'image>,
    active_source: pentax_linker::SourceIdentity,
    ordinary_patches: usize,
    pub(crate) space: NativeAddressSpace,
    metadata: SessionMetadata,
    #[cfg(test)]
    pub(crate) fail_append_after_initializer: bool,
}
impl<'plan, 'image> NativeGuestMemory<'plan, 'image> {
    pub(crate) fn from_staged(
        source: &'plan SymbolicLinkPlan<'image>,
        space: NativeAddressSpace,
        metadata: SessionMetadata,
    ) -> Self {
        Self {
            source,
            active_source: source.identity(),
            ordinary_patches: source.patch_count(),
            space,
            metadata,
            #[cfg(test)]
            fail_append_after_initializer: false,
        }
    }
    /// Initial immutable preparation source. Appended source epochs retain this
    /// history; their current identity is reported separately.
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn active_source_identity(&self) -> pentax_linker::SourceIdentity {
        self.active_source
    }
    /// Stable identity of this actual backing; identity alone is not liveness.
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.space.owner_id()
    }
    pub const fn primary_entry(&self) -> u64 {
        self.metadata.primary_entry
    }
    pub const fn module_count(&self) -> usize {
        self.metadata.module_count
    }
    pub const fn applied_relocations(&self) -> usize {
        self.metadata.applied_relocations
    }
    pub fn mapped_bytes(&self) -> u64 {
        self.space.mapped_bytes()
    }
    pub fn committed_bytes(&self) -> u64 {
        self.space.committed_bytes()
    }
    pub fn statistics(&self) -> pentax_platform::native_memory::NativeMemoryStatistics {
        self.space.statistics()
    }
    pub fn reserved_bytes(&self) -> u64 {
        self.space.reserved_bytes()
    }
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, MemoryError> {
        self.space.pause().map_err(convert)
    }
}

mod append;

/// Reuse the ordinary session's exact checked mappings, original copies,
/// relocations and final RELRO permissions on native backing at guest VAs.
/// All deferred records and TLS-bearing inputs are initially refused.
/// Eager commitment counts every mapped guest page against the resident limit.
pub fn prepare_native_memory<'plan, 'image>(
    source: &'plan SymbolicLinkPlan<'image>,
    limits: &NativeLoadLimits,
) -> Result<NativeGuestMemory<'plan, 'image>, ProcessError> {
    prepare_in(source, limits, None)
}

pub(crate) fn prepare_in<'plan, 'image>(
    source: &'plan SymbolicLinkPlan<'image>,
    limits: &NativeLoadLimits,
    space: Option<NativeAddressSpace>,
) -> Result<NativeGuestMemory<'plan, 'image>, ProcessError> {
    if source.deferred_count() != 0 {
        return Err(ProcessError::NativeDeferredRelocations {
            count: source.deferred_count(),
        });
    }
    if let Some(module) = source
        .modules()
        .iter()
        .position(|m| m.runtime_plan().tls.is_some())
    {
        return Err(ProcessError::NativeUnsupportedTls { module });
    }
    let (space, metadata) =
        stage_session_with(SessionPlan::Ordinary(source), &limits.session, |mapped| {
            create_space(mapped, limits, space)
        })?;
    Ok(NativeGuestMemory::from_staged(source, space, metadata))
}

/// Shared native owner construction; every caller preflights its full mapped
/// cost before staging and no separately mapped owner can be paired with it.
pub(crate) fn create_space(
    mapped: u64,
    limits: &NativeLoadLimits,
    space: Option<NativeAddressSpace>,
) -> Result<NativeAddressSpace, ProcessError> {
    if mapped > limits.session.resident_limit {
        return Err(ProcessError::Memory(MemoryError::BudgetExceeded));
    }
    let caps = NativeMemoryLimits {
        mapped_bytes: limits
            .session
            .mapped_limit
            .min(limits.session.resident_limit),
        reserved_bytes: limits.reserved_limit,
    };
    let mut space = match space {
        Some(space) => space,
        None => NativeAddressSpace::new(caps).map_err(|e| ProcessError::Memory(convert(e)))?,
    };
    space
        .restrict_limits(caps)
        .map_err(|e| ProcessError::Memory(convert(e)))?;
    if space.mapped_bytes() != 0 {
        return Err(ProcessError::Memory(MemoryError::Overlap));
    }
    Ok(space)
}
