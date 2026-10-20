// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded, owned guest thread attributes.
// Adapted from pinned kernel/pthread.cpp/h; see NOTICE.

//! Attributes retain guest configuration; they neither map a stack nor create
//! a host thread. Thread creation must separately validate and consume them.

use crate::budget::{Budget, Limits};
use crate::memory::{MemoryState, PthreadObjectKind};
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Service};

/// Guest configuration copied at creation, not allocation or execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadAttributes {
    pub affinity: u64,
    pub guard_size: u64,
    pub stack_address: u64,
    pub stack_size: u64,
    pub policy: i32,
    pub priority: i32,
    pub inherit_sched: i32,
    pub solosched: i32,
    pub detached: bool,
}
type Values = ThreadAttributes;

impl Default for Values {
    fn default() -> Self {
        Self {
            affinity: 0x7f,
            guard_size: 0x1000,
            stack_address: 0,
            stack_size: 0x100000,
            policy: 1,
            priority: 700,
            inherit_sched: 4,
            solosched: 0,
            detached: false,
        }
    }
}

struct Attribute {
    id: u64,
    values: Values,
}

pub(crate) struct AttributeState {
    attributes: Vec<Attribute>,
}

impl AttributeState {
    pub(crate) fn creation_values(
        &self,
        slot: u64,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<ThreadAttributes, Failure> {
        budget.work(self.attributes.len() as u64 + 1)?;
        if slot == 0 {
            return Ok(Values::default());
        }
        let id = budget.u64(guest, slot)?;
        if id < 0x10000 {
            return Ok(Values::default());
        }
        self.attributes
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.values)
            .ok_or(Failure::Unsupported("unowned creation attributes"))
    }
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        let mut attributes = Vec::new();
        attributes
            .try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self { attributes })
    }

    fn index(
        &self,
        slot: u64,
        source_checked: bool,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        if slot == 0 {
            return Err(Errno::Invalid.into());
        }
        let id = budget.u64(guest, slot)?;
        // Setters/destroy use the pinned GetPthreadAttrValue helper. Getters
        // only reject null; other non-owned references remain typed stops.
        if id == 0 || (source_checked && id < 0x10000) {
            return Err(Errno::Invalid.into());
        }
        self.attributes
            .iter()
            .position(|entry| entry.id == id)
            .ok_or(Failure::Unsupported("unowned thread attribute"))
    }

    fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        memory: &mut MemoryState,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        initial_thread: Option<(u64, ThreadAttributes)>,
    ) -> Result<u64, Failure> {
        budget.work((self.attributes.len() + 1) as u64)?;
        let a = args.0;
        use Service::*;
        if service == ThreadAttrInit {
            if a[0] == 0 {
                return Err(Failure::Unsupported("source-null thread attribute slot"));
            }
            let previous = budget.u64(guest, a[0])?;
            if memory.owns_pthread_object(previous, budget)? {
                return Err(Failure::Unsupported("reinitializing live pthread object"));
            }
            let id = memory.create_pthread_object(
                PthreadObjectKind::ThreadAttribute,
                a[0],
                guest,
                budget,
            )?;
            self.attributes.push(Attribute {
                id,
                values: Values::default(),
            });
            return Ok(0);
        }
        if service == ThreadAttrGet {
            if a[0] == 0 {
                return Err(Errno::Invalid.into());
            }
            let index = self.index(a[1], false, guest, budget)?;
            let Some((_, attributes)) = initial_thread.filter(|(pointer, _)| *pointer == a[0])
            else {
                return Err(Failure::Unsupported("attribute query for unowned thread"));
            };
            // The adapter bounds inspection to an 8 MiB stack and two page
            // guards, checking its source/owner before exposing metadata.
            budget.work(2048)?;
            let stack = guest.initial_thread_stack()?;
            // Share retained scheduling metadata with the actual active thread.
            self.attributes[index].values = Values {
                stack_address: stack.address,
                stack_size: stack.size,
                guard_size: stack.guard_size,
                ..attributes
            };
            return Ok(0);
        }
        let getter = matches!(
            service,
            ThreadAttrGetAffinity
                | ThreadAttrGetDetachState
                | ThreadAttrGetGuardSize
                | ThreadAttrGetSchedParam
                | ThreadAttrGetSchedPolicy
                | ThreadAttrGetSoloSched
                | ThreadAttrGetStack
                | ThreadAttrGetStackAddr
                | ThreadAttrGetStackSize
        );
        if getter && (a[1] == 0 || (service == ThreadAttrGetStack && a[2] == 0)) {
            return Err(Errno::Invalid.into());
        }
        let index = self.index(a[0], !getter, guest, budget)?;
        if service == ThreadAttrDestroy {
            memory.destroy_pthread_object(
                PthreadObjectKind::ThreadAttribute,
                self.attributes[index].id,
                a[0],
                guest,
                budget,
            )?;
            self.attributes.swap_remove(index);
            return Ok(0);
        }
        let value = &mut self.attributes[index].values;
        if getter {
            let (bytes, length) = match service {
                ThreadAttrGetAffinity => (value.affinity.to_le_bytes(), 8),
                ThreadAttrGetGuardSize => (value.guard_size.to_le_bytes(), 8),
                ThreadAttrGetStack | ThreadAttrGetStackAddr => {
                    (value.stack_address.to_le_bytes(), 8)
                }
                ThreadAttrGetStackSize => (value.stack_size.to_le_bytes(), 8),
                ThreadAttrGetDetachState => (u64::from(value.detached).to_le_bytes(), 4),
                ThreadAttrGetSchedParam => ((value.priority as u32 as u64).to_le_bytes(), 4),
                ThreadAttrGetSchedPolicy => ((value.policy as u32 as u64).to_le_bytes(), 4),
                ThreadAttrGetSoloSched => ((value.solosched as u32 as u64).to_le_bytes(), 4),
                _ => unreachable!(),
            };
            let first = GuestWrite {
                address: a[1],
                bytes: &bytes[..length],
            };
            if service == ThreadAttrGetStack {
                // Budget::write rejects overlapping/overflowing outputs before
                // the adapter publishes either half of the pair.
                budget.write(
                    guest,
                    &[
                        first,
                        GuestWrite {
                            address: a[2],
                            bytes: &value.stack_size.to_le_bytes(),
                        },
                    ],
                )?;
            } else {
                budget.write(guest, &[first])?;
            }
            return Ok(0);
        }
        match service {
            ThreadAttrSetAffinity => value.affinity = a[1],
            ThreadAttrSetGuardSize => value.guard_size = a[1],
            ThreadAttrSetDetachState => match a[1] as i32 {
                0 => value.detached = false,
                1 => value.detached = true,
                _ => return Err(Failure::Unsupported("source-fatal thread detach state")),
            },
            ThreadAttrSetInheritSched => match a[1] as i32 {
                0 | 4 => value.inherit_sched = a[1] as i32,
                _ => return Err(Failure::Unsupported("source-fatal thread inherit mode")),
            },
            ThreadAttrSetSchedParam => {
                if a[1] == 0 {
                    return Err(Errno::Invalid.into());
                }
                // Preserve the exact signed guest priority, not a lossy host band.
                value.priority = budget.u32(guest, a[1])? as i32;
            }
            ThreadAttrSetSchedPolicy => value.policy = a[1] as i32,
            ThreadAttrSetSoloSched => value.solosched = a[1] as i32,
            ThreadAttrSetStack => {
                if a[1] == 0 || a[2] < 0x4000 {
                    return Err(Errno::Invalid.into());
                }
                value.stack_address = a[1];
                value.stack_size = a[2];
            }
            ThreadAttrSetStackAddr => {
                if a[1] == 0 {
                    return Err(Errno::Invalid.into());
                }
                value.stack_address = a[1];
            }
            ThreadAttrSetStackSize => {
                if a[1] < 0x4000 {
                    return Err(Errno::Invalid.into());
                }
                value.stack_size = a[1];
            }
            _ => return Err(Failure::Unsupported("thread attribute service")),
        }
        Ok(0)
    }
}

