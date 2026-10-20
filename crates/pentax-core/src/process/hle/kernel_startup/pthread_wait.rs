// SPDX-License-Identifier: GPL-2.0-only

//! Retain the exact admitted blocking syscall while its guest thread is parked.

use super::*;
use pentax_kernel::{
    ConditionWait, ConditionWaitAction, EventFlagWait, EventFlagWaitAction, EventQueueWait,
    EventQueueWaitAction, MutexWait, MutexWaitAction, SemaphoreWait, SemaphoreWaitAction, Service,
};
use pentax_platform::native_memory::NativeOwnerId;

pub struct PendingThreadWait {
    owner: NativeOwnerId,
    thread_pointer: u64,
    pc: u64,
    id: u64,
    args: [u64; 6],
    ticket: Ticket,
}
enum Ticket {
    EventQueue(EventQueueWait),
    Condition(ConditionWait),
    Semaphore(SemaphoreWait),
    Mutex(MutexWait),
    EventFlag(EventFlagWait),
}
impl fmt::Debug for PendingThreadWait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingThreadWait").finish_non_exhaustive()
    }
}

pub(super) fn begin(
    kernel: &mut Kernel,
    memory: &mut KernelMemory<'_, '_, '_>,
    service: Service,
    args: CallArgs,
    pc: u64,
    id: u64,
) -> Result<NativeServiceAction, KernelStartupError> {
    let owner = memory.paused.owner_id();
    let thread_pointer = memory
        .thread_tls
        .ok_or(KernelStartupError::InvalidProvider("thread wait TLS"))?
        .thread_pointer;
    let ticket = if matches!(
        service,
        Service::WaitEventFlag
            | Service::SetEventFlag
            | Service::ClearEventFlag
            | Service::CancelEventFlag
            | Service::DeleteEventFlag
    ) {
        match kernel
            .begin_event_flag_operation(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            EventFlagWaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            EventFlagWaitAction::Waiting(ticket) => Ticket::EventFlag(ticket),
        }
    } else if service == Service::WaitEqueue {
        match kernel
            .begin_event_queue_wait(args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            EventQueueWaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            EventQueueWaitAction::Waiting(ticket) => Ticket::EventQueue(ticket),
        }
    } else if service == Service::WaitSema {
        match kernel
            .begin_semaphore_wait(args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            SemaphoreWaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            SemaphoreWaitAction::Waiting(ticket) => Ticket::Semaphore(ticket),
        }
    } else if matches!(
        service,
        Service::MutexLock
            | Service::PosixMutexLock
            | Service::MutexTimedLock
            | Service::PosixMutexTimedLock
    ) {
        match kernel
            .begin_mutex_wait(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            MutexWaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            MutexWaitAction::Waiting(ticket) => Ticket::Mutex(ticket),
        }
    } else {
        match kernel
            .begin_condition_wait(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            ConditionWaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            ConditionWaitAction::Waiting(ticket) => Ticket::Condition(ticket),
        }
    };
    Ok(NativeServiceAction::WaitThread(PendingThreadWait {
        owner,
        thread_pointer,
        pc,
        id,
        args: args.0,
        ticket,
    }))
}

impl PreparedNativeKernelStartup<'_, '_> {
    pub fn thread_wait_ready(
        &mut self,
        pending: &PendingThreadWait,
    ) -> Result<bool, KernelStartupError> {
        if pending.owner != self.pause()?.owner_id() {
            return Err(KernelStartupError::InvalidProvider("thread wait owner"));
        }
        match &pending.ticket {
            Ticket::EventQueue(ticket) => self.kernel.event_queue_wait_ready(ticket),
            Ticket::Condition(ticket) => self.kernel.condition_wait_ready(ticket),
            Ticket::Semaphore(ticket) => self.kernel.semaphore_wait_ready(ticket),
            Ticket::Mutex(ticket) => self.kernel.mutex_wait_ready(ticket),
            Ticket::EventFlag(ticket) => self.kernel.event_flag_wait_ready(ticket),
        }
        .map_err(KernelStartupError::Kernel)
    }

