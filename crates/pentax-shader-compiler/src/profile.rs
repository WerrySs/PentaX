// SPDX-License-Identifier: GPL-2.0-only
// Caller-owned stage contracts adapted from KytyPS5-Performance 209dffe:
// shader.h (ShaderVertexInputInfo, ShaderPixelInputInfo,
// ShaderComputeInputInfo, ShaderWorkgroupInputInfo), shaderBindings.h
// (ShaderBufferResource stride/swizzle/index-stride fields) and
// ShaderRecompiler.h (CompileOptions wave size and user-data range).
// PentaX modifications: only typed, validated, finite fields; no guest
// pointers, descriptor memory, title or environment state. The buffer
// specialization replaces the runtime V# read of ApplyResourceSpecialization
// with explicit caller values; PS input locations are caller-supplied results
// of the source ShaderPixelParameterLocation linkage step.

use pentax_shader::ShaderStage;

use crate::error::ProfileError;

/// Guest wave size (source `CompileOptions::wave_size`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaveSize {
    Wave32,
    Wave64,
}

impl WaveSize {
    pub fn lanes(self) -> u32 {
        match self {
            Self::Wave32 => 32,
            Self::Wave64 => 64,
        }
    }
}

/// How live reads of registers without an ABI-defined initial value are
/// handled. The pinned source silently substitutes zero; this port never does.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndefinedRegisterPolicy {
    /// Fail with `CompileError::UndefinedRegister` naming the register.
    Reject,
    /// Emit `OpUndef`, recorded in the manifest's undefined-read count.
    Undefined,
}

/// Declared access of a buffer resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferAccess {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

/// A V# held unmodified in four consecutive user SGPRs, with the specialized
/// descriptor fields the source reads at draw time (`PackedStride`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferDescription {
    /// First of the four SGPRs (4-aligned).
    pub user_sgpr: u8,
    /// V# STRIDE in bytes (14 bits).
    pub stride: u16,
    /// V# SWIZZLE_ENABLE.
    pub swizzle: bool,
    /// V# INDEX_STRIDE (0-3: 8, 16, 32, 64 lanes).
    pub index_stride: u8,
    pub access: BufferAccess,
}

/// An admitted, non-array, non-multisampled float-sampled 2D image. The
/// caller owns descriptor decoding, format, mip storage and memory authority.
/// Nonzero per-image minimum LOD uses separately bound image/sampler pairs.
/// The caller must prove equal min/mag filters, zero bias and no anisotropy
/// for those pairs, or refuse them; arbitrary sampler rewriting is not valid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageDescription {
    /// First of eight consecutive, unmodified user SGPRs (4-aligned).
    pub user_sgpr: u8,
    /// Texture minimum LOD relative to the image view base, in U4.8. The
    /// caller proves this is within the resident view's mip range.
    pub min_lod_u4_8: u16,
}

/// An admitted normalized-coordinate, non-comparison sampler. Filtering,
/// addressing and LOD limits are validated and bound by the caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SamplerDescription {
    /// First of four consecutive, unmodified user SGPRs (4-aligned).
    pub user_sgpr: u8,
}

/// Actual enabled T# counter metadata, supplied by validated descriptor decoding.
/// LOD thresholds use absolute mip U4.8; the query operand is view-relative.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LodCounterDescription {
    pub user_sgpr: u8,
    pub counter_id: u8,
    pub base_level: u8,
    pub min_lod_u4_8: u16,
}

/// The host subgroup execution the caller guarantees (for example through a
/// required subgroup size). Its size must equal the guest wave size; without
/// it, no live subgroup operation (ballot, any-lane, lane id) is accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubgroupProfile {
    pub size: u32,
}

/// Fields shared by every stage.
#[derive(Clone, Debug)]
pub struct CommonDescription {
    pub wave_size: WaveSize,
    /// First user SGPR (source `user_data_base`).
    pub user_sgpr_base: u8,
    /// Number of user SGPRs (source `user_data.size()`, at most 40).
    pub user_sgpr_count: u8,
    pub subgroup: Option<SubgroupProfile>,
    pub undefined_registers: UndefinedRegisterPolicy,
    /// First push-constant dword of the shader data (source `push_data_start_dword`).
    pub push_data_start_dword: u8,
    pub buffers: Vec<BufferDescription>,
    pub images: Vec<ImageDescription>,
    pub samplers: Vec<SamplerDescription>,
    /// Retain every actual sample before resource discovery, including dead-color
    /// samples whose enabled texture counter can have an observable GPU effect.
    pub lod_feedback: bool,
    pub lod_counters: Vec<LodCounterDescription>,
}

