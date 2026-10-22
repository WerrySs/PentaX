// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned bounded mutex/attribute state, checked guest
// slots and an explicitly admitted initial-thread profile; see NOTICE.
// Adapted from pinned kernel/pthread.cpp/h and libs/errno.h.

use crate::budget::{Budget, Limits};
use crate::memory::{MemoryState, PthreadObjectKind};
use crate::{CallArgs, Errno, Failure, GuestAccess, Service};

mod wait;
pub use wait::{MutexWait, MutexWaitAction};
mod observations;

#[derive(Clone, Copy)]
struct Attributes {
    kind: i32,
    protocol: i32,
}
struct Attribute {
    id: u64,
    value: Attributes,
}
struct Mutex {
    id: u64,
    #[allow(dead_code)] // Retained source identity; never printed as guest diagnostics.
    name: String,
    kind: i32,
    protocol: i32,
    owner: Option<u32>,
    count: u32,
    condition_waits: usize,
    mutex_waits: usize,
    // Diagnostic bookkeeping only; never consulted by ownership or readiness.
    notified_condition_waits: u64,
}

pub(crate) struct ConditionMutex {
    index: usize,
    pub(crate) id: u64,
    pub(crate) count: u32,
}
pub(crate) struct MutexState {
    attributes: Vec<Attribute>,
    mutexes: Vec<Mutex>,
    waiters: Vec<wait::Waiter>,
    max_waiters: usize,
    eligible_releases: u64,
    barging_acquisitions: u64,
    observations_overflowed: bool,
    observation_lookup_visits: u64,
    observation_mutex_index: Option<usize>,
}

