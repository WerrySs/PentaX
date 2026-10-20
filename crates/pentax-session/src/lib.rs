// SPDX-License-Identifier: GPL-2.0-only
//! Private binary transport metadata, not presentation or guest-memory authority.
//!
//! This crate performs no I/O and supplies no random nonce, display tick or
//! presentation receipt. The endpoint owner supplies those capabilities. A
//! decoded numeric identity never reconstructs an opaque core Frame.

mod decoder;
mod session;
mod wire;

pub use decoder::{Decoded, Decoder, Feed};
pub use session::{ControlEvent, PendingFrame, Session};
pub use wire::{AlphaMode, ColorEncoding, FrameFlags, FrameId, Header, Kind, Nonce};

pub const HEADER_BYTES: usize = 64;
pub const MAX_DIMENSION: u32 = 4096;
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_STAGING_BYTES: usize = 64 * 1024 * 1024;

/// Deliberate transport policy, not a console hardware limit. Callers may only
/// tighten these bounds. `held` arguments count other simultaneous capacities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_dimension: u32,
    pub max_frame_bytes: usize,
    pub max_staging_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_dimension: MAX_DIMENSION,
            max_frame_bytes: MAX_FRAME_BYTES,
            max_staging_bytes: MAX_STAGING_BYTES,
        }
    }
}
impl Limits {
    pub(crate) fn validate(self) -> Result<(), Error> {
        if self.max_dimension == 0
            || self.max_dimension > MAX_DIMENSION
            || self.max_frame_bytes == 0
            || self.max_frame_bytes > MAX_FRAME_BYTES
            || self.max_staging_bytes == 0
            || self.max_staging_bytes > MAX_STAGING_BYTES
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
    pub(crate) fn staging(self, held: usize, capacity: usize) -> Result<(), Error> {
        if held
            .checked_add(capacity)
            .is_none_or(|n| n > self.max_staging_bytes)
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}

/// Closed diagnostics: never include input bytes, nonce values or host paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Closed,
    InvalidHeader,
    UnsupportedVersion,
    UnknownKind,
    InvalidIdentity,
    InvalidFlags,
    InvalidDimensions,
    InvalidLength,
    Limit,
    Allocation,
    Truncated,
    EndOfStream,
    Backpressure,
    UnexpectedSequence,
    StaleGeneration,
    InvalidReceipt,
    DuplicateReceipt,
    InvalidVblank,
    SequenceExhausted,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "frame session: {self:?}")
    }
}
impl std::error::Error for Error {}

#[cfg(test)]
mod tests;
