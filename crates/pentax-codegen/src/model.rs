// SPDX-License-Identifier: GPL-2.0-only

use crate::{Code, ConditionCode, CpuidFeature, Mnemonic, Register, TranslationPolicy};
use iced_x86::Instruction;
use pentax_linker::{ModuleId, SymbolicLinkPlan};
use std::fmt;

/// Numeric placement request, not an allocation, pointer or execution capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePlacement {
    pub address: u64,
    pub capacity: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockLimits {
    pub max_modules: u64,
    pub max_segments: u64,
    /// Aggregate ordinary writes plus declared writes on the materialized path.
    pub max_patches: u64,
    pub max_source_bytes: u64,
    /// Includes a decoded terminating instruction, when present.
    pub max_instructions: u64,
    /// Includes the final two-byte UD2.
    pub max_output_bytes: u64,
    /// Reported capacities of per-plan Vec buffers, encoder scratch and the
    /// fixed immutable admission summary.
    /// Dependency-global tables/allocator bookkeeping are outside this bound.
    pub max_allocation_bytes: u64,
    pub max_work_units: u64,
}
impl Default for BlockLimits {
    fn default() -> Self {
        Self {
            max_modules: 128,
            max_segments: 65_536,
            max_patches: 1_000_000,
            max_source_bytes: 4096,
            max_instructions: 256,
            max_output_bytes: 4096,
            max_allocation_bytes: 1024 * 1024,
            max_work_units: 16_000_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Budget {
    Modules,
    Segments,
    Patches,
    SourceBytes,
    Instructions,
    OutputBytes,
    AllocationBytes,
    Work,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnsupportedReason {
    SegmentPrefix,
    RepeatOrLockPrefix,
    Privileged,
    SaveRestore,
    InstructionClass,
    OperandClass,
    AddressSize,
    MissingFeature(CpuidFeature),
    FarOrNon64Transfer,
    CounterBranch,
    SystemState,
}

/// Descriptive operand derived only from the terminating source instruction.
/// RIP-relative operands retain their original guest address, not the cache IP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryOperand {
    pub(crate) base: Register,
    pub(crate) index: Register,
    pub(crate) scale: u32,
    pub(crate) displacement: u64,
    pub(crate) ip_relative: Option<u64>,
    pub(crate) segment: Register,
}
impl MemoryOperand {
    pub const fn base(&self) -> Register {
        self.base
    }
    pub const fn index(&self) -> Register {
        self.index
    }
    pub const fn scale(&self) -> u32 {
        self.scale
    }
    /// Raw displacement bits; register-relative displacement is signed by the
    /// decoded instruction's address width. See `BlockExit::instruction()`.
    pub const fn displacement(&self) -> u64 {
        self.displacement
    }
    pub const fn ip_relative_address(&self) -> Option<u64> {
        self.ip_relative
    }
    pub const fn segment(&self) -> Register {
        self.segment
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTarget {
    Direct(u64),
    Register(Register),
    Memory(MemoryOperand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExitKind {
    /// No source instruction is consumed: resume at this guest PC.
    Fallthrough {
        target: u64,
    },
    Call {
        target: ControlTarget,
        return_pc: u64,
    },
    Jump {
        target: ControlTarget,
    },
    Conditional {
        condition: ConditionCode,
        target: u64,
        fallthrough: u64,
    },
    /// Dispatcher must read the original guest stack, check target and update
    /// RSP by 8+adjust only on success. No native RET is emitted.
    Return {
        adjust: u16,
    },
    Syscall,
    Interrupt {
        mnemonic: Mnemonic,
    },
    GuestException {
        code: Code,
    },
    /// Dispatcher must refuse or implement the exact retained instruction.
    Mediated {
        reason: UnsupportedReason,
    },
}

/// One privately constructed exit. Its UD2 belongs to this source-bound plan.
#[derive(Debug)]
pub struct BlockExit {
    pub(crate) guest_pc: u64,
    pub(crate) next_guest_pc: u64,
    pub(crate) cache_ip: u64,
    pub(crate) kind: ExitKind,
    pub(crate) instruction: Option<Instruction>,
}
impl BlockExit {
    pub const fn guest_pc(&self) -> u64 {
        self.guest_pc
    }
    pub const fn next_guest_pc(&self) -> u64 {
        self.next_guest_pc
    }
    pub const fn cache_ip(&self) -> u64 {
        self.cache_ip
    }
    pub const fn kind(&self) -> ExitKind {
        self.kind
    }
    /// Immutable original decode, including exact source operands/prefixes.
    /// `None` identifies a bounded fallthrough split, not a decoded instruction.
    pub const fn instruction(&self) -> Option<&Instruction> {
        self.instruction.as_ref()
    }
}

/// Per-instruction guest/cache PC provenance; constructors stay private.
#[derive(Debug)]
pub struct InstructionRecord {
    pub(crate) guest_pc: u64,
    pub(crate) source_offset: usize,
    pub(crate) source_len: usize,
    pub(crate) cache_offset: usize,
    pub(crate) emitted_len: usize,
    pub(crate) instruction: Instruction,
    pub(crate) rip_target: Option<u64>,
    // Inline immutable decoded metadata is admitted with this Vec's requested
    // and actual capacities. Nonemitted exits never need an operand template.
    pub(crate) memory_template: Option<crate::memory::InstructionMemoryTemplate>,
}
impl InstructionRecord {
    /// Numeric instruction classification, not execution or memory authority.
    /// LOCK RMW and implicit-lock memory XCHG require isolated live admission.
    pub fn has_atomic_memory(&self) -> bool {
        crate::policy::atomic_memory(&self.instruction)
    }
    pub const fn guest_pc(&self) -> u64 {
        self.guest_pc
    }
    pub const fn source_offset(&self) -> usize {
        self.source_offset
    }
    pub const fn source_len(&self) -> usize {
        self.source_len
    }
    pub const fn cache_offset(&self) -> usize {
        self.cache_offset
    }
    /// Zero for the terminating source instruction, which is never emitted.
    pub const fn emitted_len(&self) -> usize {
        self.emitted_len
    }
    pub const fn instruction(&self) -> &Instruction {
        &self.instruction
    }
    /// Allocation-free summary of the retained decoded template, including
    /// implicit memory and written GPR aliases. None means unknown; this is
    /// neither a live address/permission check nor native execution authority.
    pub fn immutable_memory_summary(&self) -> Option<(bool, crate::MemoryRegisterDependencies)> {
        self.memory_template
            .as_ref()
            .map(|template| (template.has_memory(), template.register_dependencies()))
    }
    pub const fn rip_relative_target(&self) -> Option<u64> {
        self.rip_target
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockUsage {
    pub source_bytes: u64,
    pub decoded_instructions: u64,
    pub emitted_bytes: u64,
    pub allocation_bytes: u64,
    pub work_units: u64,
}

/// Fixed decoded facts, never live source, operand or host-feature authority.
/// Constructed only from the final retained records; no separate cache exists.
#[derive(Debug)]
pub struct NativeAdmissionSummary {
    pub(crate) emitted_instructions: u64,
    pub(crate) feature_visits: u64,
    pub(crate) required_features: [u64; 4],
    pub(crate) dispatch_work_units: u64,
    pub(crate) unrepresentable_feature: bool,
    pub(crate) memory_free: bool,
    pub(crate) atomic_memory: bool,
    pub(crate) streaming_profile: bool,
}
impl NativeAdmissionSummary {
    pub const fn emitted_instructions(&self) -> u64 {
        self.emitted_instructions
    }
    /// Original all-record feature-list visits, including the nonemitted exit.
    pub const fn feature_visits(&self) -> u64 {
        self.feature_visits
    }
    /// Prior dispatch charge, excluding this summary's one-time construction.
    pub const fn dispatch_work_units(&self) -> u64 {
        self.dispatch_work_units
    }
    /// The caller supplies independently admitted host capabilities. Unknown
    /// feature indices fail closed instead of truncating the required set.
    pub fn features_supported_by(&self, available: &[u64; 4]) -> bool {
        !self.unrepresentable_feature
            && self
                .required_features
                .iter()
                .zip(available)
                .all(|(need, have)| *need & !*have == 0)
    }
    /// Every emitted analyzer would succeed with no accesses or address
    /// dependencies. Unknown templates and potential-memory forms are excluded.
    pub const fn is_memory_free(&self) -> bool {
        self.memory_free
    }
    pub const fn has_atomic_memory(&self) -> bool {
        self.atomic_memory
    }
    pub const fn requires_streaming_profile(&self) -> bool {
        self.streaming_profile
    }
}

/// Immutable bytes, owned source snapshots, source/module identity and policy.
/// There is no constructor from detached exit tables, byte buffers or addresses.
pub struct BlockPlan<'plan, 'image> {
    pub(crate) source: &'plan SymbolicLinkPlan<'image>,
    pub(crate) module: ModuleId,
    pub(crate) guest_pc: u64,
    pub(crate) original: Vec<u8>,
    pub(crate) relocated: Vec<u8>,
    pub(crate) bytes: Vec<u8>,
    pub(crate) records: Vec<InstructionRecord>,
    pub(crate) exit: BlockExit,
    pub(crate) placement: CachePlacement,
    pub(crate) policy: TranslationPolicy,
    pub(crate) emits_non_temporal_store: bool,
    pub(crate) emits_vector_memory_move: bool,
    pub(crate) emits_scalar_float_memory: bool,
    pub(crate) emits_float_min_max_memory: bool,
    pub(crate) emits_vector_broadcast_memory: bool,
    pub(crate) admission: NativeAdmissionSummary,
    pub(crate) usage: BlockUsage,
}
impl fmt::Debug for BlockPlan<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockPlan")
            .field("module", &self.module)
            .field("guest_pc", &self.guest_pc)
            .field("exit", &self.exit)
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}
impl<'plan, 'image> BlockPlan<'plan, 'image> {
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn module(&self) -> ModuleId {
        self.module
    }
    pub const fn guest_pc(&self) -> u64 {
        self.guest_pc
    }
    /// Complete immutable decode window before ordinary source relocations.
    pub fn original_bytes(&self) -> &[u8] {
        &self.original
    }
    /// Same window with source-bound ordinary writes applied before decoding.
    pub fn relocated_source_bytes(&self) -> &[u8] {
        &self.relocated
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn instructions(&self) -> &[InstructionRecord] {
        &self.records
    }
    pub const fn exit(&self) -> &BlockExit {
        &self.exit
    }
    pub const fn placement(&self) -> CachePlacement {
        self.placement
    }
    pub const fn policy(&self) -> TranslationPolicy {
        self.policy
    }
    /// Immutable emitted-body classification only, never memory authority.
    /// An actual native consumer must fence these stores before publishing
    /// completion; a nonemitted terminating store does not set this flag.
    pub const fn emits_non_temporal_store(&self) -> bool {
        self.emits_non_temporal_store
    }
    /// Exact emitted temporal-memory classification for manual-path preflight.
    /// It grants neither backing authority nor a non-temporal fence requirement.
    pub const fn emits_vector_memory_move(&self) -> bool {
        self.emits_vector_memory_move
    }
    /// Actual emitted scalar ADD/SUB/MUL reads require full live preflight,
    /// including manual execution without an independent-memory policy request.
    pub const fn emits_scalar_float_memory(&self) -> bool {
        self.emits_scalar_float_memory
    }
    /// Only actually emitted MIN/MAX memory reads set this flag. Manual entry
    /// must independently check their full live spans and address dependencies.
    pub const fn emits_float_min_max_memory(&self) -> bool {
        self.emits_float_min_max_memory
    }
    /// True only for actually emitted closed VEX broadcast memory forms.
    /// This is immutable classification, not live read or CPU authority.
    pub const fn emits_vector_broadcast_memory(&self) -> bool {
        self.emits_vector_broadcast_memory
    }
    pub const fn usage(&self) -> BlockUsage {
        self.usage
    }
    pub const fn native_admission(&self) -> &NativeAdmissionSummary {
        &self.admission
    }
    /// Only the owned final instruction boundary is an attested exit. An
    /// incidental 0F0B in data/immediates has no dispatch authority.
    pub fn attest_exit(&self, cache_ip: u64, bytes: &[u8]) -> bool {
        cache_ip == self.exit.cache_ip
            && bytes == [0x0f, 0x0b]
            && self.bytes.ends_with(&[0x0f, 0x0b])
    }
    /// Compare a complete published cache buffer against the retained emission.
    /// The future mapping owner must supply its actual paused backing bytes;
    /// this pure comparison cannot establish that an allocation exists.
    pub fn attest_emission(&self, cache_ip: u64, bytes: &[u8]) -> bool {
        cache_ip == self.placement.address && bytes == self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodegenError {
    Budget {
        kind: Budget,
        limit: u64,
    },
    Allocation,
    MissingModule {
        index: usize,
    },
    InvalidAddress,
    DeferredRelocations {
        count: usize,
    },
    MissingInitializedExecutableSource {
        guest_pc: u64,
    },
    AmbiguousSource {
        guest_pc: u64,
    },
    CacheOverlap {
        module: ModuleId,
    },
    TruncatedInstruction {
        guest_pc: u64,
    },
    InvalidInstruction {
        guest_pc: u64,
    },
    RipRelativeOutOfRange {
        guest_pc: u64,
        cache_ip: u64,
        target: u64,
    },
    Encoding {
        guest_pc: u64,
    },
    SourcePatchRange {
        address: u64,
    },
    MaterializedDeclarationCount {
        expected: usize,
        actual: usize,
    },
    UnsupportedMaterializedRelocation {
        deferred_index: usize,
        kind: u32,
    },
    InvalidMaterializedWrite {
        index: usize,
        reason: MaterializedWriteError,
    },
    DuplicateMaterializedWrite {
        deferred_index: usize,
    },
    MaterializedWindowRange {
        guest_pc: u64,
        size: usize,
    },
    MaterializedWindowMismatch {
        guest_pc: u64,
        offset: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterializedWriteError {
    DeferredIndex,
    Consumer,
    SourceRecord,
    Address,
    LogicalTarget,
    OrderingOrOverlap,
    OrdinaryPatchOverlap,
}
impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native block planning rejected: {self:?}")
    }
}
impl std::error::Error for CodegenError {}

pub(crate) struct Accounting {
    pub work: u64,
    pub allocated: u64,
    pub limits: BlockLimits,
}
impl Accounting {
    pub fn charge(&mut self, amount: u64) -> Result<(), CodegenError> {
        self.work = self
            .work
            .checked_add(amount)
            .filter(|n| *n <= self.limits.max_work_units)
            .ok_or(CodegenError::Budget {
                kind: Budget::Work,
                limit: self.limits.max_work_units,
            })?;
        Ok(())
    }
    pub fn buffer<T>(&mut self, count: usize) -> Result<Vec<T>, CodegenError> {
        let requested = count
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|v| u64::try_from(v).ok())
            .ok_or(CodegenError::Budget {
                kind: Budget::AllocationBytes,
                limit: self.limits.max_allocation_bytes,
            })?;
        self.charge(requested)?;
        if self
            .allocated
            .checked_add(requested)
            .is_none_or(|n| n > self.limits.max_allocation_bytes)
        {
            return Err(CodegenError::Budget {
                kind: Budget::AllocationBytes,
                limit: self.limits.max_allocation_bytes,
            });
        }
        let mut result = Vec::new();
        result
            .try_reserve_exact(count)
            .map_err(|_| CodegenError::Allocation)?;
        let actual = (result.capacity() as u64)
            .checked_mul(std::mem::size_of::<T>() as u64)
            .ok_or(CodegenError::Allocation)?;
        self.allocated = self
            .allocated
            .checked_add(actual)
            .filter(|n| *n <= self.limits.max_allocation_bytes)
            .ok_or(CodegenError::Budget {
                kind: Budget::AllocationBytes,
                limit: self.limits.max_allocation_bytes,
            })?;
        Ok(result)
    }

    /// Grow a plan buffer only when needed. Charge the entire replacement and
    /// movement before either occurs; retained allocation usage is cumulative,
    /// so both old and new storage remain covered even during replacement.
    pub fn grow<T>(
        &mut self,
        buffer: &mut Vec<T>,
        needed: usize,
        maximum: usize,
    ) -> Result<(), CodegenError> {
        if needed > maximum {
            return Err(CodegenError::Budget {
                kind: Budget::AllocationBytes,
                limit: self.limits.max_allocation_bytes,
            });
        }
        if needed <= buffer.capacity() {
            return Ok(());
        }
        let count = buffer.capacity().saturating_mul(2).max(needed).min(maximum);
        let movement = buffer
            .len()
            .checked_mul(std::mem::size_of::<T>())
            .and_then(|bytes| u64::try_from(bytes).ok())
            .ok_or(CodegenError::InvalidAddress)?;
        self.charge(movement)?;
        let mut replacement = self.buffer(count)?;
        replacement.append(buffer);
        *buffer = replacement;
        Ok(())
    }
}
