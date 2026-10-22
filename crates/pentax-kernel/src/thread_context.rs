// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned contexts and retained thread-affinity metadata.
// Affinity behavior adapted from pinned kernel/pthread.cpp; see NOTICE.

//! Per-thread service state selected by the runtime's admitted TLS context.
//! Selection is bookkeeping, not guest thread creation or execution authority.

use crate::pthread_keys::Specific;
use crate::{Failure, Limits, ThreadAttributes, ThreadPriorityClass};

pub(crate) struct ThreadContext {
    context: u64,
    pub(crate) id: u32,
    pub(crate) priority: i32,
    pub(crate) policy: i32,
    pub(crate) priority_class: ThreadPriorityClass,
    pub(crate) core_bound: bool,
    started: bool,
    pub(crate) affinity: u64,
    pub(crate) waiting: bool,
    pub(crate) time_wait: Option<crate::time::TimeWait>,
    pub(crate) self_pointer: Option<u64>,
    pub(crate) detached: bool,
    pub(crate) errno: Option<u64>,
    pub(crate) net_errno: Option<u64>,
    pub(crate) values: Option<Vec<Specific>>,
    pub(crate) exit_cleanup: crate::thread_exit::ThreadExitState,
}

impl ThreadContext {
    fn new(context: u64, id: u32) -> Self {
        Self {
            context,
            id,
            priority: 700,
            policy: 1,
            priority_class: ThreadPriorityClass::Normal,
            core_bound: false,
            // Main and directly selected preexisting contexts never acquire
            // the created-thread single-core demotion later in their lifetime.
            started: true,
            affinity: 0x7f,
            waiting: false,
            time_wait: None,
            self_pointer: None,
            detached: false,
            errno: None,
            net_errno: None,
            values: None,
            exit_cleanup: crate::thread_exit::ThreadExitState::default(),
        }
    }

    pub(crate) const fn context(&self) -> u64 {
        self.context
    }

    fn start(&mut self) {
        if !self.started {
            // The pinned RunThread decides once at first execution, using the
            // current creation metadata rather than relatching later affinity.
            self.core_bound = self.affinity.is_power_of_two();
            self.priority_class =
                ThreadPriorityClass::from_priority(self.priority, self.core_bound);
            self.started = true;
        }
    }
}

pub(crate) struct ThreadContexts {
    entries: Vec<ThreadContext>,
    active: usize,
    maximum: usize,
    last_id: u32,
}

