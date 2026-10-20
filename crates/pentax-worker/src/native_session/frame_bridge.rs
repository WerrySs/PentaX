// SPDX-License-Identifier: GPL-2.0-only
//! Bounded session transport outside every native entry and memory pause.
//! Numeric wire identities never reconstruct the original owned core Frame.
use pentax_core::{
    gpu::video::{FlipCompletion, Frame, PixelFormat},
    process::hle::{KernelStartupError, PreparedNativeKernelStartup},
};
use pentax_session::{ControlEvent, Decoder, FrameFlags, FrameId, Header, Kind, Nonce, Session};
use std::time::{Duration, Instant};

const CHUNK: usize = 65536;
const DECODE_RESERVE: usize = 256;

pub(crate) mod local_presenter;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Transport,
    Closed,
    Cancelled,
    Timeout,
    Limit,
    Protocol,
    ConfigurationChanged,
    Acknowledge,
    NeedsEventDelivery,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Read {
    Pending,
    Bytes(usize),
    Eof,
}
/// queue_write(true) owns a copy of the complete chunk: advance exactly once.
/// poll_write(true) means it is still pending; false means actual completion.
/// Reads copy a complete bounded endpoint chunk or retain its cursor internally.
pub(crate) trait Transport {
    fn retained_bytes(&self) -> usize;
    fn max_retained_bytes(&self) -> usize;
    fn queue_write(&mut self, bytes: &[u8], held: usize) -> Result<bool, Error>;
    fn poll_write(&mut self, held: usize) -> Result<bool, Error>;
    fn read_control(&mut self, bytes: &mut [u8], held: usize) -> Result<Read, Error>;
    fn cancel(&mut self);
}
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub wire: pentax_session::Limits,
    pub max_pumps: u64,
    pub max_idle_nanoseconds: u64,
    pub handshake_timeout: Duration,
    pub frame_timeout: Duration,
    pub session_timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            wire: Default::default(),
            max_pumps: 1_000_000,
            max_idle_nanoseconds: 30_000_000_000,
            handshake_timeout: Duration::from_secs(10),
            frame_timeout: Duration::from_secs(30),
            session_timeout: Duration::from_secs(3600),
        }
    }
}
#[derive(Clone, Copy)]
pub(super) struct ExternalWake {
    pub delay: Duration,
    pub maximum_idle_nanoseconds: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pump {
    Idle,
    Progress,
    RetryEventDelivery,
    NeedsEventDelivery,
}
#[derive(Clone, Copy)]
enum Outgoing {
    None,
    Configure(usize),
    Frame { header: usize, pixels: usize },
}

pub(crate) struct FrameBridge<'transport> {
    transport: &'transport mut dyn Transport,
    wire: Session,
    decoder: Decoder,
    limits: Limits,
    input: [u8; CHUNK],
    input_start: usize,
    input_end: usize,
    configuration: Option<Header>,
    surface_generation: u64,
    surface_handle: Option<i32>,
    pending_tick: Option<u64>,
    ready: bool,
    configuration_sent: bool,
    outgoing: Outgoing,
    write_queued: bool,
    frame: Option<Frame>,
    frame_id: Option<FrameId>,
    frame_sent: bool,
    completion: Option<FlipCompletion>,
    next_sequence: u64,
    last_take_tick: u64,
    started: Instant,
    deadline: Option<Instant>,
    pumps: u64,
    terminal: bool,
}
impl<'transport> FrameBridge<'transport> {
    pub(crate) fn new(
        transport: &'transport mut dyn Transport,
        nonce: Nonce,
        generation: u64,
        limits: Limits,
    ) -> Result<Self, Error> {
        if limits.max_pumps == 0
            || limits.max_pumps > 1_000_000
            || limits.max_idle_nanoseconds == 0
            || limits.max_idle_nanoseconds > 60_000_000_000
            || limits.handshake_timeout.is_zero()
            || limits.handshake_timeout > Duration::from_secs(30)
            || limits.frame_timeout.is_zero()
            || limits.frame_timeout > Duration::from_secs(60)
            || limits.session_timeout.is_zero()
            || limits.session_timeout > Duration::from_secs(3600)
            || transport.max_retained_bytes() > limits.wire.max_staging_bytes
        {
            return Err(Error::Limit);
        }
        let wire = Session::new(nonce, generation, limits.wire).map_err(|_| Error::Protocol)?;
        let decoder = Decoder::new(nonce, generation, limits.wire).map_err(|_| Error::Protocol)?;
        let bridge = Self {
            transport,
            wire,
            decoder,
            limits,
            input: [0; CHUNK],
            input_start: 0,
            input_end: 0,
            configuration: None,
            surface_generation: 0,
            surface_handle: None,
            pending_tick: None,
            ready: false,
            configuration_sent: false,
            outgoing: Outgoing::None,
            write_queued: false,
            frame: None,
            frame_id: None,
            frame_sent: false,
            completion: None,
            next_sequence: 1,
            last_take_tick: 0,
            started: Instant::now(),
            deadline: None,
            pumps: 0,
            terminal: false,
        };
        bridge.held_limit()?;
        Ok(bridge)
    }
    /// Includes a conservative endpoint/decoder growth reservation before any
    /// guest capture. Actual reported capacities must remain within it.
    fn held_limit(&self) -> Result<usize, Error> {
        if self.transport.retained_bytes() > self.transport.max_retained_bytes()
            || self.decoder.retained_bytes() > DECODE_RESERVE
        {
            return Err(Error::Limit);
        }
        let held = std::mem::size_of::<Self>()
            .checked_add(self.transport.max_retained_bytes())
            .and_then(|n| n.checked_add(DECODE_RESERVE))
            .and_then(|n| n.checked_add(self.frame.as_ref().map_or(0, Frame::capacity)))
            .ok_or(Error::Limit)?;
        if held > self.limits.wire.max_staging_bytes {
            return Err(Error::Limit);
        }
        Ok(held)
    }
    fn io_held(&self) -> Result<usize, Error> {
        self.held_limit()?
            .checked_sub(self.transport.max_retained_bytes())
            .ok_or(Error::Limit)
    }
    fn admit(&self, session: &mut PreparedNativeKernelStartup<'_, '_>) -> Result<(), Error> {
        session
            .graphics_video_consumer_admit(
                self.limits.wire.max_frame_bytes,
                self.limits.wire.max_staging_bytes,
                self.held_limit()?,
            )
            .map_err(|_| Error::Limit)
    }
    pub(crate) fn cancel(&mut self) {
        self.terminal = true;
        self.wire.terminate();
        self.decoder.terminate();
        self.transport.cancel();
        // Do not drop the original Frame or pending completion here. The outer
        // supervisor retains them until the actual runtime owner shuts down.
    }
    pub(crate) fn is_pending(&self) -> bool {
        self.frame.is_some()
            || self.completion.is_some()
            || self.pending_tick.is_some()
            || self.deadline.is_some()
    }
    pub(super) fn external_wake(&self) -> Result<Option<ExternalWake>, Error> {
        self.check_time()?;
        Ok(self.deadline.map(|deadline| ExternalWake {
            delay: deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(1)),
            maximum_idle_nanoseconds: self.limits.max_idle_nanoseconds,
        }))
    }
    fn check_time(&self) -> Result<(), Error> {
        if self.terminal {
            return Err(Error::Closed);
        }
        let now = Instant::now();
        if now.duration_since(self.started) >= self.limits.session_timeout
            || self.deadline.is_some_and(|end| now >= end)
        {
            return Err(Error::Timeout);
        }
        Ok(())
    }
    fn configure(
        &mut self,
        width: u32,
        height: u32,
        flags: FrameFlags,
        source_generation: u64,
    ) -> Result<(), Error> {
        let header = Header::configure(
            self.wire.nonce(),
            self.wire.generation(),
            width,
            height,
            flags,
            self.limits.wire,
        )
        .map_err(|_| Error::Protocol)?;
        if let Some(old) = self.configuration {
            if old != header || self.surface_generation != source_generation {
                return Err(Error::ConfigurationChanged);
            }
        } else {
            self.configuration = Some(header);
            self.surface_generation = source_generation;
            self.outgoing = Outgoing::Configure(0);
            self.deadline = Instant::now().checked_add(self.limits.handshake_timeout);
            if self.deadline.is_none() {
                return Err(Error::Limit);
            }
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> {
        for _ in 0..4 {
            let held = self.io_held()?;
            if self.write_queued {
                if self.transport.poll_write(held)? {
                    return Ok(());
                }
                self.write_queued = false;
            }
            let (header, position, pixels) = match self.outgoing {
                Outgoing::None => return Ok(()),
                Outgoing::Configure(position) => {
                    (self.configuration.ok_or(Error::Protocol)?, position, None)
                }
                Outgoing::Frame { header, pixels } => (
                    self.wire.pending().ok_or(Error::Protocol)?.header(),
                    header,
                    Some(pixels),
                ),
            };
            let encoded = header.encode();
            let source = if position < encoded.len() {
                &encoded[position..]
            } else if let Some(at) = pixels {
                let bytes = self.frame.as_ref().ok_or(Error::Protocol)?.rgba8();
                &bytes[at..bytes.len().min(at + CHUNK)]
            } else {
                &[]
            };
            if source.is_empty() {
                if pixels.is_some() {
                    self.frame_sent = true;
                } else {
                    self.configuration_sent = true;
                }
                self.outgoing = Outgoing::None;
                return Ok(());
            }
            let count = source.len();
            if !self.transport.queue_write(source, held)? {
                return Ok(());
            }
            self.write_queued = true;
            self.outgoing = if position < encoded.len() {
                match pixels {
                    Some(pixels) => Outgoing::Frame {
                        header: position + count,
                        pixels,
                    },
                    None => Outgoing::Configure(position + count),
                }
            } else {
                Outgoing::Frame {
                    header: position,
                    pixels: pixels.ok_or(Error::Protocol)? + count,
                }
            };
        }
        Ok(())
    }
    fn read_controls(&mut self) -> Result<(), Error> {
        if self.pending_tick.is_some() {
            return Ok(());
        }
        for _ in 0..16 {
            if self.input_start == self.input_end {
                let held = self.io_held()?;
                match self.transport.read_control(&mut self.input, held)? {
                    Read::Pending => return Ok(()),
                    Read::Eof => {
                        self.decoder.end_of_stream();
                        return Err(Error::Closed);
                    }
                    Read::Bytes(n) if n != 0 && n <= self.input.len() => {
                        self.input_start = 0;
                        self.input_end = n;
                    }
                    Read::Bytes(_) => return Err(Error::Transport),
                }
            }
            let expected_ready = self
                .configuration
                .map(Header::ready)
                .transpose()
                .map_err(|_| Error::Protocol)?;
            let ready = self.ready;
            let configuration_sent = self.configuration_sent;
            let frame_sent = self.frame_sent;
            let held = self
                .io_held()?
                .checked_add(self.transport.retained_bytes())
                .ok_or(Error::Limit)?;
            let wire = &self.wire;
            let fed = self
                .decoder
                .feed_checked(
                    &self.input[self.input_start..self.input_end],
                    held,
                    |header| {
                        match header.kind() {
                            Kind::Ready
                                if !ready
                                    && configuration_sent
                                    && Some(header) == expected_ready => {}
                            Kind::Presented if ready && frame_sent => {}
                            Kind::Vblank if ready => {}
                            Kind::Close => {}
                            _ => return Err(pentax_session::Error::InvalidHeader),
                        }
                        wire.check_incoming_header(header)
                    },
                )
                .map_err(|_| Error::Protocol)?;
            self.input_start += fed.consumed;
            if let Some(message) = fed.message {
                match self
                    .wire
                    .receive_control(&message)
                    .map_err(|_| Error::Protocol)?
                {
                    ControlEvent::Ready(_) => {
                        let deadline = Instant::now()
                            .checked_add(self.limits.frame_timeout)
                            .ok_or(Error::Limit)?;
                        self.ready = true;
                        self.deadline = Some(deadline);
                    }
                    ControlEvent::Presented(_) => {}
                    ControlEvent::Vblank(tick) => {
                        self.pending_tick = Some(tick);
                        return Ok(());
                    }
                    ControlEvent::Closed => return Err(Error::Closed),
                    _ => return Err(Error::Protocol),
                }
            }
        }
        Ok(())
    }
    pub(crate) fn pump(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'_, '_>,
    ) -> Result<Pump, Error> {
        let result = self.pump_inner(session);
        if result.is_err() {
            self.cancel();
        }
        result
    }
    fn deliver_completion(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'_, '_>,
    ) -> Result<Pump, Error> {
        match session.graphics_video_deliver_completion(&mut self.completion) {
            Ok(()) => {
                self.deadline = None;
                Ok(Pump::Progress)
            }
            // The flip count is already committed. Hold guest entry until
            // this same completion is delivered or its finite deadline fails.
            Err(KernelStartupError::InvalidProvider("video event counter")) => {
                Ok(Pump::RetryEventDelivery)
            }
            Err(_) => Ok(Pump::NeedsEventDelivery),
        }
    }
    fn pump_inner(
        &mut self,
        session: &mut PreparedNativeKernelStartup<'_, '_>,
    ) -> Result<Pump, Error> {
        self.check_time()?;
        self.pumps = self
            .pumps
            .checked_add(1)
            .filter(|n| *n <= self.limits.max_pumps)
            .ok_or(Error::Limit)?;
        self.admit(session)?;
        // A committed flip's completion has priority over all later controls:
        // none may return Progress and admit guest entry before its delivery.
        if self.completion.is_some() {
            return self.deliver_completion(session);
        }
        self.flush()?;
        self.read_controls()?;
        self.held_limit()?;
        if let Some(tick) = self.pending_tick {
            let handle = self.surface_handle.ok_or(Error::Protocol)?;
            match session.graphics_video_observe_vblank(handle, self.surface_generation, tick) {
                Ok(()) => self.pending_tick = None,
                Err(KernelStartupError::InvalidProvider("video event counter")) => {
                    return Ok(Pump::Progress);
                }
                Err(_) => return Ok(Pump::NeedsEventDelivery),
            }
        }
        if self.input_start != self.input_end {
            return Ok(Pump::Progress);
        }
        if let (Some(frame), Some(id)) = (&self.frame, self.frame_id)
            && self
                .wire
                .pending()
                .is_some_and(|pending| pending.receipt_observed())
        {
            match session.graphics_video_acknowledge_frame(frame) {
                Ok(completion) => {
                    self.wire.complete_frame(id).map_err(|_| Error::Protocol)?;
                    self.completion = Some(completion);
                    self.frame = None;
                    self.frame_id = None;
                    self.admit(session)?;
                    // Deliver before the guest can observe count1 and close
                    // its port or return. Refusal retains the same completion.
                    return self.deliver_completion(session);
                }
                // Only this exact adapter clock branch may be retried. The
                // original Frame/receipt stays owned and is never sent again.
                Err(KernelStartupError::Graphics(pentax_core::gpu::driver::Failure::Kernel(_))) => {
                    return Ok(Pump::Progress);
                }
                Err(_) => return Err(Error::Acknowledge),
            }
        }
        if self.frame.is_some() {
            return Ok(Pump::Progress);
        }
        let metadata = session
            .graphics_video_peek_ready_frame()
            .map_err(|_| Error::Acknowledge)?;
        let Some(metadata) = metadata else {
            if self.ready || self.configuration.is_none() {
                self.deadline = None;
            }
            return Ok(Pump::Idle);
        };
        let color = match metadata.source_format() {
            PixelFormat::Rgba8Srgb | PixelFormat::Bgra8Srgb => pentax_session::ColorEncoding::Srgb,
            _ => pentax_session::ColorEncoding::Unspecified,
        };
        let flags = FrameFlags::new(
            if metadata.premultiplied_alpha() {
                pentax_session::AlphaMode::Premultiplied
            } else {
                pentax_session::AlphaMode::Straight
            },
            color,
        );
        if self
            .surface_handle
            .is_some_and(|old| old != metadata.handle())
        {
            return Err(Error::ConfigurationChanged);
        }
        self.surface_handle = Some(metadata.handle());
        self.configure(
            metadata.width(),
            metadata.height(),
            flags,
            metadata.generation(),
        )?;
        if !self.ready {
            self.flush()?;
            return Ok(Pump::Progress);
        }
        if self.deadline.is_none() {
            self.deadline = Instant::now().checked_add(self.limits.frame_timeout);
            if self.deadline.is_none() {
                return Err(Error::Limit);
            }
        }
        let tick = self.wire.last_vblank();
        if tick == 0 || tick <= self.last_take_tick {
            return Ok(Pump::Progress);
        }
        self.next_sequence.checked_add(1).ok_or(Error::Limit)?;
        if metadata.capacity() > self.limits.wire.max_frame_bytes
            || self
                .held_limit()?
                .checked_add(metadata.capacity())
                .is_none_or(|n| n > self.limits.wire.max_staging_bytes)
        {
            return Err(Error::Limit);
        }
        self.last_take_tick = tick;
        let Some(frame) = session
            .graphics_video_take_frame(tick)
            .map_err(|_| Error::Acknowledge)?
        else {
            return Ok(Pump::Progress);
        };
        // Retain the genuine Frame BEFORE any later fallible protocol work.
        self.frame = Some(frame);
        let header = self
            .wire
            .begin_frame(metadata.width(), metadata.height(), flags)
            .map_err(|_| Error::Protocol)?;
        if header.id().sequence() != self.next_sequence {
            return Err(Error::Protocol);
        }
        self.next_sequence += 1;
        self.frame_id = Some(header.id());
        self.frame_sent = false;
        self.outgoing = Outgoing::Frame {
            header: 0,
            pixels: 0,
        };
        self.deadline = Some(
            Instant::now()
                .checked_add(self.limits.frame_timeout)
                .ok_or(Error::Limit)?,
        );
        self.admit(session)?;
        self.flush()?;
        Ok(Pump::Progress)
    }
}
