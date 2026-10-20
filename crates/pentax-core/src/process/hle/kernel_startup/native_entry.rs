// SPDX-License-Identifier: GPL-2.0-only

//! Source-bound main-entry call context on actual owned guest pages.
//! The observed profile follows pinned Kyty Execute/RunEntry; it does not run
//! module initializers, decode process parameters or implement exit callbacks.

use super::{KernelStartupError, PreparedNativeKernelStartup, PreparedNativeSource};
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE};
use crate::process::hle::kernel_provider::{PROGRAM_NAME_BUFFER_RVA, PROGRAM_NAME_CAPACITY};
use pentax_linker::SymbolicLinkPlan;
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryError, NativeMemoryTransaction, NativeMemoryWrite,
    NativeOwnerId, NativePermissions, NativeRange, NativeTransactionLimits, PausedNativeMemory,
};
use std::fmt;

const ARGUMENT_AREA: usize = 0x100;
const PARAMETER_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeEntryPolicy {
    /// Observed argc=1, three argv slots, SysV RDI/RSI and zero root frame.
    /// This describes a call context, not the complete firmware entry ABI.
    KytyObservedV1,
}

/// Numeric request data, never execution or preparation authority. The stack
/// and its two adjacent page guards must already be reserved by this owner and
/// entirely unmapped. The exit callback must be initialized immutable guest
/// executable code; host pointers and missing callbacks are not synthesized.
#[derive(Debug, Clone, Copy)]
pub struct NativeEntryRequest<'a> {
    pub policy: NativeEntryPolicy,
    pub stack: NativeRange,
    pub program_name: &'a [u8],
    pub exit_handler: u64,
    pub return_marker: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeEntryLimits {
    pub max_stack_bytes: u64,
    pub max_name_bytes: usize,
    /// Conservative metadata, source and argument visits, separately from the
    /// transaction's page/overlap bound and the stack's committed-byte bound.
    pub max_work_steps: u64,
    pub transaction: NativeTransactionLimits,
}
impl Default for NativeEntryLimits {
    fn default() -> Self {
        Self {
            max_stack_bytes: 8 * 1024 * 1024,
            max_name_bytes: ARGUMENT_AREA - PARAMETER_BYTES - 1,
            max_work_steps: 65_536,
            transaction: NativeTransactionLimits::default(),
        }
    }
}

/// Initial guest call registers and checked storage geometry. These integers
/// alone neither attest a live mapping nor authorize native instruction entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeEntryLayout {
    pub entry: u64,
    pub stack: NativeRange,
    pub guards: [NativeRange; 2],
    pub return_slot: u64,
    pub return_marker: u64,
    pub frame_pointer: u64,
    pub arguments: u64,
    pub argument_name: u64,
    pub exit_handler: u64,
}

/// Preparation history for one source/owner, not continuous liveness or a
/// completed process lifecycle. Only the checked core method constructs it.
///
/// ```compile_fail
/// use pentax_core::process::hle::{PreparedNativeEntry, NativeEntryLayout};
/// use pentax_linker::SymbolicLinkPlan;
/// use pentax_platform::native_memory::NativeOwnerId;
/// fn forge<'p, 'i>(owner: NativeOwnerId, source: &'p SymbolicLinkPlan<'i>,
///     layout: NativeEntryLayout) -> PreparedNativeEntry<'p, 'i> {
///     PreparedNativeEntry { owner, source, layout }
/// }
/// ```
#[derive(Debug)]
pub struct PreparedNativeEntry<'plan, 'image> {
    owner: NativeOwnerId,
    source: &'plan SymbolicLinkPlan<'image>,
    layout: NativeEntryLayout,
}
impl<'plan, 'image> PreparedNativeEntry<'plan, 'image> {
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner
    }
    pub const fn source_plan(&self) -> &'plan SymbolicLinkPlan<'image> {
        self.source
    }
    pub const fn layout(&self) -> &NativeEntryLayout {
        &self.layout
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeEntryError {
    Startup(KernelStartupError),
    Memory(NativeMemoryError),
    WrongPreparedSource,
    InvalidLayout,
    InvalidName,
    InvalidTarget,
    Budget,
}
impl fmt::Display for NativeEntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native entry preparation: {self:?}")
    }
}
impl std::error::Error for NativeEntryError {}
impl From<KernelStartupError> for NativeEntryError {
    fn from(value: KernelStartupError) -> Self {
        Self::Startup(value)
    }
}
impl From<NativeMemoryError> for NativeEntryError {
    fn from(value: NativeMemoryError) -> Self {
        Self::Memory(value)
    }
}

