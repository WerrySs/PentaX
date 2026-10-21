// SPDX-License-Identifier: GPL-2.0-only

//! Bounded cooperative scheduling on the shared native executor.

use super::*;
use pentax_core::process::hle::{
    PendingThreadCreate, PendingThreadWait, PreparedNativeThread, PreparedThreadStart,
};
use pentax_kernel::ThreadPriorityClass;
use pentax_platform::native_dispatch::NativeThreadContext;

mod observations;
#[cfg(test)]
mod tests;
pub(super) use observations::Observations;

/// A consumer deadline can make a completely parked machine externally
/// wakeable. It neither satisfies a guest wait nor consumes its sleep quota.
fn idle_slice(
    guest: Option<std::time::Duration>,
    external: Option<super::frame_bridge::ExternalWake>,
) -> Option<(std::time::Duration, Option<u64>)> {
    match (guest, external) {
        (Some(delay), Some(wake)) if wake.delay <= delay => Some((
            wake.delay.min(std::time::Duration::from_millis(1)),
            Some(wake.maximum_idle_nanoseconds),
        )),
        (None, Some(wake)) => Some((
            wake.delay.min(std::time::Duration::from_millis(1)),
            Some(wake.maximum_idle_nanoseconds),
        )),
        (Some(delay), _) => Some((delay.min(std::time::Duration::from_millis(1)), None)),
        (None, None) => None,
    }
}

// PentaX policy: preserve round-robin admission and vary only bounded guest
// execution turns. Every runnable thread still receives a turn. These ratios
// implement the source's three classes without changing Windows scheduling.
fn quantum(class: ThreadPriorityClass) -> u64 {
    match class {
        ThreadPriorityClass::High => 4096,
        ThreadPriorityClass::Normal => 2048,
        ThreadPriorityClass::Low => 1024,
    }
}

// Keep descriptors inside the bounded preallocated queue; a separate Box would
// introduce another allocation on the creation/publication path.
#[allow(clippy::large_enum_variant)]
enum Binding<'plan, 'image> {
    Main(Option<PreparedNativeThread<'plan, 'image>>),
    Child(PreparedThreadStart<'plan, 'image>),
}
struct Thread<'plan, 'image> {
    binding: Binding<'plan, 'image>,
    parked: Option<NativeThreadContext>,
    frames: Vec<(SuspendedInitializer, NativeNestedCall, NativeSessionPhase)>,
    phase: NativeSessionPhase,
    returned: bool,
    waiting: Option<(PendingThreadWait, PreSyscallToken)>,
    waiting_service: Option<&'static str>,
}
pub(super) struct Scheduling<'plan, 'image> {
    threads: Vec<Thread<'plan, 'image>>,
    active: usize,
    maximum: usize,
    last_switch: u64,
    force_switch: bool,
    idle_nanoseconds: u64,
    external_idle_nanoseconds: u64,
}
impl<'plan, 'image> Scheduling<'plan, 'image> {
    pub(super) fn new(
        main: Option<PreparedNativeThread<'plan, 'image>>,
        maximum: usize,
    ) -> Result<Self, NativeSessionError> {
        if maximum == 0 || maximum > 4096 {
            return Err(NativeSessionError::Budget("thread contexts"));
        }
        let mut threads = Vec::new();
        threads
            .try_reserve_exact(maximum)
            .map_err(|_| NativeSessionError::Allocation)?;
        threads.push(Thread {
            binding: Binding::Main(main),
            parked: None,
            frames: Vec::new(),
            phase: NativeSessionPhase::MainEntry,
            returned: false,
            waiting: None,
            waiting_service: None,
        });
        Ok(Self {
            threads,
            active: 0,
            maximum,
            last_switch: 0,
            force_switch: false,
            idle_nanoseconds: 0,
            external_idle_nanoseconds: 0,
        })
    }
    pub(super) fn is_main(&self) -> bool {
        self.active == 0
    }
    pub(super) fn active_blocked(&self) -> bool {
        self.threads[self.active].waiting.is_some() || self.threads[self.active].returned
    }

