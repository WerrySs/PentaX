// SPDX-License-Identifier: GPL-2.0-or-later

//! Owned inspection report and owner-bound packet views.

use std::{fmt, ops::Range, slice};

use crate::{Header, RegisterSpace, StopReason, packet::Kind};

/// One committed packet. Fixed size: payloads stay in the owned input and are
/// reached only through views borrowing the same report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub(crate) at: usize,
    pub(crate) header: Header,
    pub(crate) kind: Kind,
}

/// How a flat inspection ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every input dword belongs to an admitted packet. This describes syntax
    /// only and is not permission to execute anything.
    CompleteFlatSubset,
    /// Inspection ended at the header at dword `at`, which equals the
    /// committed dword count. The earlier prefix is diagnostic only and
    /// cannot be resumed.
    Stopped { at: usize, reason: StopReason },
}

/// Heap capacity owned by an [`Inspection`].
///
/// Both vectors are reserved once, exactly, before any packet is examined,
/// and never grow, so these values are also the peak heap storage. Allocator
/// bookkeeping and rounding inside the allocator are outside this accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accounting {
    input_bytes: u64,
    record_capacity: usize,
    record_bytes: u64,
}

impl Accounting {
    pub(crate) const fn new(input_bytes: u64, record_capacity: usize, record_bytes: u64) -> Self {
        Self {
            input_bytes,
            record_capacity,
            record_bytes,
        }
    }

    /// Reserved capacity of the owned input copy.
    #[must_use]
    pub const fn input_capacity_bytes(self) -> u64 {
        self.input_bytes
    }

    /// Records this inspection may commit:
    /// `min(input dwords, max_records, max_record_bytes / RECORD_STORAGE_BYTES)`.
    #[must_use]
    pub const fn record_capacity(self) -> usize {
        self.record_capacity
    }

    /// Reserved record vector capacity times [`RECORD_STORAGE_BYTES`](crate::RECORD_STORAGE_BYTES).
    #[must_use]
    pub const fn record_capacity_bytes(self) -> u64 {
        self.record_bytes
    }

    /// Input plus record capacity, saturating.
    #[must_use]
    pub const fn owned_capacity_bytes(self) -> u64 {
        self.input_bytes.saturating_add(self.record_bytes)
    }
}

/// Result of [`inspect`](crate::inspect): the owned input, its committed
/// records and how inspection ended.
///
/// Fields are private and immutable. Packet views borrow this report, so they
/// can only describe its own input. `Debug` prints a summary, never the input.
#[derive(PartialEq, Eq)]
pub struct Inspection {
    input: Vec<u8>,
    records: Vec<Record>,
    outcome: Outcome,
    committed_dwords: usize,
    visited_dwords: u64,
    accounting: Accounting,
}

impl Inspection {
    pub(crate) const fn new(
        input: Vec<u8>,
        records: Vec<Record>,
        outcome: Outcome,
        committed_dwords: usize,
        visited_dwords: u64,
        accounting: Accounting,
    ) -> Self {
        Self {
            input,
            records,
            outcome,
            committed_dwords,
            visited_dwords,
            accounting,
        }
    }

    #[must_use]
    pub const fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        matches!(self.outcome, Outcome::CompleteFlatSubset)
    }

    /// Committed packets in input order.
    #[must_use]
    pub fn packets(&self) -> Packets<'_> {
        Packets {
            records: self.records.iter(),
            dwords: self.dwords(),
        }
    }

    #[must_use]
    pub fn packet(&self, index: usize) -> Option<Packet<'_>> {
        self.records.get(index).map(|record| Packet {
            record,
            dwords: self.dwords(),
        })
    }

    /// Committed packets, excluding the packet at any stop.
    #[must_use]
    pub const fn committed_packets(&self) -> usize {
        self.records.len()
    }

    /// Dwords covered by committed packets, excluding the packet at any stop.
    #[must_use]
    pub const fn committed_dwords(&self) -> usize {
        self.committed_dwords
    }

    /// Dwords charged to the visited budget, including those of the packet
    /// that stopped inspection.
    #[must_use]
    pub const fn visited_dwords(&self) -> u64 {
        self.visited_dwords
    }

    /// The owned input, for private diagnosis.
    #[must_use]
    pub fn input(&self) -> &[u8] {
        &self.input
    }

    #[must_use]
    pub const fn input_dwords(&self) -> usize {
        self.input.len() / 4
    }

    #[must_use]
    pub const fn accounting(&self) -> Accounting {
        self.accounting
    }

    fn dwords(&self) -> &[[u8; 4]] {
        self.input.as_chunks::<4>().0
    }
}

impl fmt::Debug for Inspection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Inspection")
            .field("input_dwords", &self.input_dwords())
            .field("committed_packets", &self.records.len())
            .field("committed_dwords", &self.committed_dwords)
            .field("visited_dwords", &self.visited_dwords)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// Iterator over committed packets.
#[derive(Clone)]
pub struct Packets<'a> {
    records: slice::Iter<'a, Record>,
    dwords: &'a [[u8; 4]],
}

impl<'a> Iterator for Packets<'a> {
    type Item = Packet<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let dwords = self.dwords;
        self.records.next().map(|record| Packet { record, dwords })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl ExactSizeIterator for Packets<'_> {}

impl fmt::Debug for Packets<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Packets")
            .field("remaining", &self.records.len())
            .finish()
    }
}

