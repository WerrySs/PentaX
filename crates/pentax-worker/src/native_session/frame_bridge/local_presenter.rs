// SPDX-License-Identifier: GPL-2.0-only
//! Current-thread, supervised-worker consumer of the private frame stream.
//!
//! No method may run while guest execution or a guest-memory pause is held.
//! The original core Frame remains in FrameBridge. Only the actual platform
//! presenter supplies display ticks and successful presentation observations.

use super::{Error, Read, Transport};
use pentax_platform::presenter::{self, Presenter};
use pentax_session::{
    ColorEncoding, ControlEvent, Decoded, Decoder, FrameId, HEADER_BYTES, Header, Kind, Nonce,
    Session,
};

const CHUNK: usize = 65_536;
const CONTROL_BYTES: usize = HEADER_BYTES + 8;
const CONTROL_SLOTS: usize = 3;

/// Separate backend allocation cap and total CPU ceiling. The former includes
/// actual mapped upload memory and owned presenter metadata, not driver heaps.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub wire: pentax_session::Limits,
    pub presenter: presenter::Limits,
    pub max_presenter_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            wire: Default::default(),
            presenter: Default::default(),
            max_presenter_bytes: 24 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivered {
    None,
    Ready,
    Presented(FrameId),
}
#[derive(Clone, Copy)]
struct Packet {
    bytes: [u8; CONTROL_BYTES],
    length: usize,
    delivered: Delivered,
}
impl Packet {
    const EMPTY: Self = Self {
        bytes: [0; CONTROL_BYTES],
        length: 0,
        delivered: Delivered::None,
    };
    fn new(header: Header, payload: &[u8], delivered: Delivered) -> Result<Self, Error> {
        if header.payload_len() != payload.len() || payload.len() > 8 {
            return Err(Error::Protocol);
        }
        let mut packet = Self::EMPTY;
        packet.bytes[..HEADER_BYTES].copy_from_slice(&header.encode());
        packet.bytes[HEADER_BYTES..HEADER_BYTES + payload.len()].copy_from_slice(payload);
        packet.length = HEADER_BYTES + payload.len();
        packet.delivered = delivered;
        Ok(packet)
    }
}

/// Inline fixed queue; a short read never discards the remainder of a receipt.
struct Controls {
    packets: [Packet; CONTROL_SLOTS],
    start: usize,
    count: usize,
    cursor: usize,
}
impl Controls {
    fn new() -> Self {
        Self {
            packets: [Packet::EMPTY; CONTROL_SLOTS],
            start: 0,
            count: 0,
            cursor: 0,
        }
    }
    fn available(&self) -> bool {
        self.count < CONTROL_SLOTS
    }
    fn push(&mut self, packet: Packet) -> Result<(), Error> {
        if !self.available() {
            return Err(Error::Limit);
        }
        self.packets[(self.start + self.count) % CONTROL_SLOTS] = packet;
        self.count += 1;
        Ok(())
    }
    fn read(&mut self, output: &mut [u8]) -> Result<Option<(usize, Delivered)>, Error> {
        if output.is_empty() {
            return Err(Error::Limit);
        }
        if self.count == 0 {
            return Ok(None);
        }
        let packet = &self.packets[self.start];
        let count = output.len().min(packet.length - self.cursor);
        output[..count].copy_from_slice(&packet.bytes[self.cursor..self.cursor + count]);
        self.cursor += count;
        let delivered = if self.cursor == packet.length {
            let delivered = packet.delivered;
            self.packets[self.start] = Packet::EMPTY;
            self.start = (self.start + 1) % CONTROL_SLOTS;
            self.count -= 1;
            self.cursor = 0;
            delivered
        } else {
            Delivered::None
        };
        Ok(Some((count, delivered)))
    }
}

