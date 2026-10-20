// SPDX-License-Identifier: GPL-2.0-or-later

//! Bounded byte-memory services over owned guest memory.
//!
//! Arguments follow the profile's integer registers: full 64-bit RDI/RSI
//! pointer words and an unsigned 64-bit RDX `size_t`. Memset reduces the `int`
//! in ESI to its low byte. Pointer results return RDI unchanged in full RAX.
//! Memcmp compares unsigned bytes and publishes a deterministic -1/0/1 as a
//! 32-bit value with zero upper RAX bits; only the low word's sign is an ABI
//! meaning, and the exact magnitude is PentaX policy.
//!
//! Pointer policy (PentaX, not firmware behavior): a zero length accepts any
//! pointer strictly below `ADDRESS_LIMIT`, including null, without a mapping
//! check. A nonzero length rejects null and requires a checked end at or below
//! the limit plus complete READ (source) or WRITE (destination) coverage.
//! Memcpy refuses every nonempty overlap, including identical spans. Memmove
//! snapshots its complete source, supporting overlap in both directions.
//!
//! Every operation validates both complete spans and fallibly reserves its
//! complete scratch before one whole destination `write_fallible`; nothing is
//! chunk-committed. A failure returns no guest result or errno: the closed
//! gateway stops. Host pointer routines never touch guest addresses.
//!
//! Accounting: attempts and site-lookup work are charged per trap before the
//! site is known, so unknown traps count. Requested bytes (each call's RDX)
//! and conservative work are then charged before any lookup, allocation, read
//! or page zeroing. Work units are byte visits, not CPU time:
//! scratch preparation, source reads, comparisons and destination writes count
//! one unit per byte; each guest page a nonempty destination could touch
//! counts `PAGE_SIZE` units for possible new zeroed backing, using the
//! alignment-independent maximum `(n + 2 * PAGE_SIZE - 2) / PAGE_SIZE`; each
//! operation counts one fixed unit and each trap one unit per catalogue site.
//! Charges are monotonic and survive failed attempts. Scratch limits bound
//! the allocator-reported capacity of argument snapshots and fill buffers;
//! allocator bookkeeping beyond that capacity is not measured. Destination
//! page backing is accounted separately as logical guest resident bytes.
//! Strings use the same retained attempt/byte/work/scratch limits. Their byte
//! charges count examined and snapshot read/write spans instead of unused RDX
//! values; remaining allowances are passed into the bounded string engine
//! before invocation, and consumed charges survive errors. Its fixed call
//! work unit is charged once, independently of the shared site-lookup charge.
//! C++ destructor registration instead retains three opaque words per entry;
//! its DSO word is never an RDX span. Its independent count cap also bounds
//! retained capacity (24 bytes per entry), with at most two bounded buffers
//! during staged growth. Registration does not read memory or invoke a callback.

mod cxa_atexit;
mod heap_trace;
mod snprintf;
#[cfg(test)]
mod tests;

use super::catalog::KnownFunction;
use crate::memory::service::ServiceMemory;
use crate::memory::{ADDRESS_LIMIT, FallibleWriteError, MemoryError, PAGE_SIZE};
use crate::process::hle_strings::{
    self, StringArgument, StringArguments, StringError, StringLimit, StringLimits,
};
use std::fmt;

/// Inclusive per-session service bounds. Defaults are policy, not console maxima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryServiceLimits {
    /// Memory operations: maximum RDX span. Strings: maximum scanned bytes per
    /// argument, including NUL. A short strncpy source may pad a larger count
    /// within the independent total-byte, scratch and work limits.
    pub max_span_bytes: u64,
    /// Reported scratch capacity held simultaneously by one operation.
    pub max_scratch_bytes: u64,
    /// Every trap, including unknown sites and failed attempts.
    pub max_attempts: u64,
    /// Aggregate memory-op RDX lengths plus charged string read/write bytes.
    /// Failed attempts retain any charges consumed before their error.
    pub max_total_bytes: u64,
    pub max_work_units: u64,
    /// Retained C++ destructor registrations and their allocated record slots.
    /// Each slot is three opaque u64 words. This is a PentaX process policy.
    pub max_exit_callbacks: u64,
}

