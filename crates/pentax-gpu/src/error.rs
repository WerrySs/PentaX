// SPDX-License-Identifier: GPL-2.0-or-later

//! Positioned terminal stops and failures before ownership.

use std::{error::Error, fmt};

use crate::{Header, IndirectBuffer, LimitKind};

/// Broad class of a [`StopReason`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StopCategory {
    /// Outside the admitted subset or profile. This does not claim that
    /// hardware or firmware would reject the packet.
    Unsupported,
    /// Structurally inconsistent with the admitted grammar.
    Malformed,
    /// The framed packet extends past the end of the input.
    Truncated,
    /// An inspection budget was reached before the packet could be committed.
    Resource,
    /// A well-formed common indirect-buffer reference, observed but never
    /// followed.
    IndirectReference,
}

/// Why inspection stopped at a packet header.
///
/// The packet at the stop position contributes no record and no committed
/// dwords. Variants that read the header retain it raw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StopReason {
    /// The visited-dword or record budget would be exceeded. `requested` is
    /// the total that this packet would need.
    Resource {
        limit: LimitKind,
        requested: u64,
        maximum: u64,
    },
    /// Type 0 or type 1. These families are not admitted; they are not
    /// labeled invalid.
    UnsupportedPacketType { header: Header },
    /// A type-2 word other than the exact padding word 0x80000000.
    UnsupportedPadding { header: Header },
    /// A type-3 opcode outside the subset, including state clears, draws,
    /// dispatches, waits, synchronization, memory writes and predication.
    UnsupportedOpcode { header: Header },
    /// A NOP with any nonzero bit in its low header byte.
    UnsupportedNopProfile { header: Header },
    /// A register-write header with any of bits 7:3 set.
    UnsupportedHeaderEncoding { header: Header },
    /// A predicated register write. No intent is recorded, and nothing is
    /// assumed about whether the predicate would be active.
    PredicatedWrite { header: Header },
    /// The offset word's high 16 bits are nonzero. They are not masked or
    /// interpreted as a selector.
    UnsupportedRegisterSelector { header: Header, offset_word: u32 },
    /// An indirect-buffer opcode whose count does not frame the common
    /// four-dword form. No presumed field is read.
    UnsupportedIndirectBufferLength { header: Header },
    /// The header frames `packet_dwords`, but only `available_dwords` remain,
    /// counting the header.
    Truncated {
        header: Header,
        packet_dwords: u16,
        available_dwords: usize,
    },
    /// A register write with a count of zero carries an offset but no value.
    EmptyRegisterWrite { header: Header },
    /// `offset + values` passes the end of the 16-bit relative index domain.
    RegisterRangeOverflow {
        header: Header,
        offset: u16,
        values: u16,
    },
    /// An indirect-buffer reference whose address low word sets bits 1:0.
    MisalignedIndirectBuffer(IndirectBuffer),
    /// A four-dword indirect-buffer reference. Inspection always ends here,
    /// without reading the target or committing the reference.
    IndirectBufferReference(IndirectBuffer),
}

impl StopReason {
    #[must_use]
    pub const fn category(self) -> StopCategory {
        match self {
            Self::Resource { .. } => StopCategory::Resource,
            Self::UnsupportedPacketType { .. }
            | Self::UnsupportedPadding { .. }
            | Self::UnsupportedOpcode { .. }
            | Self::UnsupportedNopProfile { .. }
            | Self::UnsupportedHeaderEncoding { .. }
            | Self::PredicatedWrite { .. }
            | Self::UnsupportedRegisterSelector { .. }
            | Self::UnsupportedIndirectBufferLength { .. } => StopCategory::Unsupported,
            Self::Truncated { .. } => StopCategory::Truncated,
            Self::EmptyRegisterWrite { .. }
            | Self::RegisterRangeOverflow { .. }
            | Self::MisalignedIndirectBuffer(_) => StopCategory::Malformed,
            Self::IndirectBufferReference(_) => StopCategory::IndirectReference,
        }
    }