impl MutexState {
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut attributes = Vec::new();
        let mut mutexes = Vec::new();
        let mut waiters = Vec::new();
        attributes
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        mutexes
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        if waiters.capacity() > limits.max_thread_contexts {
            return Err(Failure::Limit("mutex waiter capacity"));
        }
        Ok(Self {
            attributes,
            mutexes,
            waiters,
            max_waiters: limits.max_thread_contexts,
            eligible_releases: 0,
            barging_acquisitions: 0,
            observations_overflowed: false,
            observation_lookup_visits: observations::LOOKUP_VISITS,
            observation_mutex_index: None,
        })
    }
    fn slot(slot: u64, guest: &impl GuestAccess, budget: &mut Budget) -> Result<u64, Failure> {
        budget.u64(guest, slot)
    }
    fn new_slot(
        slot: u64,
        memory: &MemoryState,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        if memory.owns_pthread_object(Self::slot(slot, guest, budget)?, budget)? {
            return Err(Failure::Unsupported("reinitializing live pthread object"));
        }
        Ok(())
    }
    fn attribute_index(
        &self,
        slot: u64,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        if slot == 0 {
            return Err(Failure::Unsupported("source-null pthread attribute slot"));
        }
        let id = Self::slot(slot, guest, budget)?;
        self.attributes
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Failure::Unsupported("unowned pthread attribute token"))
    }
    fn create_mutex(
        &mut self,
        slot: u64,
        value: Attributes,
        name: String,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        let id = memory.create_pthread_object(PthreadObjectKind::Mutex, slot, guest, budget)?;
        // Capacity/resources were admitted before atomic page/slot publication.
        self.mutexes.push(Mutex {
            id,
            name,
            kind: value.kind,
            protocol: value.protocol,
            owner: None,
            count: 0,
            condition_waits: 0,
            mutex_waits: 0,
            notified_condition_waits: 0,
        });
        Ok(self.mutexes.len() - 1)
    }
    fn mutex_index(
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
        let id = Self::slot(slot, guest, budget)?;
        if lazy && (id == 0 || id == 1) {
            // Pinned CreateObject: zero uses the default errorcheck type;
            // Exactly 1 is the adaptive static initializer, including heap cells.
            return self.create_mutex(
                slot,
                Attributes {
                    kind: if id == 1 { 4 } else { 1 },
                    protocol: 0,
                },
                String::new(),
                memory,
                guest,
                budget,
            );
        }
        if id == 0 {
            return Err(Errno::Invalid.into());
        }
        self.mutexes
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Failure::Unsupported("unowned pthread mutex token"))
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        thread: u32,
    ) -> Result<u64, Failure> {
        // Admit every bounded state lookup before any lazy/explicit publication.
        budget.work(24 + 3 * self.attributes.len() as u64 + 3 * self.mutexes.len() as u64)?;
        let a = args.0;
        use Service::*;
        match service {
            MutexAttrInit => {
                if a[0] == 0 {
                    return Err(Failure::Unsupported("source-null pthread attribute slot"));
                }
                // Attribute initialization overwrites its output without
                // inspecting old bytes; lock initialization keeps new_slot.
                let id = memory.create_pthread_object(
                    PthreadObjectKind::Attribute,
                    a[0],
                    guest,
                    budget,
                )?;
                self.attributes.push(Attribute {
                    id,
                    value: Attributes {
                        kind: 1,
                        protocol: 0,
                    },
                });
                Ok(0)
            }
            MutexAttrDestroy => {
                let index = self.attribute_index(a[0], guest, budget)?;
                memory.destroy_pthread_object(
                    PthreadObjectKind::Attribute,
                    self.attributes[index].id,
                    a[0],
                    guest,
                    budget,
                )?;
                self.attributes.swap_remove(index);
                Ok(0)
            }
            MutexAttrSetType => {
                let index = self.attribute_index(a[0], guest, budget)?;
                let kind = a[1] as u32 as i32;
                if !(1..=4).contains(&kind) {
                    return Err(Errno::Invalid.into());
                }
                self.attributes[index].value.kind = kind;
                Ok(0)
            }
            MutexAttrSetProtocol => {
                let index = self.attribute_index(a[0], guest, budget)?;
                let protocol = a[1] as u32 as i32;
                if !(0..=2).contains(&protocol) {
                    // The pinned source aborts for other values. Preserve a
                    // typed stop rather than fabricating an EINVAL result.
                    return Err(Failure::Unsupported(
                        "source-invalid pthread priority protocol",
                    ));
                }
                // Source Windows stores NONE/INHERIT/PROTECT metadata without
                // applying winpthreads scheduling. Copy it into each new mutex;
                // the separate cooperative wait path preserves exclusion for
                // all three stored values without claiming priority scheduling.
                self.attributes[index].value.protocol = protocol;
                Ok(0)
            }
            MutexInit => {
                if a[0] == 0 {
                    return Err(Errno::Invalid.into());
                }
                Self::new_slot(a[0], memory, guest, budget)?;
                let value = if a[1] == 0 {
                    Attributes {
                        kind: 1,
                        protocol: 0,
                    }
                } else {
                    self.attributes[self.attribute_index(a[1], guest, budget)?].value
                };
                let name = if a[2] == 0 {
                    String::new()
                } else {
                    budget.name(guest, a[2])?
                };
                self.create_mutex(a[0], value, name, memory, guest, budget)?;
                Ok(0)
            }
            MutexDestroy => {
                let index = self.mutex_index(a[0], false, memory, guest, budget)?;
                if self.mutexes[index].owner.is_some()
                    || self.mutexes[index].condition_waits != 0
                    || self.mutexes[index].mutex_waits != 0
                {
                    return Err(Errno::Busy.into());
                }
                memory.destroy_pthread_object(
                    PthreadObjectKind::Mutex,
                    self.mutexes[index].id,
                    a[0],
                    guest,
                    budget,
                )?;
                self.mutexes.swap_remove(index);
                Ok(0)
            }
            MutexLock | MutexTryLock | MutexTimedLock | MutexUnlock => {
                let index = self.mutex_index(a[0], true, memory, guest, budget)?;
                let value = &mut self.mutexes[index];
                if service == MutexUnlock {
                    if value.owner != Some(thread) {
                        return Err(Errno::Perm.into());
                    }
                    if value.kind == 2 && value.count > 1 {
                        value.count -= 1;
                    } else {
                        value.owner = None;
                        value.count = 0;
                        if observations::eligible(value) {
                            observations::increment(
                                &mut self.eligible_releases,
                                &mut self.observations_overflowed,
                            );
                        }
                    }
                    return Ok(0);
                }
                if value.owner == Some(thread) && value.kind == 2 {
                    value.count = value.count.checked_add(1).ok_or(Errno::Again)?;
                    return Ok(0);
                }
                if value.owner.is_some() {
                    if service == MutexTryLock {
                        return Err(Errno::Busy.into());
                    }
                    if service == MutexTimedLock {
                        let micros = u64::from(a[1] as u32);
                        if micros == 0 {
                            return Err(Errno::TimedOut.into());
                        }
                        let nanos = micros * 1000;
                        if nanos > budget.limits.max_sleep_nanoseconds {
                            return Err(Failure::Limit("pthread mutex sleep"));
                        }
                        if value.owner != Some(thread) {
                            return Err(Failure::Unsupported(if value.protocol == 0 {
                                "contended pthread mutex wait"
                            } else {
                                "contended pthread priority protocol"
                            }));
                        }
                        std::thread::sleep(std::time::Duration::from_nanos(nanos));
                        return Err(Errno::TimedOut.into());
                    }
                    return if value.owner == Some(thread) {
                        Err(Errno::Deadlock.into())
                    } else {
                        Err(Failure::Unsupported(if value.protocol == 0 {
                            "contended pthread mutex wait"
                        } else {
                            "contended pthread priority protocol"
                        }))
                    };
                }
                value.owner = Some(thread);
                value.count = 1;
                if observations::eligible(value) {
                    observations::increment(
                        &mut self.barging_acquisitions,
                        &mut self.observations_overflowed,
                    );
                }
                Ok(0)
            }
            _ => Err(Failure::Unsupported("pthread mutex service")),
        }
    }
}

