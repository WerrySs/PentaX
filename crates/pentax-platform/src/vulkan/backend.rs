// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: fixed original factories and readback; see NOTICE.

use super::{
    Budget, GpuLimits, GpuReport, OriginalGpuFixture, VulkanError,
    commands::{Batch, buffer_host_barrier},
    device::Device,
    pipeline::Pipeline,
    programs::{self, Programs},
    resources::{Buffer, Image},
};
use ash::vk;

pub(super) fn run(
    fixture: OriginalGpuFixture,
    limits: GpuLimits,
) -> Result<GpuReport, VulkanError> {
    let mut budget = Budget::new(limits);
    // Compiler/layout/resource limits fail before loading Vulkan or making FFI
    // calls. No shader is read from disk or supplied by a public caller.
    let programs = programs::prepare(fixture, &mut budget)?;
    let output_bytes = match fixture {
        OriginalGpuFixture::ComputeBuffer => 32,
        OriginalGpuFixture::TriangleRgba => 64,
    };
    let zeros = budget.buffer(output_bytes)?;
    let mut subgroup_stages = vk::ShaderStageFlags::empty();
    let mut ballot_stages = vk::ShaderStageFlags::empty();
    let mut require_subgroup = |shader: &pentax_shader_compiler::CompiledShader, stage| {
        let required = shader.manifest().required_features();
        if required.subgroup_basic() || required.subgroup_ballot() {
            subgroup_stages |= stage;
        }
        if required.subgroup_ballot() {
            ballot_stages |= stage;
        }
    };
    match &programs {
        Programs::Compute(shader) => require_subgroup(shader, vk::ShaderStageFlags::COMPUTE),
        Programs::Graphics { vertex, pixel } => {
            require_subgroup(vertex, vk::ShaderStageFlags::VERTEX);
            require_subgroup(pixel, vk::ShaderStageFlags::FRAGMENT);
        }
    }
    let device = Device::open(subgroup_stages, ballot_stages, &mut budget)?;
    let bytes = match programs {
        Programs::Compute(shader) => {
            let mut buffer = Buffer::new(
                &device,
                32,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                &mut budget,
            )?;
            buffer.write(&zeros, &mut budget)?;
            let pipeline = Pipeline::compute(&device, &shader, &buffer, &mut budget)?;
            // All resources precede Batch, so uncertain/pending submissions are
            // drained before any resource/pipeline owner can be dropped.
            let batch = Batch::begin(&device, &mut budget)?;
            budget.command(1)?;
            let barriers = [vk::BufferMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(buffer.raw)
                .offset(0)
                .size(32)];
            // SAFETY: Owned uploaded buffer, exact bounded range, recording
            // primary buffer on compatible family, no pending host access.
            unsafe {
                device.raw.cmd_pipeline_barrier(
                    batch.raw,
                    vk::PipelineStageFlags::HOST,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &barriers,
                    &[],
                )
            };
            budget.command(4)?;
            let push = programs::compute_push(&shader)?;
            // SAFETY: Manifest-bound live compute pipeline/set and128-byte push
            // range. One8-lane group writes disjoint4-byte slots in32-byte SSBO.
            // SYNC below and fence completion precede any host read/destruction.
            unsafe {
                device.raw.cmd_bind_pipeline(
                    batch.raw,
                    vk::PipelineBindPoint::COMPUTE,
                    pipeline.raw,
                );
                device.raw.cmd_bind_descriptor_sets(
                    batch.raw,
                    vk::PipelineBindPoint::COMPUTE,
                    pipeline.layout,
                    0,
                    &[pipeline.set],
                    &[],
                );
                device.raw.cmd_push_constants(
                    batch.raw,
                    pipeline.layout,
                    vk::ShaderStageFlags::COMPUTE,
                    0,
                    &push,
                );
                device.raw.cmd_dispatch(batch.raw, 1, 1, 1);
            }
            buffer_host_barrier(
                &device,
                batch.raw,
                buffer.raw,
                32,
                vk::AccessFlags::SHADER_WRITE,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                &mut budget,
            )?;
            batch.submit_wait(&mut budget)?;
            let bytes = buffer.read_completed(&mut budget)?;
            budget.work(8)?;
            if bytes.chunks_exact(4).enumerate().any(|(lane, value)| {
                u32::from_le_bytes(value.try_into().expect("exact4-byte chunk")) != lane as u32 + 9
            }) {
                return Err(VulkanError::UnexpectedReadback);
            }
            bytes
        }
        Programs::Graphics { vertex, pixel } => {
            let image = Image::rgba(&device, 4, 4, &mut budget)?;
            let mut readback =
                Buffer::new(&device, 64, vk::BufferUsageFlags::TRANSFER_DST, &mut budget)?;
            readback.write(&zeros, &mut budget)?;
            let pipeline = Pipeline::graphics(&device, &vertex, &pixel, &image, &mut budget)?;
            let batch = Batch::begin(&device, &mut budget)?;
            // The clear is BLUE, different from the shader's RED. A missing or
            // discarded draw cannot pass by reading the clear color instead.
            let clear = [vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 1.0, 1.0],
                },
            }];
            let area = vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: image.width,
                    height: image.height,
                },
            };
            let begin = vk::RenderPassBeginInfo::default()
                .render_pass(pipeline.render_pass)
                .framebuffer(pipeline.framebuffer)
                .render_area(area)
                .clear_values(&clear);
            budget.command(4)?;
            // SAFETY: Exact framebuffer/render-pass pairing and4x4 render area;
            // checked vertex/pixel shaders produce a full-screen triangle with
            // no vertex buffers/descriptors. All referenced owners remain live.
            unsafe {
                device
                    .raw
                    .cmd_begin_render_pass(batch.raw, &begin, vk::SubpassContents::INLINE);
                device.raw.cmd_bind_pipeline(
                    batch.raw,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipeline.raw,
                );
                device.raw.cmd_draw(batch.raw, 3, 1, 0, 0);
                device.raw.cmd_end_render_pass(batch.raw);
            }
            budget.command(1)?;
            let barriers = [vk::BufferMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(readback.raw)
                .offset(0)
                .size(64)];
            // SAFETY: Exact owned transfer destination, no overlapping host use.
            unsafe {
                device.raw.cmd_pipeline_barrier(
                    batch.raw,
                    vk::PipelineStageFlags::HOST,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &barriers,
                    &[],
                )
            };
            let regions = [vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
                .image_extent(vk::Extent3D {
                    width: image.width,
                    height: image.height,
                    depth: 1,
                })];
            budget.command(1)?;
            // SAFETY: Render-pass final layout+explicit external dependency make
            // color writes visible to transfer reads. Tight RGBA8 extent is64
            // bytes in the matching retained readback allocation, offset0 aligned.
            unsafe {
                device.raw.cmd_copy_image_to_buffer(
                    batch.raw,
                    image.raw,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    readback.raw,
                    &regions,
                )
            };
            buffer_host_barrier(
                &device,
                batch.raw,
                readback.raw,
                64,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::PipelineStageFlags::TRANSFER,
                &mut budget,
            )?;
            batch.submit_wait(&mut budget)?;
            let bytes = readback.read_completed(&mut budget)?;
            budget.work(16)?;
            if bytes.chunks_exact(4).any(|pixel| pixel != [255, 0, 0, 255]) {
                return Err(VulkanError::UnexpectedReadback);
            }
            bytes
        }
    };
    Ok(GpuReport {
        fixture,
        bytes,
        usage: budget.usage,
    })
}
