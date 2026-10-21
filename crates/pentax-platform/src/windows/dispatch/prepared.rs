// SPDX-License-Identifier: GPL-2.0-only

//! Reusable, explicitly unsafe native block stepping. Pure codegen declarations
//! are NOT TLS/application authority. The dedicated caller must admit them from
//! the exact prepared session before calling
//! [`step`](crate::native_dispatch::NativeBlockStepper::step).
//! `step` retains the caller's ordinary-memory obligation; `step_checked`
//! validates independent memory operands from the current observed context.
//! No raw code pointer is exposed, and neither path provides a security sandbox.

use super::ymm_bridge::VectorMode;
use super::{
    CONTEXT, CodeCache, DEFAULT_CACHE_SEARCH_GRANULES, DispatchGuard, GuestContext, RAX, RCX, RDI,
    RDX, RSI, RSP, TAIL_CELL, condition_holds, gpr_slot, memory_effective_address,
};
use crate::PlatformError;
use crate::native_memory::{
    NATIVE_ADDRESS_LIMIT, NATIVE_GUEST_PAGE, NativeMemoryError, NativeOwnerId, NativeRange,
    PausedNativeMemory,
};
use pentax_codegen::{
    CachePlacement, ControlTarget, ExitKind, MaterializedBlockPlan, MaterializedSource,
    MemoryAccessAnalyzer, MemoryAccessError, MemoryAccessKind,
};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

/// Owned guest x87 environment and physical R0..R7 bits. Status TOP selects
/// logical ST0; no field is applied to the host FPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeX87State {
    pub control_word: u16,
    pub status_word: u16,
    pub tag_word: u16,
    pub instruction_opcode: u16,
    pub instruction_pointer: u64,
    pub data_pointer: u64,
    pub registers: [[u8; 10]; 8],
}

impl NativeX87State {
    pub const fn new() -> Self {
        Self {
            control_word: 0x037f,
            status_word: 0,
            tag_word: 0xffff,
            instruction_opcode: 0,
            instruction_pointer: 0,
            data_pointer: 0,
            registers: [[0; 10]; 8],
        }
    }
}

impl Default for NativeX87State {
    fn default() -> Self {
        Self::new()
    }
}

/// Guest state only. No saved host RSP, TEB values or executable cache pointer
/// appears in this public data structure. Setters alone execute nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeCpuState {
    pub guest_pc: u64,
    /// RAX,RCX,RDX,RBX,RSP,RBP,RSI,RDI,R8..R15.
    pub gpr: [u64; 16],
    pub rflags: u64,
    pub mxcsr: u32,
    pub x87: NativeX87State,
    pub xmm: [u8; 256],
    /// Bits 255:128 of YMM0..15. Transferred only through the host-admitted
    /// full-YMM bridge; otherwise retained as software guest state.
    pub ymm_upper: [u8; 256],
    /// Modeled guest segment bases only; host FS/GS registers are never set.
    pub fs_base: Option<u64>,
    pub gs_base: Option<u64>,
}
impl NativeCpuState {
    pub const fn new(guest_pc: u64, rsp: u64) -> Self {
        let mut gpr = [0; 16];
        gpr[RSP] = rsp;
        Self {
            guest_pc,
            gpr,
            rflags: 0x202,
            mxcsr: 0x1f80,
            x87: NativeX87State::new(),
            xmm: [0; 256],
            ymm_upper: [0; 256],
            fs_base: None,
            gs_base: None,
        }
    }
}

/// Caller-selected live stack geometry and explicit outer return convention.
/// Construction verifies all ranges on the actual exclusively paused owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeStackFrame {
    pub stack: NativeRange,
    /// Reserved, unmapped guards for runtime-allocated stacks. Caller-provided
    /// stacks have no implicit guards and retain their exact byte bounds.
    pub guards: Option<[NativeRange; 2]>,
    pub return_slot: u64,
    pub return_marker: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeDispatchLimits {
    pub cache_bytes: usize,
    /// Power-of-two fixed slots, each with a private trailer cell. At most 4096.
    pub cache_slots: usize,
    pub cache_search_granules: u16,
    pub max_blocks: u64,
    pub max_source_bytes: u64,
    pub max_emitted_bytes: u64,
    pub max_work_units: u64,
    pub max_segments: u64,
    /// Retained guest contexts sharing this executor and its cumulative limits.
    pub max_thread_contexts: usize,
}
impl Default for NativeDispatchLimits {
    fn default() -> Self {
        Self {
            cache_bytes: 65_536,
            cache_slots: 1,
            // A 16 MiB neighborhood can be occupied by concurrently prepared
            // owners. Keep the search bounded but allow a 256 MiB neighborhood,
            // still within signed RIP-relative reach of the entry point.
            cache_search_granules: DEFAULT_CACHE_SEARCH_GRANULES,
            max_blocks: 10_000,
            max_source_bytes: 16 * 1024 * 1024,
            max_emitted_bytes: 16 * 1024 * 1024,
            max_work_units: 256 * 1024 * 1024,
            max_segments: 65_536,
            max_thread_contexts: 256,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeDispatchUsage {
    pub blocks: u64,
    /// Returned hardware bridge entries, excluding directly modeled exits.
    pub native_entries: u64,
    /// Successfully attested and sealed native slot publications.
    pub cache_publications: u64,
    /// Exact byte/tail/cell matches in a previously published executable slot.
    pub cache_reuses: u64,
    /// Emitted guest instructions in blocks that returned from native execution.
    /// Planning attempts, host bridge instructions and modeled exits do not count.
    pub native_instructions: u64,
    pub source_bytes: u64,
    pub emitted_bytes: u64,
    pub work_units: u64,
    /// Attempted ordinary access checks; no guest memory effect is implied.
    pub memory_access_checks: u64,
    pub memory_bytes_checked: u64,
    /// Successfully completed modeled FS/GS instructions, never native bodies.
    pub segment_operations: u64,
    /// Successful modeled reads plus writes; read-modify-write counts both.
    pub segment_bytes: u64,
    /// Completed Rust-modeled string elements; REP iterations count separately.
    pub string_elements: u64,
    /// Actual modeled string reads plus writes, including comparisons.
    pub string_bytes: u64,
    /// Completed Rust-modeled vector instructions, excluded from native counts.
    pub vector_operations: u64,
    /// Actual guest memory bytes transferred by modeled vector instructions.
    pub vector_bytes: u64,
    /// Completed Rust-modeled integer instructions, excluded from native counts.
    pub integer_operations: u64,
    /// Actual guest memory bytes read by modeled integer instructions.
    pub integer_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeDispatchError {
    Platform(PlatformError),
    Memory(NativeMemoryError),
    Budget(&'static str),
    WrongOwner,
    WrongSource,
    WrongPhase,
    InvalidState,
    InvalidStack,
    InvalidTarget(u64),
    WritableSource,
    SourceMismatch,
    EmissionMismatch,
    Allocation,
    /// Closed analyzer reason and decoded instruction class; no guest operands.
    MemoryFootprint {
        reason: MemoryAccessError,
        form: pentax_codegen::Code,
        instruction_index: usize,
    },
    MemoryIsolationRequired,
    UnsupportedSegmentInstruction,
    UnconfiguredSegment,
    UnsupportedStringInstruction,
    UnsupportedVectorInstruction,
    UnsupportedIntegerInstruction,
    UnsupportedCpuInstruction,
    TimeCounter(crate::time_counter::CounterError),
    UnsupportedX87Exception,
    VectorAlignment,
}
impl From<PlatformError> for NativeDispatchError {
    fn from(v: PlatformError) -> Self {
        Self::Platform(v)
    }
}
impl From<NativeMemoryError> for NativeDispatchError {
    fn from(v: NativeMemoryError) -> Self {
        Self::Memory(v)
    }
}
impl std::fmt::Display for NativeDispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "native block step: {self:?}")
    }
}
impl std::error::Error for NativeDispatchError {}

/// Opaque stop from BEFORE the hardware SYSCALL. Original RCX is `args[3]`.
/// Its private executor/sequence bind completion and reject cross-session replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreSyscallToken {
    owner: NativeOwnerId,
    executor: u64,
    sequence: u64,
    syscall_pc: u64,
    next_pc: u64,
    stack_pointer: u64,
    service_id: u64,
    args: [u64; 6],
    vector_args: [[u8; 16]; 8],
    rflags: u64,
}
impl PreSyscallToken {
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn syscall_pc(&self) -> u64 {
        self.syscall_pc
    }
    pub const fn next_pc(&self) -> u64 {
        self.next_pc
    }
    pub const fn stack_pointer(&self) -> u64 {
        self.stack_pointer
    }
    pub const fn service_id(&self) -> u64 {
        self.service_id
    }
    pub const fn args(&self) -> [u64; 6] {
        self.args
    }
    /// Complete raw XMM0..7 from the same observed PRE-SYSCALL CPU state.
    pub const fn vector_args(&self) -> [[u8; 16]; 8] {
        self.vector_args
    }
    pub const fn rflags(&self) -> u64 {
        self.rflags
    }
}

/// Admitted nested call frame for one pending syscall, not execution evidence.
/// Only a later opaque actual return can prove that its initializer returned.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NativeNestedCall {
    owner: NativeOwnerId,
    executor: u64,
    call: u64,
    source: pentax_linker::SourceIdentity,
    parent: PreSyscallToken,
    parent_stack_pointer: u64,
    parent_fs_base: Option<u64>,
    parent_gs_base: Option<u64>,
    entry: u64,
    args: [u64; 3],
    stack: NativeStackFrame,
}
impl std::fmt::Debug for NativeNestedCall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeNestedCall")
            .field("call", &self.call)
            .finish_non_exhaustive()
    }
}
impl NativeNestedCall {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn call_sequence(&self) -> u64 {
        self.call
    }
    pub const fn source_identity(&self) -> pentax_linker::SourceIdentity {
        self.source
    }
    pub const fn entry(&self) -> u64 {
        self.entry
    }
    pub const fn args(&self) -> [u64; 3] {
        self.args
    }
    pub const fn stack(&self) -> &NativeStackFrame {
        &self.stack
    }
    pub const fn parent_syscall(&self) -> PreSyscallToken {
        self.parent
    }
    /// Actual suspended caller RSP, captured from the executor's stopped CPU
    /// state when this frame was admitted, rather than supplied request data.
    pub const fn parent_stack_pointer(&self) -> u64 {
        self.parent_stack_pointer
    }
    pub const fn parent_fs_base(&self) -> Option<u64> {
        self.parent_fs_base
    }
    pub const fn parent_gs_base(&self) -> Option<u64> {
        self.parent_gs_base
    }
}

