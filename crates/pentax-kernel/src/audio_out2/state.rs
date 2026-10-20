// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5 f186b357 src/libs/libAudio2.cpp and audio.cpp.
// Modified for PentaX: owned bounded headless sinks and cooperative pacing.
use super::*;
use std::mem::size_of;

pub(super) const CONTEXTS: usize = 16;
pub(super) const PORTS: usize = 256;
pub(super) const SINKS: usize = 32;
pub(super) const MAX_GRAINS: u32 = 4096;
pub(super) const MAX_PCM: usize = 16 * 1024 * 1024;
pub(super) const INVALID: u64 = 0x8026_8001;
pub(super) const RESOURCE: u64 = 0x8026_8002;
pub(super) const BUSY: u64 = 0x8026_8007;
pub(super) const NOT_READY: u64 = 0x8026_8008;
pub(super) const INVALID_PORT: u64 = 0x8026_8009;
pub(super) const PORT_FULL: u64 = 0x8026_8012;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AudioOut2Usage {
    /// Actual committed object-port sample frames evaluated by the CPU mixer.
    pub mix_frames: u64,
    /// Nonempty accepted logical sink batches, never physical playback.
    pub logical_output_batches: u64,
}
#[derive(Clone, Copy, Default)]
pub(super) struct Queue {
    pub depth: u32,
    pub queued: u32,
    pub grains: u32,
    pub last: u64,
}
impl Queue {
    pub fn grain_us(self) -> u64 {
        u64::from((self.grains * 1_000_000 / 48_000).max(1000))
    }
    pub fn update(&mut self, now: u64) {
        if self.last == 0 || self.queued == 0 {
            self.last = now;
        } else if now > self.last {
            let drained = ((now - self.last) / self.grain_us()).min(u64::from(self.queued)) as u32;
            self.queued -= drained;
            self.last += u64::from(drained) * self.grain_us();
            if self.queued == 0 {
                self.last = now;
            }
        }
    }
}
pub(super) struct Context {
    pub id: u64,
    pub queue: Queue,
    pub sink: Option<usize>,
    pub mix: Vec<u8>,
}
pub(super) struct Port {
    pub id: u64,
    pub context: u64,
    pub user: u64,
    pub kind: u16,
    pub format: u32,
    pub grains: u32,
    pub sink: Option<usize>,
    pub gains: [f32; 16],
    pub ambisonics: u32,
    pub locked: bool,
    pub pcm: Vec<u8>,
}
impl Port {
    pub fn object(&self) -> bool {
        self.kind & 0xff00 == 0x100
    }
}
#[derive(Clone, Copy, Default)]
pub(super) struct Sink {
    pub frames: u32,
    pub rate: u32,
    pub next: u64,
    pub remainder: u64,
}
impl Sink {
    pub fn submit(&mut self, now: u64, blocking: bool) -> Result<u64, Failure> {
        if !blocking {
            self.next = 0;
            self.remainder = 0;
            return Ok(now);
        }
        if self.next == 0 || (now > self.next && now - self.next > 100_000) {
            self.next = now;
            self.remainder = 0;
        }
        let deadline = self.next;
        let duration = 1_000_000u64
            .checked_mul(u64::from(self.frames))
            .and_then(|n| n.checked_add(self.remainder))
            .ok_or(Failure::Limit("audio duration"))?;
        self.next = self
            .next
            .checked_add(duration / u64::from(self.rate))
            .ok_or(Failure::Limit("audio deadline"))?;
        self.remainder = duration % u64::from(self.rate);
        Ok(deadline)
    }
}
#[derive(Clone, Copy)]
pub(super) enum PendingKind {
    Deferred,
    Retry(u64),
    Paced(u64),
}
pub(super) struct Pending {
    pub serial: u64,
    pub thread: u32,
    pub service: Service,
    pub args: CallArgs,
    pub kind: PendingKind,
}
pub(crate) struct AudioState {
    pub(super) contexts: Vec<Option<Context>>,
    pub(super) ports: Vec<Option<Port>>,
    pub(super) users: Vec<u64>,
    pub(super) sinks: [Option<Sink>; SINKS],
    pub(super) pending: Vec<Pending>,
    pub(super) next_context: u64,
    pub(super) next_port: u64,
    pub(super) next_user: u64,
    pub(super) lease: Option<u64>,
    pub(super) usage: AudioOut2Usage,
    pub(super) max_waiters: usize,
}
impl Default for AudioState {
    fn default() -> Self {
        Self {
            contexts: Vec::new(),
            ports: Vec::new(),
            users: Vec::new(),
            sinks: [None; SINKS],
            pending: Vec::new(),
            next_context: 1,
            next_port: 1,
            next_user: 1,
            lease: None,
            usage: AudioOut2Usage::default(),
            max_waiters: 0,
        }
    }
}
fn reserve<T>(count: usize, held: &mut usize, budget: &mut Budget) -> Result<Vec<T>, Failure> {
    let requested = count
        .checked_mul(size_of::<T>())
        .and_then(|n| n.checked_add(*held))
        .ok_or(Failure::Limit("audio metadata"))?;
    budget.scratch(requested)?;
    let mut v = Vec::new();
    v.try_reserve_exact(count)
        .map_err(|_| Failure::Allocation)?;
    *held = v
        .capacity()
        .checked_mul(size_of::<T>())
        .and_then(|n| n.checked_add(*held))
        .ok_or(Failure::Limit("audio metadata"))?;
    budget.scratch(*held)?;
    budget.record_scratch(*held);
    Ok(v)
}
impl AudioState {
    pub(super) fn ensure_initialized(&mut self, budget: &mut Budget) -> Result<(), Failure> {
        if !self.contexts.is_empty() {
            return Ok(());
        }
        // Allocate lazily so unrelated services with deliberately tiny scratch
        // policies do not acquire an audio metadata dependency.
        let estimate = [
            (CONTEXTS, size_of::<Option<Context>>()),
            (PORTS, size_of::<Option<Port>>()),
            (budget.limits.max_handles.min(4096), size_of::<u64>()),
            (budget.limits.max_thread_contexts, size_of::<Pending>()),
        ]
        .into_iter()
        .try_fold(0usize, |total, (count, width)| {
            count.checked_mul(width).and_then(|n| total.checked_add(n))
        })
        .ok_or(Failure::Limit("audio metadata"))?;
        budget.scratch(estimate)?;
        budget.work(estimate as u64)?;
        let mut held = 0;
        let mut state = Self {
            contexts: reserve(CONTEXTS, &mut held, budget)?,
            ports: reserve(PORTS, &mut held, budget)?,
            users: reserve(budget.limits.max_handles.min(4096), &mut held, budget)?,
            sinks: [None; SINKS],
            pending: reserve(budget.limits.max_thread_contexts, &mut held, budget)?,
            next_context: 1,
            next_port: 1,
            next_user: 1,
            lease: None,
            usage: AudioOut2Usage::default(),
            max_waiters: budget.limits.max_thread_contexts,
        };
        state.contexts.resize_with(CONTEXTS, || None);
        state.ports.resize_with(PORTS, || None);
        budget.scratch(state.storage())?;
        budget.record_scratch(state.storage());
        *self = state;
        Ok(())
    }
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.contexts.iter().flatten().count()
            + self.ports.iter().flatten().count()
            + self.users.len()
    }
    pub(super) fn pcm_bytes(&self) -> usize {
        self.contexts
            .iter()
            .flatten()
            .map(|c| c.mix.capacity())
            .sum::<usize>()
            + self
                .ports
                .iter()
                .flatten()
                .map(|p| p.pcm.capacity())
                .sum::<usize>()
    }
    pub(super) fn storage(&self) -> usize {
        self.contexts.capacity() * size_of::<Option<Context>>()
            + self.ports.capacity() * size_of::<Option<Port>>()
            + self.users.capacity() * size_of::<u64>()
            + self.pending.capacity() * size_of::<Pending>()
            + self.pcm_bytes()
    }
    pub(super) fn admit_payload(&self, old: usize, new: usize) -> Result<(), Failure> {
        if self
            .pcm_bytes()
            .checked_sub(old)
            .and_then(|n| n.checked_add(new))
            .is_none_or(|n| n > MAX_PCM)
        {
            Err(Failure::Limit("audio retained PCM"))
        } else {
            Ok(())
        }
    }
    pub(super) fn charge(&self, budget: &mut Budget) -> Result<(), Failure> {
        budget
            .work((CONTEXTS + PORTS + SINKS + self.users.len() + self.pending.len() + 32) as u64)?;
        budget.scratch(self.storage())
    }
    pub(super) fn context(&self, id: u64) -> Option<usize> {
        self.contexts
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.id == id))
    }
    pub(super) fn port(&self, id: u64) -> Option<usize> {
        self.ports
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.id == id))
    }
    pub(super) fn free_sink(&self) -> Option<usize> {
        self.sinks.iter().position(Option::is_none)
    }
    pub(super) fn destroy_port(&mut self, index: usize, budget: &mut Budget) {
        if let Some(p) = self.ports[index].take() {
            if let Some(s) = p.sink {
                self.sinks[s] = None;
            }
            budget.removed_handle();
        }
    }
}
pub(super) fn next(id: u64) -> Result<u64, Failure> {
    id.checked_add(1)
        .ok_or(Failure::Limit("audio handle identities"))
}
pub(super) fn at(address: u64, offset: u64) -> Result<u64, Failure> {
    address
        .checked_add(offset)
        .ok_or(GuestError::AddressOverflow.into())
}
pub(super) fn channels(format: u32) -> usize {
    let n = (format >> 8) & 255;
    if n == 0 { 2 } else { n.min(16) as usize }
}
pub(super) fn recognized(format: u32) -> bool {
    match format & 127 {
        0 => matches!(channels(format), 1 | 2 | 8) || (channels(format) == 12 && format & 128 == 0),
        1 => matches!(channels(format), 1 | 2 | 8),
        _ => false,
    }
}