impl MutexState {
    pub(crate) fn prepare_condition_release(
        &mut self,
        slot: u64,
        thread: u32,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<ConditionMutex, Failure> {
        budget.work(self.mutexes.len() as u64 + 1)?;
        let index = self.mutex_index(slot, true, memory, guest, budget)?;
        let mutex = &self.mutexes[index];
        if mutex.owner != Some(thread) {
            return Err(Errno::Perm.into());
        }
        mutex
            .condition_waits
            .checked_add(1)
            .ok_or(Failure::Limit("condition mutex waiters"))?;
        Ok(ConditionMutex {
            index,
            id: mutex.id,
            count: mutex.count.max(1),
        })
    }

    // Called only after the waiter and every fallible admission are prepared.
    pub(crate) fn commit_condition_release(&mut self, prepared: &ConditionMutex) {
        let mutex = &mut self.mutexes[prepared.index];
        mutex.owner = None;
        mutex.count = 0;
        mutex.condition_waits += 1;
        if observations::eligible(mutex) {
            observations::increment(
                &mut self.eligible_releases,
                &mut self.observations_overflowed,
            );
        }
    }

    pub(crate) fn condition_available(
        &self,
        id: u64,
        budget: &mut Budget,
    ) -> Result<bool, Failure> {
        budget.work(self.mutexes.len() as u64 + 1)?;
        let mutex = self
            .mutexes
            .iter()
            .find(|mutex| mutex.id == id)
            .ok_or(Failure::Unsupported("stale condition mutex"))?;
        if mutex.condition_waits == 0 {
            return Err(Failure::Unsupported("unbound condition mutex"));
        }
        Ok(mutex.owner.is_none())
    }

    pub(crate) fn reacquire_condition(
        &mut self,
        id: u64,
        thread: u32,
        count: u32,
        budget: &mut Budget,
    ) -> Result<bool, Failure> {
        budget.work(self.mutexes.len() as u64)?;
        if !self.condition_available(id, budget)? {
            return Ok(false);
        }
        let mutex = self
            .mutexes
            .iter_mut()
            .find(|mutex| mutex.id == id)
            .expect("admitted mutex");
        mutex.owner = Some(thread);
        mutex.count = count;
        mutex.condition_waits -= 1;
        mutex.notified_condition_waits = mutex.notified_condition_waits.saturating_sub(1);
        Ok(true)
    }
}

fn absolute_micros(
    pointer: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<u32, Failure> {
    if pointer == 0 {
        return Err(Errno::Invalid.into());
    }
    let mut bytes = [0; 16];
    budget.read(guest, pointer, &mut bytes)?;
    let sec = i64::from_le_bytes(bytes[..8].try_into().unwrap());
    let ns = i64::from_le_bytes(bytes[8..].try_into().unwrap());
    if sec < 0 || !(0..1_000_000_000).contains(&ns) {
        return Err(Errno::Invalid.into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Errno::Invalid)?
        .as_nanos();
    let deadline = sec as u128 * 1_000_000_000 + ns as u128;
    Ok((deadline.saturating_sub(now) / 1000).min(u32::MAX as u128) as u32)
}

impl crate::Kernel {
    pub(crate) fn invoke_pthread_mutex(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        guest.preflight_pthread()?;
        use Service::*;
        let (delegate, positive) = match service {
            PosixMutexAttrInit => (MutexAttrInit, true),
            PosixMutexAttrDestroy => (MutexAttrDestroy, true),
            PosixMutexAttrSetType => (MutexAttrSetType, true),
            PosixMutexAttrSetProtocol => (MutexAttrSetProtocol, true),
            PosixMutexInit => (MutexInit, true),
            PosixMutexDestroy => (MutexDestroy, true),
            PosixMutexLock => (MutexLock, true),
            PosixMutexTryLock => (MutexTryLock, true),
            PosixMutexTimedLock => (MutexTimedLock, true),
            PosixMutexUnlock => (MutexUnlock, true),
            _ => (service, false),
        };
        let mut values = args.0;
        if service == PosixMutexInit {
            values[2] = 0;
        }
        let prepared = if service == PosixMutexTimedLock {
            absolute_micros(values[1], guest, &mut self.budget).map(|micros| {
                values[1] = u64::from(micros);
            })
        } else {
            Ok(())
        };
        let result = prepared.and_then(|()| {
            self.pthread_mutex.invoke(
                delegate,
                CallArgs(values),
                &mut self.memory,
                guest,
                &mut self.budget,
                self.threads.active().id,
            )
        });
        if positive {
            crate::posix::pthread_result(result)
        } else {
            result
        }
    }
}
