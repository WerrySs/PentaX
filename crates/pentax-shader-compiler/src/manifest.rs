// SPDX-License-Identifier: GPL-2.0-only
// The binding/interface manifest of a compiled module: a read-only view of
// the KytyPS5-Performance 209dffe `BindingLayout`, `ShaderInfo` buffers and
// stage inputs/outputs that the emitted SPIR-V relies on. PentaX addition.

use pentax_shader::ShaderStage;

/// Features actually declared by the emitted SPIR-V module. A host must check
/// and enable their API equivalents before creating or executing a pipeline.
/// These describe module requirements, never an inference about a device.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RequiredFeatures {
    pub(crate) spirv_version: u32,
    pub(crate) shader: bool,
    pub(crate) float_controls_extension: bool,
    pub(crate) signed_zero_inf_nan_preserve_capability: bool,
    pub(crate) float32_signed_zero_inf_nan_preserve: bool,
    pub(crate) shader_int64: bool,
    pub(crate) draw_parameters: bool,
    pub(crate) subgroup_basic: bool,
    pub(crate) subgroup_ballot: bool,
    pub(crate) image_query: bool,
}

impl RequiredFeatures {
    pub fn image_query(&self) -> bool {
        self.image_query
    }

    /// SPIR-V header version word, currently 1.3 (`0x0001_0300`).
    pub fn spirv_version(&self) -> u32 {
        self.spirv_version
    }
    pub fn shader(&self) -> bool {
        self.shader
    }
    /// `SPV_KHR_float_controls` is declared. The host needs the corresponding
    /// core/extension support as well as the individual preservation property.
    pub fn float_controls_extension(&self) -> bool {
        self.float_controls_extension
    }
    pub fn signed_zero_inf_nan_preserve_capability(&self) -> bool {
        self.signed_zero_inf_nan_preserve_capability
    }
    /// The entry point declares `SignedZeroInfNanPreserve 32`. Vulkan hosts
    /// must support `shaderSignedZeroInfNanPreserveFloat32`.
    pub fn float32_signed_zero_inf_nan_preserve(&self) -> bool {
        self.float32_signed_zero_inf_nan_preserve
    }
    pub fn shader_int64(&self) -> bool {
        self.shader_int64
    }
    pub fn draw_parameters(&self) -> bool {
        self.draw_parameters
    }
    pub fn subgroup_basic(&self) -> bool {
        self.subgroup_basic
    }
    pub fn subgroup_ballot(&self) -> bool {
        self.subgroup_ballot
    }
}

/// Push-constant shader data (source `BindingLayout` push-data fields).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PushData {
    /// First 32-bit dword of the shader data in the 128-byte push block.
    pub start_dword: u32,
    /// User SGPR numbers in dword order (dword `start + i` holds SGPR `user_sgprs[i]`).
    pub user_sgprs: Vec<u8>,
    /// Shader-data dword of the first buffer byte-offset word.
    pub memory_offset_dword: u32,
    /// Buffer byte offsets, packed four per dword (byte `i % 4` of word `i / 4`).
    pub memory_offset_count: u32,
}

/// One element of the storage-buffer descriptor array, in source first-use order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferElement {
    pub user_sgpr: u8,
    /// Index of the declaring `BufferDescription` in the profile.
    pub profile_index: usize,
    pub read: bool,
    pub written: bool,
    pub scalar: bool,
    pub coherent: bool,
    /// Largest immediate byte extent of any access (source `max_byte_extent`).
    pub max_byte_extent: u32,
    pub first_use_pc: u32,
}

/// One separate sampled-image descriptor, in live first-use order. It confers
/// no guest memory access. The binding is exact, in descriptor set zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageElement {
    pub user_sgpr: u8,
    pub profile_index: usize,
    pub binding: u32,
    pub first_use_pc: u32,
}

