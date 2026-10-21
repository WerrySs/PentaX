// SPDX-License-Identifier: GPL-2.0-only
//! Closed final-only cache and wait observations, never execution authority.
use super::plan_cache::PlanCacheObservations;
use pentax_kernel::HandleUsage;
use pentax_platform::native_dispatch::NativeDispatchUsage;

#[derive(Debug, Default)]
pub(super) struct PlanCacheTotals {
    occupied_collisions: u64,
    same_key_replacements: u64,
    byte_purges: u64,
    purged_plans: u64,
    retained_bytes: u64,
    peak_retained_bytes: u64,
    overflow: bool,
}

impl PlanCacheTotals {
    pub(super) fn begin_epoch(&mut self) {
        self.retained_bytes = 0;
    }

    pub(super) fn observe(
        &mut self,
        next: PlanCacheObservations,
        previous: &mut PlanCacheObservations,
    ) {
        for (total, old, new) in [
            (
                &mut self.occupied_collisions,
                previous.occupied_collisions,
                next.occupied_collisions,
            ),
            (
                &mut self.same_key_replacements,
                previous.same_key_replacements,
                next.same_key_replacements,
            ),
            (
                &mut self.byte_purges,
                previous.byte_purges,
                next.byte_purges,
            ),
            (
                &mut self.purged_plans,
                previous.purged_plans,
                next.purged_plans,
            ),
        ] {
            match new
                .checked_sub(old)
                .and_then(|delta| total.checked_add(delta))
            {
                Some(value) => *total = value,
                None => {
                    *total = u64::MAX;
                    self.overflow = true;
                }
            }
        }
        self.retained_bytes = next.retained_bytes;
        self.peak_retained_bytes = self.peak_retained_bytes.max(next.peak_retained_bytes);
        self.overflow |= next.overflow;
        *previous = next;
    }
}

pub(super) fn fields(
    native: &NativeDispatchUsage,
    plans: &PlanCacheTotals,
    handles: Option<HandleUsage>,
) -> String {
    use std::fmt::Write;
    let mut output = format!(
        "\tcache_replacements={}\tcache_evicted_key_revisits={}\tcache_history_saturated={}\tplan_cache_occupied_collisions={}\tplan_cache_same_key_replacements={}\tplan_cache_byte_purges={}\tplan_cache_purged_plans={}\tplan_cache_retained_bytes={}\tplan_cache_peak_retained_bytes={}\tplan_cache_observations_overflowed={}",
        native.cache_replacements,
        native.cache_evicted_key_revisits,
        native.cache_history_saturated,
        plans.occupied_collisions,
        plans.same_key_replacements,
        plans.byte_purges,
        plans.purged_plans,
        plans.retained_bytes,
        plans.peak_retained_bytes,
        plans.overflow,
    );
    if let Some(handles) = handles {
        let _ = write!(
            output,
            "\tpthread_mutex_waits={}\tpthread_condition_waits={}\tpthread_notified_condition_waits={}\tpthread_eligible_mutex_releases={}\tpthread_barging_mutex_acquisitions={}\tpthread_wait_observations_overflowed={}",
            handles.pthread_mutex_waits,
            handles.pthread_condition_waits,
            handles.pthread_notified_condition_waits,
            handles.pthread_eligible_mutex_releases,
            handles.pthread_barging_mutex_acquisitions,
            handles.pthread_wait_observations_overflowed,
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_totals_survive_epoch_reset_and_saturation_without_double_counting() {
        let mut totals = PlanCacheTotals::default();
        let mut previous = PlanCacheObservations::default();
        let first = PlanCacheObservations {
            occupied_collisions: 2,
            same_key_replacements: 1,
            byte_purges: 1,
            purged_plans: 3,
            retained_bytes: 128,
            peak_retained_bytes: 512,
            overflow: false,
        };
        totals.observe(first, &mut previous);
        totals.observe(first, &mut previous);
        assert_eq!(totals.occupied_collisions, 2);
        assert_eq!(totals.purged_plans, 3);
        totals.begin_epoch();
        assert_eq!(totals.retained_bytes, 0);
        previous = PlanCacheObservations::default();
        totals.observe(
            PlanCacheObservations {
                occupied_collisions: 4,
                retained_bytes: 256,
                ..Default::default()
            },
            &mut previous,
        );
        assert_eq!(totals.occupied_collisions, 6);
        assert_eq!(totals.peak_retained_bytes, 512);
        assert_eq!(totals.retained_bytes, 256);
        totals.occupied_collisions = u64::MAX;
        totals.observe(
            PlanCacheObservations {
                occupied_collisions: 5,
                ..previous
            },
            &mut previous,
        );
        assert_eq!(totals.occupied_collisions, u64::MAX);
        assert!(totals.overflow);
    }
}
