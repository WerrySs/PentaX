// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance at 209dffe9032d72749257ce18680d506b891284de:
// ir/passes/DeadCodeElimination.cpp (RemoveIdentities, EliminateDeadCode with
// dead phi webs), ResourceTracking.cpp (Tracker::Collect/AddBuffer/Merge for
// buffer handles), ShaderInfoCollection.cpp (stage inputs/outputs) and
// BindingLayout.cpp (AllocateBindings, CollectShaderData, NativeBinding).
// PentaX modifications: liveness is a marked worklist from side effects and
// branch conditions; only descriptors made of four consecutive unmodified
// user SGPRs are tracked (other SRT/BDA forms are typed refusals) and are
// validated against the exact caller profile; undefined-register and
// subgroup policies are explicit checks.

pub(crate) mod fold;
mod images;
pub(crate) use images::{discover_images, track_images};
pub(crate) mod ssa;

use pentax_shader::ShaderStage;

use crate::CompileError;
use crate::budget::{Budget, LimitKind};
use crate::error::{InterfaceErrorKind, Register, ResourceErrorKind, UnsupportedReason};
use crate::ir::{Builtin, ExportKind, InstId, Ir, Op, Value};
use crate::profile::{BufferAccess, MAX_BUFFERS, MrtFormat, Profile, UndefinedRegisterPolicy};
use crate::translate::var;

/// Source `RemoveIdentities`: every operand and branch condition is
/// rewritten to its resolved value.
pub(crate) fn remove_identities(ir: &mut Ir, budget: &mut Budget) -> Result<(), CompileError> {
    budget.charge_usize(ir.args.len() + ir.blocks.len())?;
    for index in 0..ir.args.len() {
        let resolved = ir.resolve(budget, ir.args[index])?;
        ir.args[index] = resolved;
    }
    for block in 0..ir.blocks.len() {
        if let Some(condition) = ir.blocks[block].condition {
            ir.blocks[block].condition = Some(ir.resolve(budget, condition)?);
        }
    }
    Ok(())
}

/// Source `EliminateDeadCode` (including dead phi webs) by marking from
/// side effects and branch conditions.
pub(crate) fn eliminate_dead_code(ir: &mut Ir, budget: &mut Budget) -> Result<(), CompileError> {
    remove_identities(ir, budget)?;
    let count = ir.insts.len();
    let mut live = budget.table(count, false, usize::MAX, LimitKind::Bytes)?;
    let mut work: Vec<InstId> = Vec::new();
    let mark = |id: InstId, live: &mut Vec<bool>, work: &mut Vec<InstId>, budget: &mut Budget| {
        if let Some(slot) = live.get_mut(id.index())
            && !*slot
        {
            *slot = true;
            return budget.push(work, id, usize::MAX, LimitKind::Bytes);
        }
        Ok(())
    };
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            if ir.inst(id).op.has_side_effects() {
                mark(id, &mut live, &mut work, budget)?;
            }
        }
        if let Some(Value::Inst(id)) = ir.blocks[block as usize].condition {
            mark(id, &mut live, &mut work, budget)?;
        }
    }
    while let Some(id) = work.pop() {
        for index in 0..ir.nargs(id) {
            budget.charge(1)?;
            if let Value::Inst(arg) = ir.resolve(budget, ir.arg(id, index))?
                && arg.index() < count
            {
                mark(arg, &mut live, &mut work, budget)?;
            }
        }
    }
    for block in 0..ir.blocks.len() as u32 {
        let ids = ssa::collect_block(ir, budget, block)?;
        for id in ids {
            if !live[id.index()] {
                ir.unlink(id);
            }
        }
    }
    Ok(())
}

fn register_of(variable: u32) -> Register {
    match variable {
        v if v < var::SGPRS => Register::Sgpr(v as u8),
        v if v < var::MASK_TAG => Register::SgprMask((v - var::THREAD_BIT) as u8),
        v if v < var::SCC => Register::SgprMask((v - var::MASK_TAG) as u8),
        var::SCC => Register::Scc,
        var::EXEC => Register::Exec,
        var::EXEC_LO => Register::ExecLo,
        var::EXEC_HI => Register::ExecHi,
        var::VCC => Register::Vcc,
        var::VCC_LO => Register::VccLo,
        var::VCC_HI => Register::VccHi,
        var::M0 => Register::M0,
        v => Register::Vgpr((v - var::VGPR).min(255) as u8),
    }
}

