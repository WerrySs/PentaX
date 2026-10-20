// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX AMD programs passed through the licensed Kyty compiler port.

use super::{Budget, OriginalGpuFixture, VulkanError};
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{
    BufferAccess, BufferDescription, CommonDescription, CompiledShader, ComputeDescription, Limits,
    PixelDescription, Profile, ShaderProgram, StageDescription, SubgroupProfile, VertexDescription,
    WaveSize, compile,
};

// Fixed small closed shader pair; avoid an extra infallible heap allocation.
#[allow(clippy::large_enum_variant)]
pub(super) enum Programs {
    Compute(CompiledShader),
    Graphics {
        vertex: CompiledShader,
        pixel: CompiledShader,
    },
}

const fn unary(op: u32, dst: u32, src: u32) -> u32 {
    0x7e00_0000 | (dst << 17) | (op << 9) | src
}
const fn binary(op: u32, dst: u32, src: u32, vgpr: u32) -> u32 {
    (op << 25) | (dst << 17) | (vgpr << 9) | src
}
const fn export(target: u32, src: [u32; 4]) -> [u32; 2] {
    [
        0xf800_080f | (target << 4),
        src[0] | src[1] << 8 | src[2] << 16 | src[3] << 24,
    ]
}

fn one(
    words: &[u32],
    description: StageDescription,
    stage: ShaderStage,
    budget: &mut Budget,
) -> Result<CompiledShader, VulkanError> {
    // Decode and compile have separate compiler budgets. Charge their entire
    // admitted arenas/work first: the compiler's temporary arenas are opaque
    // here, so their actual retained result size is not a substitute for a cap.
    let bytes = budget.limits.compiler_phase_bytes;
    let work = budget.limits.compiler_phase_work;
    budget.host(
        (bytes as u64)
            .checked_mul(2)
            .ok_or(VulkanError::Limit("compiler bytes"))?,
    )?;
    budget.work(
        work.checked_mul(2)
            .ok_or(VulkanError::Limit("compiler work"))?,
    )?;
    let mut limits = Limits::default();
    limits.max_bytes = bytes;
    limits.max_work = work;
    limits.max_spirv_words = budget
        .limits
        .max_shader_words
        .saturating_sub(budget.usage.shader_words);
    let program = ShaderProgram::decode(words, stage, &limits).map_err(|_| VulkanError::Compile)?;
    let profile = Profile::new(description).map_err(|_| VulkanError::Compile)?;
    let result = compile(&program, &profile, &limits).map_err(|_| VulkanError::Compile)?;
    budget.shader(result.spirv().len())?;
    check_contract(&result, stage)?;
    Ok(result)
}

/// Additional backend admission, not a general SPIR-V validator. Only the
/// actual compiler's opaque output reaches this function. Refuse newly emitted
/// capabilities until the matching device features and ABI are implemented.
pub(super) fn check_contract(
    shader: &CompiledShader,
    stage: ShaderStage,
) -> Result<(), VulkanError> {
    check_contract_inner(shader, stage, false)
}

/// The graphics preparer supplies actual user SGPR values in its stage payload.
/// Descriptor resources are still deliberately unsupported by this draw slice.
pub(super) fn check_graphics_contract(
    shader: &CompiledShader,
    stage: ShaderStage,
) -> Result<(), VulkanError> {
    if !matches!(stage, ShaderStage::Vertex | ShaderStage::Pixel) {
        return Err(VulkanError::InvalidCompilerContract("graphics stage"));
    }
    check_contract_inner(shader, stage, true)?;
    let manifest = shader.manifest();
    if !manifest.buffers().is_empty()
        || manifest.buffer_binding().is_some()
        || manifest.required_features().subgroup_basic()
        || manifest.required_features().subgroup_ballot()
        || manifest.required_subgroup_size().is_some()
        || manifest
            .push_data()
            .is_some_and(|push| push.memory_offset_count != 0)
    {
        return Err(VulkanError::InvalidCompilerContract("graphics resources"));
    }
    Ok(())
}

