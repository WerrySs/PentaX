// SPDX-License-Identifier: GPL-2.0-only
//! Original compiler products exercise closed backend admission without Vulkan.
use super::*;
use pentax_shader_compiler::{
    BufferAccess, BufferDescription, CommonDescription, Limits as CompilerLimits, PixelDescription,
    Profile, ShaderProgram, StageDescription, VertexDescription, WaveSize, compile,
};

fn shader(stage: ShaderStage, start: u8, all_users: bool) -> CompiledShader {
    let mut common = CommonDescription::new(WaveSize::Wave32);
    let descriptor = if stage == ShaderStage::Vertex { 4 } else { 0 };
    common.user_sgpr_count = if all_users { 16 } else { descriptor + 4 };
    common.push_data_start_dword = start;
    common.buffers.push(BufferDescription {
        user_sgpr: descriptor,
        stride: 0,
        swizzle: false,
        index_stride: 0,
        access: BufferAccess::ReadOnly,
    });
    let description = match stage {
        ShaderStage::Vertex => StageDescription::Vertex(VertexDescription {
            common,
            start_instance_sgpr: None,
        }),
        ShaderStage::Pixel => StageDescription::Pixel(PixelDescription::new(common)),
        _ => unreachable!("original graphics stage"),
    };
    // V_MOV_B32 v0,0; BUFFER_LOAD_DWORD v4,offset(v0),descriptor,0.
    // VS s2/s3 are entry builtins, so its real untouched V# uses s[4:7].
    // The immediate byte32 is intentionally outside a possible four-byte
    // snapshot: compiler bounds guards, never a larger host allocation, own it.
    let mut words = vec![
        0x7e00_0280,
        0xe000_0000 | (0x0c << 18) | (1 << 12) | 32,
        (128 << 24) | (u32::from(descriptor / 4) << 16) | (4 << 8),
    ];
    if all_users {
        for register in 0..16 {
            words.push((3 << 25) | (4 << 17) | (4 << 9) | register);
        }
    }
    let target = if stage == ShaderStage::Vertex { 12 } else { 0 };
    words.extend([0xf800_080f | (target << 4), 0x0404_0404, 0xbf81_0000]);
    let limits = CompilerLimits::default();
    let program = ShaderProgram::decode(&words, stage, &limits).unwrap();
    compile(&program, &Profile::new(description).unwrap(), &limits).unwrap()
}

#[test]
fn actual_readonly_manifests_require_exact_stage_ranges_and_owned_dword_snapshots() {
    for (stage, start, binding) in [(ShaderStage::Vertex, 0, 0), (ShaderStage::Pixel, 16, 57)] {
        let compiled = shader(stage, start, false);
        check_graphics_contract(&compiled, stage).unwrap();
        assert_eq!(compiled.manifest().buffer_binding(), Some(binding));
        let descriptor = compiled.manifest().buffers()[0];
        assert!(descriptor.read && !descriptor.written && !descriptor.coherent);
        assert!(descriptor.max_byte_extent > 4);
        // A guarded OOB program accepts only the actual four bytes, never an
        // invented buffer enlarged to the largest encoded immediate offset.
        assert_eq!(resource_input(&compiled, Some(&[1, 2, 3, 4])), Ok(4));
        for data in [None, Some([].as_slice()), Some([0; 3].as_slice())] {
            assert!(matches!(
                resource_input(&compiled, data),
                Err(VulkanError::InvalidCompilerContract(_))
            ));
        }
    }
    let wrong_pixel_window = shader(ShaderStage::Pixel, 0, false);
    assert!(matches!(
        check_graphics_contract(&wrong_pixel_window, ShaderStage::Pixel),
        Err(VulkanError::InvalidCompilerContract(_))
    ));
    let full_pixel = shader(ShaderStage::Pixel, 16, true);
    // Sixteen live PS SGPRs plus a packed-offset word cannot fit the stage's
    // remaining sixteen push dwords. Do not admit the compiler's UBO fallback.
    assert!(full_pixel.manifest().push_data().is_none());
    assert!(full_pixel.manifest().shader_data_binding().is_some());
    assert!(matches!(
        check_graphics_contract(&full_pixel, ShaderStage::Pixel),
        Err(VulkanError::InvalidCompilerContract(_))
    ));
}

