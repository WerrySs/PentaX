// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded process-owned keys and separate thread
// values, transactional guest output and explicit lifecycle boundaries.
// Adapted from pinned kernel/pthread.cpp/h and libs/errno.h; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Service};

pub(crate) const KEYS_MAX: usize = 256;
#[derive(Clone, Copy, Default)]
struct Key {
    generation: u64,
    // Retained guest entry; the thread-exit action owner admits callbacks as
    // data only and never calls this integer as a host function pointer.
    destructor: u64,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Specific {
    generation: u64,
    value: u64,
}
pub(crate) struct KeyState {
    keys: Vec<Key>,
}
impl KeyState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.keys
            .iter()
            .filter(|key| key.generation & 1 != 0)
            .count()
    }

    pub(crate) fn requires_cleanup(&self, values: &Option<Vec<Specific>>) -> bool {
        values.as_ref().is_some_and(|values| {
            self.keys.iter().zip(values).any(|(key, value)| {
                key.generation & 1 != 0
                    && key.destructor != 0
                    && key.generation == value.generation
                    && value.value != 0
            })
        })
    }
    /// Read one current slot's source eligibility without clearing it. The
    /// thread-exit planner commits the whole-slot exchange before admission.
    pub(crate) fn exit_callback(
        &self,
        index: usize,
        values: &Option<Vec<Specific>>,
    ) -> Option<(u64, u64)> {
        let value = values.as_ref()?.get(index)?;
        let key = self.keys.get(index)?;
        (value.value != 0
            && key.generation & 1 != 0
            && key.generation == value.generation
            && key.destructor != 0)
            .then_some((key.destructor, value.value))
    }
    pub(crate) fn new() -> Result<Self, Failure> {
        let mut keys = Vec::new();
        keys.try_reserve_exact(KEYS_MAX)
            .map_err(|_| Failure::Allocation)?;
        keys.resize(KEYS_MAX, Key::default());
        Ok(Self { keys })
    }
    fn index(key: u64) -> Option<usize> {
        let key = key as u32 as i32;
        (0..KEYS_MAX as i32).contains(&key).then_some(key as usize)
    }
    fn allocated(&self, key: u64) -> Result<usize, Failure> {
        Self::index(key)
            .filter(|&index| self.keys[index].generation & 1 != 0)
            .ok_or(Errno::Invalid.into())
    }
    fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        values: &mut Option<Vec<Specific>>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        budget.work(if service == KeyCreate {
            KEYS_MAX as u64 + 8
        } else {
            8
        })?;
        let a = args.0;
        match service {
            KeyCreate => {
                if a[0] == 0 {
                    return Err(Errno::Invalid.into());
                }
                let index = self
                    .keys
                    .iter()
                    .position(|entry| entry.generation & 1 == 0)
                    .ok_or(Errno::Again)?;
                budget.can_add_handle()?;
                let generation = self.keys[index]
                    .generation
                    .checked_add(1)
                    .ok_or(Failure::Limit("pthread key generations"))?;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &(index as u32).to_le_bytes(),
                    }],
                )?;
                self.keys[index] = Key {
                    generation,
                    destructor: a[1],
                };
                budget.added_handle();
                Ok(0)
            }
            KeyDelete => {
                let index = self.allocated(a[0])?;
                let generation = self.keys[index]
                    .generation
                    .checked_add(1)
                    .ok_or(Failure::Limit("pthread key generations"))?;
                self.keys[index] = Key {
                    generation,
                    destructor: 0,
                };
                budget.removed_handle();
                // Source deletion neither runs callbacks nor clears old thread
                // values. The generation prevents their reuse by a new key.
                Ok(0)
            }
            SetSpecific => {
                let index = self.allocated(a[0])?;
                if values.is_none() {
                    if a[1] == 0 {
                        return Ok(0);
                    }
                    let bytes = KEYS_MAX * std::mem::size_of::<Specific>();
                    budget.scratch(bytes)?;
                    budget.work(bytes as u64)?;
                    let mut allocated = Vec::new();
                    allocated
                        .try_reserve_exact(KEYS_MAX)
                        .map_err(|_| Failure::Allocation)?;
                    allocated.resize(KEYS_MAX, Specific::default());
                    *values = Some(allocated);
                    budget.record_scratch(bytes);
                }
                values.as_mut().unwrap()[index] = Specific {
                    generation: self.keys[index].generation,
                    value: a[1],
                };
                Ok(0)
            }
            GetSpecific => {
                let Some(index) = Self::index(a[0]) else {
                    return Ok(0);
                };
                let Some(values) = values else {
                    return Ok(0);
                };
                let generation = self.keys[index].generation;
                Ok(
                    if generation & 1 != 0 && values[index].generation == generation {
                        values[index].value
                    } else {
                        0
                    },
                )
            }
            _ => Err(Failure::Unsupported("pthread key service")),
        }
    }
}
impl crate::Kernel {
    pub fn active_thread_requires_cleanup(&self) -> bool {
        let active = self.threads.active();
        active.exit_cleanup.in_progress()
            || (!active.exit_cleanup.finished()
                && (self.thread_lifecycle.destructor.is_some()
                    || self.pthread_keys.requires_cleanup(&active.values)))
    }
    pub(crate) fn invoke_pthread_keys(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        guest.preflight_pthread()?;
        use Service::*;
        let (delegate, positive) = match service {
            PosixKeyCreate => (KeyCreate, true),
            PosixKeyDelete => (KeyDelete, true),
            PosixSetSpecific => (SetSpecific, true),
            _ => (service, false),
        };
        let result = self.pthread_keys.invoke(
            delegate,
            args,
            &mut self.threads.active_mut().values,
            guest,
            &mut self.budget,
        );
        if positive {
            crate::posix::pthread_result(result)
        } else {
            result
        }
    }
}