/// The first live reader PC of each live instruction matching `predicate`.
fn first_reader(ir: &Ir, budget: &mut Budget, target: InstId) -> Result<u32, CompileError> {
    let mut pc = u32::MAX;
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            for index in 0..ir.nargs(id) {
                budget.charge(1)?;
                if ir.arg(id, index) == Value::Inst(target) {
                    pc = pc.min(ir.inst(id).pc);
                }
            }
        }
        if ir.blocks[block as usize].condition == Some(Value::Inst(target)) {
            pc = pc.min(ir.blocks[block as usize].start_pc);
        }
    }
    Ok(if pc == u32::MAX {
        ir.inst(target).pc
    } else {
        pc
    })
}

/// Undefined-register policy and the exact-subgroup requirement. Returns the
/// number of live undefined reads (zero under `Reject`).
pub(crate) fn check_policies(
    ir: &Ir,
    budget: &mut Budget,
    profile: &Profile,
) -> Result<u32, CompileError> {
    let mut undefined = 0u32;
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            let inst = ir.inst(id);
            match inst.op {
                Op::Undef => match profile.common().undefined_registers {
                    UndefinedRegisterPolicy::Reject => {
                        let pc = first_reader(ir, budget, id)?;
                        return Err(CompileError::UndefinedRegister {
                            register: register_of(inst.aux),
                            pc,
                        });
                    }
                    UndefinedRegisterPolicy::Undefined => undefined += 1,
                },
                Op::Ballot | Op::AnyLane if profile.common().subgroup.is_none() => {
                    return Err(CompileError::unsupported(
                        inst.pc,
                        None,
                        UnsupportedReason::RequiresExactSubgroup,
                    ));
                }
                Op::LaneId
                    if profile.common().subgroup.is_none() && !profile.local_wave64_lane_id() =>
                {
                    return Err(CompileError::unsupported(
                        inst.pc,
                        None,
                        UnsupportedReason::RequiresExactSubgroup,
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(undefined)
}

/// One tracked buffer (source `BufferResource`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct BufferUse {
    pub user_sgpr: u8,
    pub profile_index: usize,
    pub first_use_pc: u32,
    pub max_byte_extent: u32,
    pub read: bool,
    pub written: bool,
    pub scalar: bool,
    pub coherent: bool,
}

/// Source `StageInput` / `StageOutput` (ported subset).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputKind {
    Builtin(Builtin),
    Parameter,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StageInput {
    pub kind: InputKind,
    /// Pixel parameter: the source input index (V_INTERP attribute).
    pub index: u32,
    pub location: u32,
    pub components: u32,
    pub flat: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutputKind {
    Position,
    Parameter,
    Mrt,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct StageOutput {
    pub kind: OutputKind,
    pub index: u32,
    pub location: u32,
    pub uint: bool,
}

/// The result of resource tracking, info collection and binding layout.
#[derive(Debug, Default)]
pub(crate) struct Layout {
    pub buffers: Vec<BufferUse>,
    pub images: Vec<images::ImageUse>,
    pub samplers: Vec<images::SamplerUse>,
    pub lod_counters: Vec<images::LodCounterUse>,
    pub inputs: Vec<StageInput>,
    pub outputs: Vec<StageOutput>,
    pub user_data_registers: Vec<u8>,
    pub memory_offset_dword: u32,
    pub memory_offset_count: u32,
    pub push_data_start: Option<u32>,
    pub shader_data_binding: bool,
    pub undefined_reads: u32,
}

impl Layout {
    pub fn shader_data_dwords(&self) -> u32 {
        self.memory_offset_dword + self.memory_offset_count.div_ceil(4)
    }
}

/// Source descriptor-source recognition for the direct user-data form.
fn user_data_base(ir: &Ir, budget: &Budget, handle: Value, pc: u32) -> Result<u8, CompileError> {
    descriptor_base(ir, budget, handle, pc, Op::GetBufferResource, 4)
}

fn descriptor_base(
    ir: &Ir,
    budget: &Budget,
    handle: Value,
    pc: u32,
    expected_op: Op,
    width: u32,
) -> Result<u8, CompileError> {
    let Some((id, inst)) = ir.producer(budget, handle)? else {
        return Err(CompileError::unsupported(
            pc,
            None,
            UnsupportedReason::DynamicDescriptor,
        ));
    };
    if inst.op != expected_op {
        return Err(CompileError::Internal(
            "buffer operation without buffer handle",
        ));
    }
    let mut base = None;
    for dword in 0..width {
        let value = ir.resolve(budget, ir.arg(id, dword as usize))?;
        match ir.producer(budget, value)? {
            Some((_, d)) if d.op == Op::GetUserData => {
                let expected = base.map_or(d.aux, |b: u32| b + dword);
                if d.aux != expected || (dword == 0 && d.aux % 4 != 0) {
                    return Err(CompileError::unsupported(
                        pc,
                        None,
                        UnsupportedReason::DynamicDescriptor,
                    ));
                }
                if dword == 0 {
                    base = Some(d.aux);
                }
            }
            Some((_, d)) if d.op == Op::Undef && d.aux < var::SGPRS => {
                return Err(CompileError::resource(
                    Some(pc),
                    ResourceErrorKind::OutsideUserData {
                        user_sgpr: d.aux as u8,
                    },
                ));
            }
            _ => {
                return Err(CompileError::unsupported(
                    pc,
                    None,
                    UnsupportedReason::DynamicDescriptor,
                ));
            }
        }
    }
    let base = base.ok_or(CompileError::Internal("descriptor base"))?;
    u8::try_from(base).map_err(|_| CompileError::Internal("descriptor base range"))
}

/// Source `TrackResources` for buffer handles, validated against the profile.
pub(crate) fn discover_buffers(
    ir: &Ir,
    budget: &mut Budget,
) -> Result<Vec<crate::BufferRequirement>, CompileError> {
    let mut buffers: Vec<crate::BufferRequirement> = Vec::new();
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            let inst = ir.inst(id);
            if !inst.op.is_buffer_load() && !inst.op.is_buffer_store() {
                continue;
            }
            let memory = ir
                .memory
                .get(inst.aux as usize)
                .ok_or(CompileError::Internal("memory metadata index"))?;
            let base = user_data_base(ir, budget, ir.arg(id, 0), inst.pc)?;
            let write = inst.op.is_buffer_store();
            let extent = memory.offset.saturating_add(memory.data_dwords.max(1) * 4);
            budget.charge_usize(buffers.len())?;
            if let Some(entry) = buffers.iter_mut().find(|entry| entry.user_sgpr == base) {
                entry.read |= !write;
                entry.written |= write;
                entry.scalar |= memory.scalar;
                entry.coherent |= memory.coherent;
                entry.max_byte_extent = entry.max_byte_extent.max(extent);
                entry.first_use_pc = entry.first_use_pc.min(inst.pc);
            } else {
                if buffers.len() >= MAX_BUFFERS {
                    return Err(CompileError::resource(
                        Some(inst.pc),
                        ResourceErrorKind::BufferLimit,
                    ));
                }
                budget.push(
                    &mut buffers,
                    crate::BufferRequirement {
                        user_sgpr: base,
                        read: !write,
                        written: write,
                        scalar: memory.scalar,
                        coherent: memory.coherent,
                        max_byte_extent: extent,
                        first_use_pc: inst.pc,
                    },
                    MAX_BUFFERS,
                    LimitKind::SourceTable,
                )?;
            }
        }
    }
    Ok(buffers)
}

/// Source `TrackResources` for buffer handles, validated against the profile.
pub(crate) fn track_resources(
    ir: &mut Ir,
    budget: &mut Budget,
    profile: &Profile,
    layout: &mut Layout,
) -> Result<(), CompileError> {
    for block in 0..ir.blocks.len() as u32 {
        let ids = ssa::collect_block(ir, budget, block)?;
        for id in ids {
            budget.charge(1)?;
            let inst = *ir.inst(id);
            if !inst.op.is_buffer_load() && !inst.op.is_buffer_store() {
                continue;
            }
            let memory_index = inst.aux as usize;
            let memory = *ir
                .memory
                .get(memory_index)
                .ok_or(CompileError::Internal("memory metadata index"))?;
            let base = user_data_base(ir, budget, ir.arg(id, 0), inst.pc)?;
            budget.charge_usize(profile.common().buffers.len())?;
            let Some((profile_index, description)) = profile.buffer(base) else {
                return Err(CompileError::resource(
                    Some(inst.pc),
                    ResourceErrorKind::MissingBuffer { user_sgpr: base },
                ));
            };
            let write = inst.op.is_buffer_store();
            let bytes = memory.data_dwords.max(1) * 4;
            let extent = memory.offset.saturating_add(bytes);
            budget.charge_usize(layout.buffers.len())?;
            let resource = match layout.buffers.iter().position(|b| b.user_sgpr == base) {
                Some(index) => index,
                None => {
                    if layout.buffers.len() >= MAX_BUFFERS {
                        return Err(CompileError::resource(
                            Some(inst.pc),
                            ResourceErrorKind::BufferLimit,
                        ));
                    }
                    budget.push(
                        &mut layout.buffers,
                        BufferUse {
                            user_sgpr: base,
                            profile_index,
                            first_use_pc: inst.pc,
                            max_byte_extent: 0,
                            read: false,
                            written: false,
                            scalar: false,
                            coherent: false,
                        },
                        MAX_BUFFERS,
                        LimitKind::SourceTable,
                    )?;
                    layout.buffers.len() - 1
                }
            };
            let entry = &mut layout.buffers[resource];
            entry.first_use_pc = entry.first_use_pc.min(inst.pc);
            entry.max_byte_extent = entry.max_byte_extent.max(extent);
            entry.read |= !write;
            entry.written |= write;
            entry.scalar |= memory.scalar;
            entry.coherent |= memory.coherent;
            match description.access {
                BufferAccess::ReadOnly if write => {
                    return Err(CompileError::resource(
                        Some(inst.pc),
                        ResourceErrorKind::AccessMismatch { user_sgpr: base },
                    ));
                }
                BufferAccess::WriteOnly if !write => {
                    return Err(CompileError::resource(
                        Some(inst.pc),
                        ResourceErrorKind::ReadOfWriteOnly { user_sgpr: base },
                    ));
                }
                _ => {}
            }
            if let Some(info) = ir.memory.get_mut(memory_index) {
                info.resource = resource as u32;
            }
        }
    }
    for buffer in &profile.common().buffers {
        budget.charge_usize(layout.buffers.len())?;
        if !layout
            .buffers
            .iter()
            .any(|b| b.user_sgpr == buffer.user_sgpr)
        {
            return Err(CompileError::resource(
                None,
                ResourceErrorKind::UnusedBuffer {
                    user_sgpr: buffer.user_sgpr,
                },
            ));
        }
    }
    Ok(())
}

fn add_input(
    layout: &mut Layout,
    budget: &mut Budget,
    input: StageInput,
) -> Result<(), CompileError> {
    budget.charge_usize(layout.inputs.len())?;
    if let Some(existing) = layout
        .inputs
        .iter_mut()
        .find(|i| i.kind == input.kind && i.index == input.index)
    {
        existing.components = existing.components.max(input.components);
        return Ok(());
    }
    budget.push(&mut layout.inputs, input, 64, LimitKind::SourceTable)
}

fn builtin_input(kind: Builtin) -> StageInput {
    let components = match kind {
        Builtin::FragCoord => 4,
        Builtin::WorkgroupId | Builtin::LocalInvocationId => 3,
        _ => 1,
    };
    StageInput {
        kind: InputKind::Builtin(kind),
        index: 0,
        location: 0,
        components,
        flat: false,
    }
}

/// Source `ShaderPixelExportTarget` as documented in shader.h: with a CB
/// shader mask, export i goes to the i-th target with a nonzero nibble.
fn pixel_export_target(mask: Option<u32>, export_index: u32) -> Option<u32> {
    let Some(mask) = mask else {
        return Some(export_index);
    };
    let mut seen = 0;
    for target in 0..8 {
        if (mask >> (target * 4)) & 0xf != 0 {
            if seen == export_index {
                return Some(target);
            }
            seen += 1;
        }
    }
    None
}

/// Source `CollectShaderInfo` (with `ValidateValueReferences`).
pub(crate) fn collect_info(
    ir: &Ir,
    budget: &mut Budget,
    profile: &Profile,
    layout: &mut Layout,
) -> Result<(), CompileError> {
    let stage = profile.stage();
    match stage {
        ShaderStage::Vertex => {
            add_input(layout, budget, builtin_input(Builtin::VertexIndex))?;
            add_input(layout, budget, builtin_input(Builtin::InstanceIndex))?;
        }
        ShaderStage::Pixel => {
            let ps = profile
                .pixel()
                .ok_or(CompileError::Internal("pixel profile"))?;
            if ps.position.iter().any(|&p| p) {
                add_input(layout, budget, builtin_input(Builtin::FragCoord))?;
            }
            if ps.front_face {
                add_input(layout, budget, builtin_input(Builtin::FrontFacing))?;
            }
        }
        ShaderStage::Compute => {
            let cs = profile
                .compute()
                .ok_or(CompileError::Internal("compute profile"))?;
            if cs.group_id.iter().any(|&g| g) {
                add_input(layout, budget, builtin_input(Builtin::WorkgroupId))?;
            }
            if cs.thread_ids > 0 {
                add_input(layout, budget, builtin_input(Builtin::LocalInvocationId))?;
            }
            if cs.thread_ids > 0 || cs.tg_size {
                add_input(layout, budget, builtin_input(Builtin::LocalInvocationIndex))?;
            }
        }
        _ => {
            return Err(CompileError::unsupported(
                0,
                None,
                UnsupportedReason::StageNotPorted,
            ));
        }
    }
    let mut used = [false; 32];
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            let inst = *ir.inst(id);
            match inst.op {
                Op::GetAttribute | Op::GetInterpolationParameter => {
                    let ps = profile
                        .pixel()
                        .ok_or(CompileError::Internal("attribute outside pixel"))?;
                    let attribute = inst.aux & 0xff;
                    let channel = (inst.aux >> 8) & 0xff;
                    let mode = (inst.aux >> 16) & 0xff;
                    let Some(input) = ps.inputs.get(attribute as usize) else {
                        return Err(CompileError::interface(
                            Some(inst.pc),
                            InterfaceErrorKind::PixelInputOutOfRange { input: attribute },
                        ));
                    };
                    if channel >= 4 {
                        return Err(CompileError::interface(
                            Some(inst.pc),
                            InterfaceErrorKind::InterpolationChannel { channel },
                        ));
                    }
                    if inst.op == Op::GetInterpolationParameter && (mode != 2 || !input.flat) {
                        // Source: such reads use PerVertexKHR raw vertex inputs.
                        return Err(CompileError::unsupported(
                            inst.pc,
                            None,
                            UnsupportedReason::PerVertexInterpolation,
                        ));
                    }
                    if inst.op == Op::GetAttribute && mode != 0 && mode != 1 && !input.flat {
                        // Centroid/sample/linear decorations of KYTY_INTERP_MODES.
                        return Err(CompileError::unsupported(
                            inst.pc,
                            None,
                            UnsupportedReason::PerVertexInterpolation,
                        ));
                    }
                    used[attribute as usize] = true;
                }
                Op::GetBuiltin => {
                    let kind = Builtin::from_code(inst.aux >> 8)
                        .ok_or(CompileError::Internal("builtin kind"))?;
                    match kind {
                        Builtin::BaryCoordSmooth
                        | Builtin::BaryCoordSmoothCentroid
                        | Builtin::BaryCoordSmoothSample
                        | Builtin::BaryCoordNoPerspective
                        | Builtin::BaryCoordNoPerspectiveCentroid
                        | Builtin::BaryCoordNoPerspectiveSample => {
                            return Err(CompileError::unsupported(
                                inst.pc,
                                None,
                                UnsupportedReason::PerVertexInterpolation,
                            ));
                        }
                        _ => add_input(layout, budget, builtin_input(kind))?,
                    }
                }
                _ => {}
            }
        }
    }
    if let Some(ps) = profile.pixel() {
        for (index, input) in ps.inputs.iter().enumerate() {
            if used[index] {
                add_input(
                    layout,
                    budget,
                    StageInput {
                        kind: InputKind::Parameter,
                        index: index as u32,
                        location: u32::from(input.location),
                        components: 4,
                        flat: input.flat,
                    },
                )?;
            }
        }
    }
    // Source CollectOutputs.
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            let inst = *ir.inst(id);
            if inst.op != Op::SetAttribute {
                continue;
            }
            let exp = *ir
                .exports
                .get(inst.aux as usize)
                .ok_or(CompileError::Internal("export index"))?;
            if exp.enabled == 0 {
                continue;
            }
            let output = match exp.kind {
                ExportKind::Position => StageOutput {
                    kind: OutputKind::Position,
                    index: 0,
                    location: 0,
                    uint: false,
                },
                ExportKind::Parameter => StageOutput {
                    kind: OutputKind::Parameter,
                    index: exp.index,
                    location: exp.index,
                    uint: false,
                },
                ExportKind::Mrt => {
                    let ps = profile
                        .pixel()
                        .ok_or(CompileError::Internal("MRT outside pixel"))?;
                    let Some(slot) = pixel_export_target(ps.target_shader_mask, exp.index) else {
                        // Source: an export without a colour target writes nothing.
                        continue;
                    };
                    StageOutput {
                        kind: OutputKind::Mrt,
                        index: exp.index,
                        location: slot,
                        uint: ps.mrt_formats.get(exp.index as usize) == Some(&MrtFormat::Uint),
                    }
                }
                ExportKind::Null | ExportKind::MrtZ => continue,
            };
            budget.charge_usize(layout.outputs.len())?;
            if !layout
                .outputs
                .iter()
                .any(|o| o.kind == output.kind && o.index == output.index)
            {
                budget.push(&mut layout.outputs, output, 64, LimitKind::SourceTable)?;
            }
        }
    }
    Ok(())
}