/// Unique byte endpoint and real window on the current worker thread.
/// Cancellation retains the pending decoded image until supervised teardown;
/// it never acknowledges, requeues, or reconstructs the original core Frame.
pub(crate) struct LocalPresenterTransport {
    wire: Session,
    decoder: Decoder,
    limits: Limits,
    reservation: usize,
    presenter: Option<Presenter>,
    window_generation: u64,
    configuration: Option<Header>,
    ready_delivered: bool,
    pending: Option<Decoded>,
    submitted: bool,
    controls: Controls,
    write_pending: bool,
    terminal: Option<Error>,
}
impl LocalPresenterTransport {
    pub(crate) fn new(nonce: Nonce, generation: u64, limits: Limits) -> Result<Self, Error> {
        let wire = Session::new(nonce, generation, limits.wire).map_err(codec_error)?;
        let decoder = Decoder::new(nonce, generation, limits.wire).map_err(codec_error)?;
        if limits.max_presenter_bytes == 0
            || limits.max_presenter_bytes > limits.presenter.max_cpu_bytes
            || limits.presenter.max_cpu_bytes > limits.wire.max_staging_bytes
            || limits.presenter.max_frame_bytes < limits.wire.max_frame_bytes
        {
            return Err(Error::Limit);
        }
        let reservation = std::mem::size_of::<Self>()
            .checked_add(limits.wire.max_frame_bytes)
            .and_then(|bytes| bytes.checked_add(limits.max_presenter_bytes))
            .ok_or(Error::Limit)?;
        admit(0, reservation, limits.wire.max_staging_bytes)?;
        Ok(Self {
            wire,
            decoder,
            limits,
            reservation,
            presenter: None,
            window_generation: 0,
            configuration: None,
            ready_delivered: false,
            pending: None,
            submitted: false,
            controls: Controls::new(),
            write_pending: false,
            terminal: None,
        })
    }
    fn active(&self) -> Result<(), Error> {
        self.terminal.map_or(Ok(()), Err)
    }
    fn presenter_bytes(&self) -> usize {
        self.presenter
            .as_ref()
            .map_or(0, Presenter::retained_cpu_bytes)
    }
    fn held_without_decoder(&self, held: usize) -> Result<usize, Error> {
        held.checked_add(std::mem::size_of::<Self>())
            .and_then(|bytes| bytes.checked_add(self.presenter_bytes()))
            .and_then(|bytes| bytes.checked_add(self.pending.as_ref().map_or(0, Decoded::capacity)))
            .ok_or(Error::Limit)
    }
    fn check(&self, held: usize) -> Result<(), Error> {
        self.active()?;
        if self.presenter_bytes() > self.limits.max_presenter_bytes
            || self.retained_bytes() > self.reservation
        {
            return Err(Error::Limit);
        }
        admit(
            held,
            self.retained_bytes(),
            self.limits.wire.max_staging_bytes,
        )
    }
    fn fail(&mut self, error: Error) -> Error {
        self.terminal.get_or_insert(error);
        self.wire.terminate();
        self.decoder.terminate();
        if let Some(presenter) = self.presenter.as_mut() {
            presenter.cancel();
        }
        // Keep pending and presenter owned. A terminal failure is never a
        // receipt, even if a partial control header was already transferred.
        error
    }
    fn consume(&mut self, message: Decoded, held: usize) -> Result<(), Error> {
        let header = message.header();
        match header.kind() {
            Kind::Configure => {
                if self.configuration.is_some() {
                    return Err(Error::ConfigurationChanged);
                }
                let flags = header.frame_flags().ok_or(Error::Protocol)?;
                if flags.color() != ColorEncoding::Srgb {
                    return Err(Error::ConfigurationChanged);
                }
                if !matches!(
                    self.wire.receive_control(&message).map_err(codec_error)?,
                    ControlEvent::Configured(_)
                ) {
                    return Err(Error::Protocol);
                }
                // Reserve the entire possible backend before opening a window
                // or allocating upload memory. The platform separately checks
                // actual mapped requirements and metadata against this cap.
                let other = self.held_without_decoder(held)?;
                admit(
                    other,
                    self.limits.max_presenter_bytes,
                    self.limits.wire.max_staging_bytes,
                )?;
                let presenter = Presenter::open_with_staging_cap(
                    presenter::Config {
                        width: header.width(),
                        height: header.height(),
                    },
                    self.limits.presenter,
                    self.limits.max_presenter_bytes,
                )
                .map_err(platform_error)?;
                let state = presenter.window_state();
                if state.status != presenter::WindowStatus::Active
                    || state.width != header.width()
                    || state.height != header.height()
                    || presenter.retained_cpu_bytes() > self.limits.max_presenter_bytes
                {
                    return Err(Error::ConfigurationChanged);
                }
                self.window_generation = state.generation;
                self.presenter = Some(presenter);
                self.configuration = Some(header);
                self.controls.push(Packet::new(
                    Header::ready(header).map_err(codec_error)?,
                    &[],
                    Delivered::Ready,
                )?)?;
            }
            Kind::FrameRgba8 => {
                if self.pending.is_some() {
                    return Err(Error::Protocol);
                }
                self.wire.accept_frame(header).map_err(codec_error)?;
                self.pending = Some(message);
            }
            Kind::Close => {
                self.wire.receive_control(&message).map_err(codec_error)?;
                return Err(Error::Closed);
            }
            _ => return Err(Error::Protocol),
        }
        self.check(held)
    }
    fn write(&mut self, bytes: &[u8], held: usize) -> Result<bool, Error> {
        self.check(held)?;
        if bytes.is_empty() || bytes.len() > CHUNK {
            return Err(Error::Limit);
        }
        if self.write_pending {
            return Ok(false);
        }
        let mut position = 0;
        // FrameBridge uses one header or one payload chunk. The small fixed
        // bound also rejects a hostile stream of empty control messages.
        for _ in 0..4 {
            let other = self.held_without_decoder(held)?;
            let wire = &self.wire;
            let configuration = self.configuration;
            let ready = self.ready_delivered;
            let fed = self
                .decoder
                .feed_checked(&bytes[position..], other, |header| {
                    match header.kind() {
                        Kind::Configure if configuration.is_none() => {}
                        Kind::FrameRgba8 if ready => {
                            let config =
                                configuration.ok_or(pentax_session::Error::InvalidHeader)?;
                            if header.width() != config.width()
                                || header.height() != config.height()
                                || header.frame_flags() != config.frame_flags()
                            {
                                return Err(pentax_session::Error::InvalidDimensions);
                            }
                        }
                        Kind::Close => {}
                        _ => return Err(pentax_session::Error::InvalidHeader),
                    }
                    wire.check_incoming_header(header)
                })
                .map_err(codec_error)?;
            position += fed.consumed;
            if let Some(message) = fed.message {
                self.consume(message, held)?;
            }
            if position == bytes.len() {
                self.write_pending = true;
                return Ok(true);
            }
            if fed.consumed == 0 {
                return Err(Error::Protocol);
            }
        }
        Err(Error::Limit)
    }
    fn drive(&mut self, held: usize) -> Result<(), Error> {
        self.check(held)?;
        if !self.ready_delivered {
            return Ok(());
        }
        let backend_bytes = self.presenter_bytes();
        let external = self
            .retained_bytes()
            .checked_sub(backend_bytes)
            .and_then(|bytes| bytes.checked_add(held))
            .ok_or(Error::Limit)?;
        let presenter = self.presenter.as_mut().ok_or(Error::Protocol)?;
        let state = match presenter.pump() {
            Ok(state) => state,
            Err(presenter::Error::WouldBlock) => return Ok(()),
            Err(error) => return Err(platform_error(error)),
        };
        if state.generation != self.window_generation
            || state.status != presenter::WindowStatus::Active
        {
            return Err(Error::ConfigurationChanged);
        }
        if let Some(frame) = self.pending.as_ref() {
            let header = frame.header();
            let flags = header.frame_flags().ok_or(Error::Protocol)?;
            if !self.submitted {
                let view = presenter::FrameView {
                    width: header.width(),
                    height: header.height(),
                    pixels: frame.payload(),
                    alpha: alpha(flags.alpha()),
                    encoding: presenter::ColorEncoding::Srgb,
                };
                let accepted = match presenter.submit(header.id().sequence(), view, external) {
                    Ok(accepted) => accepted,
                    Err(presenter::Error::WouldBlock) => return Ok(()),
                    Err(error) => return Err(platform_error(error)),
                };
                if accepted.generation() != self.window_generation
                    || accepted.sequence() != header.id().sequence()
                    || accepted.alpha() != alpha(flags.alpha())
                {
                    return Err(Error::Protocol);
                }
                self.submitted = true;
            }
            // Reserve a control slot before consuming the presenter's one-shot
            // completion. Numeric sequence agreement alone is insufficient.
            if !self.controls.available()
                || self.wire.pending().is_some_and(|p| p.receipt_observed())
            {
                return Ok(());
            }
            let presented = match presenter.poll_present() {
                Ok(presented) => presented,
                Err(presenter::Error::WouldBlock) => return Ok(()),
                Err(error) => return Err(platform_error(error)),
            };
            match presented {
                presenter::Presentation::Pending => {}
                presenter::Presentation::Idle => return Err(Error::Protocol),
                presenter::Presentation::Presented(receipt) => {
                    if receipt.generation() != self.window_generation
                        || receipt.sequence() != header.id().sequence()
                        || receipt.alpha() != alpha(flags.alpha())
                    {
                        return Err(Error::Protocol);
                    }
                    let response = self
                        .wire
                        .receipt_for_presented(header.id())
                        .map_err(codec_error)?;
                    self.controls.push(Packet::new(
                        response,
                        &[],
                        Delivered::Presented(header.id()),
                    )?)?;
                }
            }
        } else if self.controls.count == 0 {
            // Do not flood or replace a partially transferred tick. Equal DWM
            // counts return None; there is no timer-derived refresh estimate.
            let tick = match presenter.observe_tick() {
                Ok(tick) => tick,
                Err(presenter::Error::WouldBlock) => None,
                Err(error) => return Err(platform_error(error)),
            };
            if let Some(tick) = tick {
                if tick.generation() != self.window_generation {
                    return Err(Error::ConfigurationChanged);
                }
                let header = Header::vblank(self.wire.nonce(), self.wire.generation())
                    .map_err(codec_error)?;
                self.controls.push(Packet::new(
                    header,
                    &tick.count().to_le_bytes(),
                    Delivered::None,
                )?)?;
            }
        }
        Ok(())
    }
    fn read(&mut self, output: &mut [u8], held: usize) -> Result<Read, Error> {
        self.check(held)?;
        if output.is_empty() {
            return Err(Error::Limit);
        }
        // Even a synchronous local copy has an explicit write-completion step.
        // Do not emit Ready/Presented before FrameBridge has observed it.
        if self.write_pending {
            return Ok(Read::Pending);
        }
        self.drive(held)?;
        let Some((count, delivered)) = self.controls.read(output)? else {
            return Ok(Read::Pending);
        };
        match delivered {
            Delivered::None => {}
            Delivered::Ready => self.ready_delivered = true,
            Delivered::Presented(id) => {
                self.wire.complete_frame(id).map_err(codec_error)?;
                self.pending = None;
                self.submitted = false;
            }
        }
        Ok(Read::Bytes(count))
    }
}

