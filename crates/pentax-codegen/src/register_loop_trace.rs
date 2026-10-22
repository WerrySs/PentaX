// SPDX-License-Identifier: GPL-2.0-only
//! Optional register-only finite loops; no ordinary plan gains guest control.
use crate::*;
use iced_x86::{Encoder, FlowControl};
use std::fmt;

const EXIT: [u8; 6] = [0x0f, 0x0b, 0x90, 0x90, 0x90, 0x90];

#[derive(Default)]
pub(crate) struct Preparation {
    attempted: bool,
    pub(crate) allocated: u64,
    work: u64,
    template: Option<Template>,
}
struct Template {
    bytes: Vec<u8>,
    second_iteration: usize,
    first_exit: usize,
    second_exit: usize,
}

/// A borrowed view tied to the exact materialized block and its owned template.
/// It contains no live permission, hardware capability or host handler pointer.
pub struct RegisterLoopTrace<'block, 'view, 'plan, 'image> {
    block: &'block MaterializedBlockPlan<'view, 'plan, 'image>,
    template: &'block Template,
}
impl fmt::Debug for RegisterLoopTrace<'_, '_, '_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisterLoopTrace")
            .field("iterations", &2)
            .field("emitted_bytes", &self.template.bytes.len())
            .finish_non_exhaustive()
    }
}
impl<'block, 'view, 'plan, 'image> RegisterLoopTrace<'block, 'view, 'plan, 'image> {
    pub fn block(&self) -> &'block MaterializedBlockPlan<'view, 'plan, 'image> {
        self.block
    }
    /// Prefix ending before the two reserved cells. The two six-byte private
    /// exits must be bound and attested by an actual native runtime.
    pub fn bytes(&self) -> &'block [u8] {
        &self.template.bytes
    }
    pub fn placement(&self) -> CachePlacement {
        self.block.placement()
    }
    pub fn first_exit_offset(&self) -> usize {
        self.template.first_exit
    }
    pub fn second_exit_offset(&self) -> usize {
        self.template.second_exit
    }
    pub fn second_iteration_offset(&self) -> usize {
        self.template.second_iteration
    }
    pub fn preparation_work(&self) -> u64 {
        self.block.register_loop.work
    }
}

struct Bounds {
    body: usize,
    prefix: usize,
    work: u64,
}
fn bounds(block: &MaterializedBlockPlan<'_, '_, '_>, limits: &BlockLimits) -> Option<Bounds> {
    if !block.policy().requests_register_loop_trace()
        || block.register_loop.attempted
        || !block.native_admission().is_memory_free()
        || block.native_admission().has_atomic_memory()
        || block.emits_non_temporal_store()
    {
        return None;
    }
    match block.exit().kind() {
        ExitKind::Conditional {
            condition, target, ..
        } if condition != ConditionCode::None && target == block.guest_pc() => {}
        _ => return None,
    }
    let body = block.instructions().len().checked_sub(1)?;
    if body == 0
        || block.native_admission().emitted_instructions() != body as u64
        || block.instructions().last()?.emitted_len() != 0
        || (body as u64).checked_add(1)?.checked_mul(2)? > limits.max_instructions
    {
        return None;
    }
    let prefix = (body as u64)
        .checked_mul(30)?
        .checked_add(18)?
        .min(block.placement().capacity.checked_sub(16)?)
        .min(limits.max_output_bytes.checked_sub(16)?);
    if block
        .bytes()
        .len()
        .checked_sub(2)?
        .checked_add(body)?
        .checked_add(18)?
        > usize::try_from(prefix).ok()?
    {
        return None;
    }
    // Two bounded metadata visits per record, <=45 units for each encoder
    // attempt/retry, output allocation/copy, fixed metadata and scratch.
    let work = (body as u64)
        .checked_mul(128)?
        .checked_add(prefix.checked_mul(2)?)?
        .checked_add(256)?;
    if work
        > limits
            .max_work_units
            .checked_sub(block.usage().work_units)?
    {
        return None;
    }
    Some(Bounds {
        body,
        prefix: usize::try_from(prefix).ok()?,
        work,
    })
}

impl<'view, 'plan, 'image> MaterializedBlockPlan<'view, 'plan, 'image> {
    /// O(1) conservative optional reservation, before any extra scan/allocation.
    /// None leaves ordinary execution available and grants no trace authority.
    pub fn register_loop_trace_work_bound(&self, limits: &BlockLimits) -> Option<u64> {
        bounds(self, limits).map(|bound| bound.work)
    }
    /// Attempt at most once after an exclusive outer-work reservation. Every
    /// performed unit is returned, even on optional refusal. The ordinary plan
    /// and its usage/admission remain intact; no guest access or effect occurs.
    pub fn prepare_register_loop_trace(
        &mut self,
        limits: &BlockLimits,
        reserved_work: u64,
        max_total_allocation_bytes: u64,
    ) -> u64 {
        let Some(bound) = bounds(self, limits) else {
            return 0;
        };
        if reserved_work < bound.work {
            return 0;
        }
        let Some(available) = max_total_allocation_bytes
            .min(limits.max_allocation_bytes)
            .checked_sub(self.retained_allocation_bytes())
        else {
            return 0;
        };
        let mut accounting = Accounting {
            work: 0,
            allocated: 0,
            limits: BlockLimits {
                max_work_units: bound.work,
                max_allocation_bytes: available,
                ..*limits
            },
        };
        self.register_loop.attempted = true;
        let template = build(self, &bound, &mut accounting).ok().flatten();
        self.register_loop.allocated = accounting.allocated;
        self.register_loop.work = accounting.work;
        self.register_loop.template = template;
        accounting.work
    }
    pub fn register_loop_trace(&self) -> Option<RegisterLoopTrace<'_, 'view, 'plan, 'image>> {
        self.register_loop
            .template
            .as_ref()
            .map(|template| RegisterLoopTrace {
                block: self,
                template,
            })
    }
}