impl CommonDescription {
    pub fn new(wave_size: WaveSize) -> Self {
        Self {
            wave_size,
            user_sgpr_base: 0,
            user_sgpr_count: 16,
            subgroup: None,
            undefined_registers: UndefinedRegisterPolicy::Reject,
            push_data_start_dword: 0,
            buffers: Vec::new(),
            images: Vec::new(),
            samplers: Vec::new(),
            lod_feedback: false,
            lod_counters: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct VertexDescription {
    pub common: CommonDescription,
    /// Source `start_instance_sgpr`: an SGPR receiving BaseInstance.
    pub start_instance_sgpr: Option<u8>,
}

/// One pixel input, indexed by its V_INTERP attribute number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelInput {
    /// Linkage location (the vertex PARAM export index it reads).
    pub location: u8,
    pub flat: bool,
}

/// MRT output component class (source `target_output_mode` 7 is unsigned).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MrtFormat {
    Float,
    Uint,
}

#[derive(Clone, Debug)]
pub struct PixelDescription {
    pub common: CommonDescription,
    pub inputs: Vec<PixelInput>,
    pub perspective_center_vgpr: Option<u8>,
    pub perspective_centroid_vgpr: Option<u8>,
    pub perspective_sample_vgpr: Option<u8>,
    pub linear_center_vgpr: Option<u8>,
    pub linear_centroid_vgpr: Option<u8>,
    pub linear_sample_vgpr: Option<u8>,
    /// Source `ps_system_input_base`.
    pub system_input_base: u8,
    /// Source `ps_pos_x/y/z/w`.
    pub position: [bool; 4],
    pub front_face: bool,
    /// Source `ps_no_perspective`.
    pub no_perspective: bool,
    /// Source `target_shader_mask` (CB_SHADER_MASK compact exports); `None`
    /// sends MRT export i to target i.
    pub target_shader_mask: Option<u32>,
    pub mrt_formats: [MrtFormat; 8],
}

impl PixelDescription {
    pub fn new(common: CommonDescription) -> Self {
        Self {
            common,
            inputs: Vec::new(),
            perspective_center_vgpr: None,
            perspective_centroid_vgpr: None,
            perspective_sample_vgpr: None,
            linear_center_vgpr: None,
            linear_centroid_vgpr: None,
            linear_sample_vgpr: None,
            system_input_base: 0,
            position: [false; 4],
            front_face: false,
            no_perspective: false,
            target_shader_mask: None,
            mrt_formats: [MrtFormat::Float; 8],
        }
    }
}

#[derive(Clone, Debug)]
pub struct ComputeDescription {
    pub common: CommonDescription,
    /// Source `threads_num` (LocalSize).
    pub workgroup_size: [u32; 3],
    /// Source `thread_ids_num`: VGPRs v0..v2 receiving LocalInvocationId.
    pub thread_ids: u8,
    /// Source `group_id`: WorkgroupId components written to SGPRs.
    pub group_id: [bool; 3],
    /// Source `workgroup_register`.
    pub workgroup_register: u8,
    /// Source `tg_size_en`.
    pub tg_size: bool,
    /// Source `float_mode` (bit 4 clear flushes f32 compare inputs).
    pub float_mode: u8,
}

#[derive(Clone, Debug)]
pub enum StageDescription {
    Vertex(VertexDescription),
    Pixel(PixelDescription),
    Compute(ComputeDescription),
}

/// A validated, immutable stage profile. Fields are private so that a
/// profile can only be obtained through `Profile::new`.
#[derive(Clone, Debug)]
pub struct Profile {
    desc: StageDescription,
    vertex_instance_contract: crate::VertexInstanceContract,
}

pub(crate) const MAX_BUFFERS: usize = 64;
pub(crate) const MAX_IMAGES: usize = 16;
pub(crate) const PIXEL_INPUTS: usize = 32;
pub(crate) const PUSH_DWORDS: u32 = 32;

fn check_vgpr_pair(reg: Option<u8>) -> Result<(), ProfileError> {
    match reg {
        Some(r) if r == u8::MAX => Err(ProfileError::PixelBarycentricRegister),
        _ => Ok(()),
    }
}

fn validate_common(common: &CommonDescription) -> Result<(), ProfileError> {
    let end = u32::from(common.user_sgpr_base) + u32::from(common.user_sgpr_count);
    if common.user_sgpr_count > 40 || end > 106 {
        return Err(ProfileError::UserDataRange);
    }
    if u32::from(common.push_data_start_dword) > PUSH_DWORDS {
        return Err(ProfileError::PushDataStart);
    }
    if let Some(subgroup) = common.subgroup
        && subgroup.size != common.wave_size.lanes()
    {
        return Err(ProfileError::SubgroupSize);
    }
    if common.buffers.len() > MAX_BUFFERS
        || common.images.len() > MAX_IMAGES
        || common.samplers.len() > MAX_IMAGES
        || common.lod_counters.len() > MAX_IMAGES
    {
        return Err(ProfileError::ResourceCount);
    }
    for (width, index, reg) in common
        .images
        .iter()
        .enumerate()
        .map(|(index, image)| (8, index, image.user_sgpr))
        .chain(
            common
                .samplers
                .iter()
                .enumerate()
                .map(|(index, sampler)| (4, index, sampler.user_sgpr)),
        )
    {
        if !reg.is_multiple_of(4)
            || u32::from(reg) < u32::from(common.user_sgpr_base)
            || u32::from(reg) + width > end
        {
            return Err(ProfileError::ResourceRegister);
        }
        let duplicate = if width == 8 {
            common.images[..index]
                .iter()
                .any(|other| other.user_sgpr == reg)
        } else {
            common.samplers[..index]
                .iter()
                .any(|other| other.user_sgpr == reg)
        };
        if duplicate {
            return Err(ProfileError::DuplicateResource);
        }
    }
    if common.images.iter().any(|image| image.min_lod_u4_8 > 0xfff) {
        return Err(ProfileError::LodFeedbackProfile);
    }
    for (index, counter) in common.lod_counters.iter().enumerate() {
        if !common.lod_feedback
            || counter.base_level > 14
            || counter.min_lod_u4_8 > 0xfff
            || !common.images.iter().any(|image| {
                image.user_sgpr == counter.user_sgpr
                    && image.min_lod_u4_8
                        == counter
                            .min_lod_u4_8
                            .saturating_sub(u16::from(counter.base_level) * 256)
            })
            || common.lod_counters[..index]
                .iter()
                .any(|other| other.user_sgpr == counter.user_sgpr)
        {
            return Err(ProfileError::LodFeedbackProfile);
        }
    }
    for (index, buffer) in common.buffers.iter().enumerate() {
        let first = u32::from(buffer.user_sgpr);
        if buffer.user_sgpr % 4 != 0 || first < u32::from(common.user_sgpr_base) || first + 4 > end
        {
            return Err(ProfileError::ResourceRegister);
        }
        if buffer.stride >= 0x4000 {
            return Err(ProfileError::BufferStride);
        }
        if buffer.index_stride > 3 {
            return Err(ProfileError::BufferIndexStride);
        }
        if common.buffers[..index]
            .iter()
            .any(|other| other.user_sgpr == buffer.user_sgpr)
        {
            return Err(ProfileError::DuplicateResource);
        }
    }
    Ok(())
}

impl Profile {
    pub fn new(desc: StageDescription) -> Result<Self, ProfileError> {
        match &desc {
            StageDescription::Vertex(vertex) => {
                validate_common(&vertex.common)?;
                if vertex.common.lod_feedback {
                    return Err(ProfileError::LodFeedbackProfile);
                }
                if vertex.start_instance_sgpr.is_some_and(|reg| reg >= 106) {
                    return Err(ProfileError::StartInstanceRegister);
                }
            }
            StageDescription::Pixel(pixel) => {
                validate_common(&pixel.common)?;
                if pixel.inputs.len() > PIXEL_INPUTS {
                    return Err(ProfileError::PixelInputCount);
                }
                for (index, input) in pixel.inputs.iter().enumerate() {
                    if usize::from(input.location) >= PIXEL_INPUTS {
                        return Err(ProfileError::PixelInputLocation);
                    }
                    // Aliases share one interface variable and must agree on
                    // interpolation (source EXIT_IF on alias mismatch).
                    if pixel.inputs[..index]
                        .iter()
                        .any(|other| other.location == input.location && other.flat != input.flat)
                    {
                        return Err(ProfileError::PixelFlatAlias);
                    }
                }
                for reg in [
                    pixel.perspective_center_vgpr,
                    pixel.perspective_centroid_vgpr,
                    pixel.perspective_sample_vgpr,
                    pixel.linear_center_vgpr,
                    pixel.linear_centroid_vgpr,
                    pixel.linear_sample_vgpr,
                ] {
                    check_vgpr_pair(reg)?;
                }
                let system =
                    pixel.position.iter().filter(|&&p| p).count() + usize::from(pixel.front_face);
                if usize::from(pixel.system_input_base) + system > 256 {
                    return Err(ProfileError::PixelSystemRegister);
                }
            }
            StageDescription::Compute(compute) => {
                validate_common(&compute.common)?;
                if compute.common.lod_feedback {
                    return Err(ProfileError::LodFeedbackProfile);
                }
                let [x, y, z] = compute.workgroup_size;
                let total = u64::from(x)
                    .checked_mul(u64::from(y))
                    .and_then(|xy| xy.checked_mul(u64::from(z)))
                    .ok_or(ProfileError::WorkgroupSize)?;
                if x == 0 || y == 0 || z == 0 || total > 1024 {
                    return Err(ProfileError::WorkgroupSize);
                }
                if compute.thread_ids > 3 {
                    return Err(ProfileError::ComputeThreadIds);
                }
                let registers =
                    compute.group_id.iter().filter(|&&g| g).count() + usize::from(compute.tg_size);
                if usize::from(compute.workgroup_register) + registers > 106 {
                    return Err(ProfileError::ComputeWorkgroupRegisters);
                }
            }
        }
        Ok(Self {
            desc,
            vertex_instance_contract: crate::VertexInstanceContract::Dynamic,
        })
    }

