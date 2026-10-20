// SPDX-License-Identifier: GPL-2.0-or-later

//! Inspection budgets and exact, fallible storage reservation.

use std::fmt;

use crate::{InspectError, Storage, report::Record};

const MIB: u64 = 1024 * 1024;

/// Bytes charged for each reserved record slot on this host.
///
/// Record storage is a single exact reservation of fixed-size records, so its
/// accounted size is slot capacity times this value. Payloads and register
/// values are never copied into records.
pub const RECORD_STORAGE_BYTES: u64 = size_of::<Record>() as u64;

/// Inclusive upper bounds for one inspection.
///
/// Input and record storage are checked before allocation; the visited-dword
/// and record bounds are enforced before each packet is examined further or
/// committed. Zero limits are honoured: they admit only empty input or stop at
/// the first packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Bytes of the owned input copy.
    pub max_input_bytes: u64,
    /// Committed records, one per admitted packet.
    pub max_records: u64,
    /// Reserved record storage, in multiples of [`RECORD_STORAGE_BYTES`].
    pub max_record_bytes: u64,
    /// Dwords examined, including those of a packet that then stops.
    pub max_visited_dwords: u64,
}

impl Limits {
    /// PentaX policy, not a hardware or driver limit: 16 MiB of input,
    /// 262,144 records, 16 MiB of record storage and 4,194,304 visited dwords.
    pub const DEFAULT: Self = Self {
        max_input_bytes: 16 * MIB,
        max_records: 262_144,
        max_record_bytes: 16 * MIB,
        max_visited_dwords: 4_194_304,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A budget named by [`Limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKind {
    InputBytes,
    Records,
    RecordBytes,
    VisitedDwords,
}

impl LimitKind {
    /// The [`Limits`] field controlling this budget.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::InputBytes => "max_input_bytes",
            Self::Records => "max_records",
            Self::RecordBytes => "max_record_bytes",
            Self::VisitedDwords => "max_visited_dwords",
        }
    }
}

impl fmt::Display for LimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputBytes => "input bytes",
            Self::Records => "record count",
            Self::RecordBytes => "record storage bytes",
            Self::VisitedDwords => "visited dwords",
        })
    }
}

pub(crate) const fn ensure_within(
    limit: LimitKind,
    requested: u64,
    maximum: u64,
) -> Result<(), InspectError> {
    if requested > maximum {
        Err(InspectError::LimitExceeded {
            limit,
            requested,
            maximum,
        })
    } else {
        Ok(())
    }
}

/// Bytes occupied by `count` elements of `T`, saturating at `u64::MAX`.
pub(crate) fn bytes_for<T>(count: usize) -> u64 {
    u64::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(size_of::<T>() as u64))
        .unwrap_or(u64::MAX)
}

/// An empty vector with exactly `count` slots requested from the allocator.
/// Callers check the resulting capacity against their byte limit.
pub(crate) fn reserve_exact<T>(storage: Storage, count: usize) -> Result<Vec<T>, InspectError> {
    let mut vector = Vec::new();
    vector
        .try_reserve_exact(count)
        .map_err(|_| InspectError::AllocationFailed {
            storage,
            bytes: bytes_for::<T>(count),
        })?;
    Ok(vector)
}
