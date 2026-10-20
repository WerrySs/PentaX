// SPDX-License-Identifier: GPL-2.0-or-later

//! Image-bound static TLS preparation under explicit PentaX policy.
//!
//! Static blocks precede a modeled guest thread pointer in input-module order.
//! Optional admitted spare storage permits append-only templates without moving
//! the thread pointer. This does not define a PS5 TCB, DTV or execution capability.
//! Template sources are checked logical initialized image bytes; copies include
//! supported relocations from the exact retained plan's staged address space.
//! Ordinary image writes may overlap a PT_TLS tail's numeric virtual range:
//! only the relocated p_filesz prefix is copied, and each TLS tail stays zero.
//!
//! A separate opt-in path additionally applies typed DTPMOD64/DTPOFF64/TPOFF64
//! values computed from this layout for an exact linker TLS binding plan. The
//! ordinary staging and preparation entry points keep their deferred semantics.

mod native;
mod plan;
mod relocated;
mod storage;
#[cfg(test)]
mod tests;

pub(crate) use native::prepare_native_in as prepare_native_tls_in;
pub use native::{PreparedNativeTlsSession, load_native_relocated_static_tls};
pub use plan::plan_static_tls;
pub use relocated::{
    RelocatedStaticTlsPlan, TlsRelocationPatch, load_relocated_static_tls,
    plan_relocated_static_tls,
};
pub use storage::load_symbolic_with_static_tls;

use super::{ProcessError, UnstartedSession};
use crate::memory::{GuestMemory, MemoryError, PAGE_SIZE};
use pentax_linker::{ModuleId, RelocationSource, SymbolRef, SymbolicLinkPlan};
use std::fmt;

/// A bounded PentaX layout rule, not a firmware-derived startup ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaticTlsPolicy {
    /// TLS-bearing modules receive IDs and negative offsets in supplied order.
    PentaXVariantIIInputOrderV1,
}

/// Explicit storage profile, separate from the static-module layout policy.
/// This describes the pinned Kyty self-pointer header, not a complete PS5 TCB/DTV.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThreadControlBlockPolicy {
    #[default]
    None,
    /// A zeroed 64-byte header at TP, with its first u64 equal to guest TP.
    /// A complete RW/NX page backs it; the upper guard follows that page.
    KytyObservedSelfPointerV1,
}

/// Aggregate planning and materialization bounds, retained inside the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticTlsLimits {
    /// Opt-in thread header storage; ordinary preparation keeps no TCB.
    pub thread_control: ThreadControlBlockPolicy,
    pub max_modules: u64,
    pub max_templates: u64,
    pub max_segments: u64,
    pub max_initialized_bytes: u64,
    pub max_static_bytes: u64,
    /// Minimum RW/NX storage below each fixed TP, admitted and charged up
    /// front. Zero preserves exact initial storage. Future templates may use
    /// this capacity without moving old blocks or the thread pointer.
    pub reserved_static_bytes: u64,
    pub max_alignment: u64,
    /// Maximum requested instances, retained by the plan; default 2 is adjustable.
    pub max_instances: u64,
    /// Rounded storage pages across every requested instance; guards stay absent.
    pub max_storage_bytes: u64,
    /// Per-call segment/patch visits, template-byte comparisons and storage
    /// comparisons. Planning and materialization enforce this bound separately.
    /// Copy volume is additionally bounded by initialized/storage/instance limits.
    pub max_work_steps: u64,
}

impl Default for StaticTlsLimits {
    fn default() -> Self {
        Self {
            thread_control: ThreadControlBlockPolicy::None,
            max_modules: 128,
            max_templates: 128,
            max_segments: 65_536,
            max_initialized_bytes: 1024 * 1024,
            max_static_bytes: 4 * 1024 * 1024,
            reserved_static_bytes: 0,
            max_alignment: PAGE_SIZE,
            max_instances: 2,
            max_storage_bytes: 8 * 1024 * 1024,
            max_work_steps: 16_000_000,
        }
    }
}

