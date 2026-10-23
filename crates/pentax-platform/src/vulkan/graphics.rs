// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: owned guest draw upload, LOAD and fenced readback.
// Public KytyPS5/KytyPS5@7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3:
// src/graphics/host_gpu/renderer/renderDraw.cpp (DrawAuto/EmitDrawPrimitives),
// src/graphics/host_gpu/renderer/pipeline/pipelineCache.cpp; see NOTICE.

use super::{
    Budget, GpuLimits, GraphicsReadback, VulkanError,
    commands::{Batch, buffer_host_barrier},
    device::{Device, DeviceRequirements},
    pipeline::Pipeline,
    programs::check_graphics_contract,
    resources::{Buffer, Image, color_range},
};
use ash::vk;
use pentax_gpu::graphics::{IndexType, PreparedDraw, ShaderInterfaces, validate_shader_interfaces};
use pentax_shader::ShaderStage;
use pentax_shader_compiler::CompiledShader;

#[path = "graphics/lod_feedback.rs"]
mod lod_feedback;
#[path = "graphics/sampled.rs"]
mod sampled;

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

#[derive(Clone, Copy, PartialEq, Eq)]
struct IndexInput {
    kind: vk::IndexType,
    maximum: u32,
}

impl std::fmt::Debug for IndexInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IndexInput")
            .field("kind", &self.kind.as_raw())
            .field("maximum", &self.maximum)
            .finish()
    }
}

// Validate and charge the complete owned snapshot before reading its values.
// firstIndex/vertexOffset are both zero in this explicit profile; an index
// value need not be less than indexCount (there are no vertex-buffer inputs).
// https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdDrawIndexed.html
fn index_input(
    bytes: &[u8],
    kind: IndexType,
    count: u32,
    vertices: u32,
    first_vertex: u32,
    budget: &mut Budget,
) -> Result<IndexInput, VulkanError> {
    let (width, kind) = match kind {
        IndexType::U16 => (2usize, vk::IndexType::UINT16),
        IndexType::U32 => (4usize, vk::IndexType::UINT32),
    };
    if count == 0
        || count != vertices
        || first_vertex != 0
        || (count as usize).checked_mul(width) != Some(bytes.len())
    {
        return Err(VulkanError::InvalidCompilerContract(
            "graphics index extent",
        ));
    }
    budget.work(bytes.len() as u64)?;
    let maximum = if width == 2 {
        bytes
            .chunks_exact(2)
            .map(|word| u32::from(u16::from_le_bytes([word[0], word[1]])))
            .max()
            .unwrap_or(0)
    } else {
        bytes
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .max()
            .unwrap_or(0)
    };
    Ok(IndexInput { kind, maximum })
}

fn upload_bytes(
    target: usize,
    indices: usize,
    resources: [usize; 2],
    budget: &mut Budget,
) -> Result<(), VulkanError> {
    let upload = [indices, resources[0], resources[1]].into_iter().try_fold(
        target as u64,
        |sum, bytes| {
            sum.checked_add(bytes as u64)
                .ok_or(VulkanError::Limit("graphics host bytes"))
        },
    )?;
    budget.host(upload)?;
    if upload
        .checked_add(target as u64)
        .is_none_or(|peak| peak > budget.limits.max_host_bytes)
    {
        return Err(VulkanError::Limit("graphics host bytes"));
    }
    Ok(())
}

/// The exact snapshot is the descriptor range. No uninitialized alignment
/// prefix, partial dword, or larger inferred allocation can become shader data.
fn resource_input(shader: &CompiledShader, bytes: Option<&[u8]>) -> Result<usize, VulkanError> {
    if shader.manifest().buffers().len() != usize::from(bytes.is_some()) {
        return Err(VulkanError::InvalidCompilerContract(
            "graphics resource count",
        ));
    }
    match bytes {
        Some(bytes) if bytes.is_empty() || !bytes.len().is_multiple_of(4) => Err(
            VulkanError::InvalidCompilerContract("graphics resource extent"),
        ),
        Some(bytes) => Ok(bytes.len()),
        None => Ok(0),
    }
}