impl Transport for LocalPresenterTransport {
    fn retained_bytes(&self) -> usize {
        // Each component is independently bounded before publication; saturate
        // to a refusal value rather than letting a corrupted count wrap.
        std::mem::size_of::<Self>()
            .saturating_add(self.decoder.retained_bytes())
            .saturating_add(self.pending.as_ref().map_or(0, Decoded::capacity))
            .saturating_add(self.presenter_bytes())
    }
    fn max_retained_bytes(&self) -> usize {
        // A fixed endpoint ceiling, not an allocated future reservation.
        // FrameBridge publishes retained_bytes(), and supplies all actual
        // other/core-frame capacities before each allocation in this endpoint.
        self.reservation
    }
    fn queue_write(&mut self, bytes: &[u8], held: usize) -> Result<bool, Error> {
        let result = self.write(bytes, held);
        result.map_err(|error| self.fail(error))
    }
    fn poll_write(&mut self, held: usize) -> Result<bool, Error> {
        if let Err(error) = self.check(held) {
            return Err(self.fail(error));
        }
        self.write_pending = false;
        Ok(false)
    }
    fn read_control(&mut self, bytes: &mut [u8], held: usize) -> Result<Read, Error> {
        let result = self.read(bytes, held);
        match result {
            // The real window owner closed the stream. Retain/cancel its
            // state before reporting EOF; this is never a presentation receipt.
            Err(Error::Closed) => {
                self.fail(Error::Closed);
                Ok(Read::Eof)
            }
            other => other.map_err(|error| self.fail(error)),
        }
    }
    fn cancel(&mut self) {
        self.fail(Error::Cancelled);
    }
}