fn check_contract_inner(
    shader: &CompiledShader,
    stage: ShaderStage,
    graphics_push: bool,
) -> Result<(), VulkanError> {
    let manifest = shader.manifest();
    let required = manifest.required_features();
    if required.spirv_version() > 0x0001_0300
        || !required.shader()
        || !required.float_controls_extension()
        || !required.signed_zero_inf_nan_preserve_capability()
        || !required.float32_signed_zero_inf_nan_preserve()
        || required.shader_int64()
        || required.draw_parameters()
    {
        return Err(VulkanError::InvalidCompilerContract(
            "required device features",
        ));
    }
    if manifest.stage() != stage
        || manifest.entry_point() != "main"
        || manifest.descriptor_set() != 0
        || manifest.undefined_register_reads() != 0
        || manifest.shader_data_binding().is_some()
    {
        return Err(VulkanError::InvalidCompilerContract(
            "stage/resources/subgroup",
        ));
    }
    if (required.subgroup_basic() || required.subgroup_ballot())
        && manifest.required_subgroup_size() != Some(32)
        || manifest
            .required_subgroup_size()
            .is_some_and(|size| size != 32)
    {
        return Err(VulkanError::InvalidCompilerContract("subgroup size"));
    }
    if let Some(push) = manifest.push_data()
        && ((!push.user_sgprs.is_empty()
            && ((!graphics_push && stage != ShaderStage::Compute)
                || push.user_sgprs.iter().any(|r| *r >= 16)))
            || push
                .start_dword
                .checked_add(push.user_sgprs.len() as u32)
                .is_none_or(|end| end > 32)
            || push.memory_offset_count > 1
            || (push.memory_offset_count != 0
                && push
                    .start_dword
                    .checked_add(push.memory_offset_dword)
                    .and_then(|v| v.checked_add(1))
                    .is_none_or(|v| v > 32)))
    {
        return Err(VulkanError::InvalidCompilerContract("push data"));
    }
    let words = shader.spirv();
    if words.len() < 5 || words[0] != 0x0723_0203 || words[1] > 0x0001_0300 {
        return Err(VulkanError::InvalidCompilerContract("SPIR-V header"));
    }
    let mut at = 5;
    while at < words.len() {
        let length = (words[at] >> 16) as usize;
        let end = at
            .checked_add(length)
            .filter(|&end| length != 0 && end <= words.len())
            .ok_or(VulkanError::InvalidCompilerContract("instruction bounds"))?;
        if words[at] as u16 == 17 && (length != 2 || ![1, 61, 64, 4466].contains(&words[at + 1])) {
            return Err(VulkanError::InvalidCompilerContract("SPIR-V capability"));
        }
        at = end;
    }
    Ok(())
}

pub(super) fn compute_push(shader: &CompiledShader) -> Result<[u8; 128], VulkanError> {
    let mut bytes = [0_u8; 128];
    let push = shader
        .manifest()
        .push_data()
        .ok_or(VulkanError::InvalidCompilerContract(
            "missing compute push data",
        ))?;
    // Original raw V# descriptor: synthetic guest base0x10000, stride0,
    // bytecount32, swizzle/indexStride0. The compiler specializes its resource
    // into the retained Vulkan binding; this integer is never a host pointer.
    let descriptor = [0x10000_u32, 0, 32, 0];
    for (index, &register) in push.user_sgprs.iter().enumerate() {
        let word = descriptor
            .get(register as usize)
            .ok_or(VulkanError::InvalidCompilerContract("original user SGPR"))?;
        let offset = (push.start_dword as usize)
            .checked_add(index)
            .and_then(|v| v.checked_mul(4))
            .ok_or(VulkanError::InvalidCompilerContract("push offset"))?;
        let end = offset
            .checked_add(4)
            .filter(|&end| end <= bytes.len())
            .ok_or(VulkanError::InvalidCompilerContract("push extent"))?;
        bytes[offset..end].copy_from_slice(&word.to_le_bytes());
    }
    // The manifest's one packed buffer-byte-offset is zero. Its independently
    // checked dword remains explicitly zero in this fully initialized block.
    Ok(bytes)
}