impl Default for MemoryServiceLimits {
    fn default() -> Self {
        Self {
            max_span_bytes: 64 * 1024,
            max_scratch_bytes: 128 * 1024,
            max_attempts: 1024,
            max_total_bytes: 1024 * 1024,
            // Above the worst case admitted by the other defaults
            // (5 MiB byte visits plus two zeroed pages per attempt).
            max_work_units: 48 * 1024 * 1024,
            max_exit_callbacks: 4096,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceLimit {
    SpanBytes,
    ScratchBytes,
    Attempts,
    TotalBytes,
    WorkUnits,
    ExitCallbacks,
}

/// RDI is `First`; RSI is `Second` when it carries a pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerArgument {
    First,
    Second,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MemoryServiceError {
    Memory(MemoryError),
    /// A fallible scratch, registration or destination reservation was refused.
    Allocation,
    NullPointer {
        argument: PointerArgument,
    },
    AddressOverflow {
        argument: PointerArgument,
    },
    UnsupportedOverlap,
    /// The pinned formatter has no recoverable implementation for this case.
    UnsupportedFormat(&'static str),
    InvalidServiceState(&'static str),
    /// A needed string NUL/count/match was not reached within the scan bound.
    UnterminatedString {
        argument: PointerArgument,
        maximum: u64,
    },
    LimitExceeded {
        kind: ServiceLimit,
        requested: u64,
        maximum: u64,
    },
    CounterOverflow {
        kind: ServiceLimit,
    },
}

impl fmt::Display for MemoryServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(error) => write!(f, "guest span: {error}"),
            Self::Allocation => f.write_str("service buffer or page reservation failed"),
            Self::NullPointer { argument } => {
                write!(f, "{argument:?} pointer is null for a nonempty span")
            }
            Self::AddressOverflow { argument } => {
                write!(f, "{argument:?} span exceeds the guest address limit")
            }
            Self::UnsupportedOverlap => {
                f.write_str("copy spans overlap; this policy does not choose a direction")
            }
            Self::UnsupportedFormat(reason) => write!(f, "snprintf format: {reason}"),
            Self::InvalidServiceState(reason) => write!(f, "libc service: {reason}"),
            Self::UnterminatedString { argument, maximum } => {
                write!(f, "{argument:?} string scan exceeds {maximum} bytes")
            }
            Self::LimitExceeded {
                kind,
                requested,
                maximum,
            } => write!(f, "service {kind:?} {requested} exceeds limit {maximum}"),
            Self::CounterOverflow { kind } => write!(f, "service {kind:?} counter overflow"),
        }
    }
}

impl std::error::Error for MemoryServiceError {}

impl From<MemoryError> for MemoryServiceError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

impl From<FallibleWriteError> for MemoryServiceError {
    fn from(error: FallibleWriteError) -> Self {
        match error {
            FallibleWriteError::Memory(error) => Self::Memory(error),
            FallibleWriteError::Allocation => Self::Allocation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AbiArgs {
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
}

/// Original PRE-SYSCALL argument words; absent captures are never reconstructed.
#[derive(Clone, Copy)]
pub(super) struct VariadicContext {
    pub gp: [u64; 3],
    pub vectors: Option<[[u8; 16]; 8]>,
    pub stack_pointer: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ServiceReturn {
    pub rax: u64,
}

/// One charged trap. Only [`ServiceBudget::begin_attempt`] creates it and
/// [`invoke`] consumes it, so one stop dispatches at most once.
#[derive(Debug)]
pub(super) struct Attempt(());

/// Fixed site-comparison charge per trap: one unit per catalogue entry.
pub(super) const SITE_LOOKUP_WORK: u64 = KnownFunction::ALL.len() as u64;

/// Retained limits and monotonic counters for one prepared session.
#[derive(Debug)]
pub(super) struct ServiceBudget {
    limits: MemoryServiceLimits,
    attempts: u64,
    total_bytes: u64,
    work: u64,
    calls: [u64; KnownFunction::ALL.len()],
    string_reads: u64,
    string_writes: u64,
    string_scratch_peak: u64,
    heap_trace_address: Option<u64>,
    exit_callbacks: cxa_atexit::ExitCallbacks,
    #[cfg(test)]
    reservations: u64,
    #[cfg(test)]
    pub(super) refuse_reservation: Option<u64>,
}

impl ServiceBudget {
    pub(super) const fn new(limits: MemoryServiceLimits) -> Self {
        Self {
            limits,
            attempts: 0,
            total_bytes: 0,
            work: 0,
            calls: [0; KnownFunction::ALL.len()],
            string_reads: 0,
            string_writes: 0,
            string_scratch_peak: 0,
            heap_trace_address: None,
            exit_callbacks: cxa_atexit::ExitCallbacks::new(),
            #[cfg(test)]
            reservations: 0,
            #[cfg(test)]
            refuse_reservation: None,
        }
    }
    /// Only preparation of an exact provider installs its process-owned storage.
    pub(super) fn with_provider(mut self, bias: u64) -> Self {
        self.heap_trace_address = bias.checked_add(super::provider::HEAP_TRACE_RVA);
        self
    }
    pub(super) const fn attempts(&self) -> u64 {
        self.attempts
    }
    pub(super) const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    pub(super) const fn work(&self) -> u64 {
        self.work
    }
    pub(super) const fn calls(&self, function: KnownFunction) -> u64 {
        self.calls[function.index()]
    }
    pub(super) fn completed(&self) -> u64 {
        self.calls.iter().sum()
    }
    pub(super) const fn string_reads(&self) -> u64 {
        self.string_reads
    }
    pub(super) const fn string_writes(&self) -> u64 {
        self.string_writes
    }
    pub(super) const fn string_scratch_peak(&self) -> u64 {
        self.string_scratch_peak
    }

    fn string(
        &mut self,
        function: KnownFunction,
        args: AbiArgs,
        memory: &mut impl ServiceMemory,
    ) -> Result<ServiceReturn, MemoryServiceError> {
        let operation = function.string_operation().expect("string catalogue route");
        let before_bytes = self.total_bytes;
        let before_work = self.work;
        let limits = StringLimits {
            max_scan_bytes: self.limits.max_span_bytes,
            max_bytes: self.limits.max_total_bytes - before_bytes,
            max_work_units: self.limits.max_work_units - before_work,
            max_scratch_bytes: self.limits.max_scratch_bytes,
        };
        let (result, usage) = hle_strings::invoke(
            operation,
            StringArguments {
                first: args.rdi,
                second: args.rsi,
                third: args.rdx,
            },
            memory,
            &limits,
        )
        .into_parts();
        // Private StringUsage can only be constructed by the bounded engine,
        // which charges these remaining allowances before reads/allocations/
        // writes. These additions therefore cannot exceed the retained caps;
        // there is no late budget refusal after a successful guest write.
        self.total_bytes += usage.bytes();
        self.work += usage.work_units();
        self.string_reads += usage.read_bytes();
        self.string_writes += usage.write_bytes();
        self.string_scratch_peak = self.string_scratch_peak.max(usage.scratch_peak_bytes());
        let rax = result.map_err(|error| self.string_error(error, before_bytes, before_work))?;
        Ok(self.complete(function, rax))
    }

    fn string_error(
        &self,
        error: StringError,
        before_bytes: u64,
        before_work: u64,
    ) -> MemoryServiceError {
        let pointer = |argument| match argument {
            StringArgument::First => PointerArgument::First,
            StringArgument::Second => PointerArgument::Second,
        };
        let kind = |limit| match limit {
            StringLimit::Bytes => ServiceLimit::TotalBytes,
            StringLimit::WorkUnits => ServiceLimit::WorkUnits,
            StringLimit::ScratchBytes => ServiceLimit::ScratchBytes,
        };
        match error {
            StringError::Memory(error) => MemoryServiceError::Memory(error),
            StringError::Allocation => MemoryServiceError::Allocation,
            StringError::NullPointer { argument } => MemoryServiceError::NullPointer {
                argument: pointer(argument),
            },
            StringError::AddressOverflow { argument } => MemoryServiceError::AddressOverflow {
                argument: pointer(argument),
            },
            StringError::Unterminated { argument, limit } => {
                MemoryServiceError::UnterminatedString {
                    argument: pointer(argument),
                    maximum: limit,
                }
            }
            StringError::UnsupportedOverlap => MemoryServiceError::UnsupportedOverlap,
            StringError::CounterOverflow { kind: limit } => {
                MemoryServiceError::CounterOverflow { kind: kind(limit) }
            }
            StringError::LimitExceeded {
                kind: limit,
                requested,
                maximum,
            } => {
                let (used, maximum) = match limit {
                    StringLimit::Bytes => (before_bytes, self.limits.max_total_bytes),
                    StringLimit::WorkUnits => (before_work, self.limits.max_work_units),
                    StringLimit::ScratchBytes => (0, maximum),
                };
                match used.checked_add(requested) {
                    Some(requested) => MemoryServiceError::LimitExceeded {
                        kind: kind(limit),
                        requested,
                        maximum,
                    },
                    None => MemoryServiceError::CounterOverflow { kind: kind(limit) },
                }
            }
        }
    }

    /// Count one trap and its site lookup before the site is inspected.
    pub(super) fn begin_attempt(&mut self) -> Result<Attempt, MemoryServiceError> {
        let attempts = total(
            self.attempts,
            1,
            self.limits.max_attempts,
            ServiceLimit::Attempts,
        )?;
        let work = total(
            self.work,
            SITE_LOOKUP_WORK,
            self.limits.max_work_units,
            ServiceLimit::WorkUnits,
        )?;
        self.attempts = attempts;
        self.work = work;
        Ok(Attempt(()))
    }

    /// Native PRE-marker admission additionally visits the complete eight-byte
    /// stub and checks three live permission/ownership conditions before use.
    pub(super) fn charge_native_attestation(&mut self) -> Result<(), MemoryServiceError> {
        self.work = total(
            self.work,
            super::provider::STUB_SIZE + 3,
            self.limits.max_work_units,
            ServiceLimit::WorkUnits,
        )?;
        Ok(())
    }

    /// Pure span/scratch checks, then requested bytes and work together.
    fn admit(&mut self, function: KnownFunction, length: u64) -> Result<(), MemoryServiceError> {
        let limits = self.limits;
        within(length, limits.max_span_bytes, ServiceLimit::SpanBytes)?;
        let buffers = if function == KnownFunction::Memcmp {
            2
        } else {
            1
        };
        let scratch = length
            .checked_mul(buffers)
            .ok_or(MemoryServiceError::CounterOverflow {
                kind: ServiceLimit::ScratchBytes,
            })?;
        within(
            scratch,
            limits.max_scratch_bytes,
            ServiceLimit::ScratchBytes,
        )?;
        let total_bytes = total(
            self.total_bytes,
            length,
            limits.max_total_bytes,
            ServiceLimit::TotalBytes,
        )?;
        let cost = work(function, length).ok_or(MemoryServiceError::CounterOverflow {
            kind: ServiceLimit::WorkUnits,
        })?;
        let work = total(
            self.work,
            cost,
            limits.max_work_units,
            ServiceLimit::WorkUnits,
        )?;
        self.total_bytes = total_bytes;
        self.work = work;
        Ok(())
    }

    /// Reserve one zeroed scratch buffer; `held` is this operation's reported
    /// capacity so far. Spare capacity counts against the scratch limit.
    fn reserve(&mut self, length: u64, held: &mut u64) -> Result<Vec<u8>, MemoryServiceError> {
        let size = usize::try_from(length).map_err(|_| MemoryServiceError::Allocation)?;
        let mut buffer = Vec::new();
        #[cfg(test)]
        {
            let index = self.reservations;
            self.reservations += 1;
            if self.refuse_reservation == Some(index) {
                return Err(MemoryServiceError::Allocation);
            }
        }
        buffer
            .try_reserve_exact(size)
            .map_err(|_| MemoryServiceError::Allocation)?;
        let capacity = held.checked_add(buffer.capacity() as u64).ok_or(
            MemoryServiceError::CounterOverflow {
                kind: ServiceLimit::ScratchBytes,
            },
        )?;
        within(
            capacity,
            self.limits.max_scratch_bytes,
            ServiceLimit::ScratchBytes,
        )?;
        *held = capacity;
        // Within reserved capacity; no further allocation occurs.
        buffer.resize(size, 0);
        Ok(buffer)
    }

    fn complete(&mut self, function: KnownFunction, rax: u64) -> ServiceReturn {
        // Each completion consumed one checked attempt, so this cannot overflow.
        self.calls[function.index()] += 1;
        ServiceReturn { rax }
    }
}

fn within(value: u64, maximum: u64, kind: ServiceLimit) -> Result<(), MemoryServiceError> {
    if value > maximum {
        return Err(MemoryServiceError::LimitExceeded {
            kind,
            requested: value,
            maximum,
        });
    }
    Ok(())
}

fn total(
    used: u64,
    amount: u64,
    maximum: u64,
    kind: ServiceLimit,
) -> Result<u64, MemoryServiceError> {
    let requested = used
        .checked_add(amount)
        .ok_or(MemoryServiceError::CounterOverflow { kind })?;
    within(requested, maximum, kind)?;
    Ok(requested)
}

/// Conservative units for one admitted operation; see the module documentation.
pub(super) fn work(function: KnownFunction, length: u64) -> Option<u64> {
    let visits = match function {
        // Scratch zeroing, source read and destination write.
        KnownFunction::Memcpy | KnownFunction::Memmove => length.checked_mul(3)?,
        // Fill preparation and destination write.
        KnownFunction::Memset => length.checked_mul(2)?,
        // Two scratch buffers, two full reads and one comparison pass.
        KnownFunction::Memcmp => length.checked_mul(5)?,
        KnownFunction::HeapGetTraceInfo | KnownFunction::HeapGetTraceInfoSce => length
            .checked_mul(3)?
            .checked_add(super::provider::HEAP_TRACE_BYTES)?,
        _ => return None, // String costs are charged as their bounded scans progress.
    };
    // Memcmp never writes; an empty write touches no page.
    let zeroing = if function == KnownFunction::Memcmp || length == 0 {
        0
    } else {
        let pages = length.checked_add(2 * PAGE_SIZE - 2)? / PAGE_SIZE;
        pages.checked_mul(PAGE_SIZE)?
    };
    visits.checked_add(zeroing)?.checked_add(1)
}

fn check_pointer(
    pointer: u64,
    length: u64,
    argument: PointerArgument,
) -> Result<(), MemoryServiceError> {
    if length == 0 {
        return if pointer < ADDRESS_LIMIT {
            Ok(())
        } else {
            Err(MemoryServiceError::AddressOverflow { argument })
        };
    }
    if pointer == 0 {
        return Err(MemoryServiceError::NullPointer { argument });
    }
    match pointer.checked_add(length) {
        Some(end) if end <= ADDRESS_LIMIT => Ok(()),
        _ => Err(MemoryServiceError::AddressOverflow { argument }),
    }
}

/// Execute one registered operation for a charged trap.
///
/// No guest byte changes unless the complete destination write succeeds; a
/// returned error publishes no result and no completion count.
pub(super) fn invoke(
    attempt: Attempt,
    function: KnownFunction,
    args: AbiArgs,
    memory: &mut impl ServiceMemory,
    budget: &mut ServiceBudget,
) -> Result<ServiceReturn, MemoryServiceError> {
    invoke_with_variadic(attempt, function, args, memory, budget, None)
}

pub(super) fn invoke_with_variadic(
    _attempt: Attempt,
    function: KnownFunction,
    args: AbiArgs,
    memory: &mut impl ServiceMemory,
    budget: &mut ServiceBudget,
    variadic: Option<VariadicContext>,
) -> Result<ServiceReturn, MemoryServiceError> {
    if function == KnownFunction::Snprintf {
        return snprintf::invoke(args, variadic, memory, budget);
    }
    if function == KnownFunction::CxaAtExit {
        return budget.cxa_atexit(args);
    }
    if function.string_operation().is_some() {
        return budget.string(function, args, memory);
    }
    if matches!(
        function,
        KnownFunction::HeapGetTraceInfo | KnownFunction::HeapGetTraceInfoSce
    ) {
        return budget.heap_trace_info(function, args.rdi, memory);
    }
    let length = args.rdx;
    budget.admit(function, length)?;
    let mut held = 0;
    match function {
        KnownFunction::Memcpy | KnownFunction::Memmove => {
            let (destination, source) = (args.rdi, args.rsi);
            check_pointer(destination, length, PointerArgument::First)?;
            check_pointer(source, length, PointerArgument::Second)?;
            if length != 0 {
                if function == KnownFunction::Memcpy
                    && destination < source + length
                    && source < destination + length
                {
                    return Err(MemoryServiceError::UnsupportedOverlap);
                }
                memory.preflight_read(source, length)?;
                memory.preflight_write(destination, length)?;
                let mut snapshot = budget.reserve(length, &mut held)?;
                memory.read(source, &mut snapshot)?;
                memory.write_fallible(destination, &snapshot)?;
            }
            Ok(budget.complete(function, destination))
        }
        KnownFunction::Memset => {
            let destination = args.rdi;
            check_pointer(destination, length, PointerArgument::First)?;
            if length != 0 {
                memory.preflight_write(destination, length)?;
                let mut fill = budget.reserve(length, &mut held)?;
                fill.fill(args.rsi as u8);
                memory.write_fallible(destination, &fill)?;
            }
            Ok(budget.complete(function, destination))
        }
        KnownFunction::Memcmp => {
            let (first, second) = (args.rdi, args.rsi);
            check_pointer(first, length, PointerArgument::First)?;
            check_pointer(second, length, PointerArgument::Second)?;
            let mut result = 0_i32;
            if length != 0 {
                memory.preflight_read(first, length)?;
                memory.preflight_read(second, length)?;
                let mut left = budget.reserve(length, &mut held)?;
                let mut right = budget.reserve(length, &mut held)?;
                memory.read(first, &mut left)?;
                memory.read(second, &mut right)?;
                if let Some((a, b)) = left.iter().zip(&right).find(|(a, b)| a != b) {
                    result = if a < b { -1 } else { 1 };
                }
            }
            Ok(budget.complete(function, u64::from(result as u32)))
        }
        _ => unreachable!("string operations dispatched above"),
    }
}

#[cfg(test)]
use crate::memory::GuestMemory;
