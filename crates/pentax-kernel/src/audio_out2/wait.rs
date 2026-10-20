// SPDX-License-Identifier: GPL-2.0-only
// KytyPS5 f186b357 audio.cpp/libAudio2.cpp; bounded cooperative headless port.
use super::state::*;
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);
/// Opaque process-owned pending audio operation; not execution authority.
pub struct AudioOut2Wait {
    serial: u64,
    thread: u32,
}
impl std::fmt::Debug for AudioOut2Wait {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioOut2Wait").finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub enum AudioOut2WaitAction {
    Returned(u64),
    Waiting(AudioOut2Wait),
}
pub(super) enum Outcome {
    Returned(u64),
    Wait(PendingKind),
}

pub(super) fn is_stateful(service: Service) -> bool {
    use Service::*;
    matches!(
        service,
        AudioOut2ContextCreate
            | AudioOut2ContextDestroy
            | AudioOut2ContextAdvance
            | AudioOut2ContextPush
            | AudioOut2ContextGetQueueLevel
            | AudioOut2UserCreate
            | AudioOut2UserDestroy
            | AudioOut2PortCreate
            | AudioOut2PortDestroy
            | AudioOut2PortGetState
            | AudioOut2PortSetAttributes
    )
}
fn early(service: Service, a: [u64; 6]) -> Result<Option<u64>, Failure> {
    use Service::*;
    match service {
        AudioOut2ContextCreate if a[0] == 0 => {
            Err(Failure::Unsupported("null AudioOut2 context parameter"))
        }
        AudioOut2ContextCreate if a[3] == 0 => {
            Err(Failure::Unsupported("null AudioOut2 context output"))
        }
        AudioOut2PortCreate if a[1] == 0 => {
            Err(Failure::Unsupported("null AudioOut2 port parameter"))
        }
        AudioOut2PortCreate if a[2] == 0 => Err(Failure::Unsupported("null AudioOut2 port output")),
        AudioOut2PortGetState if a[1] == 0 => {
            Err(Failure::Unsupported("null AudioOut2 port state"))
        }
        AudioOut2UserCreate if a[1] == 0 => Ok(Some(INVALID)),
        AudioOut2PortSetAttributes if a[2] as u32 != 0 && a[1] == 0 => Ok(Some(INVALID)),
        _ => Ok(None),
    }
}
impl AudioState {
    pub(super) fn operation<G: GuestAccess>(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
        mut clock: impl FnMut(&G) -> Result<u64, Failure>,
        allow_wait: bool,
    ) -> Result<Outcome, Failure> {
        if !is_stateful(service) {
            return Err(Failure::Unsupported("AudioOut2 wait service"));
        }
        if let Some(value) = early(service, args.0)? {
            return Ok(Outcome::Returned(value));
        }
        self.ensure_initialized(budget)?;
        self.charge(budget)?;
        if self.lease.is_some() {
            return if allow_wait {
                Ok(Outcome::Wait(PendingKind::Deferred))
            } else {
                Err(Failure::Unsupported(
                    "AudioOut2 operation needs continuation",
                ))
            };
        }
        if service != Service::AudioOut2ContextPush {
            return self
                .lifecycle_with(service, args, guest, budget, clock)
                .map(Outcome::Returned);
        }
        let blocking = args.0[1] as u32 != 0;
        let Some(index) = self.context(args.0[0]) else {
            return if blocking {
                Err(Failure::Unsupported("blocking unknown AudioOut2 context"))
            } else {
                Ok(Outcome::Returned(NOT_READY))
            };
        };
        let now = clock(guest)?;
        let context = self.contexts[index].as_ref().expect("owned context");
        let mut queue = context.queue;
        queue.update(now);
        if queue.queued >= queue.depth {
            if !blocking {
                self.contexts[index].as_mut().expect("owned context").queue = queue;
                return Ok(Outcome::Returned(NOT_READY));
            }
            if !allow_wait {
                return Err(Failure::Unsupported("AudioOut2 queue needs continuation"));
            }
            let deadline = now
                .checked_add(queue.grain_us())
                .ok_or(Failure::Limit("audio deadline"))?;
            self.contexts[index].as_mut().expect("owned context").queue = queue;
            return Ok(Outcome::Wait(PendingKind::Retry(deadline)));
        }
        if queue.queued == 0 {
            queue.last = clock(guest)?;
        }
        queue.queued += 1;
        let (mix, batch, frames) = self.mix(index, budget)?;
        let mut sinks = self.sinks;
        let mut deadline = now;
        let has_batch = batch.iter().any(Option::is_some);
        for sink in batch.into_iter().flatten() {
            let s = sinks[sink]
                .as_mut()
                .ok_or(Failure::Unsupported("stale AudioOut2 sink"))?;
            deadline = deadline.max(s.submit(now, blocking)?);
        }
        let pending = blocking && deadline > now;
        // Raw invocation must refuse BEFORE queue/mix/clocks/counters change.
        if pending && !allow_wait {
            return Err(Failure::Unsupported("AudioOut2 pacing needs continuation"));
        }
        let usage = AudioOut2Usage {
            mix_frames: self
                .usage
                .mix_frames
                .checked_add(frames)
                .ok_or(Failure::Limit("audio mix counter"))?,
            logical_output_batches: self
                .usage
                .logical_output_batches
                .checked_add(u64::from(has_batch))
                .ok_or(Failure::Limit("audio batch counter"))?,
        };
        let context = self.contexts[index].as_mut().expect("owned context");
        context.queue = queue;
        context.mix = mix;
        self.sinks = sinks;
        self.usage = usage;
        if pending {
            Ok(Outcome::Wait(PendingKind::Paced(deadline)))
        } else {
            Ok(Outcome::Returned(0))
        }
    }
    fn ticket(&self, ticket: &AudioOut2Wait, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.pending.len() as u64 + 1)?;
        self.pending
            .iter()
            .position(|p| p.serial == ticket.serial && p.thread == ticket.thread)
            .ok_or(Failure::Unsupported("unowned AudioOut2 wait"))
    }
    fn admit_wait(&self) -> Result<u64, Failure> {
        if self.pending.len() >= self.max_waiters {
            return Err(Failure::Limit("audio waiters"));
        }
        NEXT_WAIT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Failure::Limit("audio wait identities"))
    }
}
impl crate::Kernel {
    #[must_use]
    pub const fn audio_out2_usage(&self) -> AudioOut2Usage {
        self.audio_out2.usage
    }