/// A separate runtime namespace; unrelated to SCE selectors or module positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsModuleId(u64);

impl TlsModuleId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Immutable diagnostics, never accepted as authoritative preparation inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TlsTemplateLayout {
    pub module: ModuleId,
    pub runtime_id: TlsModuleId,
    pub program_header_index: usize,
    pub source_vaddr: u64,
    pub load_bias: u64,
    pub source_address: u64,
    pub initialized_size: u64,
    pub mem_size: u64,
    pub alignment: u64,
    pub alignment_bias: u64,
    /// Positive distance below TP; the signed TP-relative displacement is -O.
    pub tp_offset: u64,
}

/// Private construction retains the exact immutable module set and its limits.
pub struct StaticTlsPlan<'plan, 'image> {
    link: &'plan SymbolicLinkPlan<'image>,
    templates: Vec<TlsTemplateLayout>,
    limits: StaticTlsLimits,
    static_extent: u64,
    maximum_alignment: u64,
    image_pages: Vec<PageRange>,
    image_mapped_bytes: u64,
    /// Normalized file-plus-write page union, extended by relocated planning.
    image_resident_pages: Vec<PageRange>,
    image_resident_bytes: u64,
}

impl fmt::Debug for StaticTlsPlan<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StaticTlsPlan")
            .field("policy", &self.policy())
            .field("template_count", &self.templates.len())
            .field("static_extent", &self.static_extent)
            .field("maximum_alignment", &self.maximum_alignment)
            .finish_non_exhaustive()
    }
}

impl StaticTlsPlan<'_, '_> {
    pub const fn thread_control_policy(&self) -> ThreadControlBlockPolicy {
        self.limits.thread_control
    }
    pub const fn policy(&self) -> StaticTlsPolicy {
        StaticTlsPolicy::PentaXVariantIIInputOrderV1
    }
    pub fn templates(&self) -> &[TlsTemplateLayout] {
        &self.templates
    }
    pub const fn static_extent(&self) -> u64 {
        self.static_extent
    }
    pub const fn maximum_alignment(&self) -> u64 {
        self.maximum_alignment
    }
}

