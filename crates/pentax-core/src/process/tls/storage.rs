// SPDX-License-Identifier: GPL-2.0-or-later

use super::{
    PageRange, PreparedTlsSession, RelocatedStaticTlsPlan, StaticTlsInstance, StaticTlsPlan,
    StaticTlsStorage, ThreadControlBlockPolicy, TlsBlock, TlsBudget, TlsError, TlsStorageRange,
    Work, charge, merge_ranges, page_range, round_pages,
};
use crate::memory::service::StagingMemory;
use crate::memory::{ADDRESS_LIMIT, GuestMemory, MemoryError, PAGE_SIZE, Permissions};
use crate::process::{
    ProcessError, SessionLoadLimits, SessionMetadata, SessionPlan, UnstartedSession,
    stage_session_with,
};

/// The only preparation inputs. Layout and staging both derive from one variant.
pub(super) enum Preparation<'a, 'binding, 'plan, 'image> {
    Ordinary(&'a StaticTlsPlan<'plan, 'image>),
    Relocated(&'a RelocatedStaticTlsPlan<'binding, 'plan, 'image>),
}

impl<'plan, 'image> Preparation<'_, '_, 'plan, 'image> {
    fn layout(&self) -> &StaticTlsPlan<'plan, 'image> {
        match self {
            Self::Ordinary(plan) => plan,
            Self::Relocated(plan) => &plan.layout,
        }
    }