/// One separate normalized, non-comparison sampler descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SamplerElement {
    pub user_sgpr: u8,
    pub profile_index: usize,
    pub binding: u32,
    pub first_use_pc: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputInterface {
    VertexIndex,
    InstanceIndex,
    BaseInstance,
    FragCoord,
    FrontFacing,
    WorkgroupId,
    LocalInvocationId,
    LocalInvocationIndex,
    SubgroupLocalInvocationId,
    /// A pixel vec4 parameter at `location`.
    Parameter {
        input: u32,
        location: u32,
        flat: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputInterface {
    Position,
    Parameter {
        index: u32,
        location: u32,
    },
    Color {
        export: u32,
        location: u32,
        uint: bool,
    },
}

/// Explicit compile-time dependency on the native indexed-indirect draw ABI.
/// This is metadata, not memory or GPU execution authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum VertexInstanceContract {
    #[default]
    Dynamic,
    NativeIndirectFirstInstanceZero,
}

/// Immutable description of the emitted module's external contract.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub(crate) required_features: RequiredFeatures,
    pub(crate) vertex_instance_contract: VertexInstanceContract,
    pub(crate) stage: ShaderStage,
    pub(crate) push_data: Option<PushData>,
    pub(crate) shader_data_binding: Option<u32>,
    pub(crate) buffer_binding: Option<u32>,
    pub(crate) buffers: Vec<BufferElement>,
    pub(crate) images: Vec<ImageElement>,
    pub(crate) samplers: Vec<SamplerElement>,
    pub(crate) inputs: Vec<InputInterface>,
    pub(crate) outputs: Vec<OutputInterface>,
    pub(crate) workgroup_size: Option<[u32; 3]>,
    pub(crate) required_subgroup_size: Option<u32>,
    pub(crate) undefined_register_reads: u32,
}

impl Manifest {
    pub fn images(&self) -> &[ImageElement] {
        &self.images
    }
    pub fn samplers(&self) -> &[SamplerElement] {
        &self.samplers
    }

    /// Actual recursive vector capacities retained by this manifest. Fixed
    /// metadata is inline; there are no other owned allocations in its fields.
    pub fn retained_host_bytes(&self) -> usize {
        self.images
            .capacity()
            .saturating_mul(std::mem::size_of::<ImageElement>())
            .saturating_add(
                self.samplers
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SamplerElement>()),
            )
            .saturating_add(
                self.buffers
                    .capacity()
                    .saturating_mul(std::mem::size_of::<BufferElement>()),
            )
            .saturating_add(
                self.inputs
                    .capacity()
                    .saturating_mul(std::mem::size_of::<InputInterface>()),
            )
            .saturating_add(
                self.outputs
                    .capacity()
                    .saturating_mul(std::mem::size_of::<OutputInterface>()),
            )
            .saturating_add(
                self.push_data
                    .as_ref()
                    .map_or(0, |push| push.user_sgprs.capacity()),
            )
    }

    pub fn vertex_instance_contract(&self) -> VertexInstanceContract {
        self.vertex_instance_contract
    }
    pub fn required_features(&self) -> &RequiredFeatures {
        &self.required_features
    }
    pub fn stage(&self) -> ShaderStage {
        self.stage
    }
    /// Always "main".
    pub fn entry_point(&self) -> &'static str {
        "main"
    }
    /// Descriptor set of every binding (source: set 0).
    pub fn descriptor_set(&self) -> u32 {
        0
    }
    pub fn push_data(&self) -> Option<&PushData> {
        self.push_data.as_ref()
    }
    /// Storage-buffer binding of the shader data when it does not fit in push data.
    pub fn shader_data_binding(&self) -> Option<u32> {
        self.shader_data_binding
    }
    /// Binding of the `buffers[N]` storage-buffer array (source NativeBinding).
    pub fn buffer_binding(&self) -> Option<u32> {
        self.buffer_binding
    }
    pub fn buffers(&self) -> &[BufferElement] {
        &self.buffers
    }
    pub fn inputs(&self) -> &[InputInterface] {
        &self.inputs
    }
    pub fn outputs(&self) -> &[OutputInterface] {
        &self.outputs
    }
    pub fn workgroup_size(&self) -> Option<[u32; 3]> {
        self.workgroup_size
    }
    /// The host subgroup size the module's subgroup operations require.
    pub fn required_subgroup_size(&self) -> Option<u32> {
        self.required_subgroup_size
    }
    /// Live `OpUndef` register reads (only under `UndefinedRegisterPolicy::Undefined`).
    pub fn undefined_register_reads(&self) -> u32 {
        self.undefined_register_reads
    }
}