impl ThreadContexts {
    pub(crate) fn detach(
        &mut self,
        pointer: u64,
        budget: &mut crate::budget::Budget,
    ) -> Result<bool, Failure> {
        budget.work(self.entries.len() as u64 + 1)?;
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.self_pointer == Some(pointer))
        else {
            return Ok(false);
        };
        // The pinned source succeeds when the flag was already set. Cleanup
        // and the prepared owner's stack/TLS lifetime are unchanged.
        if !entry.detached {
            entry.detached = true;
        }
        Ok(true)
    }

    pub(crate) fn require_registered(
        &self,
        context: u64,
        pointer: u64,
        budget: &mut crate::budget::Budget,
    ) -> Result<(), Failure> {
        budget.work(self.entries.len() as u64 + 1)?;
        if !self
            .entries
            .iter()
            .any(|entry| entry.context == context && entry.self_pointer == Some(pointer))
        {
            return Err(Failure::Unsupported("unregistered thread handle"));
        }
        Ok(())
    }

    fn register(
        &mut self,
        context: u64,
        pointer: u64,
        attributes: ThreadAttributes,
        guest: &mut impl crate::GuestAccess,
        budget: &mut crate::budget::Budget,
    ) -> Result<u32, Failure> {
        budget.work(self.entries.len() as u64 + 1)?;
        if context == 0
            || pointer == 0
            || self.entries[0].context == 0
            || self
                .entries
                .iter()
                .any(|entry| entry.context == context || entry.self_pointer == Some(pointer))
        {
            return Err(Failure::Unsupported(
                "duplicate or unbound thread registration",
            ));
        }
        if self.entries.len() >= self.maximum {
            return Err(Failure::Limit("thread contexts"));
        }
        let id = self
            .last_id
            .checked_add(1)
            .filter(|id| *id <= i32::MAX as u32)
            .ok_or(Failure::Limit("thread identities"))?;
        budget.write(
            guest,
            &[crate::GuestWrite {
                address: pointer,
                bytes: &id.to_le_bytes(),
            }],
        )?;
        let mut entry = ThreadContext::new(context, id);
        entry.self_pointer = Some(pointer);
        entry.detached = attributes.detached;
        entry.priority = attributes.priority;
        entry.policy = attributes.policy;
        entry.affinity = attributes.affinity;
        entry.priority_class = ThreadPriorityClass::from_priority(attributes.priority, false);
        entry.started = false;
        self.entries.push(entry);
        self.last_id = id;
        Ok(id)
    }
    pub(crate) fn new(limits: Limits, main_id: u32) -> Result<Self, Failure> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(limits.max_thread_contexts)
            .map_err(|_| Failure::Allocation)?;
        entries.push(ThreadContext::new(0, main_id));
        Ok(Self {
            entries,
            active: 0,
            maximum: limits.max_thread_contexts,
            last_id: main_id,
        })
    }

    pub(crate) fn active(&self) -> &ThreadContext {
        &self.entries[self.active]
    }
    pub(crate) fn active_mut(&mut self) -> &mut ThreadContext {
        &mut self.entries[self.active]
    }

    pub(crate) fn has_time_wait(
        &self,
        ticket: &crate::time::TimeWait,
        budget: &mut crate::budget::Budget,
    ) -> Result<bool, Failure> {
        budget.work(self.entries.len() as u64)?;
        Ok(self
            .entries
            .iter()
            .any(|entry| entry.waiting && entry.time_wait == Some(*ticket)))
    }

    pub(crate) fn priority_entry(
        &self,
        handle: u64,
        budget: &mut crate::budget::Budget,
    ) -> Result<&ThreadContext, Failure> {
        budget.work(self.entries.len() as u64 + 1)?;
        self.entries
            .iter()
            .find(|entry| entry.self_pointer == Some(handle) && !entry.exit_cleanup.finished())
            .ok_or(Failure::Unsupported("unowned or exited priority thread"))
    }

    pub(crate) fn priority_entry_mut(
        &mut self,
        handle: u64,
        budget: &mut crate::budget::Budget,
    ) -> Result<&mut ThreadContext, Failure> {
        budget.work(self.entries.len() as u64 + 1)?;
        self.entries
            .iter_mut()
            .find(|entry| entry.self_pointer == Some(handle) && !entry.exit_cleanup.finished())
            .ok_or(Failure::Unsupported("unowned or exited priority thread"))
    }

    pub(crate) fn invoke_affinity(
        &mut self,
        service: crate::Service,
        args: crate::CallArgs,
        guest: &mut impl crate::GuestAccess,
        budget: &mut crate::budget::Budget,
    ) -> Result<u64, Failure> {
        guest.preflight_pthread()?;
        let [handle, value, ..] = args.0;
        if handle == 0 {
            return Err(crate::Errno::NoSuchResource.into());
        }
        if service == crate::Service::PthreadGetaffinity && value == 0 {
            return Err(crate::Errno::Invalid.into());
        }
        budget.work(self.entries.len() as u64 + 1)?;
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.self_pointer == Some(handle))
            .ok_or(Failure::Unsupported("unowned affinity thread"))?;
        match service {
            crate::Service::PthreadSetaffinity => entry.affinity = value,
            crate::Service::PthreadGetaffinity => budget.write(
                guest,
                &[crate::GuestWrite {
                    address: value,
                    bytes: &entry.affinity.to_le_bytes(),
                }],
            )?,
            _ => return Err(Failure::Unsupported("thread affinity service")),
        }
        Ok(0)
    }

    pub(crate) fn select(
        &mut self,
        context: u64,
        budget: &mut crate::budget::Budget,
    ) -> Result<u32, Failure> {
        if context == 0 {
            return Err(Failure::Unsupported("unbound thread context"));
        }
        if self.entries[0].context == 0 {
            self.entries[0].context = context;
            self.active = 0;
            return Ok(self.entries[0].id);
        }
        budget.work(self.entries.len() as u64 + 1)?;
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.context == context)
        {
            self.entries[index].start();
            self.active = index;
            return Ok(self.entries[index].id);
        }
        if self.entries.len() >= self.maximum {
            return Err(Failure::Limit("thread contexts"));
        }
        let id = self
            .last_id
            .checked_add(1)
            .filter(|id| *id <= i32::MAX as u32)
            .ok_or(Failure::Limit("thread identities"))?;
        self.entries.push(ThreadContext::new(context, id));
        self.active = self.entries.len() - 1;
        self.last_id = id;
        Ok(id)
    }
}

impl crate::Kernel {
    /// Register a runtime-admitted context and its fresh RW/NX pthread prefix
    /// without selecting it. Retain configured guest priority and affinity;
    /// neither changes the host scheduling policy. The runtime retains
    /// backing/lifecycle ownership.
    pub fn register_thread_context(
        &mut self,
        context: u64,
        pointer: u64,
        priority: i32,
        affinity: u64,
        guest: &mut impl crate::GuestAccess,
    ) -> Result<u32, Failure> {
        self.register_thread_context_with_attributes(
            context,
            pointer,
            ThreadAttributes {
                priority,
                affinity,
                ..ThreadAttributes::default()
            },
            guest,
        )
    }

    /// Register the exact admitted creation attributes without starting the
    /// child. Its effective class/single-core decision is latched only on the
    /// first selection, from its then-current priority and affinity metadata.
    pub fn register_thread_context_with_attributes(
        &mut self,
        context: u64,
        pointer: u64,
        attributes: ThreadAttributes,
        guest: &mut impl crate::GuestAccess,
    ) -> Result<u32, Failure> {
        guest.preflight_pthread()?;
        self.threads
            .register(context, pointer, attributes, guest, &mut self.budget)
    }
    /// Select service state for a runtime-admitted thread context. The caller
    /// must bind its GuestAccess/TLS and CPU state to the same context. This
    /// neither starts a thread nor makes PthreadCreate/Join/Exit available.
    pub fn activate_thread_context(&mut self, context: u64) -> Result<u32, Failure> {
        self.threads.select(context, &mut self.budget)
    }
}