struct SuspendedCall {
    token: PreSyscallToken,
    state: NativeCpuState,
    stack: NativeStackFrame,
    initial: NativeCpuState,
    call: u64,
    nested: NativeNestedCall,
}
const MAX_NESTED_CALLS: usize = 32;
// Keep the bounded opaque vector snapshot inline; observing a stop never allocates.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeBlockStop {
    Resume { guest_pc: u64 },
    PreSyscall(PreSyscallToken),
    Returned { value: u64 },
    Unsupported { guest_pc: u64 },
}

/// Observation minted only after the selected call's checked outer return,
/// including an explicitly authenticated builtin or modeled RET continuation.
/// Numeric stop data cannot construct this.
/// It is historical evidence, not continuing liveness or an initialization policy.
///
/// ```compile_fail
/// use pentax_platform::native_dispatch::NativeCallReturn;
/// fn forge<'p, 'i>() -> NativeCallReturn<'p, 'i> {
///     NativeCallReturn { value: 0 }
/// }
/// ```
#[derive(Clone, Copy)]
pub struct NativeCallReturn<'plan, 'image> {
    source: &'plan pentax_linker::SymbolicLinkPlan<'image>,
    owner: NativeOwnerId,
    executor: u64,
    call: u64,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    value: u64,
}
impl<'plan, 'image> NativeCallReturn<'plan, 'image> {
    pub const fn source_plan(&self) -> &'plan pentax_linker::SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn executor_id(&self) -> u64 {
        self.executor
    }
    pub const fn call_sequence(&self) -> u64 {
        self.call
    }
    pub const fn entry(&self) -> u64 {
        self.initial.guest_pc
    }
    pub const fn initial_state(&self) -> &NativeCpuState {
        &self.initial
    }
    pub const fn stack(&self) -> &NativeStackFrame {
        &self.stack
    }
    pub const fn value(&self) -> u64 {
        self.value
    }
}
impl std::fmt::Debug for NativeCallReturn<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeCallReturn")
            .field("call", &self.call)
            .finish_non_exhaustive()
    }
}
// The same finite Copy token binds phase identity without a per-syscall allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Ready,
    Syscall(PreSyscallToken),
    LibcBuiltin(u64),
    Returned,
    Terminal,
}
static NEXT_EXECUTOR: AtomicU64 = AtomicU64::new(1);

#[path = "prepared/align_right.rs"]
mod align_right;
#[path = "prepared/bit_counts.rs"]
mod bit_counts;
#[path = "prepared/cache_hints.rs"]
mod cache_hints;
#[path = "prepared/libc_builtin.rs"]
mod libc_builtin;
pub use libc_builtin::NativeLibcBuiltin;
#[path = "prepared/cpu_identification.rs"]
mod cpu_identification;
#[path = "prepared/float_ops.rs"]
mod float_ops;
#[path = "prepared/fused_float.rs"]
mod fused_float;
#[path = "prepared/mask_comparisons.rs"]
mod mask_comparisons;
#[path = "prepared/multiply_add_words.rs"]
mod multiply_add_words;
#[path = "prepared/mxcsr_control.rs"]
mod mxcsr_control;
#[path = "prepared/packed_convert.rs"]
mod packed_convert;
#[path = "prepared/packed_vectors.rs"]
mod packed_vectors;
#[path = "prepared/rounding.rs"]
mod rounding;
#[path = "prepared/saturating_packs.rs"]
mod saturating_packs;
#[path = "prepared/scalar_convert.rs"]
mod scalar_convert;
#[path = "prepared/segments.rs"]
mod segments;
#[path = "prepared/strings.rs"]
mod strings;
#[path = "prepared/threads.rs"]
mod threads;
#[path = "prepared/timestamp_counter.rs"]
mod timestamp_counter;
#[path = "prepared/vector_shifts.rs"]
mod vector_shifts;
#[path = "prepared/vectors.rs"]
mod vectors;
#[path = "prepared/x87_control.rs"]
mod x87_control;
pub use threads::NativeThreadContext;

/// Retained owner, cache, CPU/context and cumulative limits without a borrowed
/// source. This opaque value has no execution interface. A source view and the
/// original unsafe preparation/operand obligations are required to run again.
pub struct NativeBlockMachine {
    owner: NativeOwnerId,
    executor: u64,
    state: NativeCpuState,
    stack: NativeStackFrame,
    cache: CodeCache,
    vector_mode: VectorMode,
    published_lengths: Vec<Option<usize>>,
    limits: NativeDispatchLimits,
    usage: NativeDispatchUsage,
    phase: Phase,
    target_visits: u64,
    memory_analyzer: MemoryAccessAnalyzer,
    call_initial: NativeCpuState,
    call_sequence: u64,
    next_call_sequence: u64,
    suspended: Vec<SuspendedCall>,
    observed_return: Option<CallObservation>,
    string_repeat: Option<strings::RepeatState>,
    source_identity: pentax_linker::SourceIdentity,
    source_modules: usize,
    declarations: Vec<pentax_codegen::DeclaredAppliedWrite>,
    thread_index: usize,
    thread_stacks: Vec<NativeStackFrame>,
}
impl std::fmt::Debug for NativeBlockMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeBlockMachine")
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}
impl NativeBlockMachine {
    pub const fn state(&self) -> &NativeCpuState {
        &self.state
    }
    pub const fn usage(&self) -> NativeDispatchUsage {
        self.usage
    }
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
}

#[derive(Clone, Copy)]
struct CallObservation {
    owner: NativeOwnerId,
    executor: u64,
    call: u64,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    value: u64,
}

