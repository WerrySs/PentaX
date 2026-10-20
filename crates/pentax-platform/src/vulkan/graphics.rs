// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: owned guest draw upload, LOAD and fenced readback.
// Public KytyPS5/KytyPS5@7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3:
// src/graphics/host_gpu/renderer/renderDraw.cpp (DrawAuto/EmitDrawPrimitives),
// src/graphics/host_gpu/renderer/pipeline/pipelineCache.cpp; see NOTICE.

use super::{
    Budget, GpuLimits, GraphicsReadback, VulkanError,
    commands::{Batch, buffer_host_barrier},
    device::Device,
    pipeline::Pipeline,
    programs::check_graphics_contract,
    resources::{Buffer, Image, color_range},
};
use ash::vk;
use pentax_gpu::graphics::PreparedDraw;
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{CompiledShader, InputInterface, OutputInterface};

fn extent(width: u32, height: u32) -> Result<usize, VulkanError> {
    width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(4))
        .filter(|n| width != 0 && height != 0 && *n != 0)
        .map(|n| n as usize)
        .ok_or(VulkanError::Limit("graphics target extent"))
}

// A conservative workload admission, not a driver execution-time bound. Each
// submitted triangle may cover the complete target; clipping does not lower it.
fn draw_work(
    vertices: u32,
    instances: u32,
    pixels: u64,
    vs: usize,
    ps: usize,
) -> Result<u64, VulkanError> {
    let vertex_work = u64::from(vertices).checked_mul(vs as u64);
    let triangles = u64::from(vertices / 3);
    let pixel_work = pixels
        .checked_mul(triangles)
        .and_then(|n| n.checked_mul(ps as u64));
    vertex_work
        .and_then(|v| pixel_work.and_then(|p| v.checked_add(p)))
        .and_then(|n| n.checked_mul(u64::from(instances)))
        .ok_or(VulkanError::Limit("graphics execution work"))
}

fn interfaces(vertex: &CompiledShader, pixel: &CompiledShader) -> Result<(), VulkanError> {
    if vertex.manifest().outputs() != [OutputInterface::Position]
        || vertex.manifest().inputs().iter().any(|input| {
            !matches!(
                input,
                InputInterface::VertexIndex | InputInterface::InstanceIndex
            )
        })
        || pixel.manifest().inputs().iter().any(|input| {
            !matches!(
                input,
                InputInterface::FragCoord | InputInterface::FrontFacing
            )
        })
        || pixel.manifest().outputs()
            != [OutputInterface::Color {
                export: 0,
                location: 0,
                uint: false,
            }]
    {
        return Err(VulkanError::InvalidCompilerContract("graphics interfaces"));
    }
    Ok(())
}

fn merge_range(
    output: &mut [u8; 128],
    initialized: &mut [bool; 32],
    start: usize,
    count: usize,
    input: &[u8; 128],
) -> Result<(), VulkanError> {
    let end = start
        .checked_add(count)
        .filter(|end| *end <= 32)
        .ok_or(VulkanError::InvalidCompilerContract("graphics push extent"))?;
    for (word, marked) in initialized.iter_mut().enumerate().take(end).skip(start) {
        let at = word * 4;
        if *marked && output[at..at + 4] != input[at..at + 4] {
            return Err(VulkanError::InvalidCompilerContract(
                "overlapping graphics push values",
            ));
        }
        output[at..at + 4].copy_from_slice(&input[at..at + 4]);
        *marked = true;
    }
    Ok(())
}

fn push_data(prepared: &PreparedDraw) -> Result<[u8; 128], VulkanError> {
    let mut output = [0; 128];
    let mut initialized = [false; 32];
    for (shader, input) in [
        (prepared.vertex(), prepared.vertex_push_data()),
        (prepared.pixel(), prepared.pixel_push_data()),
    ] {
        if let Some(push) = shader.manifest().push_data() {
            merge_range(
                &mut output,
                &mut initialized,
                push.start_dword as usize,
                push.user_sgprs.len(),
                input,
            )?;
        }
    }
    // One update includes both layout stages as required by Vulkan
    // VUID-vkCmdPushConstants-offset-01796. Stage placement comes from the actual
    // compiler manifests; unused initialized zeros are never invented SGPRs.
    Ok(output)
}