fn admit(held: usize, added: usize, maximum: usize) -> Result<(), Error> {
    if held.checked_add(added).is_none_or(|bytes| bytes > maximum) {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}
fn alpha(value: pentax_session::AlphaMode) -> presenter::AlphaMode {
    match value {
        pentax_session::AlphaMode::Straight => presenter::AlphaMode::Straight,
        pentax_session::AlphaMode::Premultiplied => presenter::AlphaMode::Premultiplied,
    }
}
fn codec_error(error: pentax_session::Error) -> Error {
    match error {
        pentax_session::Error::Limit | pentax_session::Error::Allocation => Error::Limit,
        pentax_session::Error::Closed | pentax_session::Error::EndOfStream => Error::Closed,
        _ => Error::Protocol,
    }
}
fn platform_error(error: presenter::Error) -> Error {
    match error {
        presenter::Error::Limit | presenter::Error::Allocation => Error::Limit,
        presenter::Error::Closed => Error::Closed,
        presenter::Error::Cancelled => Error::Cancelled,
        presenter::Error::NeedsReopen | presenter::Error::WindowChanged(_) => {
            Error::ConfigurationChanged
        }
        _ => Error::Transport,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pentax_session::{AlphaMode, FrameFlags};
    fn nonce() -> Nonce {
        Nonce::new([19; 16]).unwrap()
    }

    #[test]
    fn control_cursor_retains_exact_receipt_until_last_byte_and_is_bounded() {
        let mut wire = Session::new(nonce(), 9, Default::default()).unwrap();
        let frame = wire
            .begin_frame(
                3,
                2,
                FrameFlags::new(AlphaMode::Premultiplied, ColorEncoding::Srgb),
            )
            .unwrap();
        let header = Header::presented(frame.id()).unwrap();
        let mut controls = Controls::new();
        controls
            .push(Packet::new(header, &[], Delivered::Presented(frame.id())).unwrap())
            .unwrap();
        let tick = Header::vblank(nonce(), 9).unwrap();
        controls
            .push(Packet::new(tick, &81_u64.to_le_bytes(), Delivered::None).unwrap())
            .unwrap();
        controls
            .push(Packet::new(tick, &92_u64.to_le_bytes(), Delivered::None).unwrap())
            .unwrap();
        assert_eq!(controls.push(Packet::EMPTY), Err(Error::Limit));
        let mut result = [0; HEADER_BYTES];
        for (i, byte) in result.iter_mut().enumerate() {
            let (count, delivered) = controls.read(std::slice::from_mut(byte)).unwrap().unwrap();
            assert_eq!(count, 1);
            assert_eq!(
                delivered,
                if i + 1 == HEADER_BYTES {
                    Delivered::Presented(frame.id())
                } else {
                    Delivered::None
                }
            );
        }
        assert_eq!(result, header.encode());
        let mut small = [0; 5];
        let mut decoder = Decoder::new(nonce(), 9, Default::default()).unwrap();
        let mut messages = 0;
        while let Some((count, delivered)) = controls.read(&mut small).unwrap() {
            assert_eq!(delivered, Delivered::None);
            let fed = decoder.feed(&small[..count], 0).unwrap();
            assert_eq!(fed.consumed, count);
            if let Some(message) = fed.message {
                assert_eq!(message.header(), tick);
                assert_eq!(
                    message.payload(),
                    [if messages == 0 { 81 } else { 92 }, 0, 0, 0, 0, 0, 0, 0]
                );
                messages += 1;
            }
        }
        assert_eq!(messages, 2);
        assert!(controls.read(&mut small).unwrap().is_none());
    }

    #[test]
    fn preconfiguration_frames_and_foreign_headers_refuse_without_opening_window() {
        let limits = Limits::default();
        let mut actor = LocalPresenterTransport::new(nonce(), 9, limits).unwrap();
        assert_eq!(actor.read_control(&mut [0; 7], 0), Ok(Read::Pending));
        let mut wire = Session::new(nonce(), 9, limits.wire).unwrap();
        let frame = wire
            .begin_frame(
                2,
                1,
                FrameFlags::new(AlphaMode::Straight, ColorEncoding::Srgb),
            )
            .unwrap();
        assert_eq!(actor.queue_write(&frame.encode(), 0), Err(Error::Protocol));
        assert!(actor.presenter.is_none());
        assert!(actor.pending.is_none());
        assert_eq!(actor.decoder.retained_bytes(), 0);
        assert_eq!(actor.poll_write(0), Err(Error::Protocol));
        let mut foreign = LocalPresenterTransport::new(nonce(), 9, limits).unwrap();
        let config = Header::configure(
            Nonce::new([77; 16]).unwrap(),
            9,
            1,
            1,
            FrameFlags::new(AlphaMode::Straight, ColorEncoding::Srgb),
            limits.wire,
        )
        .unwrap();
        assert_eq!(
            foreign.queue_write(&config.encode(), 0),
            Err(Error::Protocol)
        );
        assert!(foreign.presenter.is_none());
        let mut bounded = LocalPresenterTransport::new(nonce(), 9, limits).unwrap();
        let held = limits.wire.max_staging_bytes - bounded.retained_bytes() + 1;
        assert_eq!(bounded.queue_write(&[0], held), Err(Error::Limit));
        assert!(bounded.presenter.is_none());
        assert_eq!(admit(usize::MAX, 1, usize::MAX), Err(Error::Limit));
        assert!(LocalPresenterTransport::new(nonce(), 0, limits).is_err());
        assert!(
            LocalPresenterTransport::new(
                nonce(),
                9,
                Limits {
                    max_presenter_bytes: usize::MAX,
                    ..limits
                }
            )
            .is_err()
        );
    }
}
