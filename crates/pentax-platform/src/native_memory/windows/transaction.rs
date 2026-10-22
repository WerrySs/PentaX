// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked exclusive transactions and prospective
// outputs, retaining source mapping publication/rollback ordering. See README.md.

use super::*;

#[derive(Clone, Copy)]
struct ProspectiveRange {
    address: u64,
    end: u64,
    permissions: NativePermissions,
    mapped: bool,
    // None preserves backing; Some(None) creates private backing; Some(Some(p))
    // creates a shared physical view. This also admits output alias checks.
    backing: Option<Option<u64>>,
}

fn steps(used: &mut u64, count: u64, maximum: u64) -> Result<(), NativeMemoryError> {
    *used = used
        .checked_add(count)
        .filter(|v| *v <= maximum)
        .ok_or(NativeMemoryError::BudgetExceeded)?;
    Ok(())
}

impl PausedNativeMemory<'_> {
    /// Prove two actual mapped spans share no backing bytes, including physical
    /// aliases at different virtual addresses. No allocation, payload read or
    /// capability is produced. The complete comparison is charged beforehand.
    pub fn preflight_disjoint_backing(
        &self,
        left: NativeRange,
        right: NativeRange,
        maximum: u64,
    ) -> Result<u64, NativeMemoryError> {
        self.owner.live()?;
        if left.size == 0 || right.size == 0 {
            return Ok(0);
        }
        let left_end = range(left.address, left.size)?;
        let right_end = range(right.address, right.size)?;
        let first = |address| address / NATIVE_GUEST_PAGE * NATIVE_GUEST_PAGE;
        let left_first = first(left.address);
        let right_first = first(right.address);
        let lp = (left_end - 1) / NATIVE_GUEST_PAGE - left_first / NATIVE_GUEST_PAGE + 1;
        let rp = (right_end - 1) / NATIVE_GUEST_PAGE - right_first / NATIVE_GUEST_PAGE + 1;
        let work = lp
            .checked_mul(rp)
            .and_then(|v| v.checked_add(lp.checked_add(rp)?.checked_mul(3)?))
            .and_then(|v| v.checked_add(1))
            .filter(|v| *v <= maximum)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        self.owner.check(left.address, left.size, None)?;
        self.owner.check(right.address, right.size, None)?;
        // The full mapped checks and original conservative charge precede every
        // shortcut. Any virtual overlap necessarily shares actual backing bytes.
        if left.address < right_end && right.address < left_end {
            return Err(NativeMemoryError::Overlap);
        }
        let (first, end) = if lp <= rp {
            (left_first, left_end)
        } else {
            (right_first, right_end)
        };
        // An exclusively owned private page has coordinate (false, virtual),
        // never a shared-section coordinate. Inspect every touched CURRENT page;
        // no first-page inference or retained mapping/permission proof is used.
        // Even if the final classification fails and the old Cartesian loop runs,
        // mapped checks lp+rp, outer coordinates lp and classification min(lp,rp)
        // total <=3*(lp+rp). The original lp*rp+3*(lp+rp)+1 charge stays intact.
        if (first..end)
            .step_by(NATIVE_GUEST_PAGE as usize)
            .all(|page| matches!(self.owner.region(page).kind, RegionKind::Private))
        {
            return Ok(work);
        }
        for page in (left_first..left_end).step_by(NATIVE_GUEST_PAGE as usize) {
            let at = page.max(left.address);
            let length = left_end.min(page + NATIVE_GUEST_PAGE) - at;
            let (kind, coordinate) = self.output_coordinate(at, None);
            for other_page in (right_first..right_end).step_by(NATIVE_GUEST_PAGE as usize) {
                let other = other_page.max(right.address);
                let other_length = right_end.min(other_page + NATIVE_GUEST_PAGE) - other;
                let (other_kind, other_coordinate) = self.output_coordinate(other, None);
                if kind == other_kind
                    && coordinate < other_coordinate + other_length
                    && other_coordinate < coordinate + length
                {
                    return Err(NativeMemoryError::Overlap);
                }
            }
        }
        Ok(work)
    }
    /// Check for repeated physical bytes through self-aliasing. No bytes change
    /// and no capability escapes. The returned charge also admits one same-size
    /// None-effect output transaction on this exclusively held owner.
    pub fn preflight_distinct_backing(
        &self,
        address: u64,
        size: u64,
        maximum: u64,
    ) -> Result<u64, NativeMemoryError> {
        self.owner.live()?;
        if size == 0 {
            return Ok(0);
        }
        let end = range(address, size)?;
        let first = address / NATIVE_GUEST_PAGE * NATIVE_GUEST_PAGE;
        let pages = (end - 1) / NATIVE_GUEST_PAGE - first / NATIVE_GUEST_PAGE + 1;
        let work = pages
            .checked_mul(pages)
            .and_then(|n| n.checked_add(pages.checked_mul(4)?))
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= maximum)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        self.owner.check(address, size, None)?;
        for page in (first..end).step_by(NATIVE_GUEST_PAGE as usize) {
            let at = page.max(address);
            let length = end.min(page + NATIVE_GUEST_PAGE) - at;
            let (kind, coordinate) = self.output_coordinate(at, None);
            for prior in (first..page).step_by(NATIVE_GUEST_PAGE as usize) {
                let other = prior.max(address);
                let other_length = end.min(prior + NATIVE_GUEST_PAGE) - other;
                let (other_kind, other_coordinate) = self.output_coordinate(other, None);
                if kind == other_kind
                    && coordinate < other_coordinate + other_length
                    && other_coordinate < coordinate + length
                {
                    return Err(NativeMemoryError::Overlap);
                }
            }
        }
        Ok(work)
    }
    /// Apply one effect and all outputs on this same owned backing. Recoverable
    /// errors publish no effect or output. Partial decommit or failed rollback poisons the
    /// owner: every later pause/read/write/source-fetch is denied, and the
    /// caller must stop rather than retry or resume execution.
    pub fn apply_transaction(
        &mut self,
        transaction: NativeMemoryTransaction<'_>,
        limits: &NativeTransactionLimits,
    ) -> Result<(), NativeMemoryError> {
        self.apply_with_write_permission(
            transaction,
            limits,
            NativePermissions::new(false, true, false),
        )
    }
    /// Publish GPU results atomically on this exact paused owner after fencing.
    /// CPU access is irrelevant; outputs require GPU write permission. This API
    /// admits no mapping effects and retains alias/overlap/byte/work checks.
    pub fn apply_gpu_writes(
        &mut self,
        writes: &[NativeMemoryWrite<'_>],
        limits: &NativeTransactionLimits,
    ) -> Result<(), NativeMemoryError> {
        self.apply_with_write_permission(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::None,
                writes,
            },
            limits,
            NativePermissions::new(false, false, false).with_gpu(false, true),
        )
    }
    fn apply_with_write_permission(
        &mut self,
        transaction: NativeMemoryTransaction<'_>,
        limits: &NativeTransactionLimits,
        required: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.owner.last_host_failure = None;
        let result = self.apply_checked_write_permission(transaction, limits, required);
        if let Err(NativeMemoryError::OperatingSystem {
            operation, code, ..
        }) = &result
        {
            let operation = match *operation {
                "CreateFileMappingW(NX pagefile)" => NativeHostMemoryOperation::SectionCreate,
                "VirtualAlloc(NX section commit)" => NativeHostMemoryOperation::SectionCommit,
                "MapViewOfFile3(NX placeholder)" => NativeHostMemoryOperation::SectionMap,
                "VirtualAlloc2(private placeholder)" => NativeHostMemoryOperation::PrivateCommit,
                "VirtualAlloc2(placeholder)" => NativeHostMemoryOperation::Reserve,
                "VirtualProtect(NX)" => NativeHostMemoryOperation::Protect,
                name if name.starts_with("backing ") => NativeHostMemoryOperation::BackingFile,
                _ => NativeHostMemoryOperation::Other,
            };
            self.owner.last_host_failure = Some(NativeHostMemoryFailure {
                operation,
                code: *code,
            });
        }
        result
    }
    fn apply_checked_write_permission(
        &mut self,
        transaction: NativeMemoryTransaction<'_>,
        limits: &NativeTransactionLimits,
        required: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        // Mutation transactions clear permission metadata before any validation
        // or possible OS effect, including a later preflight refusal. None-effect
        // payload writes do not change permissions or make source bytes trusted.
        if !matches!(transaction.effect, NativeMemoryEffect::None) {
            self.owner.invalidate_stack_preflight();
        }
        self.owner.live()?;
        let NativeMemoryTransaction { effect, writes } = transaction;
        if writes.len() > limits.max_outputs {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        let mut work = 0;
        let mut bytes = 0_u64;
        let prospective = match effect {
            NativeMemoryEffect::None => None,
            NativeMemoryEffect::AllocatePhysical { physical, size } => {
                steps(
                    &mut work,
                    (self.owner.physical_allocations.len() as u64 + 1) * 2
                        + self.owner.physical_sections.len() as u64
                        + 1,
                    limits.max_preflight_steps,
                )?;
                self.owner.preflight_physical_allocation(physical, size)?;
                None
            }
            NativeMemoryEffect::ReleasePhysical { physical, size } => {
                let allocations = self.owner.physical_allocations.len() as u64 + 1;
                let sections = self.owner.physical_sections.len() as u64 + 1;
                let visits = allocations
                    .checked_mul(allocations)
                    .and_then(|n| n.checked_mul(2))
                    .and_then(|n| n.checked_add(sections.checked_mul(allocations)?))
                    .and_then(|n| n.checked_add((self.owner.pages.len() as u64).checked_mul(10)?))
                    .ok_or(NativeMemoryError::BudgetExceeded)?;
                steps(&mut work, visits, limits.max_preflight_steps)?;
                self.owner.preflight_physical_release(physical, size)?;
                None
            }
            NativeMemoryEffect::MapPhysical {
                address,
                size,
                physical,
                permissions,
            } => {
                let end = range(address, size)?;
                let pages = size / NATIVE_GUEST_PAGE;
                let granules = size / self.owner.granularity + 2;
                steps(
                    &mut work,
                    pages
                        .checked_mul(4)
                        .and_then(|v| v.checked_add(granules * 4))
                        .and_then(|v| {
                            v.checked_add((self.owner.physical_allocations.len() as u64 + 1) * 4)
                        })
                        .ok_or(NativeMemoryError::BudgetExceeded)?,
                    limits.max_preflight_steps,
                )?;
                self.owner
                    .preflight_physical_map(address, size, physical, permissions)?;
                Some(ProspectiveRange {
                    address,
                    end,
                    permissions,
                    mapped: true,
                    backing: Some(Some(physical)),
                })
            }
            NativeMemoryEffect::MapZeroed {
                address,
                size,
                permissions,
            }
            | NativeMemoryEffect::Protect {
                address,
                size,
                permissions,
            } => {
                let end = range(address, size)?;
                if !address.is_multiple_of(NATIVE_GUEST_PAGE)
                    || !size.is_multiple_of(NATIVE_GUEST_PAGE)
                {
                    return Err(NativeMemoryError::Unaligned);
                }
                // Map/protect each perform a bounded validation pass again and
                // may perform a full rollback pass. Precharge before traversing.
                let pages = size / NATIVE_GUEST_PAGE;
                let granules = size / self.owner.granularity + 2;
                steps(
                    &mut work,
                    pages
                        .checked_mul(4)
                        .and_then(|v| v.checked_add(granules * 4))
                        .ok_or(NativeMemoryError::BudgetExceeded)?,
                    limits.max_preflight_steps,
                )?;
                match effect {
                    NativeMemoryEffect::MapZeroed { .. } => {
                        self.owner.preflight_map(address, size, true)?;
                    }
                    NativeMemoryEffect::Protect { .. } => {
                        self.owner.check(address, size, None)?;
                    }
                    _ => unreachable!(),
                }
                Some(ProspectiveRange {
                    address,
                    end,
                    permissions,
                    mapped: true,
                    backing: if matches!(effect, NativeMemoryEffect::MapZeroed { .. }) {
                        Some(None)
                    } else {
                        None
                    },
                })
            }
            NativeMemoryEffect::Unmap { address, size } => {
                let end = range(address, size)?;
                if !address.is_multiple_of(NATIVE_GUEST_PAGE)
                    || !size.is_multiple_of(NATIVE_GUEST_PAGE)
                {
                    return Err(NativeMemoryError::Unaligned);
                }
                steps(
                    &mut work,
                    (size / NATIVE_GUEST_PAGE)
                        .checked_mul(4)
                        .ok_or(NativeMemoryError::BudgetExceeded)?,
                    limits.max_preflight_steps,
                )?;
                self.owner.check(address, size, None)?;
                Some(ProspectiveRange {
                    address,
                    end,
                    permissions: NativePermissions::new(false, false, false),
                    mapped: false,
                    backing: None,
                })
            }
        };
        // Count every output and every overlap comparison before any page walk.
        let count = writes.len() as u64;
        steps(
            &mut work,
            count
                .checked_mul(count)
                .ok_or(NativeMemoryError::BudgetExceeded)?,
            limits.max_preflight_steps,
        )?;
        let mut output_pages = 0_u64;
        for write in writes {
            bytes = bytes
                .checked_add(write.bytes.len() as u64)
                .filter(|v| *v <= limits.max_output_bytes)
                .ok_or(NativeMemoryError::BudgetExceeded)?;
            if !write.bytes.is_empty() {
                let end = range(write.address, write.bytes.len() as u64)?;
                let pages = (end - 1) / NATIVE_GUEST_PAGE - write.address / NATIVE_GUEST_PAGE + 1;
                output_pages = output_pages
                    .checked_add(pages)
                    .ok_or(NativeMemoryError::BudgetExceeded)?;
                steps(
                    &mut work,
                    pages
                        .checked_mul(4)
                        .ok_or(NativeMemoryError::BudgetExceeded)?,
                    limits.max_preflight_steps,
                )?;
            }
        }
        // A pair of different guest addresses may alias the same physical
        // output bytes. Admit every chunk comparison before traversing them.
        steps(
            &mut work,
            output_pages
                .checked_mul(output_pages)
                .ok_or(NativeMemoryError::BudgetExceeded)?,
            limits.max_preflight_steps,
        )?;
        for (index, write) in writes.iter().enumerate() {
            if write.bytes.is_empty() {
                continue;
            }
            let end = range(write.address, write.bytes.len() as u64)?;
            for prior in &writes[..index] {
                if !prior.bytes.is_empty() {
                    let prior_end = range(prior.address, prior.bytes.len() as u64)?;
                    if prior.address < end && write.address < prior_end {
                        return Err(NativeMemoryError::Overlap);
                    }
                }
            }
            for page in (write.address / NATIVE_GUEST_PAGE * NATIVE_GUEST_PAGE..end)
                .step_by(NATIVE_GUEST_PAGE as usize)
            {
                let at = page.max(write.address);
                if let Some(change) = prospective.filter(|r| r.address <= page && page < r.end) {
                    if !change.mapped {
                        return Err(NativeMemoryError::Unmapped(at));
                    }
                    if change.permissions.0 & required.0 != required.0 {
                        return Err(NativeMemoryError::PermissionDenied(at));
                    }
                } else {
                    self.owner.check(
                        at,
                        (page + NATIVE_GUEST_PAGE).min(end) - at,
                        Some(required),
                    )?;
                    if let NativeMemoryEffect::ReleasePhysical { physical, size } = effect
                        && let RegionKind::Shared {
                            physical: at_physical,
                        } = self.owner.region(at).kind
                        && physical <= at_physical
                        && at_physical < physical + size
                    {
                        return Err(NativeMemoryError::Unmapped(at));
                    }
                }
            }
        }

        for (index, write) in writes
            .iter()
            .enumerate()
            .filter(|(_, w)| !w.bytes.is_empty())
        {
            for (prior_index, prior) in writes[..=index]
                .iter()
                .enumerate()
                .filter(|(_, w)| !w.bytes.is_empty())
            {
                for (at, length) in chunks(write) {
                    let (kind, coordinate) = self.output_coordinate(at, prospective);
                    for (other, other_length) in chunks(prior) {
                        if prior_index == index && other >= at {
                            continue;
                        }
                        let (other_kind, other_coordinate) =
                            self.output_coordinate(other, prospective);
                        if kind == other_kind
                            && coordinate < other_coordinate + other_length
                            && other_coordinate < coordinate + length
                        {
                            return Err(NativeMemoryError::Overlap);
                        }
                    }
                }
            }
        }

        // Phase two has no caller callback or output allocation. The effect's
        // map/protect implementations roll back failed OS work. Unmap retains
        // reservations and poisons the owner if decommit only partly succeeds;
        // irreversible page contents are never advertised as recoverable.
        match effect {
            NativeMemoryEffect::None => {}
            NativeMemoryEffect::AllocatePhysical { physical, size } => {
                self.owner.allocate_physical(physical, size)?
            }
            NativeMemoryEffect::ReleasePhysical { physical, size } => {
                self.owner
                    .release_physical(physical, size, limits.max_scratch_bytes)?
            }
            NativeMemoryEffect::MapPhysical {
                address,
                size,
                physical,
                permissions,
            } => {
                self.owner
                    .map_physical(address, size, physical, permissions)?;
            }
            NativeMemoryEffect::MapZeroed {
                address,
                size,
                permissions,
            } => {
                self.owner.map_at(address, size, permissions)?;
            }
            NativeMemoryEffect::Protect {
                address,
                size,
                permissions,
            } => {
                self.owner.protect(address, size, permissions)?;
            }
            NativeMemoryEffect::Unmap { address, size } => {
                self.owner.unmap(address, size)?;
            }
        }
        // Validated final permissions now match the prospective view. Under
        // this exclusive borrow no page can change; write repeats its checks
        // then copies without allocating or issuing a fallible OS operation.
        for write in writes {
            if self.copy_in(write.address, write.bytes, required).is_err() {
                // The prospective checks make this unreachable for a healthy
                // exclusive owner. Never expose a partially committed effect
                // or output as an ordinary retryable write failure.
                self.owner.poisoned = true;
                return Err(NativeMemoryError::Poisoned);
            }
        }
        Ok(())
    }
    fn output_coordinate(&self, at: u64, prospective: Option<ProspectiveRange>) -> (bool, u64) {
        if let Some(change) = prospective.filter(|r| r.address <= at && at < r.end)
            && let Some(backing) = change.backing
        {
            return backing.map_or((false, at), |p| (true, p + at - change.address));
        }
        let region = self.owner.region(at);
        match region.kind {
            RegionKind::Shared { physical } => (true, physical + at - region.address),
            _ => (false, at),
        }
    }
}

fn chunks(write: &NativeMemoryWrite<'_>) -> impl Iterator<Item = (u64, u64)> {
    // All ranges have already passed complete overflow and backing admission.
    let end = write.address + write.bytes.len() as u64;
    (write.address / NATIVE_GUEST_PAGE * NATIVE_GUEST_PAGE..end)
        .step_by(NATIVE_GUEST_PAGE as usize)
        .map(move |page| {
            let at = write.address.max(page);
            (at, end.min(page + NATIVE_GUEST_PAGE) - at)
        })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod private_disjoint_tests;
