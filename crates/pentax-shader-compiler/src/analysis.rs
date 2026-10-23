// SPDX-License-Identifier: GPL-2.0-only
// PentaX staged access to the source-pinned post-SSA resource tracker.
// See NOTICE: ResourceTracking.cpp and BindingLayout.cpp at 209dffe.

use crate::budget::Budget;
use crate::{
    BufferDescription, CompileError, CompiledShader, LimitKind, Limits, Profile, ProfileError,
    ShaderProgram, ir::Ir, passes,
};

/// One live direct V# requirement, in the resource tracker's first-use order.
/// These are shader facts, not descriptor values, memory permissions or bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferRequirement {
    pub user_sgpr: u8,
    pub read: bool,
    pub written: bool,
    pub scalar: bool,
    pub coherent: bool,
    pub max_byte_extent: u32,
    pub first_use_pc: u32,
}

/// One live direct eight-dword image descriptor. No memory or device authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageRequirement {
    pub user_sgpr: u8,
    pub first_use_pc: u32,
}

/// One live direct four-dword sampler descriptor.
pub type SamplerRequirement = ImageRequirement;

/// Opaque optimized IR and its original stage contract. The same finite budget
/// continues through specialization and emission; no IR or profile mutation is
/// exposed. Dropping this value performs no guest or device operation.
pub struct ShaderAnalysis {
    ir: Ir,
    profile: Profile,
    budget: Budget,
    undefined: u32,
    requirements: Vec<BufferRequirement>,
    images: Vec<ImageRequirement>,
    samplers: Vec<SamplerRequirement>,
}

impl ShaderAnalysis {
    pub fn images(&self) -> &[ImageRequirement] {
        &self.images
    }
    pub fn samplers(&self) -> &[SamplerRequirement] {
        &self.samplers
    }

    pub fn buffers(&self) -> &[BufferRequirement] {
        &self.requirements
    }

    pub fn work_units(&self) -> u64 {
        self.budget.work()
    }

    pub fn charged_bytes(&self) -> usize {
        self.budget.bytes()
    }

    /// Continue this exact analysis with explicit descriptor specializations.
    /// Only buffer fields may change; stage entry, user range, interfaces and
    /// floating/subgroup policy remain those used for translation. Tracking
    /// checks missing/unused buffers and actual read/write access again.
    pub fn finish(
        self,
        descriptions: &[BufferDescription],
    ) -> Result<CompiledShader, CompileError> {
        self.finish_with_images(descriptions, &[], &[])
    }

    /// Specialize exact live resources without exposing IR or descriptor memory.
    /// Image descriptions admit only float sampled 2D images, sampler descriptions
    /// only normalized, non-comparison sampling; backend ownership stays external.
    pub fn finish_with_images(
        self,
        descriptions: &[BufferDescription],
        image_descriptions: &[crate::ImageDescription],
        sampler_descriptions: &[crate::SamplerDescription],
    ) -> Result<CompiledShader, CompileError> {
        let Self {
            ir,
            profile,
            mut budget,
            undefined,
            requirements,
            images,
            samplers,
        } = self;
        drop((requirements, images, samplers));
        let mut images = Vec::new();
        let mut samplers = Vec::new();
        budget.reserve(
            &mut images,
            image_descriptions.len(),
            crate::profile::MAX_IMAGES,
            LimitKind::SourceTable,
        )?;
        budget.reserve(
            &mut samplers,
            sampler_descriptions.len(),
            crate::profile::MAX_IMAGES,
            LimitKind::SourceTable,
        )?;
        budget.charge_usize(image_descriptions.len() + sampler_descriptions.len())?;
        images.extend_from_slice(image_descriptions);
        samplers.extend_from_slice(sampler_descriptions);
        let mut buffers = Vec::new();
        budget.reserve(
            &mut buffers,
            descriptions.len(),
            crate::profile::MAX_BUFFERS,
            LimitKind::SourceTable,
        )?;
        budget.charge_usize(descriptions.len())?;
        buffers.extend_from_slice(descriptions);
        let profile = profile
            .with_buffers(buffers)
            .and_then(|profile| profile.with_images(images, samplers))
            .map_err(CompileError::Profile)?;
        crate::finish(ir, &profile, budget, undefined)
    }
}

/// Lower once through the same CFG, SSA, constant propagation, dead-code and
/// policy passes as `compile`, then discover actual live direct resources.
/// The caller supplies an empty resource profile and later supplies exact
/// specializations; no descriptor memory or callbacks enter the compiler.
pub fn analyze(
    program: &ShaderProgram,
    profile: Profile,
    limits: &Limits,
) -> Result<ShaderAnalysis, CompileError> {
    if !profile.common().buffers.is_empty()
        || !profile.common().images.is_empty()
        || !profile.common().samplers.is_empty()
    {
        return Err(CompileError::Profile(ProfileError::PredeclaredBuffers));
    }
    let mut budget = Budget::new(limits);
    let (ir, undefined) = crate::lower(program, &profile, &mut budget)?;
    let requirements = passes::discover_buffers(&ir, &mut budget)?;
    let (images, samplers) = passes::discover_images(&ir, &mut budget)?;
    Ok(ShaderAnalysis {
        ir,
        profile,
        budget,
        undefined,
        requirements,
        images,
        samplers,
    })
}

/// Analyze only the native indexed-indirect vertex convention. The caller must
/// bind execution to firstInstance0; the read-only emitted manifest retains
/// that dependency for preparation/backend validation. This creates no GPU
/// authority and changes no generic compile/analyze/profile policy.
pub fn analyze_native_indirect_vertex(
    program: &ShaderProgram,
    description: crate::VertexDescription,
    limits: &Limits,
) -> Result<ShaderAnalysis, CompileError> {
    let profile = Profile::new(crate::StageDescription::Vertex(description))
        .map_err(CompileError::Profile)?
        .native_indirect_vertex()
        .map_err(CompileError::Profile)?;
    analyze(program, profile, limits)
}
