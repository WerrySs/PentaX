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

pub(super) struct PlanCache<'view, 'plan, 'image> {
    source: &'view MaterializedSource<'plan, 'image>,
    policy: TranslationPolicy,
    limits: BlockLimits,
    slots: Vec<Option<MaterializedBlockPlan<'view, 'plan, 'image>>>,
    retained_bytes: u64,
    max_bytes: u64,
}

#[cfg(test)]
#[path = "plan_cache/tests.rs"]
mod tests;

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
        Ok(Self {
            source,
            policy,
            limits,
            slots,
            retained_bytes: 0,
            max_bytes: MAX_PLAN_BYTES,
        })
    }

    pub(super) fn prepare(
        &mut self,
        module: usize,
        pc: u64,
        live: &[u8],
        placement: CachePlacement,
        work: &mut u64,
        work_limit: u64,
    ) -> Result<(&MaterializedBlockPlan<'view, 'plan, 'image>, bool), NativeSessionError> {
        charge(work, live.len() as u64 + 8, work_limit, "admission work")?;
        let index = ((pc ^ (pc >> 8) ^ (pc >> 16) ^ (pc >> 32)) as usize) & (SLOTS - 1);
        let hit = self.slots[index].as_ref().is_some_and(|plan| {
            plan.guest_pc() == pc
                && plan.module().index() == module
                && plan.placement() == placement
                && plan.relocated_source_bytes() == live
        });
        if !hit {
            // Replanning changed bytes retains the canonical overlay check: a
            // modified executable image cannot replace its original declaration.
            let plan = plan_materialized_block(
                self.source,
                module,
                pc,
                live,
                placement,
                &self.policy,
                &self.limits,
            )?;
            let bytes = plan.usage().allocation_bytes;
            if bytes > self.max_bytes {
                return Err(NativeSessionError::Budget("plan cache bytes"));
            }
            let old = self.slots[index]
                .as_ref()
                .map_or(0, |plan| plan.usage().allocation_bytes);
            let remaining = self.retained_bytes - old;
            if remaining > self.max_bytes - bytes {
                charge(work, SLOTS as u64, work_limit, "admission work")?;
                for slot in &mut self.slots {
                    *slot = None;
                }
                self.retained_bytes = 0;
            } else {
                self.slots[index] = None;
                self.retained_bytes = remaining;
            }
            // Reported allocation includes replacement buffers and temporary
            // encoder storage, conservatively bounding retained plan capacities.
            self.retained_bytes += bytes;
            self.slots[index] = Some(plan);
        }
        Ok((
            self.slots[index]
                .as_ref()
                .ok_or(NativeSessionError::FixtureInvariant("missing cached plan"))?,
            hit,
        ))
    }
}
