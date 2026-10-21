// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded per-thread read/write ownership.
// Adapted from pinned kernel/pthread.cpp/h; see NOTICE.

//! Reader records share a finite process-owned pool. Cooperative wait tickets
//! are retained by the scheduler, never host-blocking or fabricated success.

use crate::budget::{Budget, Limits};
use crate::memory::{MemoryState, PthreadObjectKind};
use crate::{CallArgs, Errno, Failure, GuestAccess, Service};

mod wait;
pub use wait::{RwlockWait, RwlockWaitAction};

struct Attribute {
    id: u64,
    // The pinned initializer does not consume its attribute argument.
    _kind: i32,
}
struct Rwlock {
    id: u64,
    _name: String,
    writer: Option<u32>,
    readers: u32,
    waits: usize,
    waiting_writers: usize,
}
struct Reader {
    lock: u64,
    thread: u32,
    count: u32,
}
pub(crate) struct RwlockState {
    attributes: Vec<Attribute>,
    locks: Vec<Rwlock>,
    readers: Vec<Reader>,
    waiters: Vec<wait::Waiter>,
    maximum: usize,
    reader_reservations: usize,
}

impl RwlockState {
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut attributes = Vec::new();
        let mut locks = Vec::new();
        let mut readers = Vec::new();
        let mut waiters = Vec::new();
        attributes
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        locks
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        // This shared pair-record allowance is independent of object handles.
        // A thread may hold multiple locks until the finite pool is exhausted.
        readers
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        waiters
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        if readers.capacity() > limits.max_thread_contexts
            || waiters.capacity() > limits.max_thread_contexts
        {
            return Err(Failure::Limit("rwlock metadata capacity"));
        }
        Ok(Self {
            attributes,
            locks,
            readers,
            waiters,
            maximum: limits.max_thread_contexts,
            reader_reservations: 0,
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
            readers: 0,
            waits: 0,
            waiting_writers: 0,
        });
        Ok(self.locks.len() - 1)
    }

    fn reader_index(&self, lock: u64, thread: u32) -> Option<usize> {
        self.readers
            .iter()
            .position(|reader| reader.lock == lock && reader.thread == thread)
    }

    fn can_acquire(&self, index: usize, thread: u32, write: bool) -> Result<bool, Failure> {
        let value = &self.locks[index];
        let owns_read = self.reader_index(value.id, thread).is_some();
        if write {
            if value.writer == Some(thread) || owns_read {
                return Err(Errno::Deadlock.into());
            }
            Ok(value.writer.is_none() && value.readers == 0)
        } else {
            Ok(value.writer == Some(thread)
                || value.writer.is_none() && (value.waiting_writers == 0 || owns_read))
        }
    }

    fn acquire(
        &mut self,
        index: usize,
        thread: u32,
        write: bool,
        reserved_reader: bool,
    ) -> Result<(), Failure> {
        if write {
            self.locks[index].writer = Some(thread);
            return Ok(());
        }
        let value = &self.locks[index];
        let total = value.readers.checked_add(1).ok_or(Errno::Again)?;
        let reader = self.reader_index(value.id, thread);
        let count = reader
            .map(|index| self.readers[index].count.checked_add(1).ok_or(Errno::Again))
            .transpose()?;
        if reader.is_none()
            && self.readers.len() + self.reader_reservations - usize::from(reserved_reader)
                >= self.maximum
        {
            return Err(Failure::Limit("rwlock reader records"));
        }
        if let Some(reader) = reader {
            self.readers[reader].count = count.unwrap();
        } else {
            self.readers.push(Reader {
                lock: value.id,
                thread,
                count: 1,
            });
        }
        self.locks[index].readers = total;
        Ok(())
    }

    fn lock_index(
        &mut self,
        slot: u64,
        lazy: bool,
        needs_reader: bool,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        if slot == 0 {
            return Err(Errno::Invalid.into());
        }
        let id = budget.u64(guest, slot)?;
        if id == 0 && lazy {
            if needs_reader && self.readers.len() + self.reader_reservations >= self.maximum {
                return Err(Failure::Limit("rwlock reader records"));
            }
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
        budget.work(
            24 + 4
                * (self.attributes.len()
                    + self.locks.len()
                    + self.readers.len()
                    + self.waiters.len()) as u64,
        )?;
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
                let index = self.lock_index(a[0], false, false, memory, guest, budget)?;
                let value = &self.locks[index];
                if value.writer.is_some() || value.readers != 0 || value.waits != 0 {
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
                let index = self.lock_index(
                    a[0],
                    service != RwlockTryWriteLock,
                    matches!(
                        service,
                        RwlockReadLock | RwlockTryReadLock | RwlockTimedReadLock
                    ),
                    memory,
                    guest,
                    budget,
                )?;
                if service == RwlockUnlock {
                    // Source unlock releases a nested reader before its writer.
                    if let Some(reader) = self.reader_index(self.locks[index].id, thread) {
                        self.readers[reader].count -= 1;
                        self.locks[index].readers -= 1;
                        if self.readers[reader].count == 0 {
                            self.readers.swap_remove(reader);
                        }
                    } else if self.locks[index].writer == Some(thread) {
                        self.locks[index].writer = None;
                    } else {
                        return Err(Errno::Perm.into());
                    }
                    return Ok(0);
                }
                let write = matches!(
                    service,
                    RwlockWriteLock | RwlockTryWriteLock | RwlockTimedWriteLock
                );
                if self.can_acquire(index, thread, write)? {
                    self.acquire(index, thread, write, false)?;
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
