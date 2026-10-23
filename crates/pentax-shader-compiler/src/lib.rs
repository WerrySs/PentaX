// SPDX-License-Identifier: GPL-2.0-only
// Ported from KytyPS5-Performance at 209dffe9032d72749257ce18680d506b891284de.
// See README.md and NOTICE for source attribution and modifications.

//! A bounded, safe Rust port of the KytyPS5-Performance shader compiler path:
//! program boundary validation, `CFG::BuildGraph` and `CFG::Structurize`,
//! the frontend translators into a typed IR, SSA construction, constant
//! propagation, dead-code elimination, resource tracking, stage
//! interface collection, binding layout and binary SPIR-V emission.
//!
//! It is not an independent implementation and not a complete port: only the
//! instruction families, stages (vertex, pixel, compute) and resource forms
//! documented in the README are lowered; everything else is a typed
//! [`CompileError::Unsupported`] with the original byte PC and opcode.
//! Inputs are caller-delimited programs decoded by the read-only
//! `pentax-shader` crate plus a validated caller [`Profile`]; no guest
//! memory, descriptors, titles or environment are consulted.

#![forbid(unsafe_code)]

mod analysis;
mod budget;
pub mod cfg;
mod error;
mod ir;
mod manifest;
mod passes;
mod profile;
mod program;
mod spirv;
mod translate;

#[cfg(test)]
mod tests;

pub use analysis::{
    BufferRequirement, ImageRequirement, SamplerRequirement, ShaderAnalysis, analyze,
    analyze_native_indirect_vertex,
};
pub use budget::{LimitKind, Limits};
pub use cfg::Cfg;
pub use error::*;
pub use manifest::*;
pub use profile::*;
pub use program::{BranchTarget, ShaderProgram};

/// Exact source revision of the port.
pub const SOURCE_REVISION: &str = "209dffe9032d72749257ce18680d506b891284de";

/// A compiled module and its manifest. Nothing is returned on failure.
#[derive(Debug)]
pub struct CompiledShader {
    words: Vec<u32>,
    manifest: Manifest,
    work: u64,
    retained_bytes: usize,
}

impl CompiledShader {
    /// Generated SPIR-V words. Independent validation is performed with the
    /// official SPIR-V Tools in the original-fixture checks.
    pub fn spirv(&self) -> &[u32] {
        &self.words
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    /// Work units spent by `compile` (CFG through emission).
    pub fn work_units(&self) -> u64 {
        self.work
    }
    /// Arena capacity bytes charged during compilation.
    pub fn charged_bytes(&self) -> usize {
        self.retained_bytes
    }
    /// Actual retained SPIR-V and recursive manifest vector capacities.
    /// Compiler CFG/IR/layout scratch has been dropped before `compile`
    /// returns. This differs from the monotonic arena charge above.
    pub fn retained_host_bytes(&self) -> usize {
        self.words
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>())
            .saturating_add(self.manifest.retained_host_bytes())
    }
}

/// Compile a validated program for a validated profile (source
/// `TranslateProgram` + `CompileProgram` for the ported subset).
pub fn compile(
    program: &ShaderProgram,
    profile: &Profile,
    limits: &Limits,
) -> Result<CompiledShader, CompileError> {
    let mut budget = budget::Budget::new(limits);
    let (ir, undefined) = lower(program, profile, &mut budget)?;
    finish(ir, profile, budget, undefined)
}

fn lower(
    program: &ShaderProgram,
    profile: &Profile,
    budget: &mut budget::Budget,
) -> Result<(ir::Ir, u32), CompileError> {
    if program.stage() != profile.stage() {
        return Err(CompileError::Profile(ProfileError::StageMismatch));
    }
    let cfg = Cfg::build_in(program, budget)?;
    let mut ir = translate::translate_program(&cfg, profile, budget)?;
    passes::ssa::rewrite_to_ssa(&mut ir, budget)?;
    passes::fold::constant_propagation(&mut ir, budget)?;
    passes::eliminate_dead_code(&mut ir, budget)?;
    let undefined = passes::check_policies(&ir, budget, profile)?;
    Ok((ir, undefined))
}

fn finish(
    mut ir: ir::Ir,
    profile: &Profile,
    mut budget: budget::Budget,
    undefined: u32,
) -> Result<CompiledShader, CompileError> {
    let mut layout = passes::Layout {
        undefined_reads: undefined,
        ..passes::Layout::default()
    };
    passes::track_resources(&mut ir, &mut budget, profile, &mut layout)?;
    passes::track_images(&mut ir, &mut budget, profile, &mut layout)?;
    passes::collect_info(&ir, &mut budget, profile, &mut layout)?;
    passes::allocate_bindings(&ir, &mut budget, profile, &mut layout)?;
    let module = spirv::emit::emit_program(&ir, &layout, profile, &mut budget)?;
    let manifest = manifest_of(
        &layout,
        profile,
        module.uses_subgroup,
        module.required_features,
        &mut budget,
    )?;
    Ok(CompiledShader {
        words: module.words,
        manifest,
        work: budget.work(),
        retained_bytes: budget.bytes(),
    })
}

