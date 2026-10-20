// SPDX-License-Identifier: GPL-2.0-only
use crate::{Error, HEADER_BYTES, Header, Kind, Limits, Nonce};

/// One complete message. Debug deliberately omits all pixels/control payloads.
pub struct Decoded {
    header: Header,
    payload: Vec<u8>,
}
impl Decoded {
    pub fn header(&self) -> Header {
        self.header
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn capacity(&self) -> usize {
        self.payload.capacity()
    }
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}
impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoded")
            .field("header", &self.header)
            .field("payload_len", &self.payload.len())
            .finish()
    }
}
#[derive(Debug)]
pub struct Feed {
    pub consumed: usize,
    pub message: Option<Decoded>,
}

/// Stops after at most one message, leaving any following input unconsumed.
/// Every error is terminal and releases partial dynamic storage. The caller
/// must also terminate its Session and retain any original Frame for shutdown.
pub struct Decoder {
    limits: Limits,
    nonce: Nonce,
    generation: u64,
    bytes: [u8; HEADER_BYTES],
    header_used: usize,
    header: Option<Header>,
    payload: Vec<u8>,
    closed: bool,
}
impl Decoder {
    pub fn new(nonce: Nonce, generation: u64, limits: Limits) -> Result<Self, Error> {
        limits.validate()?;
        if generation == 0 {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self {
            limits,
            nonce,
            generation,
            bytes: [0; HEADER_BYTES],
            header_used: 0,
            header: None,
            payload: Vec::new(),
            closed: false,
        })
    }
    pub fn retained_bytes(&self) -> usize {
        self.payload.capacity()
    }
    pub fn is_closed(&self) -> bool {
        self.closed
    }
    /// `held` includes the input backing's actual capacity, any previously
    /// decoded message, retained worker Frame and consumer conversion buffers.
    /// It excludes only this decoder's reported retained_bytes(). Fixed header
    /// metadata lives inline and no unbounded queues are created by this API.
    pub fn feed(&mut self, input: &[u8], held: usize) -> Result<Feed, Error> {
        self.feed_checked(input, held, |_| Ok(()))
    }
    /// Endpoint pump uses this with Session::check_incoming_header. Called
    /// exactly once for each completed header, before payload allocation; the
    /// final Session admission is still required after the payload completes.
    pub fn feed_checked(
        &mut self,
        input: &[u8],
        held: usize,
        check: impl FnOnce(Header) -> Result<(), Error>,
    ) -> Result<Feed, Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        let result = self.feed_inner(input, held, check);
        if result.is_err() {
            self.terminate();
        }
        result
    }
    fn feed_inner(
        &mut self,
        input: &[u8],
        held: usize,
        check: impl FnOnce(Header) -> Result<(), Error>,
    ) -> Result<Feed, Error> {
        self.limits.staging(held, self.payload.capacity())?;
        let mut consumed = 0;
        if self.header.is_none() {
            let count = (HEADER_BYTES - self.header_used).min(input.len());
            self.bytes[self.header_used..self.header_used + count].copy_from_slice(&input[..count]);
            self.header_used += count;
            consumed += count;
            if self.header_used != HEADER_BYTES {
                return Ok(Feed {
                    consumed,
                    message: None,
                });
            }
            // No payload allocation is possible until every fixed field has
            // passed validation, including dimensions and exact byte product.
            let header = Header::decode(&self.bytes, self.limits)?;
            if header.id().nonce() != self.nonce {
                return Err(Error::InvalidIdentity);
            }
            if header.id().generation() != self.generation {
                return Err(Error::StaleGeneration);
            }
            check(header)?;
            self.limits.staging(held, header.payload_len())?;
            self.payload
                .try_reserve_exact(header.payload_len())
                .map_err(|_| Error::Allocation)?;
            self.limits.staging(held, self.payload.capacity())?;
            self.header = Some(header);
        }
        let header = self.header.ok_or(Error::InvalidHeader)?;
        let count = (header.payload_len() - self.payload.len()).min(input.len() - consumed);
        self.payload
            .extend_from_slice(&input[consumed..consumed + count]);
        consumed += count;
        if self.payload.len() != header.payload_len() {
            return Ok(Feed {
                consumed,
                message: None,
            });
        }
        header.validate_payload(&self.payload)?;
        let payload = std::mem::take(&mut self.payload);
        self.header = None;
        self.header_used = 0;
        self.bytes = [0; HEADER_BYTES];
        self.closed = header.kind() == Kind::Close;
        Ok(Feed {
            consumed,
            message: Some(Decoded { header, payload }),
        })
    }
    /// EOF never implies acknowledgement. Even a clean boundary terminates
    /// the supervised session; a partial header/payload is distinguished.
    pub fn end_of_stream(&mut self) -> Error {
        if self.closed {
            return Error::Closed;
        }
        let error = if self.header_used != 0 || self.header.is_some() {
            Error::Truncated
        } else {
            Error::EndOfStream
        };
        self.terminate();
        error
    }
    pub fn terminate(&mut self) {
        self.closed = true;
        self.payload = Vec::new();
        self.header = None;
        self.header_used = 0;
        self.bytes = [0; HEADER_BYTES];
    }
    /// Coordinate with Session::advance_generation; do not reinterpret a
    /// partially received message under a replacement window identity.
    pub fn advance_generation(&mut self, generation: u64) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        if self.header_used != 0 || self.header.is_some() {
            return Err(Error::Backpressure);
        }
        if generation <= self.generation {
            return Err(Error::StaleGeneration);
        }
        self.generation = generation;
        Ok(())
    }
}