    /// The raw header, unless the stop happened before it was charged.
    #[must_use]
    pub const fn header(self) -> Option<Header> {
        match self {
            Self::Resource { .. } => None,
            Self::UnsupportedPacketType { header }
            | Self::UnsupportedPadding { header }
            | Self::UnsupportedOpcode { header }
            | Self::UnsupportedNopProfile { header }
            | Self::UnsupportedHeaderEncoding { header }
            | Self::PredicatedWrite { header }
            | Self::UnsupportedRegisterSelector { header, .. }
            | Self::UnsupportedIndirectBufferLength { header }
            | Self::Truncated { header, .. }
            | Self::EmptyRegisterWrite { header }
            | Self::RegisterRangeOverflow { header, .. } => Some(header),
            Self::MisalignedIndirectBuffer(reference)
            | Self::IndirectBufferReference(reference) => Some(reference.header()),
        }
    }
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Resource {
                limit,
                requested,
                maximum,
            } => write!(
                f,
                "{limit} {requested} exceeds Limits::{} = {maximum}",
                limit.field()
            ),
            Self::UnsupportedPacketType { header } => write!(
                f,
                "type-{} header {:#010x} is outside the admitted subset",
                header.packet_type(),
                header.raw()
            ),
            Self::UnsupportedPadding { header } => write!(
                f,
                "type-2 word {:#010x} is not the exact padding word 0x80000000",
                header.raw()
            ),
            Self::UnsupportedOpcode { header } => write!(
                f,
                "type-3 opcode {:#04x} in header {:#010x} is not admitted",
                header.opcode(),
                header.raw()
            ),
            Self::UnsupportedNopProfile { header } => write!(
                f,
                "NOP header {:#010x} sets low-byte bits; only zero-flag NOPs are admitted",
                header.raw()
            ),
            Self::UnsupportedHeaderEncoding { header } => write!(
                f,
                "register-write header {:#010x} sets bits 7:3",
                header.raw()
            ),
            Self::PredicatedWrite { header } => write!(
                f,
                "register-write header {:#010x} is predicated; no intent is recorded",
                header.raw()
            ),
            Self::UnsupportedRegisterSelector {
                header,
                offset_word,
            } => write!(
                f,
                "register offset word {offset_word:#010x} after header {:#010x} has nonzero high 16 bits",
                header.raw()
            ),
            Self::UnsupportedIndirectBufferLength { header } => write!(
                f,
                "indirect-buffer header {:#010x} has count {}, not the common four-dword form",
                header.raw(),
                header.count()
            ),
            Self::Truncated {
                header,
                packet_dwords,
                available_dwords,
            } => write!(
                f,
                "header {:#010x} frames {packet_dwords} dwords, but only {available_dwords} remain",
                header.raw()
            ),
            Self::EmptyRegisterWrite { header } => write!(
                f,
                "register-write header {:#010x} carries no values",
                header.raw()
            ),
            Self::RegisterRangeOverflow {
                header,
                offset,
                values,
            } => write!(
                f,
                "register-write header {:#010x} writes {values} values from relative index {offset:#06x}, past 0x10000",
                header.raw()
            ),
            Self::MisalignedIndirectBuffer(reference) => write!(
                f,
                "indirect-buffer address low word {:#010x} sets alignment bits",
                reference.address_low_raw()
            ),
            Self::IndirectBufferReference(reference) => write!(
                f,
                "indirect-buffer reference of {} dwords (chain {}) is observed only and not followed",
                reference.size_dwords(),
                reference.chain()
            ),
        }
    }
}

/// Owned storage reserved before inspection work starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Storage {
    Input,
    Records,
}

impl fmt::Display for Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Input => "input",
            Self::Records => "record",
        })
    }
}

/// Why no inspection report was produced. Nothing is partially returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InspectError {
    /// The input is not a whole number of dwords. Checked before limits and
    /// before any allocation.
    UnalignedLength { bytes: u64 },
    /// Owned storage would exceed a byte limit. Checked before allocation,
    /// and again against the capacity actually reserved.
    LimitExceeded {
        limit: LimitKind,
        requested: u64,
        maximum: u64,
    },
    /// A size cannot be represented on this host.
    HostSizeOverflow,
    /// The allocator refused an exact reservation. `bytes` saturates at
    /// `u64::MAX` when the request itself is unrepresentable.
    AllocationFailed { storage: Storage, bytes: u64 },
}

impl fmt::Display for InspectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::UnalignedLength { bytes } => {
                write!(f, "input length {bytes} is not a multiple of four bytes")
            }
            Self::LimitExceeded {
                limit,
                requested,
                maximum,
            } => write!(
                f,
                "{limit} {requested} exceeds Limits::{} = {maximum}; raise it only for trusted input",
                limit.field()
            ),
            Self::HostSizeOverflow => f.write_str("a size cannot be represented on this host"),
            Self::AllocationFailed { storage, bytes } => {
                write!(f, "could not reserve {bytes} bytes of {storage} storage")
            }
        }
    }
}

impl Error for InspectError {}
