// SPDX-License-Identifier: GPL-2.0-only
// Rust adaptation, 2026-10-17: bounded guest-memory string services.
// Adapted behavior and interfaces: KytyPS5/KytyPS5 src/libs/libC.cpp,
// revision 763d3be3a9bc4e5a068d7cf52015030d6c03308f, and the authorized
// KytyPS5-Performance fork, revision 209dffe9032d72749257ce18680d506b891284de.
// Upstream is GPL-2.0-only and retains the following original Kyty notice:
//
// MIT License
// Copyright (c) 2021 InoriRus
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
// The retained MIT notice does not relicense the GPL adaptation above.

//! Nine string behaviors from the observed LibcInternal profile, adapted to
//! checked guest bytes. No host pointer or import-resolution authority is used.
//!
//! Source: <https://github.com/KytyPS5/KytyPS5/blob/763d3be3a9bc4e5a068d7cf52015030d6c03308f/src/libs/libC.cpp>.
//! Register arguments are RDI/RSI/RDX; `int` character arguments reduce ESI to
//! its low byte. Comparisons return normalized -1/0/1 in low signed EAX, with
//! upper RAX zero. This normalization is policy; libc specifies the sign.
//!
//! Nonempty accesses reject null and require checked guest ranges. For
//! `strncmp`/`strncpy` count zero, both pointers must be below ADDRESS_LIMIT,
//! including null, but mappings are unused. Scans stop at the needed NUL,
//! mismatch, match or count. An unterminated scan at its bound is an explicit
//! subset refusal. All copying functions reject overlapping source/destination
//! strings or spans; undefined C overlap is not assigned a new copy direction.
//!
//! One call holds the exclusive memory borrow throughout. Write services scan
//! and snapshot every needed source byte, validate the whole destination and
//! reserve complete scratch before exactly one `write_fallible`. Returned
//! errors cannot partially modify guest bytes, residency or permissions.
//!
//! Usage is charged before work and retained on errors. Bytes count charged
//! read/write spans (including repeated snapshot reads and failed accesses).
//! Work counts one fixed call unit, byte accesses, scan/comparison tests,
//! scratch zeroing, and a conservative destination-page preparation allowance.
//! These are bounded units, not CPU time. Scratch counts simultaneously held
//! allocator-reported capacities. An oversized reported reservation is refused
//! before use; allocator bookkeeping/transient failed reservations are not an
//! exact host heap bound. Guest page backing has its separate resident limit.

#[cfg(test)]
use crate::memory::GuestMemory;
use crate::memory::MemoryError;
use crate::memory::service::ServiceMemory;
use std::fmt;

mod engine;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringOperation {
    Strcmp,
    Strncmp,
    Strlen,
    Strcpy,
    Strncpy,
    Strcat,
    Strchr,
    Strrchr,
    Strstr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StringArguments {
    pub first: u64,
    pub second: u64,
    pub third: u64,
}

/// Inclusive remaining allowances supplied by the retained session budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StringLimits {
    /// Per argument: maximum examined bytes, including a terminating NUL.
    pub max_scan_bytes: u64,
    /// Combined charged read and write bytes, including failed attempts.
    pub max_bytes: u64,
    pub max_work_units: u64,
    pub max_scratch_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringArgument {
    First,
    Second,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringLimit {
    Bytes,
    WorkUnits,
    ScratchBytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StringError {
    Memory(MemoryError),
    Allocation,
    NullPointer {
        argument: StringArgument,
    },
    AddressOverflow {
        argument: StringArgument,
    },
    Unterminated {
        argument: StringArgument,
        limit: u64,
    },
    UnsupportedOverlap,
    LimitExceeded {
        kind: StringLimit,
        requested: u64,
        maximum: u64,
    },
    CounterOverflow {
        kind: StringLimit,
    },
}

impl fmt::Display for StringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(error) => write!(f, "guest string: {error}"),
            Self::Allocation => f.write_str("string scratch or guest-page reservation failed"),
            Self::NullPointer { argument } => write!(f, "{argument:?} string pointer is null"),
            Self::AddressOverflow { argument } => {
                write!(f, "{argument:?} string span exceeds guest addresses")
            }
            Self::Unterminated { argument, limit } => {
                write!(f, "{argument:?} string scan exceeds {limit} bytes")
            }
            Self::UnsupportedOverlap => f.write_str("overlapping string copy is unsupported"),
            Self::LimitExceeded {
                kind,
                requested,
                maximum,
            } => write!(f, "string {kind:?} {requested} exceeds {maximum}"),
            Self::CounterOverflow { kind } => write!(f, "string {kind:?} counter overflow"),
        }
    }
}

impl std::error::Error for StringError {}

impl From<MemoryError> for StringError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct StringUsage {
    read_bytes: u64,
    write_bytes: u64,
    work_units: u64,
    scratch_peak_bytes: u64,
}

impl StringUsage {
    pub(crate) const fn read_bytes(&self) -> u64 {
        self.read_bytes
    }
    pub(crate) const fn write_bytes(&self) -> u64 {
        self.write_bytes
    }
    /// The combined total was checked against max_bytes before either field changed.
    pub(crate) const fn bytes(&self) -> u64 {
        self.read_bytes + self.write_bytes
    }
    pub(crate) const fn work_units(&self) -> u64 {
        self.work_units
    }
    pub(crate) const fn scratch_peak_bytes(&self) -> u64 {
        self.scratch_peak_bytes
    }
}

#[derive(Debug)]
pub(crate) struct StringOutcome {
    result: Result<u64, StringError>,
    usage: StringUsage,
}

impl StringOutcome {
    pub(crate) fn into_parts(self) -> (Result<u64, StringError>, StringUsage) {
        (self.result, self.usage)
    }
}

pub(crate) fn invoke(
    operation: StringOperation,
    arguments: StringArguments,
    memory: &mut impl ServiceMemory,
    limits: &StringLimits,
) -> StringOutcome {
    engine::invoke_with_gate(operation, arguments, memory, limits, &mut || Ok(()))
}