pub(super) fn run(
    prepared: &PreparedDraw,
    limits: GpuLimits,
) -> Result<GraphicsReadback, VulkanError> {
    // All pure input/manifest/work admission precedes loader/device calls.
    let mut budget = Budget::new(limits);
    let length = extent(prepared.width(), prepared.height())?;
    if !prepared.width().is_multiple_of(64)
        || prepared.input().len() != length
        || prepared.vertex_count() == 0
        || prepared.instance_count() == 0
        || prepared
            .first_vertex()
            .checked_add(prepared.vertex_count())
            .is_none()
        || prepared
            .first_instance()
            .checked_add(prepared.instance_count())
            .is_none()
    {
        return Err(VulkanError::InvalidCompilerContract("graphics draw extent"));
    }
    check_graphics_contract(prepared.vertex(), ShaderStage::Vertex)?;
    check_graphics_contract(prepared.pixel(), ShaderStage::Pixel)?;
    interfaces(prepared.vertex(), prepared.pixel())?;
    let push = push_data(prepared)?;
    budget.shader(prepared.vertex().spirv().len())?;
    budget.shader(prepared.pixel().spirv().len())?;
    budget.work(draw_work(
        prepared.vertex_count(),
        prepared.instance_count(),
        (length / 4) as u64,
        prepared.vertex().spirv().len(),
        prepared.pixel().spirv().len(),
    )?)?;
    // Retained upload and completed owned readback coexist through publication.
    // Buffer::read_completed charges the latter before its fallible allocation.
    budget.host(length as u64)?;
    if (length as u64)
        .checked_mul(2)
        .is_none_or(|n| n > limits.max_host_bytes)
    {
        return Err(VulkanError::Limit("graphics host bytes"));
    }
    let device = Device::open(
        vk::ShaderStageFlags::empty(),
        vk::ShaderStageFlags::empty(),
        &mut budget,
    )?;
    let host = device.properties.limits;
    if prepared.width() > host.max_viewport_dimensions[0]
        || prepared.height() > host.max_viewport_dimensions[1]
        || prepared.width() > host.max_framebuffer_width
        || prepared.height() > host.max_framebuffer_height
        || host.viewport_bounds_range[0] > 0.0
        || prepared.width() as f32 > host.viewport_bounds_range[1]
        || prepared.height() as f32 > host.viewport_bounds_range[1]
    {
        return Err(VulkanError::Unsupported(
            "graphics viewport/framebuffer extent",
        ));
    }
    let image = Image::rgba_uploaded(&device, prepared.width(), prepared.height(), &mut budget)?;
    let mut upload = Buffer::new(
        &device,
        length,
        vk::BufferUsageFlags::TRANSFER_SRC,
        &mut budget,
    )?;
    upload.write(prepared.input(), &mut budget)?;
    let readback = Buffer::new(
        &device,
        length,
        vk::BufferUsageFlags::TRANSFER_DST,
        &mut budget,
    )?;
    let pipeline = Pipeline::graphics_loaded(
        &device,
        prepared.vertex(),
        prepared.pixel(),
        &image,
        &mut budget,
    )?;
    // The batch is declared last and drains uncertain submissions before any
    // pipeline, framebuffer, image or buffer owner can be destroyed.
    let batch = Batch::begin(&device, &mut budget)?;
    budget.command(9)?;
    let upload_barrier = [vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::HOST_WRITE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(upload.raw)
        .offset(0)
        .size(length as u64)];
    let initial = [vk::ImageMemoryBarrier::default()
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image.raw)
        .subresource_range(color_range())];
    let region = [vk::BufferImageCopy::default()
        .buffer_offset(0)
        .buffer_row_length(0)
        .buffer_image_height(0)
        .image_subresource(vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        })
        .image_extent(vk::Extent3D {
            width: image.width,
            height: image.height,
            depth: 1,
        })];
    let color_barrier = [vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        )
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image.raw)
        .subresource_range(color_range())];
    let begin = vk::RenderPassBeginInfo::default()
        .render_pass(pipeline.render_pass)
        .framebuffer(pipeline.framebuffer)
        .render_area(vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: image.width,
                height: image.height,
            },
        });
    // SAFETY: All dimensions, manifests, feature requirements and owned byte
    // extents were admitted above. The upload is flushed; image layout/access
    // transitions precede LOAD. No host access overlaps execution. No guest or
    // caller supplies raw handles/SPIR-V. Every owner remains live through the
    // batch's fence/drain, and all buffer-image copies cover exactly length.
    unsafe {
        device.raw.cmd_pipeline_barrier(
            batch.raw,
            vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &upload_barrier,
            &initial,
        );
        device.raw.cmd_copy_buffer_to_image(
            batch.raw,
            upload.raw,
            image.raw,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &region,
        );
        device.raw.cmd_pipeline_barrier(
            batch.raw,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &color_barrier,
        );
        device
            .raw
            .cmd_begin_render_pass(batch.raw, &begin, vk::SubpassContents::INLINE);
        device
            .raw
            .cmd_bind_pipeline(batch.raw, vk::PipelineBindPoint::GRAPHICS, pipeline.raw);
        device.raw.cmd_push_constants(
            batch.raw,
            pipeline.layout,
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            0,
            &push,
        );
        device.raw.cmd_draw(
            batch.raw,
            prepared.vertex_count(),
            prepared.instance_count(),
            prepared.first_vertex(),
            prepared.first_instance(),
        );
        device.raw.cmd_end_render_pass(batch.raw);
        // Render-pass final layout and external dependency make color writes
        // visible to the transfer, including pixels preserved by LOAD.
        device.raw.cmd_copy_image_to_buffer(
            batch.raw,
            image.raw,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            readback.raw,
            &region,
        );
    }
    buffer_host_barrier(
        &device,
        batch.raw,
        readback.raw,
        length as u64,
        vk::AccessFlags::TRANSFER_WRITE,
        vk::PipelineStageFlags::TRANSFER,
        &mut budget,
    )?;
    batch.submit_wait(&mut budget)?;
    let bytes = readback.read_completed(&mut budget)?;
    Ok(GraphicsReadback {
        bytes,
        usage: budget.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_and_whole_draw_work_bounds_do_not_wrap_or_discount_overdraw() {
        assert!(extent(0, 1).is_err());
        assert!(extent(u32::MAX, 2).is_err());
        assert_eq!(extent(3, 2), Ok(24));
        // Two triangles per instance: each can cover all ten pixels.
        assert_eq!(draw_work(6, 2, 10, 4, 5), Ok((24 + 100) * 2));
        assert!(draw_work(u32::MAX, u32::MAX, u64::MAX, 1, 1).is_err());
    }

    #[test]
    fn independent_stage_data_merges_by_live_manifest_range_and_rejects_conflicts() {
        let mut output = [0; 128];
        let mut initialized = [false; 32];
        let mut vertex = [0; 128];
        let mut pixel = [0; 128];
        vertex[..4].copy_from_slice(&11_u32.to_le_bytes());
        pixel[64..68].copy_from_slice(&29_u32.to_le_bytes());
        merge_range(&mut output, &mut initialized, 0, 1, &vertex).unwrap();
        merge_range(&mut output, &mut initialized, 16, 1, &pixel).unwrap();
        assert_eq!(&output[..4], &11_u32.to_le_bytes());
        assert_eq!(&output[64..68], &29_u32.to_le_bytes());
        assert_eq!(
            merge_range(&mut output, &mut initialized, 0, 1, &pixel),
            Err(VulkanError::InvalidCompilerContract(
                "overlapping graphics push values"
            ))
        );
        assert_eq!(
            merge_range(&mut output, &mut initialized, 32, 1, &pixel),
            Err(VulkanError::InvalidCompilerContract("graphics push extent"))
        );
        assert_eq!(&output[..4], &11_u32.to_le_bytes());
    }
}
