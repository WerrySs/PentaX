// SPDX-License-Identifier: GPL-2.0-only

//! Retain the exact admitted blocking syscall while its guest thread is parked.

use super::*;
use pentax_kernel::{
    AudioOut2Wait, AudioOut2WaitAction, ConditionWait, ConditionWaitAction, EventFlagWait,
    EventFlagWaitAction, EventQueueWait, EventQueueWaitAction, MutexWait, MutexWaitAction,
    PthreadSemaphoreWait, PthreadSemaphoreWaitAction, RwlockWait, RwlockWaitAction, SemaphoreWait,
    SemaphoreWaitAction, Service, TimeWait, TimeWaitAction,
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
    Join(pthread_create::JoinTicket),
    EventQueue(EventQueueWait),
    Condition(ConditionWait),
    Semaphore(SemaphoreWait),
    PthreadSemaphore(PthreadSemaphoreWait),
    Mutex(MutexWait),
    Rwlock(RwlockWait),
    EventFlag(EventFlagWait),
    AudioOut2(AudioOut2Wait),
    Time(TimeWait),
}
impl PendingThreadWait {
    /// Classify an actual retained wait without probing or consuming its ticket.
    pub fn is_event_queue(&self) -> bool {
        matches!(&self.ticket, Ticket::EventQueue(_))
    }
}
impl fmt::Debug for PendingThreadWait {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingThreadWait").finish_non_exhaustive()
    }
}