fn charge(work: &mut u64, amount: u64, limits: &NativeEntryLimits) -> Result<(), NativeEntryError> {
    *work = work
        .checked_add(amount)
        .filter(|value| *value <= limits.max_work_steps)
        .ok_or(NativeEntryError::Budget)?;
    Ok(())
}

fn target(
    plan: &SymbolicLinkPlan<'_>,
    paused: &PausedNativeMemory<'_>,
    address: u64,
    primary_only: bool,
    work: &mut u64,
    limits: &NativeEntryLimits,
) -> Result<(), NativeEntryError> {
    if address >= ADDRESS_LIMIT {
        return Err(NativeEntryError::InvalidTarget);
    }
    for module in plan
        .modules()
        .iter()
        .take(if primary_only { 1 } else { usize::MAX })
    {
        charge(work, 1, limits)?;
        for segment in &module.runtime_plan().segments {
            charge(work, 1, limits)?;
            if !segment.flags.executable() || segment.flags.writable() {
                continue;
            }
            let start = module.load_bias().checked_add(segment.vaddr);
            let end = start.and_then(|value| value.checked_add(segment.file_bytes.len() as u64));
            if let (Some(start), Some(end)) = (start, end)
                && start <= address
                && address < end
            {
                // Instruction-source access requires X, independently of guest
                // data-read permission. Keep execute-only mappings unchanged.
                paused.preflight_fetch_source(address, 1)?;
                if !matches!(
                    paused.preflight_write(address, 1),
                    Err(NativeMemoryError::PermissionDenied(_))
                ) {
                    return Err(NativeEntryError::InvalidTarget);
                }
                return Ok(());
            }
        }
    }
    Err(NativeEntryError::InvalidTarget)
}

