// SPDX-License-Identifier: GPL-2.0-only
use crate::{Decoded, Error, FrameFlags, FrameId, Header, Kind, Limits, Nonce};

/// Copyable metadata only; the original noncopyable core Frame stays outside
/// this crate in the worker's single owned slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingFrame {
    header: Header,
    receipt_observed: bool,
}
impl PendingFrame {
    pub fn header(self) -> Header {
        self.header
    }
    pub fn receipt_observed(self) -> bool {
        self.receipt_observed
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlEvent {
    Presented(FrameId),
    Vblank(u64),
    Closed,
}

/// Allocation-free identity/ownership tracking. This state is deliberately not
/// Clone: a live sequence/slot must not be duplicated as a transport owner.
#[derive(Debug)]
pub struct Session {
    nonce: Nonce,
    generation: u64,
    next_sequence: u64,
    last_vblank: u64,
    pending: Option<PendingFrame>,
    limits: Limits,
    terminal: bool,
}
impl Session {
    pub fn new(nonce: Nonce, generation: u64, limits: Limits) -> Result<Self, Error> {
        limits.validate()?;
        if generation == 0 {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self {
            nonce,
            generation,
            next_sequence: 1,
            last_vblank: 0,
            pending: None,
            limits,
            terminal: false,
        })
    }
    pub fn nonce(&self) -> Nonce {
        self.nonce
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn pending(&self) -> Option<PendingFrame> {
        self.pending
    }
    pub fn last_vblank(&self) -> u64 {
        self.last_vblank
    }
    pub fn is_terminal(&self) -> bool {
        self.terminal
    }
    /// Sender reservation, before handoff. On any later pipe/timeout failure,
    /// terminate rather than silently clearing or overwriting this slot.
    pub fn begin_frame(
        &mut self,
        width: u32,
        height: u32,
        flags: FrameFlags,
    ) -> Result<Header, Error> {
        self.active()?;
        if self.pending.is_some() {
            return Err(Error::Backpressure);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        let id = FrameId {
            nonce: self.nonce,
            sequence: self.next_sequence,
            generation: self.generation,
        };
        let header = Header::frame(id, width, height, flags, self.limits)?;
        self.pending = Some(PendingFrame {
            header,
            receipt_observed: false,
        });
        self.next_sequence = next;
        Ok(header)
    }
    /// Consumer admission. Rejects gaps, reordering, stale generation and a
    /// second frame while the first one remains locally owned.
    pub fn accept_frame(&mut self, header: Header) -> Result<(), Error> {
        self.active()?;
        let result = self.accept_frame_inner(header);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }
    fn accept_frame_inner(&mut self, header: Header) -> Result<(), Error> {
        self.check_incoming_header(header)?;
        if header.kind() != Kind::FrameRgba8 {
            return Err(Error::InvalidHeader);
        }
        let next = self
            .next_sequence
            .checked_add(1)
            .ok_or(Error::SequenceExhausted)?;
        self.pending = Some(PendingFrame {
            header,
            receipt_observed: false,
        });
        self.next_sequence = next;
        Ok(())
    }
    /// Allocation-free endpoint preflight for Decoder::feed_checked. It never
    /// reserves/consumes a sequence or receipt, and must be repeated by final
    /// admission after the complete payload arrives.
    pub fn check_incoming_header(&self, header: Header) -> Result<(), Error> {
        self.active()?;
        self.identity(header)?;
        match header.kind() {
            Kind::FrameRgba8 => {
                if self.pending.is_some() {
                    return Err(Error::Backpressure);
                }
                if header.id().sequence != self.next_sequence {
                    return Err(Error::UnexpectedSequence);
                }
                Header::frame(
                    header.id(),
                    header.width(),
                    header.height(),
                    header.frame_flags().ok_or(Error::InvalidFlags)?,
                    self.limits,
                )?;
                self.next_sequence
                    .checked_add(1)
                    .ok_or(Error::SequenceExhausted)?;
            }
            Kind::Presented => {
                let pending = self.pending.ok_or(Error::InvalidReceipt)?;
                if pending.header.id() != header.id() {
                    return Err(Error::InvalidReceipt);
                }
                if pending.receipt_observed {
                    return Err(Error::DuplicateReceipt);
                }
            }
            Kind::Vblank | Kind::Close => {}
        }
        Ok(())
    }
    /// Call only on the dedicated endpoint. Decoding a Presented/Vblank message
    /// does not prove its physical origin: the platform must bind that endpoint
    /// to a presenter with independently verified completion/tick capabilities.
    pub fn receive_control(&mut self, message: &Decoded) -> Result<ControlEvent, Error> {
        self.active()?;
        let result = self.receive_control_inner(message);
        if result.is_err() {
            self.terminal = true;
        }
        result
    }
    fn receive_control_inner(&mut self, message: &Decoded) -> Result<ControlEvent, Error> {
        let header = message.header();
        self.check_incoming_header(header)?;
        header.validate_payload(message.payload())?;
        match header.kind() {
            Kind::Presented => {
                self.mark_receipt(header.id())?;
                Ok(ControlEvent::Presented(header.id()))
            }
            Kind::Vblank => {
                let count = u64::from_le_bytes(
                    message
                        .payload()
                        .try_into()
                        .map_err(|_| Error::InvalidLength)?,
                );
                if count <= self.last_vblank {
                    return Err(Error::InvalidVblank);
                }
                self.last_vblank = count;
                Ok(ControlEvent::Vblank(count))
            }
            Kind::Close => {
                self.terminal = true;
                Ok(ControlEvent::Closed)
            }
            Kind::FrameRgba8 => Err(Error::InvalidHeader),
        }
    }
    /// Consumer-side bookkeeping after independently verified actual present
    /// completion. The returned header contains no authority. If transmitting
    /// it fails, terminate rather than releasing/reusing the pending slot.
    pub fn receipt_for_presented(&mut self, id: FrameId) -> Result<Header, Error> {
        self.active()?;
        let header = Header::presented(id)?;
        self.mark_receipt(id)?;
        Ok(header)
    }
    /// Worker calls this ONLY after the real core acknowledgement succeeds.
    /// A clock-only failure calls nothing here: the same original Frame and
    /// receipt-observed slot remain available without presenting/sending again.
    /// Consumer may release its local slot after successful presentation and
    /// successful receipt transmission; it must retain ownership on I/O failure.
    pub fn complete_frame(&mut self, id: FrameId) -> Result<(), Error> {
        self.active()?;
        let pending = self.pending.ok_or(Error::InvalidReceipt)?;
        if pending.header.id() != id || !pending.receipt_observed {
            return Err(Error::InvalidReceipt);
        }
        self.pending = None;
        Ok(())
    }
    /// A window generation can change only with no outstanding frame. Frame
    /// sequence is never reset/reused. Window loss during handoff terminates;
    /// continuing would require a separate checked cancel/requeue capability.
    pub fn advance_generation(&mut self, generation: u64) -> Result<(), Error> {
        self.active()?;
        if self.pending.is_some() {
            return Err(Error::Backpressure);
        }
        if generation <= self.generation {
            return Err(Error::StaleGeneration);
        }
        self.generation = generation;
        self.last_vblank = 0;
        Ok(())
    }
    /// Close, EOF, timeout and cancellation retain pending identity until the
    /// outer owner stops/reaps its worker and drops the original Frame. None of
    /// them are successful acknowledgement or a requeue operation.
    pub fn terminate(&mut self) {
        self.terminal = true;
    }
    fn active(&self) -> Result<(), Error> {
        if self.terminal {
            Err(Error::Closed)
        } else {
            Ok(())
        }
    }
    fn mark_receipt(&mut self, id: FrameId) -> Result<(), Error> {
        let pending = self.pending.as_mut().ok_or(Error::InvalidReceipt)?;
        if pending.header.id() != id {
            return Err(Error::InvalidReceipt);
        }
        if pending.receipt_observed {
            return Err(Error::DuplicateReceipt);
        }
        pending.receipt_observed = true;
        Ok(())
    }
    fn identity(&self, header: Header) -> Result<(), Error> {
        if header.id().nonce != self.nonce {
            return Err(Error::InvalidIdentity);
        }
        if header.id().generation != self.generation {
            return Err(Error::StaleGeneration);
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn exhaust_sequence(session: &mut Session) {
    session.next_sequence = u64::MAX;
}