/// Borrowed source view over the same retained machine. Its short memory pause
/// ends after each step and precedes core service work.
pub struct NativeBlockStepper<'view, 'plan, 'image> {
    source: &'view MaterializedSource<'plan, 'image>,
    machine: NativeBlockMachine,
    // Immutable initialized source metadata only. This is deliberately absent
    // from NativeBlockMachine: every reattachment starts without a memo.
    target_interval: Cell<Option<TargetInterval>>,
}
impl<'view, 'plan, 'image> NativeBlockStepper<'view, 'plan, 'image> {
    /// Reattach the SAME machine to its immutable source, or to a checked append
    /// while a syscall is pending. Preserve the executor/cache, CPU, stack and
    /// all cumulative counters. Refuse terminal state or changed old declarations.
    /// Consume the occupied slot only on success; refusal retains its machine
    /// and attempted work without allocating an error-path ownership wrapper.
    ///
    /// This checks metadata lineage and live current context, not application of
    /// the extension. Before unsafe stepping, the dedicated caller must bind the
    /// actual core publication/source/TLS proof to this owner as for `step`.
    pub fn from_machine(
        source: &'view MaterializedSource<'plan, 'image>,
        memory: &mut PausedNativeMemory<'_>,
        slot: &mut Option<NativeBlockMachine>,
    ) -> Result<Self, NativeDispatchError> {
        let machine = slot.as_mut().ok_or(NativeDispatchError::WrongPhase)?;
        let result = (|| {
            if memory.owner_id() != machine.owner {
                return Err(NativeDispatchError::WrongOwner);
            }
            if matches!(machine.phase, Phase::Terminal | Phase::LibcBuiltin(_)) {
                return Err(NativeDispatchError::WrongPhase);
            }
            let plan = source.source_plan();
            let same = plan.identity() == machine.source_identity;
            let admitted_source = if same {
                // An already attached epoch retains its complete module set;
                // its historical guest index is not a new append request.
                plan.modules().len() == machine.source_modules
            } else {
                // One atomic append adds the actual guest and, optionally,
                // one generated delta. Never accept a skipped parent, an
                // arbitrary suffix or a delta selected as the real guest.
                plan.parent_identity() == Some(machine.source_identity)
                    && plan
                        .appended_guest()
                        .is_some_and(|guest| guest.index() == machine.source_modules)
                    && matches!(
                        plan.modules().len().checked_sub(machine.source_modules),
                        Some(1 | 2)
                    )
            };
            if !admitted_source {
                return Err(NativeDispatchError::WrongSource);
            }
            if !same && !matches!(machine.phase, Phase::Syscall(_)) {
                return Err(NativeDispatchError::WrongPhase);
            }
            let declarations = source.declared_writes();
            let work = (declarations.len() as u64)
                .checked_add(machine.declarations.len() as u64)
                .and_then(|value| {
                    value.checked_mul(
                        4 * std::mem::size_of::<pentax_codegen::DeclaredAppliedWrite>() as u64,
                    )
                })
                .and_then(|value| value.checked_add(plan.modules().len() as u64 + 16))
                .ok_or(NativeDispatchError::Budget("source reattachment"))?;
            charge(
                &mut machine.usage.work_units,
                work,
                machine.limits.max_work_units,
                "source reattachment",
            )?;
            if !machine.declarations.iter().eq(declarations
                .iter()
                .filter(|write| write.consumer.index() < machine.source_modules))
            {
                return Err(NativeDispatchError::WrongSource);
            }
            let mut segments = 0_u64;
            for module in plan.modules() {
                segments = segments
                    .checked_add(module.runtime_plan().segments.len() as u64)
                    .filter(|count| *count <= machine.limits.max_segments)
                    .ok_or(NativeDispatchError::Budget("segments"))?;
            }
            let visits = segments
                .checked_add(plan.modules().len() as u64)
                .ok_or(NativeDispatchError::Budget("source reattachment"))?;
            charge(
                &mut machine.usage.work_units,
                visits
                    .checked_mul(2)
                    .ok_or(NativeDispatchError::Budget("source reattachment"))?,
                machine.limits.max_work_units,
                "source reattachment",
            )?;
            validate_state(&machine.state)?;
            validate_stack(
                memory,
                &machine.stack,
                machine.state.gpr[RSP],
                &mut machine.usage,
                &machine.limits,
            )?;
            let mut marker = [0; 8];
            memory.read(machine.stack.return_slot, &mut marker)?;
            if u64::from_le_bytes(marker) != machine.stack.return_marker {
                return Err(NativeDispatchError::InvalidStack);
            }
            if machine.phase != Phase::Returned {
                check_target(source, memory, machine.state.guest_pc)?;
            }
            let retained = snapshot_declarations(source, &mut machine.usage, &machine.limits)?;
            machine.declarations = retained;
            machine.source_identity = plan.identity();
            machine.source_modules = plan.modules().len();
            machine.target_visits = visits;
            Ok(())
        })();
        result?;
        Ok(Self {
            source,
            machine: slot.take().ok_or(NativeDispatchError::WrongPhase)?,
            target_interval: Cell::new(None),
        })
    }
    /// Allocate a private RW cache and validate source/stack metadata. This safe
    /// operation executes no guest code and does not certify declared TLS values.
    pub fn new(
        source: &'view MaterializedSource<'plan, 'image>,
        memory: &mut PausedNativeMemory<'_>,
        state: NativeCpuState,
        stack: NativeStackFrame,
        limits: &NativeDispatchLimits,
    ) -> Result<Self, NativeDispatchError> {
        if limits.cache_bytes < NATIVE_GUEST_PAGE as usize
            || !limits
                .cache_bytes
                .is_multiple_of(NATIVE_GUEST_PAGE as usize)
            || limits.cache_bytes > crate::MAX_RESERVATION_BYTES
            || limits.cache_search_granules == 0
            || !limits.cache_slots.is_power_of_two()
            || limits.cache_slots > 4096
            || !limits.cache_bytes.is_multiple_of(limits.cache_slots)
            || limits.cache_bytes / limits.cache_slots < 128
            || limits.max_thread_contexts == 0
            || limits.max_thread_contexts > 4096
        {
            return Err(NativeDispatchError::Budget("cache policy"));
        }
        validate_state(&state)?;
        let mut usage = NativeDispatchUsage::default();
        // Admit the module visits before walking caller-selected metadata.
        charge(
            &mut usage.work_units,
            source.source_plan().modules().len() as u64,
            limits.max_work_units,
            "work",
        )?;
        let mut segments = 0_u64;
        for module in source.source_plan().modules() {
            segments = segments
                .checked_add(module.runtime_plan().segments.len() as u64)
                .filter(|v| *v <= limits.max_segments)
                .ok_or(NativeDispatchError::Budget("segments"))?;
        }
        let target_visits = segments
            .checked_add(source.source_plan().modules().len() as u64)
            .ok_or(NativeDispatchError::Budget("work"))?;
        charge(
            &mut usage.work_units,
            target_visits
                .checked_add(u64::from(limits.cache_search_granules))
                .and_then(|v| v.checked_add(8))
                .ok_or(NativeDispatchError::Budget("work"))?,
            limits.max_work_units,
            "work",
        )?;
        validate_stack(memory, &stack, state.gpr[RSP], &mut usage, limits)?;
        let mut return_bytes = [0; 8];
        memory.read(stack.return_slot, &mut return_bytes)?;
        if u64::from_le_bytes(return_bytes) != stack.return_marker {
            return Err(NativeDispatchError::InvalidStack);
        }
        check_target(source, memory, state.guest_pc)?;
        let declarations = snapshot_declarations(source, &mut usage, limits)?;
        let mut thread_stacks = Vec::new();
        thread_stacks
            .try_reserve_exact(1)
            .map_err(|_| NativeDispatchError::Allocation)?;
        thread_stacks.push(stack);
        charge(
            &mut usage.work_units,
            (limits.cache_slots * std::mem::size_of::<Option<usize>>()
                + limits.cache_bytes / NATIVE_GUEST_PAGE as usize) as u64,
            limits.max_work_units,
            "cache metadata",
        )?;
        let mut published_lengths = Vec::new();
        published_lengths
            .try_reserve_exact(limits.cache_slots)
            .map_err(|_| NativeDispatchError::Allocation)?;
        if published_lengths.capacity() != limits.cache_slots {
            return Err(NativeDispatchError::Allocation);
        }
        published_lengths.resize(limits.cache_slots, None);
        let anchor = state
            .guest_pc
            .checked_add(0xffff)
            .map(|v| v & !0xffff)
            .ok_or(NativeDispatchError::InvalidState)?;
        let mut cache = None;
        for index in 0..u64::from(limits.cache_search_granules) {
            // A large data reservation can occupy all addresses above code.
            // Alternate both directions within the same finite attempt budget.
            let distance = (index / 2 + 1) * 0x10000;
            let candidate = if index.is_multiple_of(2) {
                anchor.checked_add(distance)
            } else {
                anchor.checked_sub(distance)
            };
            let Some(address) = candidate.filter(|v| {
                *v != 0
                    && v.checked_add(limits.cache_bytes as u64)
                        .is_some_and(|end| end < NATIVE_ADDRESS_LIMIT)
            }) else {
                continue;
            };
            match CodeCache::reserve(address, limits.cache_bytes) {
                Ok(value) => {
                    cache = Some(value);
                    break;
                }
                Err(PlatformError::OperatingSystem { code: 487, .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let cache = cache.ok_or(NativeDispatchError::Budget("near cache search"))?;
        charge(
            &mut usage.work_units,
            128,
            limits.max_work_units,
            "vector context admission",
        )?;
        let vector_mode = VectorMode::detect();
        let executor = NEXT_EXECUTOR
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| NativeDispatchError::Budget("executor identity"))?;
        Ok(Self {
            source,
            target_interval: Cell::new(None),
            machine: NativeBlockMachine {
                owner: memory.owner_id(),
                executor,
                state,
                stack,
                cache,
                vector_mode,
                published_lengths,
                limits: *limits,
                usage,
                phase: Phase::Ready,
                target_visits,
                memory_analyzer: MemoryAccessAnalyzer::default(),
                call_initial: state,
                call_sequence: 0,
                next_call_sequence: 0,
                suspended: Vec::new(),
                observed_return: None,
                string_repeat: None,
                source_identity: source.source_plan().identity(),
                source_modules: source.source_plan().modules().len(),
                declarations,
                thread_index: 0,
                thread_stacks,
            },
        })
    }
    pub fn placement(&self) -> CachePlacement {
        self.slot_placement(0)
    }

    /// Native vector feature requirements admitted for this exact immutable
    /// bridge mode. These policy data grant no source or execution authority.
    /// None retains the SSE2 bridge and software vector fallback.
    pub fn native_ymm_features(&self) -> Option<pentax_codegen::CpuFeatures> {
        self.machine.vector_mode.features()
    }

    /// Deterministic finite slot selection. This placement grants no execution
    /// authority; source, ownership, operands and exact emitted bytes are checked.
    pub fn placement_for(&self, guest_pc: u64) -> CachePlacement {
        let index = ((guest_pc ^ (guest_pc >> 8) ^ (guest_pc >> 16) ^ (guest_pc >> 32)) as usize)
            & (self.machine.limits.cache_slots - 1);
        self.slot_placement(index)
    }

    fn slot_placement(&self, index: usize) -> CachePlacement {
        let stride = self.machine.cache.len / self.machine.limits.cache_slots;
        CachePlacement {
            address: self.machine.cache.address() + (index * stride) as u64,
            // The native six-byte jump replaces a two-byte planner UD2.
            // Keep its extra four bytes outside the planner's usable span.
            capacity: (stride - TAIL_CELL - 4) as u64,
        }
    }
    pub const fn state(&self) -> &NativeCpuState {
        &self.machine.state
    }
    pub const fn usage(&self) -> NativeDispatchUsage {
        self.machine.usage
    }
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.machine.owner
    }

    /// Identity of the actual current executor. This scalar grants no execution
    /// authority and does not certify a returned or suspended call.
    pub const fn executor_id(&self) -> u64 {
        self.machine.executor
    }

    /// Sequence of the selected call context, including after thread exchange.
    /// This scalar alone is not evidence of an actual call return.
    pub const fn call_sequence(&self) -> u64 {
        self.machine.call_sequence
    }

    /// Revalidate the CURRENT actual outer return without consuming it or
    /// changing context. Historical copies, nested returns and observations
    /// belonging to another owner, executor, call or source cannot authorize
    /// outer-call cleanup after the selected context has changed.
    pub fn validate_call_return(
        &self,
        returned: &NativeCallReturn<'_, '_>,
    ) -> Result<(), NativeDispatchError> {
        let observed = self
            .machine
            .observed_return
            .ok_or(NativeDispatchError::WrongPhase)?;
        if self.machine.phase != Phase::Returned
            || !self.machine.suspended.is_empty()
            || self.machine.source_identity != self.source.source_plan().identity()
            || observed.owner != self.machine.owner
            || observed.executor != self.machine.executor
            || observed.call != self.machine.call_sequence
            || observed.initial != self.machine.call_initial
            || observed.stack != self.machine.stack
            || observed.value != self.machine.state.gpr[RAX]
            || self.machine.state.guest_pc != self.machine.stack.return_marker
            || self.machine.stack.return_slot.checked_add(8) != Some(self.machine.state.gpr[RSP])
            || !std::ptr::eq(returned.source, self.source.source_plan())
            || returned.owner != observed.owner
            || returned.executor != observed.executor
            || returned.call != observed.call
            || returned.initial != observed.initial
            || returned.stack != observed.stack
            || returned.value != observed.value
        {
            return Err(NativeDispatchError::WrongPhase);
        }
        Ok(())
    }

    pub fn observed_return(&self) -> Option<NativeCallReturn<'plan, 'image>> {
        self.machine.observed_return.map(|data| NativeCallReturn {
            source: self.source.source_plan(),
            owner: data.owner,
            executor: data.executor,
            call: data.call,
            initial: data.initial,
            stack: data.stack,
            value: data.value,
        })
    }

    /// Preserve the actual cache/context/counters while ending this plan borrow.
    /// This operation executes nothing and grants no source-publication proof.
    pub fn into_machine(self) -> NativeBlockMachine {
        self.machine
    }

    /// Begin another guest call only after an exact outer return. Keep the same
    /// source, owner, stack, cache, executor identity and cumulative budgets.
    /// This executes nothing and grants no module-initialization authority.
    /// Segment bases must remain those admitted for the preceding guest call.
    pub fn begin_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        state: NativeCpuState,
    ) -> Result<(), NativeDispatchError> {
        if self.machine.phase != Phase::Returned || !self.machine.suspended.is_empty() {
            return Err(NativeDispatchError::WrongPhase);
        }
        if memory.owner_id() != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        if state.fs_base != self.machine.state.fs_base
            || state.gs_base != self.machine.state.gs_base
        {
            return Err(NativeDispatchError::InvalidState);
        }
        validate_state(&state)?;
        if state.gpr[RSP] != self.machine.stack.return_slot {
            return Err(NativeDispatchError::InvalidStack);
        }
        charge(
            &mut self.machine.usage.work_units,
            self.machine
                .target_visits
                .checked_add(8)
                .ok_or(NativeDispatchError::Budget("call work"))?,
            self.machine.limits.max_work_units,
            "call work",
        )?;
        validate_stack(
            memory,
            &self.machine.stack,
            state.gpr[RSP],
            &mut self.machine.usage,
            &self.machine.limits,
        )?;
        let mut bytes = [0; 8];
        memory.read(self.machine.stack.return_slot, &mut bytes)?;
        if u64::from_le_bytes(bytes) != self.machine.stack.return_marker {
            return Err(NativeDispatchError::InvalidStack);
        }
        self.check_target(memory, state.guest_pc)?;
        if !matches!(
            memory.preflight_write(state.guest_pc, 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(NativeDispatchError::WritableSource);
        }
        let call_sequence = self
            .machine
            .next_call_sequence
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("call sequence"))?;
        self.machine.call_initial = state;
        self.machine.call_sequence = call_sequence;
        self.machine.next_call_sequence = call_sequence;
        self.machine.observed_return = None;
        self.machine.state = state;
        self.machine.phase = Phase::Ready;
        Ok(())
    }

    /// Suspend the exact pending syscall and prepare a guest callee below the
    /// caller's 128-byte red zone. Keep the same owned stack/cache/CPU/thread and
    /// cumulative limits. This writes a return slot but executes no instruction.
    /// The dedicated caller must separately admit the actual module/source and
    /// its initialization policy before entering the callee through unsafe step.
    pub fn begin_syscall_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        token: PreSyscallToken,
        entry: u64,
        args: [u64; 3],
    ) -> Result<NativeNestedCall, NativeDispatchError> {
        if self.machine.phase != Phase::Syscall(token) || token.executor != self.machine.executor {
            return Err(NativeDispatchError::WrongPhase);
        }
        if memory.owner_id() != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        if self.machine.suspended.len() >= MAX_NESTED_CALLS {
            return Err(NativeDispatchError::Budget("nested calls"));
        }
        let work = ((self.machine.suspended.len() + 1) as u64)
            .checked_mul(std::mem::size_of::<SuspendedCall>() as u64)
            .and_then(|value| value.checked_add(self.machine.target_visits + 32))
            .ok_or(NativeDispatchError::Budget("nested call work"))?;
        charge(
            &mut self.machine.usage.work_units,
            work,
            self.machine.limits.max_work_units,
            "nested call work",
        )?;
        let call = self
            .machine
            .next_call_sequence
            .checked_add(1)
            .ok_or(NativeDispatchError::Budget("call sequence"))?;
        let slot = self.machine.state.gpr[RSP]
            .checked_sub(128)
            .map(|value| value & !15)
            .and_then(|value| value.checked_sub(8))
            .ok_or(NativeDispatchError::InvalidStack)?;
        let stack = NativeStackFrame {
            return_slot: slot,
            ..self.machine.stack
        };
        validate_stack(
            memory,
            &stack,
            slot,
            &mut self.machine.usage,
            &self.machine.limits,
        )?;
        self.check_target(memory, entry)?;
        if !matches!(
            memory.preflight_write(entry, 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ) {
            return Err(NativeDispatchError::WritableSource);
        }
        let mut next = self.machine.state;
        next.guest_pc = entry;
        next.gpr[RSP] = slot;
        [next.gpr[RDI], next.gpr[RSI], next.gpr[RDX]] = args;
        validate_state(&next)?;
        self.machine
            .suspended
            .try_reserve_exact(1)
            .map_err(|_| NativeDispatchError::Allocation)?;
        if self.machine.suspended.capacity() > MAX_NESTED_CALLS {
            return Err(NativeDispatchError::Budget("nested call allocation"));
        }
        let nested = NativeNestedCall {
            owner: self.machine.owner,
            executor: self.machine.executor,
            call,
            source: self.source.source_plan().identity(),
            parent: token,
            parent_stack_pointer: self.machine.state.gpr[RSP],
            parent_fs_base: self.machine.state.fs_base,
            parent_gs_base: self.machine.state.gs_base,
            entry,
            args,
            stack,
        };
        // A failed complete-span write changes neither context nor call stack.
        memory.write(slot, &stack.return_marker.to_le_bytes())?;
        self.machine.suspended.push(SuspendedCall {
            token,
            state: self.machine.state,
            stack: self.machine.stack,
            initial: self.machine.call_initial,
            call: self.machine.call_sequence,
            nested,
        });
        self.machine.state = next;
        self.machine.stack = stack;
        self.machine.call_initial = next;
        self.machine.call_sequence = call;
        self.machine.next_call_sequence = call;
        self.machine.observed_return = None;
        self.machine.phase = Phase::Ready;
        Ok(nested)
    }

    /// Restore only the innermost suspended caller after an ACTUAL returned
    /// callee, then complete its original syscall with a core-admitted result.
    /// Numeric Returned values and data-only exit resolution cannot authorize
    /// this transition. Failed validation preserves the suspended context.
    pub fn complete_syscall_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        nested: NativeNestedCall,
        value: u64,
    ) -> Result<(), NativeDispatchError> {
        let token = self.restore_syscall_call(memory, nested)?;
        if let Err(error) = self.complete_syscall(token, value) {
            self.machine.phase = Phase::Terminal;
            return Err(error);
        }
        Ok(())
    }

