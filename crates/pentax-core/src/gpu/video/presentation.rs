// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owner-bound flip snapshots and receipts.
// Adapted from pinned videoOut.cpp FlipQueue; see NOTICE.

use super::pixels::{PresentationLimits, snapshot};
use super::{Failure, Surface, index};
use pentax_platform::native_memory::{NativeOwnerId, PausedNativeMemory};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipSource {
    Cpu,
    GpuEop,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlipTicket {
    id: u64,
    owner: NativeOwnerId,
    handle: i32,
    generation: u64,
}
impl FlipTicket {
    pub fn handle(self) -> i32 {
        self.handle
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipSubmission {
    Accepted(FlipTicket),
    Rejected(u64),
}

/// Owned real surface pixels. Hand-off transfers its byte budget to the caller;
/// a worker/UI mailbox must independently bound outstanding frame capacities.
pub struct Frame {
    ticket: FlipTicket,
    surface: Surface,
    pixels: Vec<u8>,
    presentation_tick: u64,
}
/// Oldest READY payload metadata only: no pixels, ticket, address or handoff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadyFrameMetadata {
    width: u32,
    height: u32,
    format: super::PixelFormat,
    premultiplied_alpha: bool,
    capacity: usize,
    generation: u64,
    handle: i32,
}
impl ReadyFrameMetadata {
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
    pub fn source_format(self) -> super::PixelFormat {
        self.format
    }
    pub fn premultiplied_alpha(self) -> bool {
        self.premultiplied_alpha
    }
    pub fn capacity(self) -> usize {
        self.capacity
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn handle(self) -> i32 {
        self.handle
    }
}
impl Frame {
    pub fn ticket(&self) -> FlipTicket {
        self.ticket
    }
    pub fn width(&self) -> u32 {
        self.surface.width
    }
    pub fn height(&self) -> u32 {
        self.surface.height
    }
    pub fn rgba8(&self) -> &[u8] {
        &self.pixels
    }
    pub fn source_format(&self) -> super::PixelFormat {
        self.surface.format
    }
    pub fn premultiplied_alpha(&self) -> bool {
        self.surface.options & 32 != 0
    }
    pub fn capacity(&self) -> usize {
        self.pixels.capacity()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlipStatus {
    pub count: u64,
    pub process_time: u64,
    pub flip_arg: i64,
    pub process_counter: u64,
    pub gpu_pending: i32,
    pub pending: i32,
    pub current_buffer: i32,
    pub submit_counter: u64,
}
impl Default for FlipStatus {
    fn default() -> Self {
        Self {
            count: 0,
            process_time: 0,
            flip_arg: -1,
            process_counter: 0,
            gpu_pending: 0,
            pending: 0,
            current_buffer: -1,
            submit_counter: 0,
        }
    }
}
impl FlipStatus {
    pub fn bytes(self) -> [u8; 128] {
        let mut bytes = [0; 128];
        for (offset, value) in [
            (0, self.count),
            (8, self.process_time),
            (24, self.flip_arg as u64),
            (40, self.process_counter),
            (64, self.submit_counter),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (48, self.gpu_pending),
            (52, self.pending),
            (56, self.current_buffer),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
/// Event data is published only after the consumer confirms presentation.
/// Root integration must deliver this through retained real equeue owners.
#[derive(Debug, PartialEq, Eq)]
pub struct FlipCompletion {
    owner: NativeOwnerId,
    handle: i32,
    generation: u64,
    flip_arg: i64,
    count: u64,
    buffer: i32,
}
impl FlipCompletion {
    pub fn owner(&self) -> NativeOwnerId {
        self.owner
    }
    pub fn handle(&self) -> i32 {
        self.handle
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn flip_arg(&self) -> i64 {
        self.flip_arg
    }
    pub fn count(&self) -> u64 {
        self.count
    }
    pub fn buffer(&self) -> i32 {
        self.buffer
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reserved,
    Recording,
    Ready,
    Presenting,
}
struct Request {
    ticket: FlipTicket,
    buffer: i32,
    flip_arg: i64,
    submit_counter: u64,
    source: FlipSource,
    phase: Phase,
    frame: Option<Frame>,
}
pub(super) struct Queue {
    owner: Option<NativeOwnerId>,
    requests: [Option<Request>; 48],
    statuses: [FlipStatus; 3],
    pub(super) held: usize,
    work: u64,
    last_tick: [u64; 3],
}
impl Default for Queue {
    fn default() -> Self {
        Self {
            owner: None,
            requests: std::array::from_fn(|_| None),
            statuses: [FlipStatus::default(); 3],
            held: 0,
            work: 0,
            last_tick: [0; 3],
        }
    }
}
impl Queue {
    pub(super) fn bind_owner(&mut self, owner: NativeOwnerId) -> Result<(), Failure> {
        if self.owner.is_some_and(|expected| expected != owner) {
            return Err(Failure::Memory);
        }
        self.owner = Some(owner);
        Ok(())
    }
    fn position(&self, ticket: FlipTicket) -> Result<usize, Failure> {
        if self.owner != Some(ticket.owner) {
            return Err(Failure::Memory);
        }
        self.requests
            .iter()
            .position(|r| r.as_ref().is_some_and(|r| r.ticket == ticket))
            .ok_or(Failure::UnsupportedVideo)
    }
    pub(super) fn reset(&mut self, slot: usize) {
        self.statuses[slot] = FlipStatus::default();
        self.last_tick[slot] = 0;
    }
}
impl super::State {
    /// Preserve FIFO: a later ready request cannot bypass an older incomplete
    /// or handed-off request. Does not observe ticks or mutate queue state.
    pub(crate) fn peek_ready_frame(&self) -> Result<Option<ReadyFrameMetadata>, Failure> {
        let request = self
            .presentation
            .requests
            .iter()
            .flatten()
            .min_by_key(|r| r.ticket.id);
        let Some(request) = request else {
            return Ok(None);
        };
        if request.phase != Phase::Ready {
            return Ok(None);
        }
        let frame = request.frame.as_ref().ok_or(Failure::UnsupportedVideo)?;
        if self
            .port(request.ticket.handle)
            .is_none_or(|p| p.generation != request.ticket.generation)
        {
            return Err(Failure::UnsupportedVideo);
        }
        Ok(Some(ReadyFrameMetadata {
            width: frame.width(),
            height: frame.height(),
            format: frame.source_format(),
            premultiplied_alpha: frame.premultiplied_alpha(),
            capacity: frame.capacity(),
            generation: request.ticket.generation,
            handle: request.ticket.handle,
        }))
    }
    pub(in crate::gpu) fn retained_frame_bytes(&self) -> usize {
        self.presentation.held
    }
    pub(in crate::gpu) fn admit_gpu_capture_budget(
        &self,
        work: u64,
        limits: &PresentationLimits,
    ) -> Result<(), Failure> {
        self.presentation
            .work
            .checked_add(work)
            .filter(|n| *n <= limits.max_work)
            .map(|_| ())
            .ok_or(Failure::Limit)
    }

    /// Metadata only, not readiness or execution authority. Called for the
    /// whole ordered stream before its first possible GPU side effect.
    pub(in crate::gpu) fn gpu_capture_surface(
        &self,
        flip: pentax_gpu::graphics::Flip,
    ) -> Result<Surface, Failure> {
        let port = self.port(flip.handle()).ok_or(Failure::UnsupportedVideo)?;
        if !matches!(flip.mode(), 1..=4) || port.master_handle.is_some() || port.slave_count != 0 {
            return Err(Failure::UnsupportedVideo);
        }
        let surface = self
            .surface(flip.handle(), flip.buffer())
            .ok_or(Failure::UnsupportedVideo)?;
        if surface.compression != super::Compression::None
            || surface.metadata.is_some()
            || surface.bytes_per_pixel != 4
            || surface.format != super::PixelFormat::Rgba8Srgb
            || surface.options != 0
            || surface.dcc_control != 0
        {
            return Err(Failure::UnsupportedVideo);
        }
        Ok(surface)
    }
    pub(in crate::gpu) fn gpu_pending(&self, handle: i32) -> Result<usize, Failure> {
        let slot = index(handle)
            .filter(|i| self.ports[*i].open)
            .ok_or(Failure::UnsupportedVideo)?;
        usize::try_from(self.presentation.statuses[slot].pending)
            .map_err(|_| Failure::UnsupportedVideo)
    }

    /// Consume genuine current draw authority while its unique pause loan is
    /// still held. No API accepts a public old Report, numeric serial or tag.
    pub(in crate::gpu) fn submit_gpu_flip<'borrow, 'owner>(
        &mut self,
        current: crate::gpu::graphics::CurrentDraw<'borrow, 'owner>,
        flip: pentax_gpu::graphics::Flip,
        limits: &PresentationLimits,
        clock: impl FnOnce(&mut PausedNativeMemory<'owner>) -> Result<u64, Failure>,
    ) -> (&'borrow mut PausedNativeMemory<'owner>, Result<(), Failure>) {
        current.consume(|memory, completed| {
            self.check_owner(memory.owner_id())?;
            let surface = self.gpu_capture_surface(flip)?;
            let geometry = completed.geometry();
            if completed.owner() != memory.owner_id()
                || completed.layout() != crate::gpu::graphics::TargetLayout::RenderTarget64KbRgba8
                || surface.data != completed.target()
                || surface.width != geometry.width()
                || surface.height != geometry.height()
                || surface.pitch != geometry.pitch()
                || surface.data.size != geometry.guest_bytes() as u64
            {
                return Err(Failure::UnsupportedVideo);
            }
            let slot = index(flip.handle()).ok_or(Failure::UnsupportedVideo)?;
            let previous_status = self.presentation.statuses[slot];
            let previous_owner = self.presentation.owner;
            let ticket = match self.reserve_flip_with_clock(
                flip.handle(),
                flip.buffer(),
                flip.mode(),
                flip.argument(),
                FlipSource::GpuEop,
                memory.owner_id(),
                || clock(memory),
            )? {
                FlipSubmission::Accepted(ticket) => ticket,
                FlipSubmission::Rejected(0x8029_0012) => return Err(Failure::Waiting),
                FlipSubmission::Rejected(_) => return Err(Failure::UnsupportedVideo),
            };
            let result = (|| {
                self.prepare_flip(ticket, memory, limits)?;
                let position = self.presentation.position(ticket)?;
                let request = self.presentation.requests[position].as_mut().unwrap();
                if request.phase != Phase::Recording
                    || request.source != FlipSource::GpuEop
                    || request
                        .frame
                        .as_ref()
                        .is_none_or(|frame| frame.surface != surface)
                    || self.ports[slot].state.generation != ticket.generation
                {
                    return Err(Failure::UnsupportedVideo);
                }
                // Fence and full retiled writeback preceded CurrentDraw. This
                // changes GPU readiness only, never presentation.
                request.phase = Phase::Ready;
                Ok(())
            })();
            if result.is_err() {
                // Roll back only this failed reservation/capture. Earlier
                // ready frames and the retired draw remain intact; the driver
                // remains terminal after those irreversible GPU effects.
                if let Some(position) = self
                    .presentation
                    .requests
                    .iter()
                    .position(|r| r.as_ref().is_some_and(|r| r.ticket == ticket))
                    && let Some(request) = self.presentation.requests[position].take()
                    && let Some(frame) = request.frame
                {
                    self.presentation.held -= frame.capacity();
                }
                self.presentation.statuses[slot] = previous_status;
                self.presentation.owner = previous_owner;
            }
            result
        })
    }
    /// Read-only ownership attestation; queries never bind an unowned state.
    /// Fresh state may report source null/invalid-handle errors before any
    /// video operation has selected an owner. Initialized foreign state refuses.
    pub(crate) fn check_owner(&self, owner: NativeOwnerId) -> Result<(), Failure> {
        if self
            .presentation
            .owner
            .is_some_and(|expected| expected != owner)
        {
            Err(Failure::Memory)
        } else {
            Ok(())
        }
    }
    pub(crate) fn flip_status(&self, handle: i32) -> Option<FlipStatus> {
        let slot = index(handle)?;
        self.ports[slot]
            .open
            .then_some(self.presentation.statuses[slot])
    }
    // Four source ABI words plus source/clock/owner context remain explicit at
    // this private admission boundary; no guest pointer or host callback enters.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reserve_flip(
        &mut self,
        handle: i32,
        buffer: i32,
        mode: i32,
        flip_arg: i64,
        source: FlipSource,
        submit_counter: u64,
        owner: NativeOwnerId,
    ) -> Result<FlipSubmission, Failure> {
        self.reserve_flip_with_clock(handle, buffer, mode, flip_arg, source, owner, || {
            Ok(submit_counter)
        })
    }
    /// Source rejection ordering is complete before the clock may observe or
    /// initialize its process epoch. All publication after a successful sample
    /// is infallible: the fixed slot and unique nonce are already admitted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reserve_flip_with_clock(
        &mut self,
        handle: i32,
        buffer: i32,
        mode: i32,
        flip_arg: i64,
        source: FlipSource,
        owner: NativeOwnerId,
        clock: impl FnOnce() -> Result<u64, Failure>,
    ) -> Result<FlipSubmission, Failure> {
        let reject = |code| Ok(FlipSubmission::Rejected(code));
        let Some(slot) = index(handle).filter(|slot| self.ports[*slot].open) else {
            return reject(0x8029_000b);
        };
        if !matches!(mode, 1..=4 | 8 | 9) {
            return reject(0x8029_0006);
        }
        if !(-2..16).contains(&buffer) {
            return reject(0x8029_000a);
        }
        let state = self.ports[slot].state;
        if state.master_handle.is_some() != (mode == 8) || (state.slave_count != 0) != (mode == 9) {
            return reject(0x8029_0006);
        }
        if buffer >= 0 && self.ports[slot].buffers.surface(buffer).is_none() {
            return reject(0x8029_000a);
        }
        // Atomic multi-port compositing and explicit black/blank modes are not
        // interpreted as ordinary guest pixels or silently flattened layers.
        if state.master_handle.is_some() || state.slave_count != 0 || buffer < 0 {
            return Err(Failure::UnsupportedVideo);
        }
        if self
            .presentation
            .owner
            .is_some_and(|expected| expected != owner)
        {
            return Err(Failure::Memory);
        }
        if self.presentation.statuses[slot].pending >= 16 {
            return reject(0x8029_0012);
        }
        let position = self
            .presentation
            .requests
            .iter()
            .position(Option::is_none)
            .ok_or(Failure::Limit)?;
        let id = NEXT_REQUEST
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| Failure::Limit)?;
        let ticket = FlipTicket {
            id,
            owner,
            handle,
            generation: state.generation,
        };
        let submit_counter = clock()?;
        self.presentation.requests[position] = Some(Request {
            ticket,
            buffer,
            flip_arg,
            submit_counter,
            source,
            phase: Phase::Reserved,
            frame: None,
        });
        self.presentation.owner = Some(owner);
        let status = &mut self.presentation.statuses[slot];
        status.pending += 1;
        status.gpu_pending += i32::from(source == FlipSource::GpuEop);
        status.submit_counter = submit_counter;
        Ok(FlipSubmission::Accepted(ticket))
    }
    /// Snapshot only while the actual owner is paused. No completion is inferred
    /// from a numeric address, buffer index, source tag or successful reservation.
    pub(crate) fn prepare_flip(
        &mut self,
        ticket: FlipTicket,
        memory: &PausedNativeMemory<'_>,
        limits: &PresentationLimits,
    ) -> Result<(), Failure> {
        let position = self.presentation.position(ticket)?;
        if memory.owner_id() != ticket.owner {
            return Err(Failure::Memory);
        }
        let request = self.presentation.requests[position].as_ref().unwrap();
        if request.phase != Phase::Reserved
            || self
                .port(ticket.handle)
                .is_none_or(|p| p.generation != ticket.generation)
        {
            return Err(Failure::UnsupportedVideo);
        }
        let surface = self
            .surface(ticket.handle, request.buffer)
            .ok_or(Failure::UnsupportedVideo)?;
        let pixels = snapshot(
            surface,
            memory,
            self.presentation.held,
            limits,
            &mut self.presentation.work,
        )?;
        let held = self
            .presentation
            .held
            .checked_add(pixels.capacity())
            .ok_or(Failure::Limit)?;
        let request = self.presentation.requests[position].as_mut().unwrap();
        request.frame = Some(Frame {
            ticket,
            surface,
            pixels,
            presentation_tick: 0,
        });
        request.phase = Phase::Recording;
        self.presentation.held = held;
        Ok(())
    }
    /// CPU preparation completed by the exclusive synchronous snapshot. GPU-EOP
    /// remains pending until an actual opaque fenced-submit receipt is integrated;
    /// a request source tag is deliberately insufficient authority.
    pub(crate) fn complete_flip(&mut self, ticket: FlipTicket) -> Result<(), Failure> {
        let position = self.presentation.position(ticket)?;
        let request = self.presentation.requests[position].as_mut().unwrap();
        if request.phase != Phase::Recording
            || request.frame.is_none()
            || request.source != FlipSource::Cpu
        {
            return Err(Failure::UnsupportedVideo);
        }
        request.phase = Phase::Ready;
        Ok(())
    }
    /// Only the oldest pending request may be handed off. No flip status or
    /// event advances when pixels merely become available to the transport.
    pub(crate) fn take_frame(&mut self, vblank_count: u64) -> Result<Option<Frame>, Failure> {
        let position = self
            .presentation
            .requests
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.as_ref().map(|r| (i, r.ticket.id)))
            .min_by_key(|(_, id)| *id)
            .map(|(i, _)| i);
        let Some(position) = position else {
            return Ok(None);
        };
        let handle = self.presentation.requests[position]
            .as_ref()
            .unwrap()
            .ticket
            .handle;
        let slot = index(handle).ok_or(Failure::UnsupportedVideo)?;
        let interval = u64::from(self.ports[slot].state.flip_rate) + 1;
        if vblank_count == 0
            || vblank_count <= self.presentation.last_tick[slot]
            || !vblank_count.is_multiple_of(interval)
        {
            return Ok(None);
        }
        let request = self.presentation.requests[position].as_mut().unwrap();
        if request.phase != Phase::Ready {
            return Ok(None);
        }
        let mut frame = request.frame.take().ok_or(Failure::UnsupportedVideo)?;
        frame.presentation_tick = vblank_count;
        self.presentation.held -= frame.capacity();
        request.phase = Phase::Presenting;
        Ok(Some(frame))
    }
    /// Call only after actual consumer presentation, with real process clocks.
    /// A closed/reopened port or consumed receipt cannot receive stale success.
    #[cfg(test)]
    pub(crate) fn acknowledge_frame(
        &mut self,
        frame: &Frame,
        process_time: u64,
        process_counter: u64,
    ) -> Result<FlipCompletion, Failure> {
        self.acknowledge_frame_with_clock(frame, || Ok((process_time, process_counter)))
    }

    pub(crate) fn acknowledge_frame_with_clock(
        &mut self,
        frame: &Frame,
        clock: impl FnOnce() -> Result<(u64, u64), Failure>,
    ) -> Result<FlipCompletion, Failure> {
        let ticket = frame.ticket;
        let position = self.presentation.position(ticket)?;
        let slot = index(ticket.handle).ok_or(Failure::UnsupportedVideo)?;
        let request = self.presentation.requests[position].as_ref().unwrap();
        if request.phase != Phase::Presenting
            || frame.presentation_tick == 0
            || frame.presentation_tick <= self.presentation.last_tick[slot]
            || self
                .port(ticket.handle)
                .is_none_or(|p| p.generation != ticket.generation)
        {
            return Err(Failure::UnsupportedVideo);
        }
        let mut status = self.presentation.statuses[slot];
        status.count = status.count.checked_add(1).ok_or(Failure::Limit)?;
        // Admit exact owner/phase/generation and count before the real clock
        // observation. A refused/stale frame cannot initialize a process epoch;
        // a clock failure leaves this acknowledgement wholly uncommitted.
        let (process_time, process_counter) = clock()?;
        status.process_time = process_time;
        status.process_counter = process_counter;
        status.flip_arg = request.flip_arg;
        status.current_buffer = request.buffer;
        status.submit_counter = request.submit_counter;
        status.pending -= 1;
        status.gpu_pending -= i32::from(request.source == FlipSource::GpuEop);
        let completion = FlipCompletion {
            owner: ticket.owner,
            handle: ticket.handle,
            generation: ticket.generation,
            flip_arg: request.flip_arg,
            count: status.count,
            buffer: request.buffer,
        };
        self.presentation.statuses[slot] = status;
        self.presentation.last_tick[slot] = frame.presentation_tick;
        self.presentation.requests[position] = None;
        Ok(completion)
    }
    pub(crate) fn buffer_pending(&self, handle: i32, buffer: i32) -> bool {
        self.presentation
            .requests
            .iter()
            .flatten()
            .any(|r| r.ticket.handle == handle && r.buffer == buffer)
    }
    pub(super) fn cancel_flips(&mut self, handle: i32) {
        for slot in &mut self.presentation.requests {
            if slot.as_ref().is_some_and(|r| r.ticket.handle == handle) {
                if let Some(frame) = slot.as_ref().unwrap().frame.as_ref() {
                    self.presentation.held -= frame.capacity();
                }
                *slot = None;
            }
        }
        if let Some(slot) = index(handle) {
            self.presentation.reset(slot);
        }
    }
}