    // Only the dedicated analyzer issues this private context. Generic public
    // Profile::new/compile/analyze have no caller boolean or context setter.
    pub(crate) fn native_indirect_vertex(mut self) -> Result<Self, ProfileError> {
        if !matches!(&self.desc, StageDescription::Vertex(vertex) if vertex.start_instance_sgpr.is_none())
        {
            return Err(ProfileError::NativeIndirectInstanceContract);
        }
        self.vertex_instance_contract =
            crate::VertexInstanceContract::NativeIndirectFirstInstanceZero;
        Ok(self)
    }
    pub(crate) fn vertex_instance_contract(&self) -> crate::VertexInstanceContract {
        self.vertex_instance_contract
    }
    pub fn stage(&self) -> ShaderStage {
        match self.desc {
            StageDescription::Vertex(_) => ShaderStage::Vertex,
            StageDescription::Pixel(_) => ShaderStage::Pixel,
            StageDescription::Compute(_) => ShaderStage::Compute,
        }
    }

    pub fn description(&self) -> &StageDescription {
        &self.desc
    }

    pub(crate) fn with_buffers(
        mut self,
        buffers: Vec<BufferDescription>,
    ) -> Result<Self, ProfileError> {
        let common = match &mut self.desc {
            StageDescription::Vertex(v) => &mut v.common,
            StageDescription::Pixel(p) => &mut p.common,
            StageDescription::Compute(c) => &mut c.common,
        };
        common.buffers = buffers;
        let context = self.vertex_instance_contract;
        let mut result = Self::new(self.desc)?;
        result.vertex_instance_contract = context;
        Ok(result)
    }