fn manifest_of(
    layout: &passes::Layout,
    profile: &Profile,
    uses_subgroup: bool,
    required_features: RequiredFeatures,
    budget: &mut budget::Budget,
) -> Result<Manifest, CompileError> {
    use passes::{InputKind, OutputKind};
    let stage = profile.stage();
    let mut user_sgprs = Vec::new();
    budget.reserve(
        &mut user_sgprs,
        layout.user_data_registers.len(),
        106,
        LimitKind::SourceTable,
    )?;
    budget.charge_usize(layout.user_data_registers.len())?;
    user_sgprs.extend_from_slice(&layout.user_data_registers);
    let push_data = layout.push_data_start.map(|start| PushData {
        start_dword: start,
        user_sgprs,
        memory_offset_dword: layout.memory_offset_dword,
        memory_offset_count: layout.memory_offset_count,
    });
    let mut buffers = Vec::new();
    for buffer in &layout.buffers {
        budget.push(
            &mut buffers,
            BufferElement {
                user_sgpr: buffer.user_sgpr,
                profile_index: buffer.profile_index,
                read: buffer.read,
                written: buffer.written,
                scalar: buffer.scalar,
                coherent: buffer.coherent,
                max_byte_extent: buffer.max_byte_extent,
                first_use_pc: buffer.first_use_pc,
            },
            64,
            LimitKind::SourceTable,
        )?;
    }
    let mut images = Vec::new();
    let mut samplers = Vec::new();
    for (index, entry) in layout.images.iter().enumerate() {
        budget.push(
            &mut images,
            ImageElement {
                user_sgpr: entry.user_sgpr,
                profile_index: entry.profile_index,
                binding: spirv::emit::native_binding(
                    stage,
                    spirv::emit::BINDING_IMAGES + index as u32,
                ),
                first_use_pc: entry.first_use_pc,
            },
            profile::MAX_IMAGES,
            LimitKind::SourceTable,
        )?;
    }
    for (index, entry) in layout.samplers.iter().enumerate() {
        budget.push(
            &mut samplers,
            SamplerElement {
                user_sgpr: entry.user_sgpr,
                profile_index: entry.profile_index,
                binding: spirv::emit::native_binding(
                    stage,
                    spirv::emit::BINDING_SAMPLERS + index as u32,
                ),
                first_use_pc: entry.first_use_pc,
            },
            profile::MAX_IMAGES,
            LimitKind::SourceTable,
        )?;
    }
    let mut inputs = Vec::new();
    for input in &layout.inputs {
        let entry = match input.kind {
            InputKind::Builtin(kind) => match kind {
                ir::Builtin::VertexIndex => InputInterface::VertexIndex,
                ir::Builtin::InstanceIndex => InputInterface::InstanceIndex,
                ir::Builtin::BaseInstance => InputInterface::BaseInstance,
                ir::Builtin::FragCoord => InputInterface::FragCoord,
                ir::Builtin::FrontFacing => InputInterface::FrontFacing,
                ir::Builtin::WorkgroupId => InputInterface::WorkgroupId,
                ir::Builtin::LocalInvocationId => InputInterface::LocalInvocationId,
                ir::Builtin::LocalInvocationIndex => InputInterface::LocalInvocationIndex,
                _ => return Err(CompileError::Internal("manifest builtin")),
            },
            InputKind::Parameter => InputInterface::Parameter {
                input: input.index,
                location: input.location,
                flat: input.flat,
            },
        };
        budget.push(&mut inputs, entry, 128, LimitKind::SourceTable)?;
    }
    if uses_subgroup {
        budget.push(
            &mut inputs,
            InputInterface::SubgroupLocalInvocationId,
            128,
            LimitKind::SourceTable,
        )?;
    }
    let mut outputs = Vec::new();
    for output in &layout.outputs {
        let entry = match output.kind {
            OutputKind::Position => OutputInterface::Position,
            OutputKind::Parameter => OutputInterface::Parameter {
                index: output.index,
                location: output.location,
            },
            OutputKind::Mrt => OutputInterface::Color {
                export: output.index,
                location: output.location,
                uint: output.uint,
            },
        };
        budget.push(&mut outputs, entry, 128, LimitKind::SourceTable)?;
    }
    Ok(Manifest {
        vertex_instance_contract: profile.vertex_instance_contract(),
        required_features,
        stage,
        push_data,
        shader_data_binding: layout
            .shader_data_binding
            .then(|| spirv::emit::native_binding(stage, spirv::emit::BINDING_SHADER_DATA)),
        buffer_binding: (!layout.buffers.is_empty())
            .then(|| spirv::emit::native_binding(stage, spirv::emit::BINDING_BUFFERS)),
        buffers,
        images,
        samplers,
        inputs,
        outputs,
        workgroup_size: profile.compute().map(|cs| cs.workgroup_size),
        required_subgroup_size: if uses_subgroup {
            profile.common().subgroup.map(|s| s.size)
        } else {
            None
        },
        undefined_register_reads: layout.undefined_reads,
    })
}