/// Source `AllocateBindings` (user data and memory offsets in push data,
/// buffers in the Buffers array; separate sampled-image and sampler bindings;
/// no GDS, LDS or SRT).
pub(crate) fn allocate_bindings(
    ir: &Ir,
    budget: &mut Budget,
    profile: &Profile,
    layout: &mut Layout,
) -> Result<(), CompileError> {
    let mut registers = [false; 106];
    for block in 0..ir.blocks.len() as u32 {
        for id in ir.block_insts(block) {
            budget.charge(1)?;
            let inst = ir.inst(id);
            if inst.op == Op::GetUserData {
                let slot = registers
                    .get_mut(inst.aux as usize)
                    .ok_or(CompileError::Internal("user data register"))?;
                *slot = true;
            }
        }
    }
    for (reg, &used) in registers.iter().enumerate() {
        if used {
            budget.push(
                &mut layout.user_data_registers,
                reg as u8,
                106,
                LimitKind::SourceTable,
            )?;
        }
    }
    layout.memory_offset_dword = layout.user_data_registers.len() as u32;
    layout.memory_offset_count = layout.buffers.len() as u32;
    let size = layout.shader_data_dwords();
    let start = u32::from(profile.common().push_data_start_dword);
    // Source PushData::CanFit / StartFor.
    layout.push_data_start = (size != 0 && start <= 32 && size <= 32 - start).then_some(start);
    layout.shader_data_binding = size != 0 && layout.push_data_start.is_none();
    Ok(())
}