    pub(crate) fn with_images(
        mut self,
        images: Vec<ImageDescription>,
        samplers: Vec<SamplerDescription>,
    ) -> Result<Self, ProfileError> {
        let common = match &mut self.desc {
            StageDescription::Vertex(v) => &mut v.common,
            StageDescription::Pixel(p) => &mut p.common,
            StageDescription::Compute(c) => &mut c.common,
        };
        common.images = images;
        common.samplers = samplers;
        let context = self.vertex_instance_contract;
        let mut result = Self::new(self.desc)?;
        result.vertex_instance_contract = context;
        Ok(result)
    }

    pub(crate) fn with_lod_counters(
        mut self,
        counters: Vec<LodCounterDescription>,
    ) -> Result<Self, ProfileError> {
        let StageDescription::Pixel(pixel) = &mut self.desc else {
            if counters.is_empty() {
                return Ok(self);
            }
            return Err(ProfileError::LodFeedbackProfile);
        };
        pixel.common.lod_counters = counters;
        Self::new(self.desc)
    }

    pub fn common(&self) -> &CommonDescription {
        match &self.desc {
            StageDescription::Vertex(v) => &v.common,
            StageDescription::Pixel(p) => &p.common,
            StageDescription::Compute(c) => &c.common,
        }
    }

    pub(crate) fn wave64(&self) -> bool {
        self.common().wave_size == WaveSize::Wave64
    }

    pub(crate) fn pixel(&self) -> Option<&PixelDescription> {
        match &self.desc {
            StageDescription::Pixel(p) => Some(p),
            _ => None,
        }
    }

    pub(crate) fn compute(&self) -> Option<&ComputeDescription> {
        match &self.desc {
            StageDescription::Compute(c) => Some(c),
            _ => None,
        }
    }

    pub(crate) fn vertex(&self) -> Option<&VertexDescription> {
        match &self.desc {
            StageDescription::Vertex(v) => Some(v),
            _ => None,
        }
    }

    pub(crate) fn buffer(&self, user_sgpr: u8) -> Option<(usize, &BufferDescription)> {
        self.common()
            .buffers
            .iter()
            .enumerate()
            .find(|(_, buffer)| buffer.user_sgpr == user_sgpr)
    }
}