    pub fn thread_wait_delay(
        &mut self,
        pending: &PendingThreadWait,
    ) -> Result<Option<std::time::Duration>, KernelStartupError> {
        if pending.owner != self.pause()?.owner_id() {
            return Err(KernelStartupError::InvalidProvider("thread wait owner"));
        }
        match &pending.ticket {
            Ticket::EventQueue(ticket) => self
                .kernel
                .event_queue_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::Condition(_) => Ok(None),
            Ticket::Semaphore(ticket) => self
                .kernel
                .semaphore_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::Mutex(ticket) => self
                .kernel
                .mutex_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::EventFlag(ticket) => self
                .kernel
                .event_flag_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
        }
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn check_thread_wait(
        &self,
        pending: &PendingThreadWait,
        token: pentax_platform::native_dispatch::PreSyscallToken,
    ) -> Result<(), KernelStartupError> {
        if pending.owner != self.owner_id()
            || token.owner_id() != pending.owner
            || token.syscall_pc() != pending.pc
            || token.service_id() != pending.id
            || token.args() != pending.args
            || self
                .tls()
                .instances()
                .get(self.active_tls)
                .is_none_or(|instance| instance.thread_pointer != pending.thread_pointer)
        {
            return Err(KernelStartupError::InvalidProvider(
                "thread wait continuation",
            ));
        }
        Ok(())
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    pub fn complete_thread_wait(
        &mut self,
        pending: &PendingThreadWait,
        token: pentax_platform::native_dispatch::PreSyscallToken,
        current: &PreparedNativeSource<'_, '_>,
        engine: &pentax_platform::native_dispatch::NativeBlockStepper<'_, '_, '_>,
    ) -> Result<Option<u64>, KernelStartupError> {
        self.check_thread_wait(pending, token)?;
        if current.owner_id() != self.owner_id()
            || current.materialized_source().source_plan().identity()
                != self.memory().active_source_identity()
        {
            return Err(KernelStartupError::InvalidProvider("thread wait source"));
        }
        // Full token equality includes the executor's private sequence, raw
        // vector snapshot and phase. Public PC/ID/arguments alone cannot prove
        // that this is still its pending continuation. Check before any wait
        // completion consumes a ticket, writes outputs or transfers ownership.
        engine.validate_pending_syscall(token).map_err(|_| {
            KernelStartupError::InvalidProvider("thread wait executor continuation")
        })?;
        // Admission is bound to its actual saved syscall and owner, rather than
        // the old source epoch: another thread may append an admitted module
        // while this one waits. The executor retains and validates that lineage.
        match &pending.ticket {
            Ticket::Mutex(ticket) => self
                .kernel
                .complete_mutex_wait(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::Condition(ticket) => self
                .kernel
                .complete_condition_wait(ticket)
                .map(|ready| ready.then_some(0))
                .map_err(KernelStartupError::Kernel),
            Ticket::Semaphore(_) | Ticket::EventQueue(_) | Ticket::EventFlag(_) => {
                let (mut paused, instances) =
                    self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
                let mut guest = KernelMemory {
                    paused: &mut paused,
                    source: Some(current.materialized_source().source_plan()),
                    supplied_modules: self.plan.kernel_provider_index(),
                    lookup_limits: None,
                    lookup_work: None,
                    loaded_modules: None,
                    thread_tls: instances.get(self.active_tls),
                    initial_entry: None,
                };
                match &pending.ticket {
                    Ticket::Semaphore(ticket) => {
                        self.kernel.complete_semaphore_wait(ticket, &mut guest)
                    }
                    Ticket::EventQueue(ticket) => {
                        self.kernel.complete_event_queue_wait(ticket, &mut guest)
                    }
                    Ticket::EventFlag(ticket) => {
                        self.kernel.complete_event_flag_wait(ticket, &mut guest)
                    }
                    _ => unreachable!(),
                }
                .map_err(KernelStartupError::Kernel)
            }
        }
    }
}
