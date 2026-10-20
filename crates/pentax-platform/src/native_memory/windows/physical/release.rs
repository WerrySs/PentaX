// SPDX-License-Identifier: GPL-2.0-only

//! Stage all live physical pieces and alias rollback data before detaching views.
use super::*;

#[derive(Clone, Copy)]
struct ReleasedView {
    page: Page,
    physical: u64,
    offset: u64,
    section: usize,
}
struct ReleasePlan {
    allocations: Vec<PhysicalAllocation>,
    views: Vec<ReleasedView>,
    close: Vec<usize>,
}

impl NativeAddressSpace {
    fn physical_release_plan(
        &self,
        physical: u64,
        size: u64,
        maximum: usize,
    ) -> Result<ReleasePlan, NativeMemoryError> {
        self.preflight_physical_release(physical, size)?;
        let end = physical + size;
        let affected = |page: &Page| {
            matches!(self.region(page.address).kind,
            RegionKind::Shared { physical: at } if physical <= at && at < end)
        };
        let count = self.pages.iter().filter(|p| affected(p)).count();
        let cap = self.physical_allocations.len() + 1;
        let bytes = cap
            .checked_mul(std::mem::size_of::<PhysicalAllocation>())
            .and_then(|v| {
                count
                    .checked_mul(std::mem::size_of::<ReleasedView>())
                    .and_then(|n| n.checked_add(v))
            })
            .and_then(|v| {
                self.physical_sections
                    .len()
                    .checked_mul(std::mem::size_of::<usize>())
                    .and_then(|n| n.checked_add(v))
            })
            .filter(|v| *v <= maximum)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let mut plan = ReleasePlan {
            allocations: Vec::new(),
            views: Vec::new(),
            close: Vec::new(),
        };
        plan.allocations
            .try_reserve_exact(cap)
            .map_err(|_| NativeMemoryError::Allocation)?;
        plan.views
            .try_reserve_exact(count)
            .map_err(|_| NativeMemoryError::Allocation)?;
        plan.close
            .try_reserve_exact(self.physical_sections.len())
            .map_err(|_| NativeMemoryError::Allocation)?;
        let actual = plan
            .allocations
            .capacity()
            .checked_mul(std::mem::size_of::<PhysicalAllocation>())
            .and_then(|v| {
                plan.views
                    .capacity()
                    .checked_mul(std::mem::size_of::<ReleasedView>())
                    .and_then(|n| n.checked_add(v))
            })
            .and_then(|v| {
                plan.close
                    .capacity()
                    .checked_mul(std::mem::size_of::<usize>())
                    .and_then(|n| n.checked_add(v))
            })
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        if actual > maximum {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        debug_assert!(actual >= bytes);
        for a in &self.physical_allocations {
            let stop = a.start + a.size;
            if stop <= physical || end <= a.start {
                plan.allocations.push(*a);
            } else {
                if a.start < physical {
                    plan.allocations.push(PhysicalAllocation {
                        size: physical - a.start,
                        ..*a
                    });
                }
                if end < stop {
                    plan.allocations.push(PhysicalAllocation {
                        start: end,
                        size: stop - end,
                        offset: a.offset + end - a.start,
                        ..*a
                    });
                }
            }
        }
        for page in self.pages.iter().filter(|p| affected(p)) {
            let RegionKind::Shared { physical: at } = self.region(page.address).kind else {
                unreachable!()
            };
            let a = self.physical_allocation(at, NATIVE_GUEST_PAGE)?;
            plan.views.push(ReleasedView {
                page: *page,
                physical: at,
                offset: a.offset + at - a.start,
                section: a.section,
            });
        }
        for (slot, section) in self.physical_sections.iter().enumerate() {
            if section.is_some() && !plan.allocations.iter().any(|a| a.section == slot) {
                plan.close.push(slot);
            }
        }
        Ok(plan)
    }
    fn restore_physical_views(&mut self, views: &[ReleasedView]) -> bool {
        for view in views.iter().rev() {
            if !self.rollback_operation() {
                self.poisoned = true;
                continue;
            }
            let section = self.physical_sections[view.section].as_mut().unwrap();
            // Borrow the disjoint section and reservation fields together.
            let at = self
                .reservations
                .partition_point(|r| r.address() <= view.page.address);
            let region = self.reservations[at - 1].region_mut(view.page.address);
            if region
                .map_shared(section, view.offset, view.physical, view.page.permissions)
                .is_err()
            {
                self.poisoned = true;
            }
        }
        !self.poisoned
    }
    pub(in super::super) fn release_physical(
        &mut self,
        physical: u64,
        size: u64,
        maximum: usize,
    ) -> Result<(), NativeMemoryError> {
        let plan = self.physical_release_plan(physical, size, maximum)?;
        for (index, view) in plan.views.iter().enumerate() {
            let result = self
                .operation()
                .and_then(|()| self.region_mut(view.page.address).unmap());
            if let Err(error) = result {
                return Err(if self.restore_physical_views(&plan.views[..index]) {
                    error
                } else {
                    NativeMemoryError::Poisoned
                });
            }
        }
        let mut reclaimed = 0_u64;
        for (index, slot) in plan.close.iter().enumerate() {
            if let Err(error) = self.operation() {
                if index != 0 {
                    self.poisoned = true;
                    return Err(NativeMemoryError::Poisoned);
                }
                return Err(if self.restore_physical_views(&plan.views) {
                    error
                } else {
                    NativeMemoryError::Poisoned
                });
            }
            let section = self.physical_sections[*slot].as_mut().unwrap();
            if !section.close() {
                self.poisoned = true;
                return Err(NativeMemoryError::Poisoned);
            }
            reclaimed += section.size;
        }
        // No fallible work remains. Keep sections referenced by surviving
        // physical pieces: Windows cannot reclaim their partial commitment.
        // Their whole capacity stays charged until their final piece is freed.
        for slot in &plan.close {
            self.physical_sections[*slot] = None;
        }
        self.physical_allocations = plan.allocations;
        self.pages.retain(|p| {
            plan.views
                .binary_search_by_key(&p.address, |v| v.page.address)
                .is_err()
        });
        self.physical_bytes -= reclaimed;
        Ok(())
    }
}
