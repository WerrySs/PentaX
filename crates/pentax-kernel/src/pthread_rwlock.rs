// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned initial-thread read/write locks.
// Adapted from pinned kernel/pthread.cpp/h; see NOTICE.

//! The current profile admits one guest thread. Reader recursion and reading
//! while owning the writer are real retained state. Contended waits require
//! scheduler-owned queues before additional guest threads can be admitted.

use crate::budget::{Budget, Limits};
use crate::memory::{MemoryState, PthreadObjectKind};
use crate::{CallArgs, Errno, Failure, GuestAccess, Service};

struct Attribute {
    id: u64,
    // The pinned initializer does not consume its attribute argument.
    _kind: i32,
}
struct Rwlock {
    id: u64,
    _name: String,
    writer: Option<u32>,
    reader: Option<u32>,
    readers: u32,
}
pub(crate) struct RwlockState {
    attributes: Vec<Attribute>,
    locks: Vec<Rwlock>,
}

impl RwlockState {
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut attributes = Vec::new();
        let mut locks = Vec::new();
        attributes
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        locks
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self { attributes, locks })
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
            .ok_or(Failure::Unsupported("unowned rwlock attribute"))
    }

    fn create(
        &mut self,
        slot: u64,
        name: String,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        let id = memory.create_pthread_object(PthreadObjectKind::Rwlock, slot, guest, budget)?;
        self.locks.push(Rwlock {
            id,
            _name: name,
            writer: None,
            reader: None,
            readers: 0,
        });
        Ok(self.locks.len() - 1)
    }

    fn lock_index(
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
            return self.create(slot, String::new(), memory, guest, budget);
        }
        self.locks
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Failure::Unsupported("unowned rwlock object"))
    }

    fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        thread: u32,
    ) -> Result<u64, Failure> {
        budget.work((self.attributes.len() + self.locks.len() + 1) as u64)?;
        let a = args.0;
        use Service::*;
        match service {
            RwlockAttrInit | RwlockInit => {
                if a[0] == 0 {
                    return if service == RwlockInit {
                        Err(Errno::Invalid.into())
                    } else {
                        Err(Failure::Unsupported("source-null rwlock attribute slot"))
                    };
                }
                let previous = budget.u64(guest, a[0])?;
                if memory.owns_pthread_object(previous, budget)? {
                    return Err(Failure::Unsupported("reinitializing live pthread object"));
                }
                if service == RwlockAttrInit {
                    let id = memory.create_pthread_object(
                        PthreadObjectKind::RwlockAttribute,
                        a[0],
                        guest,
                        budget,
                    )?;
                    self.attributes.push(Attribute { id, _kind: 1 });
                } else {
                    // Pinned PthreadRwlockInitNamed ignores attr; never read it.
                    let name = if a[2] == 0 {
                        String::new()
                    } else {
                        budget.name(guest, a[2])?
                    };
                    self.create(a[0], name, memory, guest, budget)?;
                }
                Ok(0)
            }
            RwlockAttrDestroy => {
                let index = self.attribute_index(a[0], guest, budget)?;
                memory.destroy_pthread_object(
                    PthreadObjectKind::RwlockAttribute,
                    self.attributes[index].id,
                    a[0],
                    guest,
                    budget,
                )?;
                self.attributes.swap_remove(index);
                Ok(0)
            }
            RwlockAttrSetType => {
                let index = self.attribute_index(a[0], guest, budget)?;
                // The pinned source accepts every signed int without validation.
                self.attributes[index]._kind = a[1] as i32;
                Ok(0)
            }
            RwlockDestroy => {
                let index = self.lock_index(a[0], false, memory, guest, budget)?;
                let value = &self.locks[index];
                if value.writer.is_some() || value.readers != 0 {
                    return Err(Errno::Busy.into());
                }
                memory.destroy_pthread_object(
                    PthreadObjectKind::Rwlock,
                    value.id,
                    a[0],
                    guest,
                    budget,
                )?;
                self.locks.swap_remove(index);
                Ok(0)
            }
            RwlockReadLock | RwlockWriteLock | RwlockTryReadLock | RwlockTryWriteLock
            | RwlockTimedReadLock | RwlockTimedWriteLock | RwlockUnlock => {
                // Unlike the other acquisition routes, source Trywrlock does
                // not lazily initialize a zero slot.
                let index =
                    self.lock_index(a[0], service != RwlockTryWriteLock, memory, guest, budget)?;
                let value = &mut self.locks[index];
                if service == RwlockUnlock {
                    // Source unlock releases a nested reader before its writer.
                    if value.reader == Some(thread) && value.readers != 0 {
                        value.readers -= 1;
                        if value.readers == 0 {
                            value.reader = None;
                        }
                    } else if value.writer == Some(thread) {
                        value.writer = None;
                    } else {
                        return Err(Errno::Perm.into());
                    }
                    return Ok(0);
                }
                let write = matches!(
                    service,
                    RwlockWriteLock | RwlockTryWriteLock | RwlockTimedWriteLock
                );
                if write {
                    if value.writer == Some(thread) || value.reader == Some(thread) {
                        return Err(Errno::Deadlock.into());
                    }
                    if value.writer.is_none() && value.readers == 0 {
                        value.writer = Some(thread);
                        return Ok(0);
                    }
                } else if value.writer.is_none() || value.writer == Some(thread) {
                    if value.reader.is_some_and(|reader| reader != thread) {
                        return Err(Failure::Unsupported("additional rwlock reader thread"));
                    }
                    let readers = value.readers.checked_add(1).ok_or(Errno::Again)?;
                    value.reader = Some(thread);
                    value.readers = readers;
                    return Ok(0);
                }
                if matches!(service, RwlockTryReadLock | RwlockTryWriteLock) {
                    return Err(Errno::Busy.into());
                }
                if matches!(service, RwlockTimedReadLock | RwlockTimedWriteLock) && a[1] as u32 == 0
                {
                    return Err(Errno::TimedOut.into());
                }
                Err(Failure::Unsupported("contended rwlock wait"))
            }
            _ => Err(Failure::Unsupported("rwlock service")),
        }
    }
}

impl crate::Kernel {
    pub(crate) fn invoke_pthread_rwlock(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        guest.preflight_pthread()?;
        use Service::*;
        let (delegate, positive) = match service {
            PosixRwlockInit => (RwlockInit, true),
            PosixRwlockDestroy => (RwlockDestroy, true),
            PosixRwlockWriteLock => (RwlockWriteLock, true),
            _ => (service, false),
        };
        let mut values = args.0;
        if service == PosixRwlockInit {
            values[2] = 0;
        }
        let result = self.pthread_rwlock.invoke(
            delegate,
            CallArgs(values),
            &mut self.memory,
            guest,
            &mut self.budget,
            self.threads.active().id,
        );
        if positive {
            crate::posix::pthread_result(result)
        } else {
            result
        }
    }
}