fn storage_limits(lengths: [usize; 2], host: &vk::PhysicalDeviceLimits) -> Result<(), VulkanError> {
    let count = lengths.into_iter().filter(|bytes| *bytes != 0).count() as u32;
    // Binding57 is a sparse binding number, not 58 descriptors. Both stages
    // expose at most one storage descriptor and the render pass has one color
    // attachment, which counts against the fragment per-stage resource limit.
    // https://docs.vulkan.org/spec/latest/chapters/limits.html
    if count != 0
        && (host.max_bound_descriptor_sets < 1
            || host.max_descriptor_set_storage_buffers < count
            || host.max_per_stage_descriptor_storage_buffers < 1
            || (lengths[0] != 0 && host.max_per_stage_resources < 1)
            || (lengths[1] != 0
                && (host.max_per_stage_resources < 2
                    || host.max_fragment_combined_output_resources < 2))
            || lengths
                .into_iter()
                .any(|bytes| bytes as u64 > u64::from(host.max_storage_buffer_range)))
    {
        return Err(VulkanError::Unsupported(
            "graphics storage descriptor limits",
        ));
    }
    Ok(())
}

fn interface_limits(
    interfaces: ShaderInterfaces,
    vertex: u32,
    pixel: u32,
) -> Result<(), VulkanError> {
    // Actual vec4 declarations consume complete locations even for sparse or
    // aliased ATTR mappings. Built-ins also count against component limits.
    // Vulkan interfaces: shader input/output Location and Component rules.
    let locations_fit =
        |mask: u32, components: u32| mask == 0 || 32 - mask.leading_zeros() <= components / 4;
    if interfaces.vertex_components > vertex
        || interfaces.pixel_components > pixel
        || !locations_fit(interfaces.vertex_locations, vertex)
        || !locations_fit(interfaces.pixel_locations, pixel)
    {
        return Err(VulkanError::Unsupported(
            "graphics interface component limits",
        ));
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

fn merge_stage_push(
    shader: &CompiledShader,
    input: &[u8; 128],
    output: &mut [u8; 128],
    initialized: &mut [bool; 32],
) -> Result<(), VulkanError> {
    if let Some(push) = shader.manifest().push_data() {
        merge_range(
            output,
            initialized,
            push.start_dword as usize,
            push.user_sgprs.len(),
            input,
        )?;
        let offset_words = push.memory_offset_count.div_ceil(4) as usize;
        if offset_words != 0 {
            let start = push
                .start_dword
                .checked_add(push.memory_offset_dword)
                .ok_or(VulkanError::InvalidCompilerContract(
                    "graphics buffer push extent",
                ))? as usize;
            let end = start
                .checked_add(offset_words)
                .filter(|end| *end <= 32)
                .ok_or(VulkanError::InvalidCompilerContract(
                    "graphics buffer push extent",
                ))?;
            if input[start * 4..end * 4].iter().any(|byte| *byte != 0) {
                return Err(VulkanError::InvalidCompilerContract(
                    "graphics buffer upload offset",
                ));
            }
            merge_range(output, initialized, start, offset_words, input)?;
        }
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
        merge_stage_push(shader, input, &mut output, &mut initialized)?;
    }
    // One update includes both layout stages as required by Vulkan
    // VUID-vkCmdPushConstants-offset-01796. Full compiler manifests include the
    // actual SGPR payload and packed zero-prefix bytes, within each16-word stage.
    Ok(output)
}

pub(super) fn run(
    prepared: &PreparedDraw,
    limits: GpuLimits,
) -> Result<GraphicsReadback, VulkanError> {
    run_with_lod(prepared, limits, None)
}

pub(super) fn run_with_lod(
    prepared: &PreparedDraw,
    limits: GpuLimits,
    initial: Option<&pentax_gpu::lod_stats::Counters>,
) -> Result<GraphicsReadback, VulkanError> {
    // All pure input/manifest/work admission precedes loader/device calls.
    let mut budget = Budget::new(limits);
    budget.work(1024)?;
    let sampled_upload_bytes = sampled::upload_length(prepared)?;
    let has_feedback = prepared.pixel().manifest().lod_feedback_binding().is_some();
    if prepared
        .pixel()
        .manifest()
        .lod_feedback_binding()
        .is_some_and(|binding| binding != 90)
        || has_feedback
            && (initial.is_none() || prepared.pixel().manifest().lod_counters().is_empty())
    {
        return Err(VulkanError::InvalidCompilerContract(
            "initialized LOD feedback storage",
        ));
    }
    for counter in prepared.pixel().manifest().lod_counters() {
        let image = prepared
            .pixel()
            .manifest()
            .images()
            .iter()
            .find(|image| image.user_sgpr == counter.user_sgpr)
            .ok_or(VulkanError::InvalidCompilerContract(
                "LOD feedback image provenance",
            ))?;
        let texture = prepared
            .pixel_images()
            .iter()
            .find(|texture| texture.binding() == image.binding)
            .ok_or(VulkanError::InvalidCompilerContract(
                "LOD feedback image resource",
            ))?;
        let descriptor = texture.descriptor();
        if descriptor.counter_id != Some(counter.counter_id)
            || descriptor.base_level != counter.base_level
            || descriptor.min_lod_u4_8 != counter.min_lod_u4_8
        {
            return Err(VulkanError::InvalidCompilerContract(
                "LOD feedback descriptor provenance",
            ));
        }
    }
    for texture in prepared.pixel_images() {
        if texture.descriptor().counter_id.is_some() && initial.is_none() {
            return Err(VulkanError::Unsupported("uninitialized image LOD feedback"));
        }
    }
    if prepared
        .pixel_samplers()
        .iter()
        .any(|sampler| sampler.descriptor().lod_bias_s6_8 != 0)
    {
        return Err(VulkanError::Unsupported("sampled LOD bias"));
    }
    let first_instance = prepared.first_instance();
    let vertex_offset = prepared.vertex_offset();
    prepared
        .validate_native_instance_contract()
        .map_err(|_| VulkanError::InvalidCompilerContract("native indirect instance context"))?;
    if prepared.native_indirect_offsets().is_some() && (first_instance != 0 || vertex_offset != 0) {
        return Err(VulkanError::InvalidCompilerContract(
            "native indirect instance context",
        ));
    }
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
    let resource_bytes = [
        prepared.vertex_buffer().map(|buffer| buffer.bytes()),
        prepared.pixel_buffer().map(|buffer| buffer.bytes()),
    ];
    let resource_lengths = [
        resource_input(prepared.vertex(), resource_bytes[0])?,
        resource_input(prepared.pixel(), resource_bytes[1])?,
    ];
    budget.work(256)?;
    let interfaces = validate_shader_interfaces(prepared.vertex(), prepared.pixel())
        .map_err(|_| VulkanError::InvalidCompilerContract("graphics interfaces"))?;
    let push = push_data(prepared)?;
    let indices = if let Some(indices) = prepared.indices() {
        let input = index_input(
            indices.bytes(),
            indices.index_type(),
            indices.count(),
            prepared.vertex_count(),
            prepared.first_vertex(),
            &mut budget,
        )?;
        Some((indices, input))
    } else {
        None
    };
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
    budget.host(sampled_upload_bytes as u64)?;
    upload_bytes(
        length,
        indices.map_or(0, |(indices, _)| indices.bytes().len()),
        resource_lengths,
        &mut budget,
    )?;
    let device = Device::open_required(
        DeviceRequirements {
            subgroup_stages: if has_feedback {
                vk::ShaderStageFlags::FRAGMENT
            } else {
                vk::ShaderStageFlags::empty()
            },
            arithmetic_stages: if has_feedback {
                vk::ShaderStageFlags::FRAGMENT
            } else {
                vk::ShaderStageFlags::empty()
            },
            fragment_stores_and_atomics: has_feedback,
            // Real fragment LOD reductions/election retain their qualified
            // native32 profile, independent of width-unconstrained barriers.
            required_subgroup_size: has_feedback.then_some(32),
            ..Default::default()
        },
        &mut budget,
    )?;
    let host = device.properties.limits;
    storage_limits(resource_lengths, &host)?;
    if has_feedback {
        let pixel_storage = 1 + u32::from(resource_lengths[1] != 0);
        let total_storage = pixel_storage + u32::from(resource_lengths[0] != 0);
        if host.max_storage_buffer_range < pentax_gpu::lod_stats::COUNTER_BYTES as u32
            || host.max_per_stage_descriptor_storage_buffers < pixel_storage
            || host.max_descriptor_set_storage_buffers < total_storage
            || host.max_fragment_combined_output_resources < pixel_storage + 1
        {
            return Err(VulkanError::Unsupported(
                "LOD feedback storage device limits",
            ));
        }
    }
    interface_limits(
        interfaces,
        host.max_vertex_output_components,
        host.max_fragment_input_components,
    )?;
    if let Some((_, input)) = indices {
        device.check_index_value(input.maximum)?;
    }
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
    let index_buffer = if let Some((indices, _)) = indices {
        let mut buffer = Buffer::new(
            &device,
            indices.bytes().len(),
            vk::BufferUsageFlags::INDEX_BUFFER,
            &mut budget,
        )?;
        buffer.write(indices.bytes(), &mut budget)?;
        Some(buffer)
    } else {
        None
    };
    let mut storage = [None, None];
    for (index, bytes) in resource_bytes.into_iter().enumerate() {
        if let Some(bytes) = bytes {
            let mut buffer = Buffer::new(
                &device,
                bytes.len(),
                vk::BufferUsageFlags::STORAGE_BUFFER,
                &mut budget,
            )?;
            buffer.write(bytes, &mut budget)?;
            storage[index] = Some(buffer);
        }
    }
    let sampled =
        sampled::SampledResources::new(&device, prepared, initial.is_some(), &mut budget)?;
    let feedback = if has_feedback {
        Some(lod_feedback::Feedback::new(
            &device,
            initial.ok_or(VulkanError::InvalidCompilerContract(
                "missing LOD initial words",
            ))?,
            &mut budget,
        )?)
    } else {
        None
    };
    let mut sampled_bindings = sampled.bindings();
    sampled_bindings.lod_feedback = feedback.as_ref().map(|buffer| &buffer.storage);
    // Admit the future completed pixel Vec before any submitted GPU effect.
    if budget
        .usage
        .host_bytes
        .checked_add(length as u64)
        .and_then(|bytes| {
            bytes.checked_add(if has_feedback {
                pentax_gpu::lod_stats::COUNTER_BYTES as u64
            } else {
                0
            })
        })
        .is_none_or(|bytes| bytes > budget.limits.max_host_bytes)
    {
        return Err(VulkanError::Limit("graphics readback host bytes"));
    }
    let pipeline = if prepared.pixel_images().is_empty()
        && prepared.pixel_samplers().is_empty()
        && !has_feedback
    {
        Pipeline::graphics_loaded(
            &device,
            prepared.vertex(),
            prepared.pixel(),
            &image,
            [storage[0].as_ref(), storage[1].as_ref()],
            &mut budget,
        )?
    } else {
        Pipeline::graphics_loaded_sampled(
            &device,
            prepared.vertex(),
            prepared.pixel(),
            &image,
            [storage[0].as_ref(), storage[1].as_ref()],
            sampled_bindings,
            &mut budget,
        )?
    };
    // The batch is declared last and drains uncertain submissions before any
    // pipeline, framebuffer, image or buffer owner can be destroyed.
    let batch = Batch::begin(&device, &mut budget)?;
    sampled.record(&device, &batch, prepared, &mut budget)?;
    if let Some(feedback) = &feedback {
        feedback.begin(&device, &batch, &mut budget)?;
    }
    budget.command(
        (if index_buffer.is_some() { 10 } else { 9 })
            + u32::from(pipeline.set != vk::DescriptorSet::null()),
    )?;
    let mut upload_barriers = [vk::BufferMemoryBarrier::default(); 4];
    upload_barriers[0] = vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::HOST_WRITE)
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(upload.raw)
        .offset(0)
        .size(length as u64);
    let mut upload_stages = vk::PipelineStageFlags::TRANSFER;
    let mut upload_count = if let Some(buffer) = &index_buffer {
        upload_barriers[1] = vk::BufferMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::INDEX_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(buffer.raw)
            .offset(0)
            .size(buffer.length() as u64);
        upload_stages |= vk::PipelineStageFlags::VERTEX_INPUT;
        2
    } else {
        1
    };
    for (stage, buffer) in storage.iter().enumerate() {
        if let Some(buffer) = buffer {
            upload_barriers[upload_count] = vk::BufferMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(buffer.raw)
                .offset(0)
                .size(buffer.length() as u64);
            upload_stages |= if stage == 0 {
                vk::PipelineStageFlags::VERTEX_SHADER
            } else {
                vk::PipelineStageFlags::FRAGMENT_SHADER
            };
            upload_count += 1;
        }
    }
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
            upload_stages,
            vk::DependencyFlags::empty(),
            &[],
            &upload_barriers[..upload_count],
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
        if pipeline.set != vk::DescriptorSet::null() {
            device.raw.cmd_bind_descriptor_sets(
                batch.raw,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.layout,
                0,
                &[pipeline.set],
                &[],
            );
        }
        device.raw.cmd_push_constants(
            batch.raw,
            pipeline.layout,
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            0,
            &push,
        );
        if let (Some(buffer), Some((indices, input))) = (&index_buffer, indices) {
            // Dedicated fully bound INDEX_BUFFER, offset0 obeys either element
            // alignment. HOST_WRITE visibility precedes index fetch; restart
            // stays disabled in the fixed triangle-list pipeline. The complete
            // index extent and actual enabled device range were checked above.
            // https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdBindIndexBuffer.html
            device
                .raw
                .cmd_bind_index_buffer(batch.raw, buffer.raw, 0, input.kind);
            device.raw.cmd_draw_indexed(
                batch.raw,
                indices.count(),
                prepared.instance_count(),
                0,             // The uploaded index span already starts at the actual firstIndex.
                vertex_offset, // Same value validated against the native compiler context above.
                first_instance,
            );
        } else {
            device.raw.cmd_draw(
                batch.raw,
                prepared.vertex_count(),
                prepared.instance_count(),
                prepared.first_vertex(),
                first_instance,
            );
        }
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
    if let Some(feedback) = &feedback {
        feedback.end(&device, &batch, &mut budget)?;
    }
    batch.submit_wait(&mut budget)?;
    let bytes = readback.read_completed(&mut budget)?;
    let lod_counters = feedback
        .as_ref()
        .map(|feedback| feedback.completed(&mut budget))
        .transpose()?;
    Ok(GraphicsReadback {
        bytes,
        lod_counters,
        usage: budget.usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameter_interface_limits_count_sparse_locations_aliases_and_builtins() {
        let sparse = ShaderInterfaces {
            vertex_locations: 1 << 31,
            pixel_locations: 1 << 31,
            vertex_components: 8,
            pixel_components: 4,
        };
        assert!(interface_limits(sparse, 128, 128).is_ok());
        assert!(interface_limits(sparse, 64, 128).is_err());
        assert!(interface_limits(sparse, 128, 64).is_err());
        let full = ShaderInterfaces {
            vertex_locations: u32::MAX,
            pixel_locations: u32::MAX,
            vertex_components: 132,
            pixel_components: 133,
        };
        assert!(interface_limits(full, 132, 136).is_ok());
        assert!(interface_limits(full, 128, 136).is_err());
        assert!(interface_limits(full, 132, 132).is_err());
        let aliases_and_system = ShaderInterfaces {
            vertex_locations: 1 << 7,
            pixel_locations: 1 << 7,
            vertex_components: 8,
            pixel_components: 9,
        };
        assert!(interface_limits(aliases_and_system, 32, 32).is_ok());
        assert!(interface_limits(aliases_and_system, 32, 8).is_err());
    }

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
    fn index_snapshots_preserve_unsigned_values_and_refuse_incomplete_or_unadmitted_scans() {
        let mut budget = Budget::new(GpuLimits::default());
        let short = [0, 0x80, 0xff, 0xff, 1, 0];
        let input = index_input(&short, IndexType::U16, 3, 3, 0, &mut budget).unwrap();
        assert_eq!(input.kind.as_raw(), vk::IndexType::UINT16.as_raw());
        assert_eq!(input.maximum, 65_535); // Legal value above the three indices.
        let wide = [0, 0, 0, 0x80, 0xff, 0xff, 0xff, 0xff, 0, 0, 1, 0];
        let input = index_input(&wide, IndexType::U32, 3, 3, 0, &mut budget).unwrap();
        assert_eq!(input.kind.as_raw(), vk::IndexType::UINT32.as_raw());
        assert_eq!(input.maximum, u32::MAX); // No host range is inferred here.
        assert_eq!(budget.usage.work, 18);

        for (bytes, kind, count, vertices, first) in [
            (&short[..5], IndexType::U16, 3, 3, 0),
            (&wide[..11], IndexType::U32, 3, 3, 0),
            (&short[..], IndexType::U16, 3, 2, 0),
            (&short[..], IndexType::U16, 3, 3, 1),
            (&[][..], IndexType::U32, 0, 0, 0),
        ] {
            assert_eq!(
                index_input(bytes, kind, count, vertices, first, &mut budget),
                Err(VulkanError::InvalidCompilerContract(
                    "graphics index extent"
                ))
            );
            assert_eq!(budget.usage.work, 18);
        }
        let mut refused = Budget::new(GpuLimits {
            max_work: 5,
            ..GpuLimits::default()
        });
        assert_eq!(
            index_input(&short, IndexType::U16, 3, 3, 0, &mut refused),
            Err(VulkanError::Limit("work"))
        );
        assert_eq!(refused.usage.work, 0);
    }

    #[test]
    fn index_upload_and_completed_target_share_the_finite_host_ceiling() {
        let mut fits = Budget::new(GpuLimits {
            max_host_bytes: 20,
            ..GpuLimits::default()
        });
        upload_bytes(8, 4, [0; 2], &mut fits).unwrap();
        assert_eq!(fits.usage.host_bytes, 12);
        let readback = fits.buffer(8).unwrap();
        assert_eq!(readback.len(), 8);
        assert_eq!(fits.usage.host_bytes, 20);
        let mut refused = Budget::new(GpuLimits {
            max_host_bytes: 19,
            ..GpuLimits::default()
        });
        assert_eq!(
            upload_bytes(8, 4, [0; 2], &mut refused),
            Err(VulkanError::Limit("graphics host bytes"))
        );
        assert!(
            upload_bytes(
                usize::MAX,
                1,
                [0; 2],
                &mut Budget::new(GpuLimits::default())
            )
            .is_err()
        );
        // Preserve the old non-indexed refusal order: reject the upload first,
        // then its simultaneous completed target, without a device allocation.
        let mut upload_refused = Budget::new(GpuLimits {
            max_host_bytes: 7,
            ..GpuLimits::default()
        });
        assert_eq!(
            upload_bytes(8, 0, [0; 2], &mut upload_refused),
            Err(VulkanError::Limit("host bytes"))
        );
        assert_eq!(upload_refused.usage.host_bytes, 0);
        let mut readback_refused = Budget::new(GpuLimits {
            max_host_bytes: 15,
            ..GpuLimits::default()
        });
        assert_eq!(
            upload_bytes(8, 0, [0; 2], &mut readback_refused),
            Err(VulkanError::Limit("graphics host bytes"))
        );
        assert_eq!(readback_refused.usage.host_bytes, 8);
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

#[cfg(test)]
#[path = "graphics/buffer_tests.rs"]
mod buffer_tests;