fn inverse_opcode(condition: ConditionCode) -> Option<u8> {
    Some(match condition {
        ConditionCode::o => 0x81,
        ConditionCode::no => 0x80,
        ConditionCode::b => 0x83,
        ConditionCode::ae => 0x82,
        ConditionCode::e => 0x85,
        ConditionCode::ne => 0x84,
        ConditionCode::be => 0x87,
        ConditionCode::a => 0x86,
        ConditionCode::s => 0x89,
        ConditionCode::ns => 0x88,
        ConditionCode::p => 0x8b,
        ConditionCode::np => 0x8a,
        ConditionCode::l => 0x8d,
        ConditionCode::ge => 0x8c,
        ConditionCode::le => 0x8f,
        ConditionCode::g => 0x8e,
        _ => return None,
    })
}
fn declined<T>() -> Result<Option<T>, CodegenError> {
    Ok(None)
}

fn build(
    block: &MaterializedBlockPlan<'_, '_, '_>,
    bound: &Bounds,
    accounting: &mut Accounting,
) -> Result<Option<Template>, CodegenError> {
    let records = &block.instructions()[..bound.body];
    for record in records {
        accounting.charge(64)?;
        let instruction = record.instruction();
        let Some((memory, dependencies)) = record.immutable_memory_summary() else {
            return declined();
        };
        // Immutable memory-free admission is independently checked per record.
        // Register aliasing needs no predicted state: B2 executes the original
        // operations on the actual hardware outputs of B1. RSP and potentially
        // trapping integer division remain outside this optional contract.
        if memory
            || dependencies.address != 0
            || record.emitted_len() == 0
            || instruction.flow_control() != FlowControl::Next
            || record.has_atomic_memory()
            || instruction.is_privileged()
            || matches!(instruction.mnemonic(), Mnemonic::Div | Mnemonic::Idiv)
            || dependencies.written & (1 << 4) != 0
            || (0..instruction.op_count()).any(|operand| {
                instruction.op_kind(operand) == OpKind::Register
                    && instruction.op_register(operand).full_register() == Register::RSP
            })
        {
            return declined();
        }
    }
    let ExitKind::Conditional { condition, .. } = block.exit().kind() else {
        return declined();
    };
    let Some(inverse) = inverse_opcode(condition) else {
        return declined();
    };
    let metadata = std::mem::size_of::<Template>() as u64;
    accounting.charge(metadata)?;
    accounting.allocated = accounting
        .allocated
        .checked_add(metadata)
        .filter(|bytes| *bytes <= accounting.limits.max_allocation_bytes)
        .ok_or(CodegenError::Budget {
            kind: Budget::AllocationBytes,
            limit: accounting.limits.max_allocation_bytes,
        })?;
    let mut bytes = accounting.buffer::<u8>(bound.prefix)?;
    let scratch = accounting.buffer::<u8>(15)?;
    let mut encoder = Encoder::new(64);
    encoder.set_buffer(scratch);
    let first = &block.bytes()[..block.bytes().len() - 2];
    accounting.charge(first.len() as u64)?;
    bytes.extend_from_slice(first);
    let branch = bytes.len();
    accounting.charge(18)?;
    bytes.extend([0x0f, inverse, 0, 0, 0, 0]);
    let second_iteration = bytes.len();
    for record in records {
        accounting.charge(30)?;
        let ip = block
            .placement()
            .address
            .checked_add(bytes.len() as u64)
            .ok_or(CodegenError::InvalidAddress)?;
        let instruction = record.instruction();
        let mut emission = *instruction;
        if let Some(target) = record.rip_relative_target() {
            let delta = i128::from(target) - i128::from(ip) - instruction.len() as i128;
            let eip_lea = instruction.memory_base() == Register::EIP
                && instruction.mnemonic() == Mnemonic::Lea;
            if target >= crate::source::ADDRESS_LIMIT || !eip_lea && i32::try_from(delta).is_err() {
                if instruction.mnemonic() != Mnemonic::Lea {
                    return declined();
                }
                emission = crate::plan::lea_constant(instruction, target)?;
            }
        }
        let mut result = encoder.encode(&emission, ip);
        if result.is_err()
            && instruction.mnemonic() == Mnemonic::Lea
            && let Some(target) = record.rip_relative_target()
        {
            accounting.charge(15)?;
            let mut scratch = encoder.take_buffer();
            scratch.clear();
            encoder.set_buffer(scratch);
            emission = crate::plan::lea_constant(instruction, target)?;
            result = encoder.encode(&emission, ip);
        }
        let Ok(emitted) = result else {
            return declined();
        };
        let mut encoded = encoder.take_buffer();
        if bytes
            .len()
            .checked_add(emitted)
            .and_then(|len| len.checked_add(12))
            .is_none_or(|len| len > bound.prefix)
        {
            return declined();
        }
        bytes.extend_from_slice(&encoded);
        encoded.clear();
        encoder.set_buffer(encoded);
    }
    let second_exit = bytes.len();
    bytes.extend(EXIT);
    let first_exit = bytes.len();
    bytes.extend(EXIT);
    let displacement =
        i32::try_from(first_exit - branch - 6).map_err(|_| CodegenError::InvalidAddress)?;
    bytes[branch + 2..branch + 6].copy_from_slice(&displacement.to_le_bytes());
    Ok(Some(Template {
        bytes,
        second_iteration,
        first_exit,
        second_exit,
    }))
}