    /// A committed flip must publish its retained notification before any
    /// guest entry. This wait uses only the independently bounded host lane.
    pub(super) fn wait_consumer_retry(
        &mut self,
        wake: super::frame_bridge::ExternalWake,
    ) -> Result<(), NativeSessionError> {
        let delay = wake.delay.min(std::time::Duration::from_millis(1));
        charge(
            &mut self.external_idle_nanoseconds,
            delay.as_nanos() as u64,
            wake.maximum_idle_nanoseconds,
            "frame idle waits",
        )?;
        std::thread::sleep(delay);
        Ok(())
    }

    pub(super) fn create(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'plan, 'image>,
        proof: &PreparedNativeSource<'_, '_>,
        engine: &mut NativeBlockStepper<'_, '_, '_>,
        pending: PendingThreadCreate,
        token: PreSyscallToken,
    ) -> Result<(), NativeSessionError> {
        if self.threads.len() >= self.maximum {
            return Err(NativeSessionError::Budget("thread contexts"));
        }
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(32)
            .map_err(|_| NativeSessionError::Allocation)?;
        let child = session.prepare_created_thread(proof, pending)?;
        let layout = *child.layout();
        let stack = NativeStackFrame {
            stack: layout.stack,
            guards: layout.guards,
            return_slot: layout.return_slot,
            return_marker: layout.return_marker,
        };
        let mut cpu = NativeCpuState::new(layout.entry, layout.return_slot);
        cpu.gpr[7] = layout.argument;
        cpu.fs_base = Some(child.thread().thread_pointer());
        cpu.mxcsr = engine.state().mxcsr;
        cpu.x87 = engine.state().x87;
        let parked = engine.prepare_thread(&mut session.pause()?, cpu, stack)?;
        if parked.index() != self.threads.len() {
            return Err(NativeSessionError::FixtureInvariant(
                "thread queue identity",
            ));
        }
        // Queue capacity and all CPU/storage admission precede guest output.
        // No allocation or fallible queue mutation follows successful publication.
        let value = session.publish_created_thread(proof, &child, &parked, token)?;
        self.threads.push(Thread {
            binding: Binding::Child(child),
            parked: Some(parked),
            frames,
            phase: NativeSessionPhase::ThreadEntry,
            returned: false,
            waiting: None,
            waiting_service: None,
        });
        engine.complete_syscall(token, value)?;
        self.force_switch = true;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn rotate(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'plan, 'image>,
        proof: &PreparedNativeSource<'_, '_>,
        engine: &mut NativeBlockStepper<'_, '_, '_>,
        frames: &mut Vec<(SuspendedInitializer, NativeNestedCall, NativeSessionPhase)>,
        phase: &mut NativeSessionPhase,
        external: Option<super::frame_bridge::ExternalWake>,
        interactive: bool,
    ) -> Result<bool, NativeSessionError> {
        let blocks = engine.usage().blocks;
        if !self.force_switch
            && blocks.saturating_sub(self.last_switch)
                < quantum(session.active_thread_priority_class())
        {
            return Ok(true);
        }
        self.force_switch = false;
        self.last_switch = blocks;
        let mut next = None;
        for offset in 1..self.threads.len() {
            let index = (self.active + offset) % self.threads.len();
            if self.threads[index].returned {
                continue;
            }
            if let Some((pending, _)) = &self.threads[index].waiting
                && !session.thread_wait_ready(pending)?
            {
                continue;
            }
            next = Some(index);
            break;
        }
        let Some(next) = next else {
            if let Some((pending, _)) = &self.threads[self.active].waiting
                && session.thread_wait_ready(pending)?
            {
                return self.resume(session, proof, engine);
            }
            if self.threads[self.active].waiting.is_some() || self.threads[self.active].returned {
                let mut earliest = None;
                for thread in &self.threads {
                    if let Some((pending, _)) = &thread.waiting
                        && let Some(delay) = session.thread_wait_delay(pending)?
                    {
                        earliest =
                            Some(earliest.map_or(delay, |old: std::time::Duration| old.min(delay)));
                    }
                }
                if let Some((sleep, external_maximum)) = idle_slice(earliest, external) {
                    // No live memory borrow or guest execution while all CPUs
                    // are parked. Accept full-width finite deadlines; only
                    // requested host-idle slices consume the sleep budget.
                    if let Some(maximum) = external_maximum {
                        charge(
                            &mut self.external_idle_nanoseconds,
                            sleep.as_nanos() as u64,
                            maximum,
                            "frame idle waits",
                        )?;
                    } else {
                        charge(
                            &mut self.idle_nanoseconds,
                            sleep.as_nanos() as u64,
                            if interactive {
                                u64::MAX
                            } else {
                                session.plan().limits().kernel.max_sleep_nanoseconds
                            },
                            "idle waits",
                        )?;
                    }
                    std::thread::sleep(sleep);
                    self.force_switch = true;
                    return Ok(false);
                }
                return Err(
                    if self.threads.iter().any(|thread| thread.waiting.is_some()) {
                        NativeSessionError::ThreadDeadlock
                    } else {
                        NativeSessionError::FixtureInvariant("no runnable guest thread")
                    },
                );
            }
            return Ok(true);
        };
        let parked =
            self.threads[next]
                .parked
                .as_mut()
                .ok_or(NativeSessionError::FixtureInvariant(
                    "missing parked thread",
                ))?;
        engine.exchange_thread(&mut session.pause()?, parked)?;
        let previous = self.threads[next].parked.take();
        self.threads[self.active].parked = previous;
        let binding = match &self.threads[next].binding {
            Binding::Main(thread) => thread.as_ref(),
            Binding::Child(thread) => Some(thread.thread()),
        }
        .ok_or(NativeSessionError::WrongPreparedSource)?;
        session.activate_thread_context(binding)?;
        std::mem::swap(frames, &mut self.threads[self.active].frames);
        std::mem::swap(frames, &mut self.threads[next].frames);
        self.threads[self.active].phase = *phase;
        *phase = self.threads[next].phase;
        self.active = next;
        self.resume(session, proof, engine)
    }

    fn resume(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'plan, 'image>,
        proof: &PreparedNativeSource<'_, '_>,
        engine: &mut NativeBlockStepper<'_, '_, '_>,
    ) -> Result<bool, NativeSessionError> {
        if let Some((pending, token)) = &self.threads[self.active].waiting {
            let Some(value) = session.complete_thread_wait(pending, *token, proof, engine)? else {
                // Opening a source drain barrier can admit the next real wait
                // without completing its syscall. Keep its exact token/CPU
                // parked and reconsider readiness on another scheduling pass.
                self.force_switch = true;
                return Ok(false);
            };
            engine.complete_syscall(*token, value)?;
            self.threads[self.active].waiting = None;
            self.threads[self.active].waiting_service = None;
        }
        Ok(true)
    }

    pub(super) fn wait(
        &mut self,
        session: &PreparedNativeKernelStartup<'plan, 'image>,
        pending: PendingThreadWait,
        token: PreSyscallToken,
    ) -> Result<(), NativeSessionError> {
        session.check_thread_wait(&pending, token)?;
        let thread = &mut self.threads[self.active];
        if thread.returned || thread.waiting.is_some() {
            return Err(NativeSessionError::FixtureInvariant("thread park phase"));
        }
        thread.waiting = Some((pending, token));
        thread.waiting_service = session.last_admitted_kernel_service();
        self.force_switch = true;
        Ok(())
    }

    pub(super) fn returned(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'plan, 'image>,
        proof: &PreparedNativeSource<'_, '_>,
        engine: &mut NativeBlockStepper<'_, '_, '_>,
    ) -> Result<(), NativeSessionError> {
        let Binding::Child(start) = &self.threads[self.active].binding else {
            return Err(NativeSessionError::FixtureInvariant(
                "main thread return dispatch",
            ));
        };
        if session.advance_created_thread_exit(proof, start, engine)? {
            self.threads[self.active].returned = true;
            self.force_switch = true;
        }
        Ok(())
    }
}