pub(super) fn prepare(
    fixture: OriginalGpuFixture,
    budget: &mut Budget,
) -> Result<Programs, VulkanError> {
    let common = || {
        let mut description = CommonDescription::new(WaveSize::Wave32);
        description.subgroup = Some(SubgroupProfile { size: 32 });
        description
    };
    match fixture {
        OriginalGpuFixture::ComputeBuffer => {
            let mut description = common();
            budget.host(std::mem::size_of::<BufferDescription>() as u64)?;
            description
                .buffers
                .try_reserve_exact(1)
                .map_err(|_| VulkanError::Allocation)?;
            description.buffers.push(BufferDescription {
                user_sgpr: 0,
                stride: 0,
                swizzle: false,
                index_stride: 0,
                access: BufferAccess::WriteOnly,
            });
            let words = [
                binary(0x25, 1, 137, 0),                // V_ADD_U32 v1, 9, v0 (local lane).
                binary(0x1a, 2, 130, 0),                // V_LSHLREV_B32 v2, 2, v0 (byte offset).
                0xe000_0000 | (0x1c << 18) | (1 << 12), // BUFFER_STORE_DWORD.
                (128 << 24) | (1 << 8) | 2,
                0xbf81_0000,
            ];
            let shader = one(
                &words,
                StageDescription::Compute(ComputeDescription {
                    common: description,
                    workgroup_size: [8, 1, 1],
                    thread_ids: 1,
                    group_id: [false; 3],
                    workgroup_register: 16,
                    tg_size: false,
                    float_mode: 0xc0,
                }),
                ShaderStage::Compute,
                budget,
            )?;
            let m = shader.manifest();
            if m.workgroup_size() != Some([8, 1, 1])
                || m.buffers().len() != 1
                || m.buffer_binding().is_none()
                || !m.buffers()[0].written
                || m.buffers()[0].read
                || m.buffers()[0].coherent
                || m.buffers()[0].max_byte_extent > 32
            {
                return Err(VulkanError::InvalidCompilerContract(
                    "original compute buffer",
                ));
            }
            Ok(Programs::Compute(shader))
        }
        OriginalGpuFixture::TriangleRgba => {
            // Full-screen triangle: x=((VertexIndex<<1)&2)*2-1,
            // y=(VertexIndex&2)*2-1. Pinned vertex ABI places VertexIndex in v5.
            let position = export(0x0c, [0, 1, 2, 3]);
            let vertex = [
                binary(0x1a, 0, 129, 5),
                binary(0x1b, 0, 130, 0), // V_AND_B32 (0x13 is V_MIN_U32).
                unary(6, 0, 256),
                binary(8, 0, 244, 0),
                binary(3, 0, 243, 0),
                binary(0x1b, 1, 130, 5),
                unary(6, 1, 257),
                binary(8, 1, 244, 1),
                binary(3, 1, 243, 1),
                unary(1, 2, 128),
                unary(1, 3, 242),
                position[0],
                position[1],
                0xbf81_0000,
            ];
            let vertex = one(
                &vertex,
                StageDescription::Vertex(VertexDescription {
                    common: common(),
                    start_instance_sgpr: None,
                }),
                ShaderStage::Vertex,
                budget,
            )?;
            let color = export(0, [0, 1, 1, 0]);
            let pixel = [
                unary(1, 0, 242),
                unary(1, 1, 128),
                color[0],
                color[1],
                0xbf81_0000,
            ];
            let pixel = one(
                &pixel,
                StageDescription::Pixel(PixelDescription::new(common())),
                ShaderStage::Pixel,
                budget,
            )?;
            if vertex.manifest().buffer_binding().is_some()
                || pixel.manifest().buffer_binding().is_some()
                || !vertex.manifest().buffers().is_empty()
                || !pixel.manifest().buffers().is_empty()
            {
                return Err(VulkanError::InvalidCompilerContract(
                    "original graphics descriptors",
                ));
            }
            Ok(Programs::Graphics { vertex, pixel })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate_optional(name: &str, shader: &CompiledShader) {
        let Some(validator) = std::env::var_os("PENTAX_VULKAN_SPIRV_VAL") else {
            return;
        };
        let directory = std::env::var_os("PENTAX_VULKAN_SHADER_OUTPUT")
            .expect("explicit original-output directory");
        std::fs::create_dir_all(&directory).unwrap();
        let path = std::path::PathBuf::from(directory).join(format!("{name}.spv"));
        let bytes: Vec<u8> = shader
            .spirv()
            .iter()
            .flat_map(|word| word.to_le_bytes())
            .collect();
        std::fs::write(&path, bytes).unwrap();
        let output = std::process::Command::new(validator)
            .args(["--target-env", "vulkan1.2"])
            .arg(&path)
            .output()
            .expect("explicit official SPIR-V validator");
        assert!(
            output.status.success(),
            "official validator rejected original {name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn original_programs_compile_with_retained_binding_and_stage_metadata_without_ffi() {
        for fixture in [
            OriginalGpuFixture::ComputeBuffer,
            OriginalGpuFixture::TriangleRgba,
        ] {
            let mut budget = Budget::new(super::super::GpuLimits::default());
            let prepared = prepare(fixture, &mut budget).unwrap();
            assert!(budget.usage.shader_words > 0);
            match prepared {
                Programs::Compute(shader) => {
                    validate_optional("compute-original", &shader);
                    assert_eq!(shader.manifest().buffers().len(), 1);
                    // The original all-active store folds its EXEC guard; no
                    // host subgroup requirement may be invented for that case.
                    assert_eq!(shader.manifest().required_subgroup_size(), None);
                    assert!(!shader.manifest().required_features().subgroup_ballot());
                    let push = compute_push(&shader).unwrap();
                    let layout = shader.manifest().push_data().unwrap();
                    let start = layout.start_dword as usize * 4;
                    assert_eq!(
                        u32::from_le_bytes(push[start..start + 4].try_into().unwrap()),
                        0x10000
                    );
                    assert_eq!(
                        u32::from_le_bytes(push[start + 8..start + 12].try_into().unwrap()),
                        32
                    );
                    let offset = (layout.start_dword + layout.memory_offset_dword) as usize * 4;
                    assert_eq!(&push[offset..offset + 4], &[0; 4]);
                }
                Programs::Graphics { vertex, pixel } => {
                    validate_optional("vertex-original", &vertex);
                    validate_optional("pixel-original", &pixel);
                    assert_eq!(vertex.manifest().stage(), ShaderStage::Vertex);
                    assert_eq!(pixel.manifest().stage(), ShaderStage::Pixel);
                }
            }
        }
    }

    #[test]
    fn compiler_phase_and_word_limits_refuse_before_any_gpu_path() {
        let mut budget = Budget::new(super::super::GpuLimits {
            max_host_bytes: 0,
            ..super::super::GpuLimits::default()
        });
        assert!(matches!(
            prepare(OriginalGpuFixture::ComputeBuffer, &mut budget),
            Err(VulkanError::Limit("host bytes"))
        ));
        assert_eq!(budget.usage.objects, 0);
        let mut budget = Budget::new(super::super::GpuLimits {
            max_shader_words: 0,
            ..super::super::GpuLimits::default()
        });
        assert!(matches!(
            prepare(OriginalGpuFixture::TriangleRgba, &mut budget),
            Err(VulkanError::Compile)
        ));
        assert_eq!(budget.usage.objects, 0);
    }
}