    pub fn begin_audio_out2_wait(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<AudioOut2WaitAction, Failure> {
        if !is_stateful(service) {
            return Err(Failure::Unsupported("AudioOut2 wait service"));
        }
        self.budget.attempt()?;
        guest.preflight_pthread()?;
        if self.threads.active().waiting {
            return Err(Failure::Unsupported("thread already waiting"));
        }
        if let Some(value) = early(service, args.0)? {
            return Ok(AudioOut2WaitAction::Returned(value));
        }
        self.audio_out2.ensure_initialized(&mut self.budget)?;
        // Reserve every possible late continuation before operation effects.
        let serial = self.audio_out2.admit_wait()?;
        let thread = self.threads.active().id;
        let result = self.audio_out2.operation(
            service,
            args,
            guest,
            &mut self.budget,
            |g| self.time.process_clock(g).map(|c| c.microseconds),
            true,
        )?;
        self.budget
            .retained_audio_bytes(self.audio_out2.pcm_bytes());
        match result {
            Outcome::Returned(value) => Ok(AudioOut2WaitAction::Returned(value)),
            Outcome::Wait(kind) => {
                if matches!(kind, PendingKind::Paced(_)) {
                    self.audio_out2.lease = Some(serial);
                }
                self.audio_out2.pending.push(Pending {
                    serial,
                    thread,
                    service,
                    args,
                    kind,
                });
                self.threads.active_mut().waiting = true;
                Ok(AudioOut2WaitAction::Waiting(AudioOut2Wait {
                    serial,
                    thread,
                }))
            }
        }
    }
    pub fn audio_out2_wait_ready(
        &mut self,
        ticket: &AudioOut2Wait,
        guest: &impl GuestAccess,
    ) -> Result<bool, Failure> {
        let i = self.audio_out2.ticket(ticket, &mut self.budget)?;
        match self.audio_out2.pending[i].kind {
            PendingKind::Deferred => Ok(self.audio_out2.lease.is_none()),
            PendingKind::Retry(deadline) => {
                if self.audio_out2.lease.is_some() {
                    return Ok(false);
                }
                Ok(self.time.process_clock(guest)?.microseconds >= deadline)
            }
            PendingKind::Paced(deadline) => {
                Ok(self.time.process_clock(guest)?.microseconds >= deadline)
            }
        }
    }
    pub fn audio_out2_wait_delay(
        &mut self,
        ticket: &AudioOut2Wait,
        guest: &impl GuestAccess,
    ) -> Result<Option<Duration>, Failure> {
        let i = self.audio_out2.ticket(ticket, &mut self.budget)?;
        let deadline = match self.audio_out2.pending[i].kind {
            PendingKind::Deferred => return Ok(None),
            PendingKind::Retry(t) if self.audio_out2.lease.is_none() => t,
            PendingKind::Retry(_) => return Ok(None),
            PendingKind::Paced(t) => t,
        };
        let now = self.time.process_clock(guest)?.microseconds;
        Ok(Some(Duration::from_micros(deadline.saturating_sub(now))))
    }
    pub fn complete_audio_out2_wait(
        &mut self,
        ticket: &AudioOut2Wait,
        guest: &mut impl GuestAccess,
    ) -> Result<Option<u64>, Failure> {
        guest.preflight_pthread()?;
        if self.threads.active().id != ticket.thread || !self.threads.active().waiting {
            return Err(Failure::Unsupported("AudioOut2 wait thread"));
        }
        let i = self.audio_out2.ticket(ticket, &mut self.budget)?;
        self.audio_out2.charge(&mut self.budget)?;
        if !self.audio_out2_wait_ready(ticket, guest)? {
            return Ok(None);
        }
        let (service, args, kind) = {
            let p = &self.audio_out2.pending[i];
            (p.service, p.args, p.kind)
        };
        let outcome = if matches!(kind, PendingKind::Paced(_)) {
            if self.audio_out2.lease != Some(ticket.serial) {
                return Err(Failure::Unsupported("AudioOut2 lease owner"));
            }
            self.audio_out2.lease = None;
            Outcome::Returned(0)
        } else {
            self.audio_out2.operation(
                service,
                args,
                guest,
                &mut self.budget,
                |g| self.time.process_clock(g).map(|c| c.microseconds),
                true,
            )?
        };
        self.budget
            .retained_audio_bytes(self.audio_out2.pcm_bytes());
        match outcome {
            Outcome::Returned(value) => {
                self.audio_out2.pending.remove(i);
                self.threads.active_mut().waiting = false;
                Ok(Some(value))
            }
            Outcome::Wait(kind) => {
                if matches!(kind, PendingKind::Paced(_)) {
                    self.audio_out2.lease = Some(ticket.serial);
                }
                self.audio_out2.pending[i].kind = kind;
                Ok(None)
            }
        }
    }
    pub(crate) fn invoke_audio_out2(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        let outcome = self.audio_out2.operation(
            service,
            args,
            guest,
            &mut self.budget,
            |g| self.time.process_clock(g).map(|c| c.microseconds),
            false,
        )?;
        self.budget
            .retained_audio_bytes(self.audio_out2.pcm_bytes());
        match outcome {
            Outcome::Returned(value) => Ok(value),
            Outcome::Wait(_) => unreachable!("raw audio preflight"),
        }
    }
}