/// A committed packet, borrowed from its [`Inspection`].
#[derive(Clone, Copy)]
pub struct Packet<'a> {
    record: &'a Record,
    dwords: &'a [[u8; 4]],
}

impl<'a> Packet<'a> {
    /// Dword index of the header in the owned input.
    #[must_use]
    pub const fn at(self) -> usize {
        self.record.at
    }

    #[must_use]
    pub const fn header(self) -> Header {
        self.record.header
    }

    /// Packet length, header included.
    #[must_use]
    pub const fn dwords(self) -> usize {
        self.record.kind.dwords() as usize
    }

    #[must_use]
    pub const fn dword_range(self) -> Range<usize> {
        self.record.at..self.record.at.saturating_add(self.dwords())
    }

    /// The packet's original bytes, header included.
    #[must_use]
    pub fn bytes(self) -> &'a [u8] {
        span(self.dwords, self.record.at, self.dwords()).as_flattened()
    }

    #[must_use]
    pub fn body(self) -> PacketBody<'a> {
        let at = self.record.at;
        match self.record.kind {
            Kind::Padding => PacketBody::Padding,
            Kind::HeaderOnlyNop => PacketBody::HeaderOnlyNop,
            Kind::Nop { body_dwords } => {
                let first = at.saturating_add(1);
                PacketBody::Nop(NopPayload {
                    first,
                    words: span(self.dwords, first, usize::from(body_dwords)),
                })
            }
            Kind::Register {
                space,
                offset,
                values,
            } => {
                let first = at.saturating_add(2);
                PacketBody::RegisterWrite(RegisterWrite {
                    space,
                    offset,
                    first,
                    words: span(self.dwords, first, usize::from(values)),
                })
            }
        }
    }
}

impl fmt::Debug for Packet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Packet")
            .field("at", &self.at())
            .field("header", &self.header())
            .field("body", &self.body())
            .finish()
    }
}

/// Typed content of a committed packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketBody<'a> {
    /// The exact type-2 word 0x80000000.
    Padding,
    /// A zero-flag NOP whose count is 0x3fff: one header dword, no body.
    HeaderOnlyNop,
    /// A zero-flag ordinary NOP with an uninterpreted payload.
    Nop(NopPayload<'a>),
    /// An encoded relative register-write intent; not an applied write.
    RegisterWrite(RegisterWrite<'a>),
}

/// An ordinary NOP payload, retained as opaque bytes of the owned input.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NopPayload<'a> {
    first: usize,
    words: &'a [[u8; 4]],
}

impl<'a> NopPayload<'a> {
    #[must_use]
    pub fn bytes(self) -> &'a [u8] {
        self.words.as_flattened()
    }

    /// Dword range in the owned input.
    #[must_use]
    pub const fn dword_range(self) -> Range<usize> {
        self.first..self.first.saturating_add(self.words.len())
    }
}

impl fmt::Debug for NopPayload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NopPayload")
            .field("dwords", &self.dword_range())
            .finish()
    }
}

/// An encoded direct relative register-write intent.
///
/// Values are the original dwords of the owned input. Nothing records whether
/// the write would take effect, what the register is or what it held before.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RegisterWrite<'a> {
    space: RegisterSpace,
    offset: u16,
    first: usize,
    words: &'a [[u8; 4]],
}

impl<'a> RegisterWrite<'a> {
    #[must_use]
    pub const fn space(self) -> RegisterSpace {
        self.space
    }

    /// Relative index of the first value. The offset word's high 16 bits are
    /// zero for every admitted write.
    #[must_use]
    pub const fn offset(self) -> u16 {
        self.offset
    }

    /// Relative indices written, ending at or before 0x10000.
    #[must_use]
    pub const fn relative_indices(self) -> Range<u32> {
        let start = self.offset as u32;
        start..start + self.words.len() as u32
    }

    /// Number of values; at least one for every admitted write.
    #[must_use]
    pub const fn len(self) -> usize {
        self.words.len()
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.words.is_empty()
    }

    #[must_use]
    pub fn value(self, index: usize) -> Option<u32> {
        self.words.get(index).map(|word| u32::from_le_bytes(*word))
    }

    pub fn values(self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.words.iter().map(|word| u32::from_le_bytes(*word))
    }

    /// Original value bytes, little-endian.
    #[must_use]
    pub fn value_bytes(self) -> &'a [u8] {
        self.words.as_flattened()
    }

    /// Dword range of the values in the owned input.
    #[must_use]
    pub const fn value_range(self) -> Range<usize> {
        self.first..self.first.saturating_add(self.words.len())
    }
}

impl fmt::Debug for RegisterWrite<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisterWrite")
            .field("space", &self.space)
            .field("offset", &self.offset)
            .field("values", &self.value_range())
            .finish()
    }
}

/// `count` dwords from `first`; empty only if a record invariant were broken.
fn span(dwords: &[[u8; 4]], first: usize, count: usize) -> &[[u8; 4]] {
    first
        .checked_add(count)
        .and_then(|end| dwords.get(first..end))
        .unwrap_or_default()
}