    /// After an attested actual return, restore its exact suspended syscall
    /// and admit another guest callback below the same caller red zone. The
    /// original syscall remains pending; its continuation is never advanced.
    /// Each callback receives a fresh call sequence and retains cumulative
    /// work, execution, stack and cache limits. A failed next-call admission
    /// makes this executor terminal instead of partially resuming its caller.
    pub fn continue_syscall_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        nested: NativeNestedCall,
        entry: u64,
        args: [u64; 3],
    ) -> Result<NativeNestedCall, NativeDispatchError> {
        let token = self.restore_syscall_call(memory, nested)?;
        match self.begin_syscall_call(memory, token, entry, args) {
            Ok(call) => Ok(call),
            Err(error) => {
                self.machine.phase = Phase::Terminal;
                Err(error)
            }
        }
    }

    /// Revalidate the CURRENT innermost actual return without consuming it or
    /// changing guest/kernel state. A copied historical observation cannot
    /// authorize service publication after its executor has already resumed.
    pub fn validate_syscall_call_return(
        &self,
        nested: NativeNestedCall,
        returned: &NativeCallReturn<'_, '_>,
    ) -> Result<(), NativeDispatchError> {
        let saved = self
            .machine
            .suspended
            .last()
            .ok_or(NativeDispatchError::WrongPhase)?;
        let observed = self
            .machine
            .observed_return
            .ok_or(NativeDispatchError::WrongPhase)?;
        if self.machine.phase != Phase::Returned
            || saved.nested != nested
            || self.machine.owner != nested.owner
            || self.machine.executor != nested.executor
            || self.machine.call_sequence != nested.call
            || observed.call != nested.call
            || observed.executor != nested.executor
            || observed.owner != nested.owner
            || observed.initial.guest_pc != nested.entry
            || observed.stack != nested.stack
            || [
                observed.initial.gpr[RDI],
                observed.initial.gpr[RSI],
                observed.initial.gpr[RDX],
            ] != nested.args
            || saved.state.gpr[RSP] != nested.parent_stack_pointer
            || saved.state.fs_base != nested.parent_fs_base
            || saved.state.gs_base != nested.parent_gs_base
            || self.machine.state.fs_base != saved.state.fs_base
            || self.machine.state.gs_base != saved.state.gs_base
            || !std::ptr::eq(returned.source, self.source.source_plan())
            || returned.owner != observed.owner
            || returned.executor != observed.executor
            || returned.call != observed.call
            || returned.initial != observed.initial
            || returned.stack != observed.stack
            || returned.value != observed.value
        {
            return Err(NativeDispatchError::WrongPhase);
        }
        Ok(())
    }

    // Shared actual-return validation. Data-only Returned observations cannot
    // reach either syscall completion or chained callback admission.
    fn restore_syscall_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        nested: NativeNestedCall,
    ) -> Result<PreSyscallToken, NativeDispatchError> {
        let saved = self
            .machine
            .suspended
            .last()
            .ok_or(NativeDispatchError::WrongPhase)?;
        let observed = self
            .machine
            .observed_return
            .ok_or(NativeDispatchError::WrongPhase)?;
        if self.machine.phase != Phase::Returned
            || saved.nested != nested
            || self.machine.executor != nested.executor
            || memory.owner_id() != nested.owner
            || self.machine.call_sequence != nested.call
            || observed.call != nested.call
            || observed.executor != nested.executor
            || observed.owner != nested.owner
            || observed.initial.guest_pc != nested.entry
            || observed.stack != nested.stack
            || [
                observed.initial.gpr[RDI],
                observed.initial.gpr[RSI],
                observed.initial.gpr[RDX],
            ] != nested.args
            || self.machine.state.fs_base != saved.state.fs_base
            || self.machine.state.gs_base != saved.state.gs_base
        {
            return Err(NativeDispatchError::WrongPhase);
        }
        charge(
            &mut self.machine.usage.work_units,
            self.machine.target_visits + 32,
            self.machine.limits.max_work_units,
            "nested return work",
        )?;
        validate_state(&saved.state)?;
        validate_stack(
            memory,
            &saved.stack,
            saved.state.gpr[RSP],
            &mut self.machine.usage,
            &self.machine.limits,
        )?;
        self.check_target(memory, saved.token.next_pc)?;
        let mut marker = [0; 8];
        memory.read(saved.stack.return_slot, &mut marker)?;
        if u64::from_le_bytes(marker) != saved.stack.return_marker {
            return Err(NativeDispatchError::InvalidStack);
        }
        let saved = self
            .machine
            .suspended
            .pop()
            .ok_or(NativeDispatchError::WrongPhase)?;
        // The floating-point environment belongs to the continuing guest
        // thread. Restoring its caller snapshot would erase status flags
        // raised by a real initializer, or an intentional environment change.
        let mxcsr = self.machine.state.mxcsr;
        let x87 = self.machine.state.x87;
        self.machine.state = saved.state;
        self.machine.state.mxcsr = mxcsr;
        self.machine.state.x87 = x87;
        self.machine.stack = saved.stack;
        self.machine.call_initial = saved.initial;
        self.machine.call_sequence = saved.call;
        self.machine.observed_return = None;
        self.machine.phase = Phase::Syscall(saved.token);
        Ok(saved.token)
    }

    /// Execute one admitted block; release its memory borrow before services.
    ///
    /// # Safety
    /// The caller must independently bind this exact declaration and owner to
    /// the actual prepared session and typed TLS plan: same active published
    /// source epoch, no remaining unapplied deferred record, and exact
    /// deferred-index/consumer/source/
    /// address/value agreement for every applied TLS write. CallerDeclared
    /// consistency and byte equality alone are not that proof.
    ///
    /// Every ordinary instruction's explicit/implicit memory access must remain
    /// within this owner's live committed pages with appropriate permissions.
    /// In particular no native operand may address Rust state, host stack, code
    /// cache, another allocation or an unmapped address. The runtime does not
    /// instrument those accesses, recover faults or provide hostile-code
    /// containment. The dedicated child and its controller must account for
    /// fatal exceptions/hangs; they do not establish memory safety by themselves.
    /// No other thread may access this memory or bridge context during the step.
    pub unsafe fn step(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        self.preflight_block(memory, block)?;
        // Even manual stepping cannot bypass checked atomic/non-temporal or
        // the closed vector-move family's backing, rights and alignment.
        // This includes the six already-native aligned legacy memory moves.
        self.preflight_atomic_memory(memory, block)?;
        // SAFETY: This entry retains the caller's complete ordinary-memory and
        // preparation obligations documented above.
        unsafe { self.enter_block(memory, block) }
    }

    /// Observe one actual SYSCALL exit without executing a native instruction.
    /// Ordinary bodies and all other exit kinds are refused before admission.
    /// The returned token still requires the core's service/site authorization.
    pub fn step_syscall_exit(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        if block.instructions().len() != 1
            || block.instructions()[0].emitted_len() != 0
            || block.bytes() != [0x0f, 0x0b]
            || !matches!(block.exit().kind(), ExitKind::Syscall)
            || block.guest_pc() != block.exit().guest_pc()
        {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        self.preflight_block(memory, block)?;
        // SAFETY: The single checked SYSCALL has no native operand or body.
        // Exact UD2-only emission makes publish return false before any bridge
        // entry; enter_block also cross-checks zero native instruction count.
        // Normal source/owner/stack preflight and Rust exit resolution retain
        // their accounting, live-byte attestation and real pending-token phase.
        unsafe { self.enter_block(memory, block) }
    }

    /// Resolve one original RET through the owned stack without entering a
    /// native body. This is an observed control-flow return, not execution of
    /// an arbitrary callback body. Ordinary instructions and other exits fail.
    pub fn step_return_exit(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        if block.instructions().len() != 1
            || block.instructions()[0].emitted_len() != 0
            || block.bytes() != [0x0f, 0x0b]
            || !matches!(block.exit().kind(), ExitKind::Return { adjust: 0 })
            || block.guest_pc() != block.exit().guest_pc()
        {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        self.preflight_block(memory, block)?;
        // SAFETY: Exact UD2-only emission has no hardware body or ordinary
        // operand. Return resolution reads only through this paused owner's
        // checked stack access; normal source/phase/state validation remains.
        unsafe { self.enter_block(memory, block) }
    }

    /// Check ordinary explicit/implicit memory operands before entering a block.
    /// A memory instruction's address registers must be unchanged by previous
    /// instructions, including partial, conditional and implicit writes. Thus
    /// all effective addresses use the current observed context. Atomics remain
    /// isolated. Conditional accesses require both possible rights.
    /// Unmodeled footprints fail before cache publication or native execution.
    ///
    /// # Safety
    /// Bind this exact declaration/owner to the actual prepared session and
    /// complete typed TLS plan, as required by `step`. No other thread may
    /// access its backing or bridge context during the step. Ordinary operand
    /// bounds and permissions are checked here rather than assumed from the
    /// caller. This does not provide guest fault recovery, prevent native
    /// arithmetic exceptions or make the worker a security sandbox.
    pub unsafe fn step_checked(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        self.preflight_block(memory, block)?;
        self.preflight_ordinary_memory(memory, block)?;
        // SAFETY: The caller supplies exact preparation provenance and exclusive
        // context ownership. The current independent instruction footprints were
        // independently derived and checked on these same paused pages. No
        // guest instruction or service runs between this check and entry.
        unsafe { self.enter_block(memory, block) }
    }

    fn preflight_atomic_memory(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<(), NativeDispatchError> {
        charge(
            &mut self.machine.usage.work_units,
            block.instructions().len() as u64,
            self.machine.limits.max_work_units,
            "atomic classification",
        )?;
        if block.emits_non_temporal_store()
            || block.emits_vector_memory_move()
            || block.emits_scalar_float_memory()
            || block.native_admission().has_atomic_memory()
        {
            self.preflight_ordinary_memory(memory, block)?;
        }
        Ok(())
    }

    fn preflight_ordinary_memory(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<(), NativeDispatchError> {
        // Bound both metadata walks and dependency-table analysis before use.
        // Iced's fixed profile scratch uses its ordinary dependency allocator.
        let work = (block.instructions().len() as u64)
            .checked_mul(128)
            .ok_or(NativeDispatchError::Budget("memory analysis"))?;
        charge(
            &mut self.machine.usage.work_units,
            work,
            self.machine.limits.max_work_units,
            "memory analysis",
        )?;
        if block.native_admission().is_memory_free() {
            return Ok(());
        }
        let mut written_registers = 0_u16;
        let emitted = block.native_admission().emitted_instructions();
        for (instruction_index, record) in block
            .instructions()
            .iter()
            .enumerate()
            .filter(|(_, record)| record.emitted_len() != 0)
        {
            let accesses = self
                .machine
                .memory_analyzer
                .analyze(record, &self.machine.state.gpr)
                .map_err(|reason| NativeDispatchError::MemoryFootprint {
                    reason,
                    form: record.instruction().code(),
                    instruction_index,
                })?;
            let dependencies = accesses.register_dependencies();
            if dependencies.address & written_registers != 0
                || (record.has_atomic_memory() && emitted != 1)
            {
                return Err(NativeDispatchError::MemoryIsolationRequired);
            }
            written_registers |= dependencies.written;
            for access in accesses.accesses() {
                charge(
                    &mut self.machine.usage.work_units,
                    access.size * 2 + 8,
                    self.machine.limits.max_work_units,
                    "memory checks",
                )?;
                self.machine.usage.memory_access_checks = self
                    .machine
                    .usage
                    .memory_access_checks
                    .checked_add(1)
                    .ok_or(NativeDispatchError::Budget("memory checks"))?;
                self.machine.usage.memory_bytes_checked = self
                    .machine
                    .usage
                    .memory_bytes_checked
                    .checked_add(access.size)
                    .ok_or(NativeDispatchError::Budget("memory bytes"))?;
                if matches!(
                    access.kind,
                    MemoryAccessKind::Read | MemoryAccessKind::ReadWrite
                ) {
                    memory.preflight_read(access.address, access.size)?;
                }
                if matches!(
                    access.kind,
                    MemoryAccessKind::Write | MemoryAccessKind::ReadWrite
                ) {
                    memory.preflight_write(access.address, access.size)?;
                }
            }
        }
        Ok(())
    }

    /// # Safety
    /// Exact preparation/context ownership and ordinary operand confinement
    /// must come from either the caller of `step` or the checks in `step_checked`.
    unsafe fn enter_block(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        charge(
            &mut self.machine.usage.work_units,
            block.instructions().len() as u64,
            self.machine.limits.max_work_units,
            "instruction observations",
        )?;
        let native_instructions = self
            .machine
            .usage
            .native_instructions
            .checked_add(block.native_admission().emitted_instructions())
            .ok_or(NativeDispatchError::Budget("native instructions"))?;
        if (native_instructions != self.machine.usage.native_instructions)
            != (block.bytes().len() > 2)
        {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        if block.bytes().len() > 2 && self.machine.vector_mode.features().is_some() {
            // Admit both full 512-byte context transfers before publication or
            // native entry. Saving after execution cannot consume a new quota.
            charge(
                &mut self.machine.usage.work_units,
                1024,
                self.machine.limits.max_work_units,
                "vector context transfer",
            )?;
        }
        if self.publish(memory, block)? {
            let streaming = block.emits_non_temporal_store();
            let entries = self
                .machine
                .usage
                .native_entries
                .checked_add(1)
                .ok_or(NativeDispatchError::Budget("native entries"))?;
            let _guard = DispatchGuard::acquire()?;
            let mut context = GuestContext::zeroed();
            context.gpr = self.machine.state.gpr;
            context.rflags = self.machine.state.rflags;
            context.mxcsr = self.machine.state.mxcsr;
            self.machine.vector_mode.load(
                &mut context,
                &self.machine.state.xmm,
                &self.machine.state.ymm_upper,
            );
            context.guest_base = end(self.machine.stack.stack)?;
            context.guest_limit = self.machine.stack.stack.address;
            context.rip = block.placement().address;
            // Fail closed if anything after actual execution rejects its result.
            self.machine.phase = Phase::Terminal;
            // SAFETY: The caller and live checks establish the exact source,
            // owner, TLS and current operands. Publication either read back a
            // sealed slot or verified its exact live bytes, tail and cell. The
            // bounded placement belongs to this cache. The process-wide guard
            // owns CONTEXT; no Rust reference crosses assembly.
            unsafe {
                *CONTEXT.0.get() = context;
                self.machine.vector_mode.enter();
                if streaming {
                    super::complete_non_temporal_stores();
                }
                context = *CONTEXT.0.get();
            }
            self.machine.usage.native_entries = entries;
            self.machine.state.gpr = context.gpr;
            self.machine.state.rflags = context.rflags;
            self.machine.state.mxcsr = context.mxcsr;
            self.machine.vector_mode.save(
                &context,
                &mut self.machine.state.xmm,
                &mut self.machine.state.ymm_upper,
            );
        } else {
            // An attested UD2-only plan has no native body. Resolve its exit
            // from the same admitted CPU/memory state without entering hardware.
            self.machine.phase = Phase::Terminal;
        }
        self.machine.usage.native_instructions = native_instructions;
        self.machine.state.guest_pc = block.exit().guest_pc();
        validate_state(&self.machine.state)?;
        check_rsp(&self.machine.stack, self.machine.state.gpr[RSP])?;
        let result = self.resolve(memory, block)?;
        if let NativeBlockStop::Returned { value } = result {
            self.machine.observed_return = Some(CallObservation {
                owner: self.machine.owner,
                executor: self.machine.executor,
                call: self.machine.call_sequence,
                initial: self.machine.call_initial,
                stack: self.machine.stack,
                value,
            });
        }
        Ok(result)
    }

    fn check_target(
        &self,
        memory: &PausedNativeMemory<'_>,
        target: u64,
    ) -> Result<(), NativeDispatchError> {
        if target >= NATIVE_ADDRESS_LIMIT {
            return Err(NativeDispatchError::InvalidTarget(target));
        }
        let interval = match self.target_interval.get() {
            Some(interval) if interval.contains(target) => interval,
            _ => initialized_target_interval(self.source, target)?,
        };
        // The memo proves only immutable source membership. Every target,
        // including a different byte in the same interval, checks this owner's
        // current logical-X backing. Failed scans/fetches publish no new memo.
        memory.preflight_fetch_source(target, 1)?;
        self.target_interval.set(Some(interval));
        Ok(())
    }

    fn preflight_block(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<(), NativeDispatchError> {
        if self.machine.phase != Phase::Ready {
            return Err(NativeDispatchError::WrongPhase);
        }
        if memory.owner_id() != self.machine.owner {
            return Err(NativeDispatchError::WrongOwner);
        }
        if !std::ptr::eq(block.declaration(), self.source)
            || !std::ptr::eq(block.source_plan(), self.source.source_plan())
            || block.guest_pc() != self.machine.state.guest_pc
        {
            return Err(NativeDispatchError::WrongSource);
        }
        validate_state(&self.machine.state)?;
        validate_stack(
            memory,
            &self.machine.stack,
            self.machine.state.gpr[RSP],
            &mut self.machine.usage,
            &self.machine.limits,
        )?;
        charge(
            &mut self.machine.usage.blocks,
            1,
            self.machine.limits.max_blocks,
            "blocks",
        )?;
        charge(
            &mut self.machine.usage.source_bytes,
            block.usage().source_bytes,
            self.machine.limits.max_source_bytes,
            "source bytes",
        )?;
        // Replacing the planner's two-byte UD2 needs a six-byte jump plus its
        // eight-byte cell. Account for all bytes actually published/read back.
        let native_bytes = (block.bytes().len() as u64)
            .checked_add(12)
            .ok_or(NativeDispatchError::Budget("emitted bytes"))?;
        charge(
            &mut self.machine.usage.emitted_bytes,
            native_bytes,
            self.machine.limits.max_emitted_bytes,
            "emitted bytes",
        )?;
        // Preserve the original record and feature-visit charges even though
        // their immutable totals were computed once during plan construction.
        charge(
            &mut self.machine.usage.work_units,
            block.instructions().len() as u64,
            self.machine.limits.max_work_units,
            "work",
        )?;
        let work = block
            .native_admission()
            .dispatch_work_units()
            .checked_add(
                self.machine
                    .target_visits
                    .checked_mul(3)
                    .ok_or(NativeDispatchError::Budget("work"))?,
            )
            // Conservative copy/initialization/attestation visits. Dependency
            // tables, allocator bookkeeping and OS-call internals are outside
            // this deterministic work-unit policy.
            .and_then(|v| v.checked_add(native_bytes.checked_mul(6)?))
            .and_then(|v| v.checked_add(block.usage().source_bytes.checked_mul(3)?))
            .and_then(|v| v.checked_add(block.native_admission().feature_visits()))
            .and_then(|v| v.checked_add(16))
            .ok_or(NativeDispatchError::Budget("work"))?;
        charge(
            &mut self.machine.usage.work_units,
            work,
            self.machine.limits.max_work_units,
            "work",
        )?;
        Ok(())
    }

    /// Validate the exact pending syscall without advancing its continuation.
    /// Services with an irreversible ownership transfer must perform this check
    /// before changing that ownership, while retaining exclusive executor access.
    /// This proves executor phase, not authority to invoke a guest service.
    pub fn validate_pending_syscall(
        &self,
        token: PreSyscallToken,
    ) -> Result<(), NativeDispatchError> {
        if self.machine.phase != Phase::Syscall(token) || token.executor != self.machine.executor {
            return Err(NativeDispatchError::WrongPhase);
        }
        Ok(())
    }

    /// Complete only the currently pending token. The dedicated caller uses this
    /// after the core adapter attests the site/fullID and performs the service;
    /// possession of this token alone does not certify service authority.
    /// This is the explicit PentaX marker continuation, not firmware FMASK ABI.
    pub fn complete_syscall(
        &mut self,
        token: PreSyscallToken,
        return_value: u64,
    ) -> Result<(), NativeDispatchError> {
        self.validate_pending_syscall(token)?;
        self.machine.state.gpr[RAX] = return_value;
        self.machine.state.gpr[RCX] = token.next_pc;
        self.machine.state.gpr[11] = token.rflags;
        self.machine.state.guest_pc = token.next_pc;
        self.machine.phase = Phase::Ready;
        Ok(())
    }

    fn publish(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<bool, NativeDispatchError> {
        if ((block.policy().requests_native_packed_float_memory()
            || block.policy().requests_native_streaming_stores()
            || block.policy().requests_native_vector_moves()
            || block.policy().requests_native_scalar_float())
            && !block.policy().requests_native_ymm_registers())
            || ((block.policy().requests_native_ymm_registers()
                || block.policy().requests_native_packed_float_memory()
                || block.policy().requests_native_streaming_stores()
                || block.policy().requests_native_vector_moves()
                || block.policy().requests_native_scalar_float())
                && self.machine.vector_mode.features().is_none())
        {
            return Err(NativeDispatchError::InvalidState);
        }
        let stride = self.machine.cache.len / self.machine.limits.cache_slots;
        let base_offset = block
            .placement()
            .address
            .checked_sub(self.machine.cache.address())
            .and_then(|value| usize::try_from(value).ok())
            .filter(|offset| {
                offset.is_multiple_of(stride) && *offset / stride < self.machine.limits.cache_slots
            })
            .ok_or(NativeDispatchError::EmissionMismatch)?;
        let slot = base_offset / stride;
        if block.placement() != self.slot_placement(slot) {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        if (block.native_admission().requires_streaming_profile()
            && !block.policy().requests_native_streaming_stores())
            || !self
                .machine
                .vector_mode
                .supports_admission(block.native_admission())
        {
            return Err(NativeDispatchError::InvalidState);
        }
        attest_live_source(memory, block)?;
        let bytes = block.bytes();
        if bytes.len() < 2 || !block.attest_emission(block.placement().address, bytes) {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        let offset = bytes.len() - 2;
        if !block.attest_exit(block.placement().address + offset as u64, &bytes[offset..]) {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        if offset == 0 {
            return Ok(false);
        }
        let cell_offset = stride - TAIL_CELL;
        if offset.checked_add(6).is_none_or(|end| end > cell_offset) {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        let trailer = self.machine.vector_mode.trailer();
        let relative = i32::try_from(cell_offset as i64 - offset as i64 - 6)
            .map_err(|_| NativeDispatchError::EmissionMismatch)?;
        let mut tail = [0xff, 0x25, 0, 0, 0, 0];
        tail[2..].copy_from_slice(&relative.to_le_bytes());
        let page_mask = NATIVE_GUEST_PAGE as usize - 1;
        let page_start = base_offset & !page_mask;
        let page_end = (base_offset + stride + page_mask) & !page_mask;
        let page_bytes = page_end - page_start;
        if self
            .machine
            .cache
            .range_executable(page_start, page_bytes)?
            && self.machine.published_lengths[slot] == Some(offset + 6)
            && self.machine.cache.matches(base_offset, &bytes[..offset])?
            && self.machine.cache.matches(base_offset + offset, &tail)?
            && self
                .machine
                .cache
                .matches(base_offset + cell_offset, &trailer.to_le_bytes())?
        {
            self.machine.usage.cache_reuses = self
                .machine
                .usage
                .cache_reuses
                .checked_add(1)
                .ok_or(NativeDispatchError::Budget("cache reuse observations"))?;
            return Ok(true);
        }
        let publications = self.machine.usage.cache_publications.checked_add(1).ok_or(
            NativeDispatchError::Budget("cache publication observations"),
        )?;
        self.machine.published_lengths[slot] = None;
        self.machine
            .cache
            .arm_writable_range(page_start, page_bytes)?;
        self.machine.cache.write(base_offset, &bytes[..offset])?;
        self.machine.cache.write(base_offset + offset, &tail)?;
        self.machine
            .cache
            .write(base_offset + cell_offset, &trailer.to_le_bytes())?;
        // Independent native-tail attestation: the planner's UD2 proof is not
        // reused after replacing it with the engine-controlled jump/cell.
        if !self.machine.cache.matches(base_offset, &bytes[..offset])?
            || !self.machine.cache.matches(base_offset + offset, &tail)?
            || !self
                .machine
                .cache
                .matches(base_offset + cell_offset, &trailer.to_le_bytes())?
        {
            return Err(NativeDispatchError::EmissionMismatch);
        }
        self.machine
            .cache
            .seal_executable_range(page_start, page_bytes)?;
        self.machine.published_lengths[slot] = Some(offset + 6);
        self.machine.usage.cache_publications = publications;
        Ok(true)
    }

    fn resolve(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        block: &MaterializedBlockPlan<'_, 'plan, 'image>,
    ) -> Result<NativeBlockStop, NativeDispatchError> {
        if block
            .exit()
            .instruction()
            .is_some_and(cpu_identification::supported)
        {
            return self.emulate_cpu_identification(memory, block);
        }
        if block
            .exit()
            .instruction()
            .is_some_and(timestamp_counter::supported)
        {
            return self.emulate_timestamp(memory, block);
        }
        if block
            .exit()
            .instruction()
            .is_some_and(cache_hints::supported)
        {
            return self.emulate_cache_hint(memory, block);
        }
        if block.exit().instruction().is_some_and(strings::supported) {
            return self.emulate_string(memory, block);
        }
        if block.exit().instruction().is_some_and(vectors::supported) {
            return self.emulate_vector(memory, block);
        }
        if block
            .exit()
            .instruction()
            .is_some_and(bit_counts::supported)
        {
            return self.emulate_bit_count(memory, block);
        }
        let target = match block.exit().kind() {
            ExitKind::Fallthrough { target } => target,
            ExitKind::Conditional {
                condition,
                target,
                fallthrough,
            } => {
                if condition_holds(condition, self.machine.state.rflags)
                    .ok_or(NativeDispatchError::InvalidState)?
                {
                    target
                } else {
                    fallthrough
                }
            }
            ExitKind::Jump { target } => self.target(memory, target)?,
            ExitKind::Call { target, return_pc } => {
                let target = self.target(memory, target)?;
                self.check_target(memory, target)?;
                self.check_target(memory, return_pc)?;
                let rsp = self.machine.state.gpr[RSP]
                    .checked_sub(8)
                    .ok_or(NativeDispatchError::InvalidStack)?;
                check_rsp(&self.machine.stack, rsp)?;
                // A failed write leaves both the actual stack and context RSP
                // unchanged. Native straight-line work before this exit is not
                // claimed transactional and is never automatically retried.
                memory.write(rsp, &return_pc.to_le_bytes())?;
                self.machine.state.gpr[RSP] = rsp;
                target
            }
            ExitKind::Return { adjust } => {
                let rsp = self.machine.state.gpr[RSP];
                let mut bytes = [0; 8];
                memory.read(rsp, &mut bytes)?;
                let target = u64::from_le_bytes(bytes);
                let next_rsp = rsp
                    .checked_add(8)
                    .and_then(|v| v.checked_add(u64::from(adjust)))
                    .ok_or(NativeDispatchError::InvalidStack)?;
                check_rsp(&self.machine.stack, next_rsp)?;
                if target == self.machine.stack.return_marker {
                    if rsp != self.machine.stack.return_slot || adjust != 0 {
                        return Err(NativeDispatchError::InvalidTarget(target));
                    }
                    self.machine.state.gpr[RSP] = next_rsp;
                    self.machine.state.guest_pc = target;
                    self.machine.phase = Phase::Returned;
                    return Ok(NativeBlockStop::Returned {
                        value: self.machine.state.gpr[RAX],
                    });
                }
                self.check_target(memory, target)?;
                self.machine.state.gpr[RSP] = next_rsp;
                target
            }
            ExitKind::Syscall => {
                let mut vector_args = [[0; 16]; 8];
                for (argument, register) in vector_args
                    .iter_mut()
                    .zip(self.machine.state.xmm.chunks_exact(16))
                {
                    argument.copy_from_slice(register);
                }
                let token = PreSyscallToken {
                    owner: self.machine.owner,
                    executor: self.machine.executor,
                    sequence: self.machine.usage.blocks,
                    syscall_pc: block.exit().guest_pc(),
                    stack_pointer: self.machine.state.gpr[RSP],
                    next_pc: block.exit().next_guest_pc(),
                    service_id: self.machine.state.gpr[RAX],
                    args: [
                        self.machine.state.gpr[RDI],
                        self.machine.state.gpr[RSI],
                        self.machine.state.gpr[RDX],
                        self.machine.state.gpr[RCX],
                        self.machine.state.gpr[8],
                        self.machine.state.gpr[9],
                    ],
                    vector_args,
                    rflags: self.machine.state.rflags,
                };
                self.check_target(memory, token.next_pc)?;
                self.machine.phase = Phase::Syscall(token);
                return Ok(NativeBlockStop::PreSyscall(token));
            }
            ExitKind::Mediated {
                reason: pentax_codegen::UnsupportedReason::SegmentPrefix,
            } => {
                return self.emulate_segment(memory, block);
            }
            _ => {
                return Ok(NativeBlockStop::Unsupported {
                    guest_pc: block.exit().guest_pc(),
                });
            }
        };
        self.check_target(memory, target)?;
        self.machine.state.guest_pc = target;
        self.machine.phase = Phase::Ready;
        Ok(NativeBlockStop::Resume { guest_pc: target })
    }

    fn target(
        &self,
        memory: &PausedNativeMemory<'_>,
        target: ControlTarget,
    ) -> Result<u64, NativeDispatchError> {
        match target {
            ControlTarget::Direct(address) => Ok(address),
            ControlTarget::Register(register) => gpr_slot(register)
                .map(|index| self.machine.state.gpr[index])
                .ok_or(NativeDispatchError::InvalidState),
            ControlTarget::Memory(operand) => {
                let mut context = GuestContext::zeroed();
                context.gpr = self.machine.state.gpr;
                let address = memory_effective_address(&context, &operand)
                    .ok_or(NativeDispatchError::InvalidState)?;
                let mut bytes = [0; 8];
                memory.read(address, &mut bytes)?;
                Ok(u64::from_le_bytes(bytes))
            }
        }
    }
}

#[cfg(test)]
#[path = "prepared/tests.rs"]
mod tests;

impl Drop for NativeBlockMachine {
    fn drop(&mut self) {
        self.state = NativeCpuState::new(0, 0);
        self.call_initial = NativeCpuState::new(0, 0);
        self.observed_return = None;
        self.phase = Phase::Terminal;
    }
}

// Shared entry-window attestation; neither this check nor a builtin bypass
// grants authority to execute a supplied library's body.
fn attest_live_source(
    memory: &PausedNativeMemory<'_>,
    block: &MaterializedBlockPlan<'_, '_, '_>,
) -> Result<(), NativeDispatchError> {
    let end_pc = if block.exit().instruction().is_some() {
        block.exit().next_guest_pc()
    } else {
        block.exit().guest_pc()
    };
    let consumed = usize::try_from(
        end_pc
            .checked_sub(block.guest_pc())
            .ok_or(NativeDispatchError::SourceMismatch)?,
    )
    .map_err(|_| NativeDispatchError::SourceMismatch)?;
    let expected = block
        .relocated_source_bytes()
        .get(..consumed)
        .ok_or(NativeDispatchError::SourceMismatch)?;
    // Source logical-X remains physically NX and must also be logically
    // immutable. Safe re-protection between steps cannot admit RWX source.
    let mut address = block.guest_pc();
    while address < end_pc {
        match memory.preflight_write(address, 1) {
            Err(NativeMemoryError::PermissionDenied(_)) => {}
            Ok(()) => return Err(NativeDispatchError::WritableSource),
            Err(error) => return Err(error.into()),
        }
        address = (address | (NATIVE_GUEST_PAGE - 1))
            .checked_add(1)
            .ok_or(NativeDispatchError::SourceMismatch)?;
    }
    // One small scratch buffer serves every entry. The exclusive pause
    // keeps these bounded comparisons coherent even for larger callers.
    let mut live = [0; 128];
    for (index, chunk) in expected.chunks(live.len()).enumerate() {
        let address = block
            .guest_pc()
            .checked_add((index * live.len()) as u64)
            .ok_or(NativeDispatchError::SourceMismatch)?;
        memory.fetch_source(address, &mut live[..chunk.len()])?;
        if live[..chunk.len()] != *chunk {
            return Err(NativeDispatchError::SourceMismatch);
        }
    }
    Ok(())
}

fn snapshot_declarations(
    source: &MaterializedSource<'_, '_>,
    usage: &mut NativeDispatchUsage,
    limits: &NativeDispatchLimits,
) -> Result<Vec<pentax_codegen::DeclaredAppliedWrite>, NativeDispatchError> {
    let count = source.declared_writes().len();
    let bytes = count
        .checked_mul(std::mem::size_of::<pentax_codegen::DeclaredAppliedWrite>())
        .filter(|bytes| *bytes <= 64 * 1024 * 1024)
        .ok_or(NativeDispatchError::Budget("source declarations"))?;
    charge(
        &mut usage.work_units,
        bytes as u64,
        limits.max_work_units,
        "source declarations",
    )?;
    let mut result = Vec::new();
    result
        .try_reserve_exact(count)
        .map_err(|_| NativeDispatchError::Allocation)?;
    if result
        .capacity()
        .checked_mul(std::mem::size_of::<pentax_codegen::DeclaredAppliedWrite>())
        .is_none_or(|size| size > 64 * 1024 * 1024)
    {
        return Err(NativeDispatchError::Budget("source declarations"));
    }
    result.extend_from_slice(source.declared_writes());
    Ok(result)
}

fn charge(
    value: &mut u64,
    amount: u64,
    limit: u64,
    name: &'static str,
) -> Result<(), NativeDispatchError> {
    let next = value
        .checked_add(amount)
        .filter(|v| *v <= limit)
        .ok_or(NativeDispatchError::Budget(name))?;
    *value = next;
    Ok(())
}
fn end(range: NativeRange) -> Result<u64, NativeDispatchError> {
    range
        .address
        .checked_add(range.size)
        .filter(|v| range.size != 0 && *v <= NATIVE_ADDRESS_LIMIT)
        .ok_or(NativeDispatchError::InvalidStack)
}
fn validate_state(state: &NativeCpuState) -> Result<(), NativeDispatchError> {
    const FLAGS: u64 = (1 << 0)
        | (1 << 1)
        | (1 << 2)
        | (1 << 4)
        | (1 << 6)
        | (1 << 7)
        | (1 << 9)
        | (1 << 10)
        | (1 << 11);
    if state
        .fs_base
        .is_some_and(|base| base >= NATIVE_ADDRESS_LIMIT)
        || state
            .gs_base
            .is_some_and(|base| base >= NATIVE_ADDRESS_LIMIT)
        || state.rflags & !FLAGS != 0
        || state.rflags & 0x202 != 0x202
        || !mxcsr_control::valid(state.mxcsr)
        || state.x87.control_word != x87_control::control_word(state.x87.control_word)
        || state.x87.instruction_opcode & !0x07ff != 0
    {
        return Err(NativeDispatchError::InvalidState);
    }
    Ok(())
}
fn check_rsp(stack: &NativeStackFrame, rsp: u64) -> Result<(), NativeDispatchError> {
    let minimum = stack
        .stack
        .address
        .checked_add(128)
        .ok_or(NativeDispatchError::InvalidStack)?;
    if (minimum..=end(stack.stack)?).contains(&rsp) {
        Ok(())
    } else {
        Err(NativeDispatchError::InvalidStack)
    }
}
fn validate_stack(
    memory: &PausedNativeMemory<'_>,
    stack: &NativeStackFrame,
    rsp: u64,
    usage: &mut NativeDispatchUsage,
    limits: &NativeDispatchLimits,
) -> Result<(), NativeDispatchError> {
    let stack_end = end(stack.stack)?;
    if stack
        .return_slot
        .checked_add(8)
        .is_none_or(|v| v > stack_end)
        || stack.return_slot < stack.stack.address
        || stack.return_slot % 16 != 8
    {
        return Err(NativeDispatchError::InvalidStack);
    }
    charge(
        &mut usage.work_units,
        stack.stack.size.div_ceil(NATIVE_GUEST_PAGE) + 1,
        limits.max_work_units,
        "stack work",
    )?;
    if let Some([lower, upper]) = stack.guards {
        for range in [stack.stack, lower, upper] {
            if !range.address.is_multiple_of(NATIVE_GUEST_PAGE)
                || !range.size.is_multiple_of(NATIVE_GUEST_PAGE)
            {
                return Err(NativeDispatchError::InvalidStack);
            }
            end(range)?;
        }
        if end(lower)? != stack.stack.address || stack_end != upper.address {
            return Err(NativeDispatchError::InvalidStack);
        }
        charge(
            &mut usage.work_units,
            lower.size / NATIVE_GUEST_PAGE + upper.size / NATIVE_GUEST_PAGE,
            limits.max_work_units,
            "stack work",
        )?;
        memory.preflight_reserved_unmapped(lower.address, lower.size)?;
        memory.preflight_reserved_unmapped(upper.address, upper.size)?;
    }
    if memory
        .preflight_read_write_non_executable(stack.stack.address, stack.stack.size)
        .is_err()
    {
        // The common valid span needs one live page walk. On failure preserve
        // the original read-before-write-before-NX error and address priority.
        memory.preflight_read(stack.stack.address, stack.stack.size)?;
        memory.preflight_write(stack.stack.address, stack.stack.size)?;
        if memory
            .preflight_non_executable(stack.stack.address, stack.stack.size)
            .is_err()
        {
            return Err(NativeDispatchError::InvalidStack);
        }
    }
    check_rsp(stack, rsp)
}
fn check_target(
    source: &MaterializedSource<'_, '_>,
    memory: &PausedNativeMemory<'_>,
    target: u64,
) -> Result<(), NativeDispatchError> {
    initialized_target_interval(source, target)?;
    memory.preflight_fetch_source(target, 1)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TargetInterval {
    start: u64,
    end: u64,
}
impl TargetInterval {
    fn contains(self, target: u64) -> bool {
        self.start <= target && target < self.end
    }
}

struct TargetSegment {
    bias: u64,
    vaddr: u64,
    initialized_bytes: u64,
    eligible: bool,
}

fn initialized_target_interval(
    source: &MaterializedSource<'_, '_>,
    target: u64,
) -> Result<TargetInterval, NativeDispatchError> {
    // Preserve the original scanner's lazy module/segment order. In
    // particular, an earlier eligible overflow still precedes a later match;
    // later malformed intervals are not visited after an earlier match.
    scan_target_segments(
        target,
        source.source_plan().modules().iter().flat_map(|module| {
            module
                .runtime_plan()
                .segments
                .iter()
                .map(move |segment| TargetSegment {
                    bias: module.load_bias(),
                    vaddr: segment.vaddr,
                    initialized_bytes: segment.file_bytes.len() as u64,
                    eligible: segment.flags.executable() && !segment.flags.writable(),
                })
        }),
    )
}

fn scan_target_segments(
    target: u64,
    segments: impl IntoIterator<Item = TargetSegment>,
) -> Result<TargetInterval, NativeDispatchError> {
    if target >= NATIVE_ADDRESS_LIMIT {
        return Err(NativeDispatchError::InvalidTarget(target));
    }
    for segment in segments {
        if !segment.eligible {
            continue;
        }
        let start = segment
            .vaddr
            .checked_add(segment.bias)
            .ok_or(NativeDispatchError::InvalidTarget(target))?;
        let interval = TargetInterval {
            start,
            end: start
                .checked_add(segment.initialized_bytes)
                .ok_or(NativeDispatchError::InvalidTarget(target))?,
        };
        if interval.contains(target) {
            return Ok(interval);
        }
    }
    Err(NativeDispatchError::InvalidTarget(target))
}