pub(super) fn begin_join(
    kernel: &mut Kernel,
    memory: &mut KernelMemory<'_, '_, '_>,
    runtime: &mut pthread_create::ThreadRuntime,
    args: CallArgs,
    pc: u64,
    id: u64,
) -> Result<NativeServiceAction, KernelStartupError> {
    if let Some(value) = kernel
        .admit_pthread_join(args, memory)
        .map_err(KernelStartupError::Kernel)?
    {
        return Ok(NativeServiceAction::Returned(value));
    }
    let waiter = memory
        .thread_tls
        .ok_or(KernelStartupError::InvalidProvider("pthread join TLS"))?
        .thread_pointer;
    let maximum = memory
        .lookup_limits
        .ok_or(KernelStartupError::InvalidProvider("pthread join work"))?
        .max_work;
    charge(
        memory
            .lookup_work
            .as_deref_mut()
            .ok_or(KernelStartupError::InvalidProvider("pthread join work"))?,
        runtime.join_scan_work(),
        maximum,
    )?;
    match runtime
        .begin_join(kernel, args, waiter)
        .map_err(KernelStartupError::Kernel)?
    {
        pthread_create::JoinAction::Returned(value) => Ok(NativeServiceAction::Returned(value)),
        pthread_create::JoinAction::Wait(ticket) => {
            Ok(NativeServiceAction::WaitThread(PendingThreadWait {
                owner: memory.paused.owner_id(),
                thread_pointer: waiter,
                pc,
                id,
                args: args.0,
                ticket: Ticket::Join(ticket),
            }))
        }
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
    let ticket = if service == Service::Usleep {
        match kernel
            .begin_time_wait(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            TimeWaitAction::Returned(value) => return Ok(NativeServiceAction::Returned(value)),
            TimeWaitAction::Waiting(ticket) => Ticket::Time(ticket),
        }
    } else if matches!(
        service,
        Service::AudioOut2ContextCreate
            | Service::AudioOut2ContextDestroy
            | Service::AudioOut2ContextAdvance
            | Service::AudioOut2ContextPush
            | Service::AudioOut2ContextGetQueueLevel
            | Service::AudioOut2UserCreate
            | Service::AudioOut2UserDestroy
            | Service::AudioOut2PortCreate
            | Service::AudioOut2PortDestroy
            | Service::AudioOut2PortGetState
            | Service::AudioOut2PortSetAttributes
    ) {
        // Source state locking also covers lifecycle and PCM mutation. An
        // in-flight paced submission may defer any of these calls, not only
        // ContextPush; preserve its exact original syscall in the same adapter.
        match kernel
            .begin_audio_out2_wait(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            AudioOut2WaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            AudioOut2WaitAction::Waiting(ticket) => Ticket::AudioOut2(ticket),
        }
    } else if matches!(
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
    } else if matches!(service, Service::PosixSemWait | Service::PthreadSemWait) {
        match kernel
            .begin_pthread_semaphore_wait(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            PthreadSemaphoreWaitAction::Returned(value) => {
                return Ok(NativeServiceAction::Returned(value));
            }
            PthreadSemaphoreWaitAction::Waiting(ticket) => Ticket::PthreadSemaphore(ticket),
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
    } else if matches!(
        service,
        Service::RwlockReadLock
            | Service::RwlockWriteLock
            | Service::RwlockTimedReadLock
            | Service::RwlockTimedWriteLock
            | Service::PosixRwlockWriteLock
    ) {
        match kernel
            .begin_rwlock_wait(service, args, memory)
            .map_err(KernelStartupError::Kernel)?
        {
            RwlockWaitAction::Returned(value) => return Ok(NativeServiceAction::Returned(value)),
            RwlockWaitAction::Waiting(ticket) => Ticket::Rwlock(ticket),
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
            Ticket::Join(ticket) => {
                self.kernel
                    .poll_pthread_join()
                    .map_err(KernelStartupError::Kernel)?;
                self.thread_runtime.join_ready(ticket)
            }
            Ticket::EventQueue(ticket) => self.kernel.event_queue_wait_ready(ticket),
            Ticket::Condition(ticket) => self.kernel.condition_wait_ready(ticket),
            Ticket::Semaphore(ticket) => self.kernel.semaphore_wait_ready(ticket),
            Ticket::PthreadSemaphore(ticket) => self.kernel.pthread_semaphore_wait_ready(ticket),
            Ticket::Mutex(ticket) => self.kernel.mutex_wait_ready(ticket),
            Ticket::Rwlock(ticket) => self.kernel.rwlock_wait_ready(ticket),
            Ticket::EventFlag(ticket) => self.kernel.event_flag_wait_ready(ticket),
            Ticket::AudioOut2(_) | Ticket::Time(_) => {
                let (mut paused, instances) =
                    self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
                let guest = KernelMemory {
                    paused: &mut paused,
                    source: None,
                    supplied_modules: self.plan.kernel_provider_index(),
                    lookup_limits: None,
                    lookup_work: None,
                    loaded_modules: None,
                    thread_tls: instances.get(self.active_tls),
                    initial_entry: None,
                };
                // This is an observation of the same process-microsecond
                // clock, possibly while a different guest thread is selected.
                // No PCM read, guest write or pending-ticket consumption occurs.
                match &pending.ticket {
                    Ticket::AudioOut2(ticket) => self.kernel.audio_out2_wait_ready(ticket, &guest),
                    Ticket::Time(ticket) => self.kernel.time_wait_ready(ticket, &guest),
                    _ => unreachable!(),
                }
            }
        }
        .map_err(KernelStartupError::Kernel)
    }

    /// Eligibility only: no wait, guest output or ticket is consumed here.
    pub fn thread_wait_has_vblank(
        &mut self,
        pending: &PendingThreadWait,
    ) -> Result<bool, KernelStartupError> {
        if pending.owner != self.pause()?.owner_id() {
            return Err(KernelStartupError::InvalidProvider("thread wait owner"));
        }
        let Ticket::EventQueue(ticket) = &pending.ticket else {
            return Ok(false);
        };
        let generations = std::array::from_fn(|slot| {
            self.graphics
                .as_ref()
                .and_then(|driver| driver.video_port(slot as i32 + 1))
                .map(|port| port.generation)
        });
        self.kernel
            .event_queue_wait_has_vblank(ticket, generations)
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
            Ticket::Condition(_) | Ticket::Join(_) | Ticket::PthreadSemaphore(_) => Ok(None),
            Ticket::Semaphore(ticket) => self
                .kernel
                .semaphore_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::Mutex(ticket) => self
                .kernel
                .mutex_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::Rwlock(ticket) => self
                .kernel
                .rwlock_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::EventFlag(ticket) => self
                .kernel
                .event_flag_wait_delay(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::AudioOut2(_) | Ticket::Time(_) => {
                let (mut paused, instances) =
                    self.hle.pause_with_tls().map_err(KernelStartupError::Hle)?;
                let guest = KernelMemory {
                    paused: &mut paused,
                    source: None,
                    supplied_modules: self.plan.kernel_provider_index(),
                    lookup_limits: None,
                    lookup_work: None,
                    loaded_modules: None,
                    thread_tls: instances.get(self.active_tls),
                    initial_entry: None,
                };
                match &pending.ticket {
                    Ticket::AudioOut2(ticket) => self.kernel.audio_out2_wait_delay(ticket, &guest),
                    Ticket::Time(ticket) => self.kernel.time_wait_delay(ticket, &guest),
                    _ => unreachable!(),
                }
                .map_err(KernelStartupError::Kernel)
            }
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
            Ticket::Rwlock(ticket) => self
                .kernel
                .complete_rwlock_wait(ticket)
                .map_err(KernelStartupError::Kernel),
            Ticket::Condition(ticket) => self
                .kernel
                .complete_condition_wait(ticket)
                .map(|ready| ready.then_some(0))
                .map_err(KernelStartupError::Kernel),
            Ticket::Join(_)
            | Ticket::Semaphore(_)
            | Ticket::PthreadSemaphore(_)
            | Ticket::EventQueue(_)
            | Ticket::EventFlag(_)
            | Ticket::AudioOut2(_)
            | Ticket::Time(_) => {
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
                    Ticket::Join(ticket) => self.thread_runtime.complete_join(
                        &mut self.kernel,
                        ticket,
                        pending.thread_pointer,
                        &mut guest,
                    ),
                    Ticket::Semaphore(ticket) => {
                        self.kernel.complete_semaphore_wait(ticket, &mut guest)
                    }
                    Ticket::PthreadSemaphore(ticket) => self
                        .kernel
                        .complete_pthread_semaphore_wait(ticket, &mut guest),
                    Ticket::EventQueue(ticket) => {
                        self.kernel.complete_event_queue_wait(ticket, &mut guest)
                    }
                    Ticket::EventFlag(ticket) => {
                        self.kernel.complete_event_flag_wait(ticket, &mut guest)
                    }
                    Ticket::AudioOut2(ticket) => {
                        self.kernel.complete_audio_out2_wait(ticket, &mut guest)
                    }
                    Ticket::Time(ticket) => self.kernel.complete_time_wait(ticket, &guest),
                    _ => unreachable!(),
                }
                .map_err(KernelStartupError::Kernel)
            }
        }
    }
}