#[test]
fn independent_stage_payloads_include_packed_offsets_and_refuse_nonzero_prefixes() {
    let vertex = shader(ShaderStage::Vertex, 0, false);
    let pixel = shader(ShaderStage::Pixel, 16, false);
    let mut output = [0; 128];
    let mut initialized = [false; 32];
    let mut inputs = [[0; 128]; 2];
    for (index, compiled) in [&vertex, &pixel].into_iter().enumerate() {
        let push = compiled.manifest().push_data().unwrap();
        for word in 0..push.user_sgprs.len() {
            let at = (push.start_dword as usize + word) * 4;
            let value = ((index + 1) as u32 * 100 + word as u32).to_le_bytes();
            inputs[index][at..at + 4].copy_from_slice(&value);
        }
        merge_stage_push(compiled, &inputs[index], &mut output, &mut initialized).unwrap();
        let offset = (push.start_dword + push.memory_offset_dword) as usize;
        assert!(initialized[offset]);
        assert_eq!(&output[offset * 4..offset * 4 + 4], &[0; 4]);
    }
    assert_eq!(&output[..4], &100_u32.to_le_bytes());
    assert_eq!(&output[64..68], &200_u32.to_le_bytes());
    let layout = pixel.manifest().push_data().unwrap();
    let offset = (layout.start_dword + layout.memory_offset_dword) as usize * 4;
    for byte in 0..4 {
        let mut changed = inputs[1];
        changed[offset + byte] = 1;
        assert_eq!(
            merge_stage_push(&pixel, &changed, &mut [0; 128], &mut [false; 32]),
            Err(VulkanError::InvalidCompilerContract(
                "graphics buffer upload offset"
            ))
        );
    }
    let mut changed = inputs[0];
    changed[0] ^= 1;
    assert_eq!(
        merge_stage_push(&vertex, &changed, &mut output, &mut initialized),
        Err(VulkanError::InvalidCompilerContract(
            "overlapping graphics push values"
        ))
    );
}

#[test]
fn stage_uploads_and_completed_target_share_exact_host_budget_before_device_effects() {
    let mut budget = Budget::new(GpuLimits {
        max_host_bytes: 40,
        ..Default::default()
    });
    upload_bytes(8, 4, [8, 12], &mut budget).unwrap();
    assert_eq!(budget.usage.host_bytes, 32);
    assert_eq!(budget.buffer(8).unwrap().len(), 8);
    assert_eq!(budget.usage.host_bytes, 40);
    assert_eq!(budget.usage.objects, 0);
    let mut short = Budget::new(GpuLimits {
        max_host_bytes: 39,
        ..Default::default()
    });
    assert_eq!(
        upload_bytes(8, 4, [8, 12], &mut short),
        Err(VulkanError::Limit("graphics host bytes"))
    );
    assert_eq!(short.usage.host_bytes, 32);
    assert_eq!(short.usage.objects, 0);
    assert!(upload_bytes(1, 0, [usize::MAX, 1], &mut Budget::new(Default::default())).is_err());
}

#[test]
fn sparse_binding_numbers_obey_real_per_stage_set_range_and_fragment_limits() {
    let host = vk::PhysicalDeviceLimits {
        max_bound_descriptor_sets: 1,
        max_descriptor_set_storage_buffers: 2,
        max_per_stage_descriptor_storage_buffers: 1,
        max_per_stage_resources: 2,
        max_fragment_combined_output_resources: 2,
        max_storage_buffer_range: 16,
        ..Default::default()
    };
    storage_limits([4, 16], &host).unwrap();
    for changed in [
        vk::PhysicalDeviceLimits {
            max_bound_descriptor_sets: 0,
            ..host
        },
        vk::PhysicalDeviceLimits {
            max_descriptor_set_storage_buffers: 1,
            ..host
        },
        vk::PhysicalDeviceLimits {
            max_per_stage_descriptor_storage_buffers: 0,
            ..host
        },
        vk::PhysicalDeviceLimits {
            max_per_stage_resources: 1,
            ..host
        },
        vk::PhysicalDeviceLimits {
            max_fragment_combined_output_resources: 1,
            ..host
        },
        vk::PhysicalDeviceLimits {
            max_storage_buffer_range: 15,
            ..host
        },
    ] {
        assert_eq!(
            storage_limits([4, 16], &changed),
            Err(VulkanError::Unsupported(
                "graphics storage descriptor limits"
            ))
        );
    }
    let one_stage = vk::PhysicalDeviceLimits {
        max_descriptor_set_storage_buffers: 1,
        max_per_stage_resources: 1,
        max_fragment_combined_output_resources: 1,
        ..host
    };
    storage_limits([4, 0], &one_stage).unwrap();
    assert!(storage_limits([0, 4], &one_stage).is_err());
    // A draw without descriptors preserves the original empty-layout admission.
    storage_limits([0, 0], &vk::PhysicalDeviceLimits::default()).unwrap();
}
