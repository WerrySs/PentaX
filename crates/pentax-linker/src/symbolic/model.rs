// SPDX-License-Identifier: GPL-2.0-or-later

use pentax_loader::{ExecutableView, RuntimeLoadPlan};

use crate::relative::LogicalRange;
use crate::{Limits, RelocationSource, SceLimits, SceMetadata};

/// Immutable plan identity. Copies/clones retain it; a separately planned
/// source gets another identity. This is not publication or execution evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceIdentity(u64);
impl SourceIdentity {
    pub(super) fn next() -> Result<Self, super::SymbolicError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map(Self)
        .map_err(|_| super::SymbolicError::SourceIdentityExhausted)
    }
}

/// One immutable executable and its retained guest load bias. Input zero is primary.
#[derive(Debug, Clone)]
pub struct ModuleInput<'a> {
    pub(super) view: ExecutableView<'a>,
    pub(super) load_bias: u64,
}

impl<'a> ModuleInput<'a> {
    #[must_use]
    pub const fn new(view: ExecutableView<'a>, load_bias: u64) -> Self {
        Self { view, load_bias }
    }
}

/// Stable identity within this plan, assigned in input order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(pub(super) usize);

impl ModuleId {
    #[must_use]
    pub const fn index(self) -> usize {
        self.0
    }
}

/// An original symbol table position in one supplied image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolRef {
    pub module: ModuleId,
    pub symbol: usize,
}

/// Why an examined definition supplies no provider under this policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderExclusionReason {
    /// Both selectors occur only in the image's needed-module/import-library
    /// roles. This does not classify the definition's body or resolution.
    NotOfferedImportScope,
}

/// An image-bound provider exclusion, without copied names or an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderExclusion {
    pub(super) symbol: SymbolRef,
    pub(super) reason: ProviderExclusionReason,
}

impl ProviderExclusion {
    #[must_use]
    pub const fn symbol(&self) -> SymbolRef {
        self.symbol
    }
    #[must_use]
    pub const fn reason(&self) -> ProviderExclusionReason {
        self.reason
    }
}

/// Exact decoded descriptor versions; compatibility is never inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescriptorVersions {
    pub module_major: u8,
    pub module_minor: u8,
    pub library: u16,
}

/// Ordinary-address handling of an absent, declared WEAK import. TLS and
/// version/type conflicts remain errors. This never creates callable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MissingWeakPolicy {
    #[default]
    Reject,
    /// ELF zero symbol value; R_X86_64_64 still applies its explicit addend.
    NullAddressV1,
}

/// Bounds for the complete session, rather than independent per-image allowances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolicLimits {
    pub missing_weak: MissingWeakPolicy,
    /// Aggregate table bytes, symbol names, symbols and relocations.
    pub inspection: Limits,
    /// Aggregate dynamic entries, descriptor records and metadata name bytes.
    /// The single-name limit continues to apply to each metadata name.
    pub sce: SceLimits,
    pub max_modules: u64,
    /// Examined qualified provider candidates, including unoffered records.
    pub max_exports: u64,
    pub max_patches: u64,
    /// Bytes of owned module/library names in provider and query keys.
    pub max_key_bytes: u64,
    /// Descriptor/identity comparisons, candidate visits, ordered-index
    /// operations and retained provider-exclusion diagnostics.
    pub max_lookup_steps: u64,
}

impl Default for SymbolicLimits {
    fn default() -> Self {
        Self {
            missing_weak: MissingWeakPolicy::Reject,
            inspection: Limits::DEFAULT,
            sce: SceLimits::DEFAULT,
            max_modules: 128,
            max_exports: 1_000_000,
            max_patches: 1_000_000,
            max_key_bytes: 64 * 1024 * 1024,
            max_lookup_steps: 16_000_000,
        }
    }
}

/// The origin of a value, distinct from the consumer's relocation record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Definition(SymbolRef),
    Absolute(SymbolRef),
    Relative,
    /// ELF symbol index zero denotes a literal S=0; this is not an unresolved import.
    ZeroSymbolIndex,
    /// Explicit opt-in missing-WEAK S=0, distinct from the reserved index zero
    /// and from any provider definition or unavailable callable marker.
    UnresolvedWeak(SymbolRef),
}

/// A checked eight-byte little-endian write, never applied by the linker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolicPatch {
    pub(super) consumer: ModuleId,
    pub(super) address: u64,
    pub(super) value: u64,
    pub(super) source: RelocationSource,
    pub(super) resolution: Resolution,
}

/// One checked metadata transition, not evidence that live memory was changed.
/// Only an opt-in append planner can construct these parent/next-bound records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackTransition {
    pub(super) parent: SourceIdentity,
    pub(super) next: SourceIdentity,
    pub(super) old: SymbolicPatch,
    pub(super) new: SymbolicPatch,
    pub(super) old_provider: SymbolRef,
    pub(super) new_provider: SymbolRef,
}

impl FallbackTransition {
    #[must_use]
    pub const fn parent_identity(&self) -> SourceIdentity {
        self.parent
    }
    #[must_use]
    pub const fn next_identity(&self) -> SourceIdentity {
        self.next
    }
    #[must_use]
    pub const fn old_patch(&self) -> SymbolicPatch {
        self.old
    }
    #[must_use]
    pub const fn new_patch(&self) -> SymbolicPatch {
        self.new
    }
    #[must_use]
    pub const fn old_provider(&self) -> SymbolRef {
        self.old_provider
    }
    #[must_use]
    pub const fn new_provider(&self) -> SymbolRef {
        self.new_provider
    }
}

