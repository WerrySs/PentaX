// SPDX-License-Identifier: GPL-2.0-or-later

//! Flat, single-pass inspection with transactional commits.

use crate::{
    Header, InspectError, LimitKind, Limits, RECORD_STORAGE_BYTES, StopReason, Storage,
    limits::{bytes_for, ensure_within, reserve_exact},
    packet::{self, Kind},
    report::{Accounting, Inspection, Outcome, Record},
};

/// Copies `bytes` into an owned buffer and inspects it as a flat command
/// stream of little-endian dwords.
///
/// Before any allocation, the length must be a multiple of four and within
/// [`Limits::max_input_bytes`]. The input and the record vector are then each
/// reserved exactly once; reserved capacity is checked against its byte limit,
/// and neither vector grows afterwards. Allocation failure is reported as
/// [`InspectError::AllocationFailed`] with no report.
///
/// Each packet is then handled in a fixed order: charge its header to the
/// visited budget, classify the header (type, opcode, flags, count), check
/// that the input holds the framed length, charge the remaining dwords, check
/// the body, and finally check record capacity and commit. The first failure
/// is returned as [`Outcome::Stopped`] at that header, with no record or
/// committed dwords for the packet. Work is linear in the input length.
///
/// # Errors
///
/// Returns [`InspectError`] when the input is unaligned, too large, cannot be
/// sized on this host or cannot be reserved. Packet-level problems are never
/// errors: they end the returned report with a positioned stop.
pub fn inspect(bytes: &[u8], limits: &Limits) -> Result<Inspection, InspectError> {
    let length = u64::try_from(bytes.len()).map_err(|_| InspectError::HostSizeOverflow)?;
    if !length.is_multiple_of(4) {
        return Err(InspectError::UnalignedLength { bytes: length });
    }
    ensure_within(LimitKind::InputBytes, length, limits.max_input_bytes)?;

    let mut input = reserve_exact::<u8>(Storage::Input, bytes.len())?;
    let input_bytes = bytes_for::<u8>(input.capacity());
    ensure_within(LimitKind::InputBytes, input_bytes, limits.max_input_bytes)?;
    input.extend_from_slice(bytes);

    let dwords = input.as_chunks::<4>().0;
    let capacity = record_capacity(dwords.len(), limits)?;
    let records = reserve_exact::<Record>(Storage::Records, capacity)?;
    let record_bytes = bytes_for::<Record>(records.capacity());
    ensure_within(
        LimitKind::RecordBytes,
        record_bytes,
        limits.max_record_bytes,
    )?;

    let mut walk = Walk {
        dwords,
        records,
        capacity,
        cursor: 0,
        visited: 0,
        limits,
    };
    let outcome = walk.run();
    let Walk {
        records,
        cursor,
        visited,
        ..
    } = walk;
    Ok(Inspection::new(
        input,
        records,
        outcome,
        cursor,
        visited,
        Accounting::new(input_bytes, capacity, record_bytes),
    ))
}

/// Every committed record covers at least one dword, so the input length also
/// bounds the useful reservation.
fn record_capacity(input_dwords: usize, limits: &Limits) -> Result<usize, InspectError> {
    let input_dwords = u64::try_from(input_dwords).map_err(|_| InspectError::HostSizeOverflow)?;
    let by_bytes = limits.max_record_bytes / RECORD_STORAGE_BYTES;
    let capacity = input_dwords.min(limits.max_records).min(by_bytes);
    usize::try_from(capacity).map_err(|_| InspectError::HostSizeOverflow)
}

struct Walk<'a> {
    dwords: &'a [[u8; 4]],
    /// Reserved with at least `capacity` slots; pushes never reallocate.
    records: Vec<Record>,
    capacity: usize,
    /// Committed dwords; also the next header position.
    cursor: usize,
    visited: u64,
    limits: &'a Limits,
}

impl Walk<'_> {
    fn run(&mut self) -> Outcome {
        loop {
            let dwords = self.dwords;
            let remaining = dwords.get(self.cursor..).unwrap_or_default();
            let Some(first) = remaining.first() else {
                return Outcome::CompleteFlatSubset;
            };
            if let Err(reason) = self.step(u32::from_le_bytes(*first), remaining) {
                return Outcome::Stopped {
                    at: self.cursor,
                    reason,
                };
            }
        }
    }

    /// Examines the packet at the cursor; `remaining` starts at its header.
    fn step(&mut self, raw: u32, remaining: &[[u8; 4]]) -> Result<(), StopReason> {
        self.charge(1)?;
        let header = Header::new(raw);
        let framing = packet::classify(header)?;
        let Some(span) = remaining.get(..usize::from(framing.dwords)) else {
            return Err(StopReason::Truncated {
                header,
                packet_dwords: framing.dwords,
                available_dwords: remaining.len(),
            });
        };
        self.charge(u64::from(framing.dwords.saturating_sub(1)))?;
        let kind = packet::decode_body(header, framing, span)?;
        self.commit(header, kind)
    }

    fn charge(&mut self, dwords: u64) -> Result<(), StopReason> {
        let maximum = self.limits.max_visited_dwords;
        match self.visited.checked_add(dwords) {
            Some(visited) if visited <= maximum => {
                self.visited = visited;
                Ok(())
            }
            requested => Err(StopReason::Resource {
                limit: LimitKind::VisitedDwords,
                requested: requested.unwrap_or(u64::MAX),
                maximum,
            }),
        }
    }

    fn commit(&mut self, header: Header, kind: Kind) -> Result<(), StopReason> {
        if self.records.len() >= self.capacity {
            return Err(self.records_exhausted());
        }
        let at = self.cursor;
        self.records.push(Record { at, header, kind });
        self.cursor = at.saturating_add(usize::from(kind.dwords()));
        Ok(())
    }

    /// Names the budget that set the record capacity. The input length can
    /// never be the binding bound, because each record covers a dword.
    fn records_exhausted(&self) -> StopReason {
        let requested = u64::try_from(self.records.len()).map_or(u64::MAX, |n| n.saturating_add(1));
        if requested > self.limits.max_records {
            StopReason::Resource {
                limit: LimitKind::Records,
                requested,
                maximum: self.limits.max_records,
            }
        } else {
            StopReason::Resource {
                limit: LimitKind::RecordBytes,
                requested: requested.saturating_mul(RECORD_STORAGE_BYTES),
                maximum: self.limits.max_record_bytes,
            }
        }
    }
}
