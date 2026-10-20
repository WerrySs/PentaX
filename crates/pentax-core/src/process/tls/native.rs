// SPDX-License-Identifier: GPL-2.0-only

//! Typed TLS over the same owned physically NX pages used by native HLE.

use super::storage::{Backing, Preparation, prepare_with};
use super::{
    RelocatedStaticTlsPlan, StaticTlsInstance, StaticTlsStorage, ThreadControlBlockPolicy,
    TlsError, TlsTemplateLayout,
};
use crate::memory::MemoryError;
use crate::memory::native::{NativeGuestMemory, NativeLoadLimits, create_space};
use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryError, PausedNativeMemory};

/// Actual native backing with immutable template and instance diagnostics.
/// The initial source remains borrowed; current epoch history and TLS metadata
/// update only after private transactional publication. Temporary plans do not
/// become self-references.
/// An explicit observed self-pointer header is optional. No host FS mutation,
/// complete firmware TCB/DTV or executable conversion is exposed.
#[derive(Debug)]
pub struct PreparedNativeTlsSession<'plan, 'image> {
    pub(crate) memory: NativeGuestMemory<'plan, 'image>,
    templates: Vec<TlsTemplateLayout>,
    instances: Vec<StaticTlsInstance>,
    thread_control: ThreadControlBlockPolicy,
    limits: super::StaticTlsLimits,
    initializers: Vec<Vec<u8>>,
}

impl<'plan, 'image> PreparedNativeTlsSession<'plan, 'image> {
    pub const fn thread_control_policy(&self) -> ThreadControlBlockPolicy {
        self.thread_control
    }
    pub fn memory(&self) -> &NativeGuestMemory<'plan, 'image> {
        &self.memory
    }
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, TlsError> {
        self.memory.pause().map_err(TlsError::Memory)
    }
    pub fn templates(&self) -> &[TlsTemplateLayout] {
        &self.templates
    }
    pub fn instances(&self) -> &[StaticTlsInstance] {
        &self.instances
    }
    pub(crate) fn pause_with_instances(
        &mut self,
    ) -> Result<(PausedNativeMemory<'_>, &[StaticTlsInstance]), TlsError> {
        Ok((
            self.memory.pause().map_err(TlsError::Memory)?,
            &self.instances,
        ))
    }
}

mod append;
mod threads;

/// Stage exact ordinary and typed TLS writes before final permissions and copy
/// relocated initialized templates into independent guarded RW/NX blocks.
/// Every mapped page is eagerly committed and charged to resident limits.
/// Native guards must fit inside the image/storage's owned 64 KiB reservation
/// geometry; unsupported boundary placements fail before OS allocation.
pub fn load_native_relocated_static_tls<'plan, 'image>(
    plan: &RelocatedStaticTlsPlan<'_, 'plan, 'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
) -> Result<PreparedNativeTlsSession<'plan, 'image>, TlsError> {
    prepare_native_in(plan, storage, limits, None)
}

pub(crate) fn prepare_native_in<'plan, 'image>(
    plan: &RelocatedStaticTlsPlan<'_, 'plan, 'image>,
    storage: &StaticTlsStorage<'_>,
    limits: &NativeLoadLimits,
    space: Option<NativeAddressSpace>,
) -> Result<PreparedNativeTlsSession<'plan, 'image>, TlsError> {
    let granularity = match &space {
        Some(owner) => owner.allocation_granularity(),
        None => {
            pentax_platform::host_layout()
                .map_err(|_| {
                    TlsError::Memory(MemoryError::Native(NativeMemoryError::UnsupportedHost))
                })?
                .allocation_granularity as u64
        }
    };
    if granularity != 64 * 1024 {
        return Err(TlsError::NativeAllocationGranularity {
            actual: granularity,
        });
    }
    let source = plan.bindings().base_plan();
    let mut materialized = prepare_with(
        Preparation::Relocated(plan),
        storage,
        &limits.session,
        Backing::Native {
            reserved_limit: limits.reserved_limit,
        },
        |mapped| create_space(mapped, limits, space),
    )?;
    let mut initializers =
        threads::allocate_initializers(&materialized.templates, &plan.layout.limits)?;
    {
        let paused = materialized
            .memory
            .pause()
            .map_err(|e| TlsError::Memory(MemoryError::Native(e)))?;
        for (template, bytes) in materialized.templates.iter().zip(&mut initializers) {
            paused
                .read(template.source_address, bytes)
                .map_err(|e| TlsError::Memory(MemoryError::Native(e)))?;
        }
    }
    Ok(PreparedNativeTlsSession {
        memory: NativeGuestMemory::from_staged(source, materialized.memory, materialized.metadata),
        templates: materialized.templates,
        instances: materialized.instances,
        thread_control: plan.layout.thread_control_policy(),
        limits: plan.layout.limits,
        initializers,
    })
}