/// Explicit guest TP positions only; no source, template or host pointer inputs.
pub struct StaticTlsStorage<'a> {
    pub thread_pointers: &'a [u64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TlsStorageRange {
    pub address: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TlsBlock {
    pub module: ModuleId,
    pub runtime_id: TlsModuleId,
    pub address: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct StaticTlsInstance {
    pub thread_pointer: u64,
    /// Initialized header extent (64 bytes), backed by one complete mapped page.
    /// Its remaining page bytes stay zero; the profile defines no DTV content.
    pub thread_control_block: Option<TlsStorageRange>,
    /// No static storage when the complete static extent is zero. An explicit
    /// thread-header profile can still request its separate mapping and guards.
    pub storage: Option<TlsStorageRange>,
    pub guard_pages: Option<[u64; 2]>,
    pub blocks: Vec<TlsBlock>,
}

/// Owned unstarted memory with initialized static blocks, without a TCB or DTV.
/// Public access remains read-only and cannot grant CPU or native execution.
#[derive(Debug)]
pub struct PreparedTlsSession {
    // Restricted to process descendants for the closed original fixture only.
    pub(super) session: UnstartedSession,
    templates: Vec<TlsTemplateLayout>,
    instances: Vec<StaticTlsInstance>,
}

impl PreparedTlsSession {
    pub fn memory(&self) -> &GuestMemory {
        self.session.memory()
    }
    pub fn primary_entry(&self) -> u64 {
        self.session.primary_entry()
    }
    pub fn module_count(&self) -> usize {
        self.session.module_count()
    }
    pub fn applied_relocations(&self) -> usize {
        self.session.applied_relocations()
    }
    pub fn templates(&self) -> &[TlsTemplateLayout] {
        &self.templates
    }
    pub fn instances(&self) -> &[StaticTlsInstance] {
        &self.instances
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsBudget {
    Modules,
    Templates,
    Segments,
    InitializedBytes,
    StaticBytes,
    Alignment,
    Instances,
    StorageBytes,
    WorkSteps,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TlsError {
    Budget {
        kind: TlsBudget,
        limit: u64,
    },
    InvalidTemplate {
        module: usize,
        reason: &'static str,
    },
    SourceNotInitialized {
        module: usize,
        address: u64,
    },
    SourceNotReadable {
        module: usize,
        address: u64,
    },
    TemplateBytesMismatch {
        module: usize,
    },
    AddressOverflow,
    InvalidStorage {
        instance: usize,
        reason: &'static str,
    },
    StorageOverlap {
        instance: usize,
    },
    DeferredRelocations {
        count: usize,
    },
    /// Records without a supported TLS binding keep relocated preparation
    /// unavailable; the first original remaining record is retained.
    RemainingDeferredRelocations {
        count: usize,
        module: ModuleId,
        source: RelocationSource,
    },
    /// S+A lies outside [0, p_memsz) of the provider block, or a DTPMOD64
    /// addend is nonzero, under this initial PentaX preparation profile.
    TlsAddendRange {
        consumer: ModuleId,
        source: RelocationSource,
        definition: Option<SymbolRef>,
    },
    /// The signed TP-relative value does not fit in 64 bits.
    TlsValueRange {
        consumer: ModuleId,
        source: RelocationSource,
        definition: Option<SymbolRef>,
    },
    /// The binding's provider PT_TLS identity disagrees with the derived layout.
    TlsProviderMismatch {
        consumer: ModuleId,
        source: RelocationSource,
        definition: Option<SymbolRef>,
    },
    MemoryBudget {
        resident: bool,
        needed: u64,
        limit: u64,
    },
    NativeReservationBudget {
        needed: u64,
        limit: u64,
    },
    NativeGuardOutsideReservation {
        instance: usize,
    },
    NativeAllocationGranularity {
        actual: u64,
    },
    Staging(ProcessError),
    Memory(MemoryError),
    GuardMapped {
        instance: usize,
    },
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget { kind, limit } => {
                write!(f, "static TLS {kind:?} budget {limit} exceeded")
            }
            Self::InvalidTemplate { module, reason } => {
                write!(f, "module {module} static TLS template: {reason}")
            }
            Self::SourceNotInitialized { module, address } => write!(
                f,
                "module {module} TLS source {address:#x} is outside logical initialized image bytes"
            ),
            Self::SourceNotReadable { module, address } => write!(
                f,
                "module {module} TLS source {address:#x} is not readable under the static TLS policy"
            ),
            Self::TemplateBytesMismatch { module } => write!(
                f,
                "module {module} PT_TLS payload differs from its logical initialized image source"
            ),
            Self::AddressOverflow => {
                write!(f, "static TLS address or size exceeds the supported range")
            }
            Self::InvalidStorage { instance, reason } => {
                write!(f, "static TLS instance {instance}: {reason}")
            }
            Self::StorageOverlap { instance } => write!(
                f,
                "static TLS instance {instance} storage or guards overlap an image or another reservation"
            ),
            Self::DeferredRelocations { count } => write!(
                f,
                "static TLS materialization requires complete relocations; {count} records are deferred"
            ),
            Self::RemainingDeferredRelocations {
                count,
                module,
                source,
            } => write!(
                f,
                "relocated static TLS preparation has {count} unbound deferred records, first module {} {source}",
                module.index()
            ),
            Self::TlsAddendRange {
                consumer,
                source,
                definition,
            } => write!(
                f,
                "module {} {source}: TLS addend for module {} symbol {:?} is outside the provider block policy",
                consumer.index(),
                definition.map_or(*consumer, |symbol| symbol.module).index(),
                definition.map(|symbol| symbol.symbol)
            ),
            Self::TlsValueRange {
                consumer,
                source,
                definition,
            } => write!(
                f,
                "module {} {source}: TP-relative value for module {} symbol {:?} exceeds signed 64 bits",
                consumer.index(),
                definition.map_or(*consumer, |symbol| symbol.module).index(),
                definition.map(|symbol| symbol.symbol)
            ),
            Self::TlsProviderMismatch {
                consumer,
                source,
                definition,
            } => write!(
                f,
                "module {} {source}: provider module {} symbol {:?} PT_TLS identity differs from the static TLS layout",
                consumer.index(),
                definition.map_or(*consumer, |symbol| symbol.module).index(),
                definition.map(|symbol| symbol.symbol)
            ),
            Self::MemoryBudget {
                resident,
                needed,
                limit,
            } => write!(
                f,
                "static TLS session needs {needed} {} bytes, exceeding limit {limit}",
                if *resident { "resident" } else { "mapped" }
            ),
            Self::Staging(error) => write!(f, "static TLS image staging: {error}"),
            Self::NativeReservationBudget { needed, limit } => write!(
                f,
                "native static TLS needs {needed} reserved bytes, exceeding limit {limit}"
            ),
            Self::NativeGuardOutsideReservation { instance } => write!(
                f,
                "native static TLS instance {instance} guard lies outside owned reservation geometry"
            ),
            Self::NativeAllocationGranularity { actual } => write!(
                f,
                "native static TLS preparation supports 64 KiB reservation granularity, observed {actual}"
            ),
            Self::Memory(error) => write!(f, "static TLS guest storage: {error}"),
            Self::GuardMapped { instance } => {
                write!(f, "static TLS instance {instance} guard page is mapped")
            }
        }
    }
}

impl std::error::Error for TlsError {}

impl From<MemoryError> for TlsError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}

#[derive(Debug, Clone, Copy)]
struct PageRange {
    start: u64,
    end: u64,
}

impl PageRange {
    fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

struct Work {
    used: u64,
    limit: u64,
}

impl Work {
    fn charge(&mut self, amount: u64) -> Result<(), TlsError> {
        charge(&mut self.used, amount, self.limit, TlsBudget::WorkSteps)
    }
}

fn charge(total: &mut u64, amount: u64, limit: u64, kind: TlsBudget) -> Result<(), TlsError> {
    *total = total
        .checked_add(amount)
        .filter(|value| *value <= limit)
        .ok_or(TlsError::Budget { kind, limit })?;
    Ok(())
}

fn round_pages(size: u64) -> Result<u64, TlsError> {
    size.checked_add(PAGE_SIZE - 1)
        .map(crate::memory::page_base)
        .ok_or(TlsError::AddressOverflow)
}

fn page_range(start: u64, size: u64) -> Result<PageRange, TlsError> {
    let end = start
        .checked_add(size)
        .filter(|end| start < crate::memory::ADDRESS_LIMIT && *end <= crate::memory::ADDRESS_LIMIT)
        .ok_or(TlsError::AddressOverflow)?;
    Ok(PageRange {
        start: crate::memory::page_base(start),
        end: round_pages(end)?,
    })
}

fn merge_ranges(ranges: &mut Vec<PageRange>) -> Result<u64, TlsError> {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut written = 0;
    for index in 0..ranges.len() {
        let range = ranges[index];
        if written != 0 && range.start <= ranges[written - 1].end {
            ranges[written - 1].end = ranges[written - 1].end.max(range.end);
        } else {
            ranges[written] = range;
            written += 1;
        }
    }
    ranges.truncate(written);
    ranges.iter().try_fold(0_u64, |sum, range| {
        sum.checked_add(range.end - range.start)
            .ok_or(TlsError::AddressOverflow)
    })
}
