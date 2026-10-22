// SPDX-License-Identifier: GPL-2.0-only

//! Pure plans retained only for one borrowed source epoch and cache placement.
//! A hit never authorizes execution; the caller still fetches live source and
//! the checked stepper revalidates owner, source, stack and actual operands.

use super::*;
use pentax_codegen::{CachePlacement, MaterializedBlockPlan};

// Match the runtime's 4096 native placements so distinct retained native slots
// do not compete for a smaller decoded-plan table. Plan bytes stay independent.
const SLOTS: usize = 4096;
const MAX_PLAN_BYTES: u64 = 8 * 1024 * 1024;

mod initialized_windows;

/// Diagnostics for committed pure-plan retention, never execution authority.
/// Collision and same-key replacement describe the occupied incoming slot;
/// either may coincide with a byte-pressure purge. They must not be added to
/// purge counts as disjoint reasons. Gauges use the existing retained plan
/// allocation ledger, including its conservative encoder/replacement charges.
/// They exclude slot/memo metadata and are not a simultaneous/process heap peak.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct PlanCacheObservations {
    pub occupied_collisions: u64,
    pub same_key_replacements: u64,
    pub byte_purges: u64,
    pub purged_plans: u64,
    pub retained_bytes: u64,
    pub peak_retained_bytes: u64,
    pub overflow: bool,
}

impl PlanCacheObservations {
    fn add(value: &mut u64, count: u64, overflow: &mut bool) {
        if let Some(next) = value.checked_add(count) {
            *value = next;
        } else {
            *value = u64::MAX;
            *overflow = true;
        }
    }
    fn committed(&mut self, replacement: Option<bool>, purged: Option<u64>, retained: u64) {
        match replacement {
            Some(true) => Self::add(&mut self.occupied_collisions, 1, &mut self.overflow),
            Some(false) => Self::add(&mut self.same_key_replacements, 1, &mut self.overflow),
            None => {}
        }
        if let Some(plans) = purged {
            Self::add(&mut self.byte_purges, 1, &mut self.overflow);
            Self::add(&mut self.purged_plans, plans, &mut self.overflow);
        }
        self.retained_bytes = retained;
        self.peak_retained_bytes = self.peak_retained_bytes.max(retained);
    }
}

pub(super) struct PlanCache<'view, 'plan, 'image> {
    source: &'view MaterializedSource<'plan, 'image>,
    policy: TranslationPolicy,
    limits: BlockLimits,
    slots: Vec<Option<MaterializedBlockPlan<'view, 'plan, 'image>>>,
    // A replacement hint only. Source, policy and epoch are fixed by Self;
    // every hit still compares the complete live window and placement.
    recency: [u64; SLOTS],
    retained_bytes: u64,
    max_bytes: u64,
    initialized_windows: initialized_windows::Memo<'view, 'plan, 'image>,
    observations: PlanCacheObservations,
}

#[cfg(test)]
#[path = "plan_cache/affine_tests.rs"]
mod affine_tests;
#[cfg(test)]
#[path = "plan_cache/observation_tests.rs"]
mod observation_tests;
#[cfg(test)]
#[path = "plan_cache/register_tests.rs"]
mod register_tests;
#[cfg(test)]
#[path = "plan_cache/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "plan_cache/two_way_tests.rs"]
mod two_way_tests;