impl<'plan, 'image> PreparedNativeKernelStartup<'plan, 'image> {
    /// Atomically stage a main-entry stack, root frame and argc/argv object.
    /// All validation precedes the map/output transaction. Recoverable errors
    /// leave the stack unmapped; a poisoned OS owner must be discarded.
    /// This neither executes the entry nor supplies missing lifecycle work.
    pub fn prepare_process_entry(
        &mut self,
        proof: &PreparedNativeSource<'plan, 'image>,
        request: NativeEntryRequest<'_>,
        limits: &NativeEntryLimits,
    ) -> Result<PreparedNativeEntry<'plan, 'image>, NativeEntryError> {
        let source = proof.materialized_source().source_plan();
        if proof.owner_id() != self.owner_id()
            || !std::ptr::eq(source, self.plan().link_plan())
            || !std::ptr::eq(source, self.memory().source_plan())
        {
            return Err(NativeEntryError::WrongPreparedSource);
        }
        if self.initial_entry.is_some() {
            return Err(NativeEntryError::InvalidLayout);
        }
        let NativeEntryPolicy::KytyObservedV1 = request.policy;
        if request.program_name.is_empty()
            || request.program_name.len() > limits.max_name_bytes
            || request.program_name.len() >= ARGUMENT_AREA - PARAMETER_BYTES
        {
            return Err(NativeEntryError::InvalidName);
        }
        let mut work = 0;
        charge(&mut work, request.program_name.len() as u64 + 38, limits)?;
        if request.program_name.contains(&0) {
            return Err(NativeEntryError::InvalidName);
        }
        let stack = request.stack;
        let stack_end = stack
            .address
            .checked_add(stack.size)
            .filter(|end| {
                end.checked_add(PAGE_SIZE)
                    .is_some_and(|value| value <= ADDRESS_LIMIT)
            })
            .ok_or(NativeEntryError::InvalidLayout)?;
        if stack.address < PAGE_SIZE
            || !stack.address.is_multiple_of(PAGE_SIZE)
            || stack.size == 0
            || !stack.size.is_multiple_of(PAGE_SIZE)
            || stack.size > limits.max_stack_bytes
            || request.return_marker == 0
            || request.return_marker >= ADDRESS_LIMIT
        {
            return Err(NativeEntryError::InvalidLayout);
        }
        let arguments = stack_end
            .checked_sub(ARGUMENT_AREA as u64)
            .ok_or(NativeEntryError::InvalidLayout)?;
        // Execute passes params-0x1000 to RunEntry, which aligns it and lays
        // down two zero words; its CALL then pushes one guest return address.
        let frame_pointer = arguments
            .checked_sub(0x1000 + 16)
            .ok_or(NativeEntryError::InvalidLayout)?;
        let return_slot = frame_pointer
            .checked_sub(8)
            .filter(|value| {
                value
                    .checked_sub(128)
                    .is_some_and(|red_zone| red_zone >= stack.address)
            })
            .ok_or(NativeEntryError::InvalidLayout)?;
        let layout = NativeEntryLayout {
            entry: source.primary_entry(),
            stack,
            guards: [
                NativeRange {
                    address: stack.address - PAGE_SIZE,
                    size: PAGE_SIZE,
                },
                NativeRange {
                    address: stack_end,
                    size: PAGE_SIZE,
                },
            ],
            return_slot,
            return_marker: request.return_marker,
            frame_pointer,
            arguments,
            argument_name: arguments + PARAMETER_BYTES as u64,
            exit_handler: request.exit_handler,
        };
        charge(
            &mut work,
            stack.size / PAGE_SIZE + 2 + ARGUMENT_AREA as u64 + 24 + PROGRAM_NAME_CAPACITY as u64,
            limits,
        )?;
        let program_name_address = self.plan().kernel_provider_bias() + PROGRAM_NAME_BUFFER_RVA;
        let mut program_name = [0; PROGRAM_NAME_CAPACITY];
        program_name[..request.program_name.len()].copy_from_slice(request.program_name);
        let mut paused = self.pause()?;
        target(source, &paused, layout.entry, true, &mut work, limits)?;
        target(
            source,
            &paused,
            layout.exit_handler,
            false,
            &mut work,
            limits,
        )?;
        for guard in layout.guards {
            paused.preflight_reserved_unmapped(guard.address, guard.size)?;
        }
        paused.preflight_reserved_unmapped(stack.address, stack.size)?;
        let mut arguments = [0; ARGUMENT_AREA];
        arguments[..4].copy_from_slice(&1_u32.to_le_bytes());
        arguments[8..16].copy_from_slice(&layout.argument_name.to_le_bytes());
        arguments[PARAMETER_BYTES..PARAMETER_BYTES + request.program_name.len()]
            .copy_from_slice(request.program_name);
        let mut frame = [0; 24];
        frame[..8].copy_from_slice(&layout.return_marker.to_le_bytes());
        let writes = [
            NativeMemoryWrite {
                address: layout.arguments,
                bytes: &arguments,
            },
            NativeMemoryWrite {
                address: layout.return_slot,
                bytes: &frame,
            },
            NativeMemoryWrite {
                address: program_name_address,
                bytes: &program_name,
            },
        ];
        paused.apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::MapZeroed {
                    address: stack.address,
                    size: stack.size,
                    permissions: NativePermissions::READ_WRITE,
                },
                writes: &writes,
            },
            &limits.transaction,
        )?;
        let owner = paused.owner_id();
        self.initial_entry = Some(layout);
        Ok(PreparedNativeEntry {
            owner,
            source,
            layout,
        })
    }
}
