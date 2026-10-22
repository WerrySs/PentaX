// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned conditions, attributes and cooperative waiters.
// Adapted from pinned kernel/pthread.cpp/h and libs/errno.h; see NOTICE.

//! Owned conditions and bounded cooperative wait queues. Wait admission and
//! mutex reacquisition are separate from completing the saved guest syscall.

use crate::budget::{Budget, Limits};
use crate::memory::{MemoryState, PthreadObjectKind};
use crate::{CallArgs, Errno, Failure, GuestAccess, Service};
mod wait;
pub use wait::{ConditionWait, ConditionWaitAction};

struct Attribute {
    id: u64,
    clock: i32,
}
struct Condition {
    id: u64,
    #[allow(dead_code)] // Copied clock/name retained for the future wait implementation.
    clock: i32,
    #[allow(dead_code)]
    name: String,
}
pub(crate) struct ConditionState {
    attributes: Vec<Attribute>,
    conditions: Vec<Condition>,
    waiters: Vec<wait::Waiter>,
    max_waiters: usize,
}

impl ConditionState {
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut attributes = Vec::new();
        let mut conditions = Vec::new();
        let mut waiters = Vec::new();
        attributes
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        conditions
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            attributes,
            conditions,
            waiters,
            max_waiters: limits.max_thread_contexts,
        })
    }

    fn attribute_index(
        &self,
        slot: u64,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        if slot == 0 {
            return Err(Errno::Invalid.into());
        }
        let id = budget.u64(guest, slot)?;
        self.attributes
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Failure::Unsupported("unowned condition attribute"))
    }

    fn create_condition(
        &mut self,
        slot: u64,
        clock: i32,
        name: String,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        let id = memory.create_pthread_object(PthreadObjectKind::Condition, slot, guest, budget)?;
        // Both metadata vectors were reserved before the backing transaction.
        self.conditions.push(Condition { id, clock, name });
        Ok(self.conditions.len() - 1)
    }

    fn condition_index(
        &mut self,
        slot: u64,
        lazy: bool,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        if slot == 0 {
            return Err(Errno::Invalid.into());
        }
        let id = budget.u64(guest, slot)?;
        if id == 0 && lazy {
            return self.create_condition(slot, 0, String::new(), memory, guest, budget);
        }
        self.conditions
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Failure::Unsupported("unowned condition object"))
    }

    fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        mutexes: &mut crate::pthread_mutex::MutexState,
    ) -> Result<u64, Failure> {
        budget.work(
            (self.attributes.len() + self.conditions.len() + self.waiters.len() + 1) as u64,
        )?;
        let a = args.0;
        use Service::*;
        match service {
            CondAttrInit | CondInit => {
                if a[0] == 0 {
                    return if service == CondInit {
                        Err(Errno::Invalid.into())
                    } else {
                        Err(Failure::Unsupported("source-null condition attribute slot"))
                    };
                }
                if service == CondAttrInit {
                    // The source unconditionally overwrites this output.
                    let id = memory.create_pthread_object(
                        PthreadObjectKind::ConditionAttribute,
                        a[0],
                        guest,
                        budget,
                    )?;
                    self.attributes.push(Attribute { id, clock: 0 });
                } else {
                    let previous = budget.u64(guest, a[0])?;
                    if memory.owns_pthread_object(previous, budget)? {
                        return Err(Failure::Unsupported("reinitializing live pthread object"));
                    }
                    let clock = if a[1] == 0 {
                        0
                    } else {
                        self.attributes[self.attribute_index(a[1], guest, budget)?].clock
                    };
                    let name = if a[2] == 0 {
                        String::new()
                    } else {
                        budget.name(guest, a[2])?
                    };
                    self.create_condition(a[0], clock, name, memory, guest, budget)?;
                }
                Ok(0)
            }
            CondAttrDestroy => {
                let index = self.attribute_index(a[0], guest, budget)?;
                memory.destroy_pthread_object(
                    PthreadObjectKind::ConditionAttribute,
                    self.attributes[index].id,
                    a[0],
                    guest,
                    budget,
                )?;
                self.attributes.swap_remove(index);
                Ok(0)
            }
            CondAttrSetClock => {
                if a[0] == 0 || budget.u64(guest, a[0])? == 0 {
                    return Err(Errno::Invalid.into());
                }
                let index = self.attribute_index(a[0], guest, budget)?;
                let clock = a[1] as i32;
                // Pinned GetPosixClockId: realtime and monotonic aliases only.
                if !matches!(clock, 0 | 4 | 5 | 7..=13 | 16..=19) {
                    return Err(Errno::Invalid.into());
                }
                self.attributes[index].clock = clock;
                Ok(0)
            }
            CondDestroy => {
                let index = self.condition_index(a[0], false, memory, guest, budget)?;
                if self.has_waiters(self.conditions[index].id) {
                    return Err(Errno::Busy.into());
                }
                memory.destroy_pthread_object(
                    PthreadObjectKind::Condition,
                    self.conditions[index].id,
                    a[0],
                    guest,
                    budget,
                )?;
                self.conditions.swap_remove(index);
                Ok(0)
            }
            CondSignal | CondBroadcast => {
                let index = self.condition_index(a[0], true, memory, guest, budget)?;
                self.notify(self.conditions[index].id, service == CondBroadcast, mutexes);
                Ok(0)
            }
            _ => Err(Failure::Unsupported("condition waiter scheduling")),
        }
    }
}

impl crate::Kernel {
    pub(crate) fn invoke_pthread_condition(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        guest.preflight_pthread()?;
        use Service::*;
        let (delegate, positive) = match service {
            PosixCondAttrInit => (CondAttrInit, true),
            PosixCondAttrDestroy => (CondAttrDestroy, true),
            PosixCondAttrSetClock => (CondAttrSetClock, true),
            PosixCondInit => (CondInit, true),
            PosixCondSignal => (CondSignal, true),
            PosixCondBroadcast => (CondBroadcast, true),
            _ => (service, false),
        };
        let mut values = args.0;
        if service == PosixCondInit {
            values[2] = 0;
        }
        let result = self.pthread_condition.invoke(
            delegate,
            CallArgs(values),
            &mut self.memory,
            guest,
            &mut self.budget,
            &mut self.pthread_mutex,
        );
        if positive {
            crate::posix::pthread_result(result)
        } else {
            result
        }
    }
}