impl<'view, 'plan, 'image> PlanCache<'view, 'plan, 'image> {
    pub(super) fn new(
        source: &'view MaterializedSource<'plan, 'image>,
        policy: TranslationPolicy,
        limits: BlockLimits,
        work: &mut u64,
        work_limit: u64,
    ) -> Result<Self, NativeSessionError> {
        let bytes = (SLOTS as u64)
            .checked_mul(std::mem::size_of::<Option<MaterializedBlockPlan<'_, '_, '_>>>() as u64)
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<[u64; SLOTS]>() as u64))
            .ok_or(NativeSessionError::Budget("plan cache allocation"))?;
        charge(work, bytes, work_limit, "admission work")?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(SLOTS)
            .map_err(|_| NativeSessionError::Allocation)?;
        if slots.capacity() != SLOTS {
            return Err(NativeSessionError::Allocation);
        }
        slots.resize_with(SLOTS, || None);
        // Optional metadata shares the retained-plan byte ceiling. Admission or
        // allocation failure leaves the original scanner available; a plan may
        // reclaim this entire allocation before its existing capacity decision.
        let initialized_windows =
            initialized_windows::Memo::new(source, MAX_PLAN_BYTES, work, work_limit);
        Ok(Self {
            source,
            policy,
            limits,
            slots,
            recency: [0; SLOTS],
            retained_bytes: 0,
            max_bytes: MAX_PLAN_BYTES,
            initialized_windows,
            observations: PlanCacheObservations::default(),
        })
    }

    /// The caller retains the original per-attempt epoch metadata charge before
    /// this numeric lookup. A memo hit never authorizes live source or execution.
    pub(super) fn initialized_window(
        &mut self,
        pc: u64,
        maximum: u64,
    ) -> Result<(usize, u64), NativeSessionError> {
        self.initialized_windows
            .yield_to_plans(self.retained_bytes, 0, self.max_bytes);
        self.initialized_windows.get(pc, maximum)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        module: usize,
        pc: u64,
        live: &[u8],
        placement: CachePlacement,
        stamp: u64,
        work: &mut u64,
        work_limit: u64,
    ) -> Result<
        (
            &MaterializedBlockPlan<'view, 'plan, 'image>,
            bool,
            PlanCacheObservations,
        ),
        NativeSessionError,
    > {
        charge(work, live.len() as u64 + 8, work_limit, "admission work")?;
        self.initialized_windows
            .yield_to_plans(self.retained_bytes, 0, self.max_bytes);
        let primary = ((pc ^ (pc >> 8) ^ (pc >> 16) ^ (pc >> 32)) as usize) & (SLOTS - 1);
        let pair = [primary, primary ^ 1];
        let exact = pair.into_iter().find(|&index| {
            self.slots[index].as_ref().is_some_and(|plan| {
                plan.guest_pc() == pc
                    && plan.module().index() == module
                    && plan.placement() == placement
                    && plan.relocated_source_bytes() == live
            })
        });
        let hit = exact.is_some();
        let index = exact.unwrap_or_else(|| {
            // Supersede a changed placement/window of the same logical key
            // before using a second way. A plan is never retargeted in place.
            pair.into_iter()
                .find(|&index| {
                    self.slots[index].as_ref().is_some_and(|plan| {
                        plan.guest_pc() == pc && plan.module().index() == module
                    })
                })
                .or_else(|| pair.into_iter().find(|&index| self.slots[index].is_none()))
                .unwrap_or_else(|| {
                    if self.recency[primary] <= self.recency[primary ^ 1] {
                        primary
                    } else {
                        primary ^ 1
                    }
                })
        });
        if !hit {
            // Replanning changed bytes retains the canonical overlay check: a
            // modified executable image cannot replace its original declaration.
            let mut plan = plan_materialized_block(
                self.source,
                module,
                pc,
                live,
                placement,
                &self.policy,
                &self.limits,
            )?;
            // The original plan exists before optional work is considered.
            // Reserve the whole bounded attempt and a possible existing full
            // purge before any extra scan, allocation or encode. No guest work
            // runs while this exclusive allowance is held; unused reservation
            // is never charged and actual attempted work is never refunded.
            if plan.usage().allocation_bytes <= self.max_bytes
                && let Some(reserved) = plan.affine_loop_trace_work_bound(&self.limits)
                && work_limit
                    .checked_sub(*work)
                    .and_then(|left| left.checked_sub(SLOTS as u64))
                    .is_some_and(|left| reserved <= left)
            {
                let spent = plan.prepare_affine_loop_trace(
                    &self.limits,
                    reserved,
                    self.max_bytes.min(self.limits.max_allocation_bytes),
                );
                // The checked exclusive reservation above guarantees this
                // original monotonic commit cannot refuse after preparation.
                charge(work, spent, work_limit, "admission work")?;
            }
            if plan.retained_allocation_bytes() <= self.max_bytes
                && let Some(reserved) = plan.register_loop_trace_work_bound(&self.limits)
                && work_limit
                    .checked_sub(*work)
                    .and_then(|left| left.checked_sub(SLOTS as u64))
                    .is_some_and(|left| reserved <= left)
            {
                let spent = plan.prepare_register_loop_trace(
                    &self.limits,
                    reserved,
                    self.max_bytes.min(self.limits.max_allocation_bytes),
                );
                // The checked exclusive reservation above guarantees this
                // original monotonic commit cannot refuse after preparation.
                charge(work, spent, work_limit, "admission work")?;
            }
            let bytes = plan.retained_allocation_bytes();
            // Classify only a valid replacement plan. A later refusal must
            // leave every diagnostic counter/gauge unchanged as well.
            let replacement = self.slots[index]
                .as_ref()
                .map(|old| old.guest_pc() != pc || old.module().index() != module);
            let old = self.slots[index]
                .as_ref()
                .map_or(0, |plan| plan.retained_allocation_bytes());
            let remaining = self.retained_bytes - old;
            // The independent planner's prior allocation limit is unchanged.
            // Its temporary allocation can coexist with the same <=8MiB
            // retained sum as before. Memo bytes yield before the old retained
            // capacity/refusal/eviction logic, never displacing a valid plan.
            self.initialized_windows
                .yield_to_plans(remaining, bytes, self.max_bytes);
            if bytes > self.max_bytes {
                return Err(NativeSessionError::Budget("plan cache bytes"));
            }
            let mut purged = None;
            if remaining > self.max_bytes - bytes {
                charge(work, SLOTS as u64, work_limit, "admission work")?;
                let mut removed = 0_u64;
                for (index, slot) in self.slots.iter_mut().enumerate() {
                    removed += u64::from(slot.is_some()); // At most SLOTS.
                    *slot = None;
                    self.recency[index] = 0;
                }
                self.retained_bytes = 0;
                purged = Some(removed);
            } else {
                self.slots[index] = None;
                self.retained_bytes = remaining;
            }
            // Reported allocation includes replacement buffers and temporary
            // encoder storage, conservatively bounding retained plan capacities.
            self.retained_bytes += bytes;
            self.slots[index] = Some(plan);
            self.observations
                .committed(replacement, purged, self.retained_bytes);
        }
        // The caller's already checked block counter is only a retention hint.
        // No new clock, overflow path or guest admission authority is created.
        self.recency[index] = stamp;
        Ok((
            self.slots[index]
                .as_ref()
                .ok_or(NativeSessionError::FixtureInvariant("missing cached plan"))?,
            hit,
            self.observations,
        ))
    }
}