    fn source(&self) -> SessionPlan<'_, '_, 'plan, 'image> {
        match self {
            Self::Ordinary(plan) => SessionPlan::Ordinary(plan.link),
            Self::Relocated(plan) => SessionPlan::Relocated(plan),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Backing {
    Sparse,
    Native { reserved_limit: u64 },
}

pub(super) struct Materialized<M> {
    pub memory: M,
    pub metadata: SessionMetadata,
    pub templates: Vec<super::TlsTemplateLayout>,
    pub instances: Vec<StaticTlsInstance>,
}

/// Stage the retained module set and copy its relocated initialized TLS bytes.
///
/// The only external addresses are requested guest TP positions. A private plan
/// owns all source identity and layout authority. Complete relocations, logical
/// source backing, byte agreement, independent guarded RW/NX destinations and
/// aggregate mapped/resident costs are checked before any guest allocation.
/// Failure drops the entire unpublished session. Only an explicitly requested
/// observed header may be created; no DTV, host FS mutation or public
/// executable/mutable session capability is produced.
pub fn load_symbolic_with_static_tls(
    plan: &StaticTlsPlan<'_, '_>,
    storage: &StaticTlsStorage<'_>,
    limits: &SessionLoadLimits,
) -> Result<PreparedTlsSession, TlsError> {
    if plan.link.deferred_count() != 0 {
        return Err(TlsError::DeferredRelocations {
            count: plan.link.deferred_count(),
        });
    }
    prepare(Preparation::Ordinary(plan), storage, limits)
}

pub(super) fn prepare(
    preparation: Preparation<'_, '_, '_, '_>,
    storage: &StaticTlsStorage<'_>,
    limits: &SessionLoadLimits,
) -> Result<PreparedTlsSession, TlsError> {
    let materialized = prepare_with(preparation, storage, limits, Backing::Sparse, |_| {
        Ok(GuestMemory::new(limits.resident_limit, limits.mapped_limit))
    })?;
    let metadata = materialized.metadata;
    Ok(PreparedTlsSession {
        session: UnstartedSession {
            memory: materialized.memory,
            primary_entry: metadata.primary_entry,
            module_count: metadata.module_count,
            applied_relocations: metadata.applied_relocations,
            deferred_relocations: metadata.deferred_relocations,
        },
        templates: materialized.templates,
        instances: materialized.instances,
    })
}

/// The same source/layout/storage checks and copy loop serve sparse and native
/// backing. The callback constructs an empty owner only after all preflight.
pub(super) fn prepare_with<M: StagingMemory>(
    preparation: Preparation<'_, '_, '_, '_>,
    storage: &StaticTlsStorage<'_>,
    limits: &SessionLoadLimits,
    backing: Backing,
    create: impl FnOnce(u64) -> Result<M, ProcessError>,
) -> Result<Materialized<M>, TlsError> {
    let plan = preparation.layout();
    let mut work = Work {
        used: 0,
        limit: plan.limits.max_work_steps,
    };
    let mut instance_count = 0;
    charge(
        &mut instance_count,
        storage.thread_pointers.len() as u64,
        plan.limits.max_instances,
        TlsBudget::Instances,
    )?;
    work.charge(instance_count)?;
    work.charge(plan.templates.len() as u64)?;
    let rounded_size = round_pages(plan.static_extent.max(plan.limits.reserved_static_bytes))?;
    let tcb_page = match plan.limits.thread_control {
        ThreadControlBlockPolicy::None => 0,
        ThreadControlBlockPolicy::KytyObservedSelfPointerV1 => PAGE_SIZE,
    };
    let instance_bytes = rounded_size
        .checked_add(tcb_page)
        .ok_or(TlsError::AddressOverflow)?;
    let storage_bytes = instance_bytes
        .checked_mul(instance_count)
        .ok_or(TlsError::AddressOverflow)?;
    if storage_bytes > plan.limits.max_storage_bytes {
        return Err(TlsError::Budget {
            kind: TlsBudget::StorageBytes,
            limit: plan.limits.max_storage_bytes,
        });
    }
    let mapped = plan
        .image_mapped_bytes
        .checked_add(storage_bytes)
        .ok_or(TlsError::AddressOverflow)?;
    if mapped > limits.mapped_limit {
        return Err(TlsError::MemoryBudget {
            resident: false,
            needed: mapped,
            limit: limits.mapped_limit,
        });
    }
    let mut instances = Vec::new();
    let mut reservations: Vec<PageRange> = Vec::new();
    let mut resident_ranges = Vec::new();
    for (index, &tp) in storage.thread_pointers.iter().enumerate() {
        let invalid = |reason| TlsError::InvalidStorage {
            instance: index,
            reason,
        };
        if tp >= ADDRESS_LIMIT || !tp.is_multiple_of(PAGE_SIZE) {
            return Err(invalid("guest TP must be page-aligned and in range"));
        }
        work.charge(plan.templates.len() as u64)?;
        let (range, guards) = if instance_bytes == 0 {
            (None, None)
        } else {
            let start = tp
                .checked_sub(rounded_size)
                .ok_or_else(|| invalid("storage underflows TP"))?;
            let before = start
                .checked_sub(PAGE_SIZE)
                .ok_or_else(|| invalid("lower guard underflows"))?;
            let upper = tp.checked_add(tcb_page).ok_or(TlsError::AddressOverflow)?;
            let end = upper
                .checked_add(PAGE_SIZE)
                .filter(|end| *end <= ADDRESS_LIMIT)
                .ok_or_else(|| invalid("upper guard exceeds guest range"))?;
            let reservation = PageRange { start: before, end };
            work.charge(plan.image_pages.len() as u64)?;
            work.charge(reservations.len() as u64)?;
            if plan
                .image_pages
                .iter()
                .chain(&reservations)
                .any(|other| reservation.overlaps(*other))
            {
                return Err(TlsError::StorageOverlap { instance: index });
            }
            reservations.push(reservation);
            (
                (rounded_size != 0).then_some(TlsStorageRange {
                    address: start,
                    size: rounded_size,
                }),
                Some([before, upper]),
            )
        };
        let thread_control_block = if tcb_page == 0 {
            None
        } else {
            work.charge(64)?;
            resident_ranges.push(page_range(tp, 64)?);
            Some(TlsStorageRange {
                address: tp,
                size: 64,
            })
        };
        let mut blocks = Vec::new();
        for template in &plan.templates {
            let address = tp
                .checked_sub(template.tp_offset)
                .ok_or_else(|| invalid("block underflows TP"))?;
            let end = address
                .checked_add(template.mem_size)
                .ok_or(TlsError::AddressOverflow)?;
            if end > tp || address % template.alignment != template.alignment_bias {
                return Err(invalid("block layout or alignment congruence"));
            }
            if template.initialized_size != 0 {
                resident_ranges.push(page_range(address, template.initialized_size)?);
            }
            blocks.push(TlsBlock {
                module: template.module,
                runtime_id: template.runtime_id,
                address,
                size: template.mem_size,
            });
        }
        instances.push(StaticTlsInstance {
            thread_pointer: tp,
            thread_control_block,
            storage: range,
            guard_pages: guards,
            blocks,
        });
    }
    let sparse_resident = plan
        .image_resident_bytes
        .checked_add(merge_ranges(&mut resident_ranges)?)
        .ok_or(TlsError::AddressOverflow)?;
    let resident = match backing {
        Backing::Sparse => sparse_resident,
        Backing::Native { reserved_limit } => {
            native_reservations(plan, &instances, reserved_limit, &mut work)?;
            mapped
        }
    };
    if resident > limits.resident_limit {
        return Err(TlsError::MemoryBudget {
            resident: true,
            needed: resident,
            limit: limits.resident_limit,
        });
    }
    // No separately supplied session or memory can be paired with this plan.
    // Relocated staging applies its TLS writes before final permissions.
    let (mut memory, metadata) =
        stage_session_with(preparation.source(), limits, |_| create(mapped))
            .map_err(TlsError::Staging)?;
    check_guards(&mut memory, &instances, false)?;
    for instance in &instances {
        if let Some(range) = instance.storage {
            memory.map(range.address, range.size, Permissions::READ_WRITE)?;
        }
        if let Some(header) = instance.thread_control_block {
            memory.map(header.address, PAGE_SIZE, Permissions::READ_WRITE)?;
            memory.write(header.address, &instance.thread_pointer.to_le_bytes())?;
        }
    }
    let mut buffer = [0; 4096];
    for instance in &instances {
        for (template, block) in plan.templates.iter().zip(&instance.blocks) {
            let mut offset = 0;
            while offset < template.initialized_size {
                let count = (template.initialized_size - offset).min(buffer.len() as u64) as usize;
                memory.read(template.source_address + offset, &mut buffer[..count])?;
                memory.write(block.address + offset, &buffer[..count])?;
                offset += count as u64;
            }
            // Both fresh sparse and fresh committed native mappings supply a
            // complete zero tail without copying original uninitialized bytes.
        }
    }
    check_guards(
        &mut memory,
        &instances,
        matches!(backing, Backing::Native { .. }),
    )?;
    Ok(Materialized {
        memory,
        metadata,
        templates: plan.templates.clone(),
        instances,
    })
}

fn check_guards(
    memory: &mut impl StagingMemory,
    instances: &[StaticTlsInstance],
    require_owned: bool,
) -> Result<(), TlsError> {
    for (index, instance) in instances.iter().enumerate() {
        if let Some(guards) = instance.guard_pages {
            for guard in guards {
                if require_owned && !memory.owns_guard_reservation(guard, PAGE_SIZE) {
                    return Err(TlsError::NativeGuardOutsideReservation { instance: index });
                }
                if !matches!(memory.read(guard, &mut [0]), Err(MemoryError::Unmapped(_))) {
                    return Err(TlsError::GuardMapped { instance: index });
                }
            }
        }
    }
    Ok(())
}

fn native_reservations(
    plan: &StaticTlsPlan<'_, '_>,
    instances: &[StaticTlsInstance],
    limit: u64,
    work: &mut Work,
) -> Result<(), TlsError> {
    // The native TLS factory separately requires 64 KiB allocation granularity.
    // Guards must fall inside owned reservations while remaining uncommitted.
    const GRANULARITY: u64 = 64 * 1024;
    let count = (plan.image_pages.len() as u64)
        .checked_add(instances.len() as u64)
        .and_then(|count| {
            count.checked_add(
                if plan.limits.thread_control == ThreadControlBlockPolicy::None {
                    0
                } else {
                    instances.len() as u64
                },
            )
        })
        .ok_or(TlsError::AddressOverflow)?;
    work.charge(
        count
            .checked_mul(u64::from(count.max(1).ilog2()) + 2)
            .ok_or(TlsError::AddressOverflow)?,
    )?;
    let mut reservations = plan.image_pages.to_vec();
    for instance in instances {
        if let Some(range) = instance.storage {
            reservations.push(PageRange {
                start: range.address,
                end: range.address + range.size,
            });
        }
        if let Some(header) = instance.thread_control_block {
            reservations.push(page_range(header.address, header.size)?);
        }
    }
    for range in &mut reservations {
        range.start &= !(GRANULARITY - 1);
        range.end = range
            .end
            .checked_add(GRANULARITY - 1)
            .ok_or(TlsError::AddressOverflow)?
            & !(GRANULARITY - 1);
    }
    let reserved = merge_ranges(&mut reservations)?;
    if reserved > limit {
        return Err(TlsError::NativeReservationBudget {
            needed: reserved,
            limit,
        });
    }
    // A native owner can retain one allocation record per reservation unit.
    // Charge the final actual-owner guard scans before any OS allocation too.
    let guards = instances
        .iter()
        .filter(|instance| instance.guard_pages.is_some())
        .count() as u64;
    work.charge(
        guards
            .checked_mul(2)
            .and_then(|checks| checks.checked_mul(reserved / GRANULARITY))
            .ok_or(TlsError::AddressOverflow)?,
    )?;
    for (index, instance) in instances.iter().enumerate() {
        if let Some(guards) = instance.guard_pages {
            work.charge(
                (reservations.len() as u64)
                    .checked_mul(2)
                    .ok_or(TlsError::AddressOverflow)?,
            )?;
            for guard in guards {
                if !reservations
                    .iter()
                    .any(|range| range.start <= guard && guard + PAGE_SIZE <= range.end)
                {
                    return Err(TlsError::NativeGuardOutsideReservation { instance: index });
                }
            }
        }
    }
    Ok(())
}