impl SymbolicPatch {
    #[must_use]
    pub const fn consumer(&self) -> ModuleId {
        self.consumer
    }
    #[must_use]
    pub const fn address(&self) -> u64 {
        self.address
    }
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }
    #[must_use]
    pub const fn source(&self) -> RelocationSource {
        self.source
    }
    #[must_use]
    pub const fn resolution(&self) -> Resolution {
        self.resolution
    }
}

/// A structurally inspected record with no invented width, target or value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionDeferredRelocation {
    pub(super) module: ModuleId,
    pub(super) source: RelocationSource,
}

impl SessionDeferredRelocation {
    #[must_use]
    pub const fn module(&self) -> ModuleId {
        self.module
    }
    #[must_use]
    pub const fn source(&self) -> RelocationSource {
        self.source
    }
}

/// Runtime mappings and inspected records derived from the same original image.
#[derive(Clone)]
pub struct PlannedModule<'a> {
    pub(super) input: &'a [u8],
    pub(super) id: ModuleId,
    pub(super) runtime: RuntimeLoadPlan<'a>,
    pub(super) load_bias: u64,
    pub(super) metadata: Option<SceMetadata>,
    pub(super) ranges: Vec<LogicalRange>,
}

impl std::fmt::Debug for PlannedModule<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlannedModule")
            .field("id", &self.id)
            .field("runtime", &self.runtime)
            .field("load_bias", &self.load_bias)
            .field("metadata", &self.metadata)
            .field("ranges", &self.ranges)
            .finish_non_exhaustive()
    }
}

impl<'a> PlannedModule<'a> {
    /// Original inspected input, before relocations. This is read-only source
    /// provenance, not a live file/memory or execution capability.
    #[must_use]
    pub const fn source_bytes(&self) -> &'a [u8] {
        self.input
    }
    #[must_use]
    pub const fn id(&self) -> ModuleId {
        self.id
    }
    #[must_use]
    pub const fn runtime_plan(&self) -> &RuntimeLoadPlan<'a> {
        &self.runtime
    }
    #[must_use]
    pub const fn load_bias(&self) -> u64 {
        self.load_bias
    }
}

/// A whole-session read-only plan. Construction and all mutable fields are private.
///
/// Only the supplied image bytes define records and providers. No module discovery,
/// host pointers, guest writes, TLS setup or execution is performed. The exact
/// descriptor version policy is deliberately narrower than a platform ABI.
#[derive(Debug, Clone)]
pub struct SymbolicLinkPlan<'a> {
    pub(super) identity: SourceIdentity,
    pub(super) parent: Option<SourceIdentity>,
    pub(super) appended_guest: Option<ModuleId>,
    pub(super) modules: Vec<PlannedModule<'a>>,
    pub(super) patches: Vec<SymbolicPatch>,
    pub(super) deferred: Vec<SessionDeferredRelocation>,
    pub(super) provider_exclusions: Vec<ProviderExclusion>,
    pub(super) primary_entry: u64,
    pub(super) retired_fallbacks: Vec<SymbolRef>,
    pub(super) fallback_transitions: Vec<FallbackTransition>,
}

impl<'a> SymbolicLinkPlan<'a> {
    #[must_use]
    pub const fn identity(&self) -> SourceIdentity {
        self.identity
    }
    /// Set only by a checked append which preserves its predecessor, with any
    /// approved function fallback changes described by exact transition records.
    #[must_use]
    pub const fn parent_identity(&self) -> Option<SourceIdentity> {
        self.parent
    }
    /// First newly added source in this checked append. An optional following
    /// delta provider is metadata, not the application's module or initializer.
    #[must_use]
    pub const fn appended_guest(&self) -> Option<ModuleId> {
        self.appended_guest
    }
    /// Approved retired providers persist through later source appends.
    #[must_use]
    pub fn retired_fallbacks(&self) -> &[SymbolRef] {
        &self.retired_fallbacks
    }
    /// Only this append's changed old patches, sorted by destination address.
    #[must_use]
    pub fn fallback_transitions(&self) -> &[FallbackTransition] {
        &self.fallback_transitions
    }
    #[must_use]
    pub fn modules(&self) -> &[PlannedModule<'a>] {
        &self.modules
    }
    /// Nonoverlapping writes sorted by biased guest address.
    #[must_use]
    pub fn patches(&self) -> &[SymbolicPatch] {
        &self.patches
    }
    #[must_use]
    pub fn deferred(&self) -> &[SessionDeferredRelocation] {
        &self.deferred
    }
    #[must_use]
    pub fn patch_count(&self) -> usize {
        self.patches.len()
    }
    #[must_use]
    pub fn deferred_count(&self) -> usize {
        self.deferred.len()
    }
    /// Exclusions in input-module/symbol order within the provider scan only.
    /// Images without an export-library scope and undefined, local, hidden,
    /// internal, unnamed or unqualified symbols are outside this scan.
    #[must_use]
    pub fn provider_exclusions(&self) -> &[ProviderExclusion] {
        &self.provider_exclusions
    }
    #[must_use]
    pub fn provider_exclusion_count(&self) -> usize {
        self.provider_exclusions.len()
    }
    /// Checked biased entry in module zero's own executable logical bytes.
    #[must_use]
    pub const fn primary_entry(&self) -> u64 {
        self.primary_entry
    }
}