impl crate::Kernel {
    pub(crate) fn invoke_pthread_attr(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        guest.preflight_pthread()?;
        use Service::*;
        let (delegate, positive) = match service {
            PosixThreadAttrInit => (ThreadAttrInit, true),
            PosixThreadAttrDestroy => (ThreadAttrDestroy, true),
            PosixThreadAttrGet => (ThreadAttrGet, true),
            PosixThreadAttrGetStack => (ThreadAttrGetStack, true),
            PosixThreadAttrGetStackSize => (ThreadAttrGetStackSize, true),
            PosixThreadAttrGetDetachState => (ThreadAttrGetDetachState, true),
            PosixThreadAttrGetGuardSize => (ThreadAttrGetGuardSize, true),
            PosixThreadAttrGetSchedParam => (ThreadAttrGetSchedParam, true),
            PosixThreadAttrGetSchedPolicy => (ThreadAttrGetSchedPolicy, true),
            PosixThreadAttrSetStack => (ThreadAttrSetStack, true),
            PosixThreadAttrSetStackSize => (ThreadAttrSetStackSize, true),
            PosixThreadAttrSetDetachState => (ThreadAttrSetDetachState, true),
            PosixThreadAttrSetGuardSize => (ThreadAttrSetGuardSize, true),
            PosixThreadAttrSetSchedParam => (ThreadAttrSetSchedParam, true),
            PosixThreadAttrSetSchedPolicy => (ThreadAttrSetSchedPolicy, true),
            PosixThreadAttrSetInheritSched => (ThreadAttrSetInheritSched, true),
            _ => (service, false),
        };
        let result = self.pthread_attr.invoke(
            delegate,
            args,
            &mut self.memory,
            guest,
            &mut self.budget,
            self.threads.active().self_pointer.map(|pointer| {
                (
                    pointer,
                    ThreadAttributes {
                        priority: self.threads.active().priority,
                        policy: self.threads.active().policy,
                        affinity: self.threads.active().affinity,
                        ..ThreadAttributes::default()
                    },
                )
            }),
        );
        if positive {
            crate::posix::pthread_result(result)
        } else {
            result
        }
    }
}
