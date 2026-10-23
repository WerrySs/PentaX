// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: manifest-bound descriptors and owned pipelines; see NOTICE.

use super::{
    Budget, VulkanError,
    device::{Device, result},
    resources::{Buffer, Image, SampledImage, Sampler},
};
use ash::vk;
use pentax_shader_compiler::CompiledShader;

#[derive(Clone, Copy)]
pub(super) struct SampledBindings<'d, 'r> {
    pub images: &'r [SampledImage<'d>],
    pub image_bindings: &'r [u32],
    pub samplers: &'r [Sampler<'d>],
    pub sampler_bindings: &'r [u32],
}
impl SampledBindings<'_, '_> {
    pub const fn empty() -> Self {
        Self {
            images: &[],
            image_bindings: &[],
            samplers: &[],
            sampler_bindings: &[],
        }
    }
}

enum Resource<'d, 'r> {
    Buffer(&'r Buffer<'d>),
    Graphics {
        image: &'r Image<'d>,
        buffers: [Option<&'r Buffer<'d>>; 2],
        sampled: SampledBindings<'d, 'r>,
    },
}
pub(super) struct Pipeline<'d, 'r> {
    device: &'d Device,
    // Retains the actual resource, not a detached raw handle lifetime.
    resource: Resource<'d, 'r>,
    pub raw: vk::Pipeline,
    pub layout: vk::PipelineLayout,
    pub set: vk::DescriptorSet,
    set_layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    pub render_pass: vk::RenderPass,
    pub framebuffer: vk::Framebuffer,
}
struct Shader<'d> {
    device: &'d Device,
    raw: vk::ShaderModule,
}
impl<'d> Shader<'d> {
    fn new(
        device: &'d Device,
        compiled: &CompiledShader,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        budget.object()?;
        let info = vk::ShaderModuleCreateInfo::default().code(compiled.spirv());
        // SAFETY: Only opaque compiler output from original factories or the
        // checked compute preparer reaches this constructor; capabilities/device contract were
        // checked before FFI. The aligned word slice stays live through the call.
        let raw = result(
            unsafe { device.raw.create_shader_module(&info, None) },
            "vkCreateShaderModule",
        )?;
        Ok(Self { device, raw })
    }
}
impl Drop for Shader<'_> {
    fn drop(&mut self) {
        // SAFETY: Pipeline creation has completed and takes no ownership of the
        // shader module; the unique module is not referenced by pending work.
        unsafe { self.device.raw.destroy_shader_module(self.raw, None) };
    }
}
impl<'d, 'r> Pipeline<'d, 'r> {
    fn empty(device: &'d Device, resource: Resource<'d, 'r>) -> Self {
        Self {
            device,
            resource,
            raw: vk::Pipeline::null(),
            layout: vk::PipelineLayout::null(),
            set: vk::DescriptorSet::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            render_pass: vk::RenderPass::null(),
            framebuffer: vk::Framebuffer::null(),
        }
    }
    fn layouts(
        &mut self,
        bindings: &[vk::DescriptorSetLayoutBinding<'_>],
        stages: vk::ShaderStageFlags,
        budget: &mut Budget,
    ) -> Result<(), VulkanError> {
        budget.object()?;
        let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(bindings);
        // SAFETY: Binding counts/types/stages come only from the retained checked
        // compiler manifest. No push-descriptor/indexing extensions are assumed.
        self.set_layout = result(
            unsafe { self.device.raw.create_descriptor_set_layout(&info, None) },
            "vkCreateDescriptorSetLayout",
        )?;
        budget.object()?;
        let sets = [self.set_layout];
        let push = [vk::PushConstantRange::default()
            .stage_flags(stages)
            .offset(0)
            .size(128)];
        let info = vk::PipelineLayoutCreateInfo::default()
            .set_layouts(&sets)
            .push_constant_ranges(&push);
        // SAFETY: One live compatible set layout; core128-byte push range was
        // checked against device limits. Borrows survive the synchronous call.
        self.layout = result(
            unsafe { self.device.raw.create_pipeline_layout(&info, None) },
            "vkCreatePipelineLayout",
        )?;
        Ok(())
    }
    pub fn compute(
        device: &'d Device,
        shader: &CompiledShader,
        buffer: &'r Buffer<'d>,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        let mut out = Self::empty(device, Resource::Buffer(buffer));
        let binding =
            shader
                .manifest()
                .buffer_binding()
                .ok_or(VulkanError::InvalidCompilerContract(
                    "missing buffer binding",
                ))?;
        if shader.manifest().buffers().len() != 1 || !buffer.length().is_multiple_of(4) {
            return Err(VulkanError::InvalidCompilerContract(
                "buffer descriptor count/extent",
            ));
        }
        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(binding)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE)];
        out.layouts(&bindings, vk::ShaderStageFlags::COMPUTE, budget)?;
        budget.object()?;
        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)];
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&sizes);
        // SAFETY: One bounded storage-buffer descriptor/set, unique owned pool.
        out.pool = result(
            unsafe { device.raw.create_descriptor_pool(&info, None) },
            "vkCreateDescriptorPool",
        )?;
        budget.object()?;
        let layouts = [out.set_layout];
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(out.pool)
            .set_layouts(&layouts);
        // SAFETY: The pool has capacity for this exact single layout/output slot.
        let code = unsafe {
            (device.raw.fp_v1_0().allocate_descriptor_sets)(
                device.raw.handle(),
                &info,
                &mut out.set,
            )
        };
        result(code.result(), "vkAllocateDescriptorSets")?;
        let buffers = [vk::DescriptorBufferInfo::default()
            .buffer(buffer.raw)
            .offset(0)
            .range(buffer.length() as u64)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(out.set)
            .dst_binding(binding)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&buffers)];
        budget.work(1)?;
        // SAFETY: Exact manifest binding, one fully allocated buffer, no pending
        // descriptor use. The Pipeline retains a borrow of that actual buffer.
        unsafe { device.raw.update_descriptor_sets(&writes, &[]) };
        let module = Shader::new(device, shader, budget)?;
        let stage = vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::COMPUTE)
            .module(module.raw)
            .name(c"main");
        let info = vk::ComputePipelineCreateInfo::default()
            .stage(stage)
            .layout(out.layout);
        budget.object()?;
        // SAFETY: Checked compute shader, matching retained descriptor/push
        // layout and no derivative/cache base. Output stored before error check
        // so partial pipeline creation is cleaned up on every failure.
        let code = unsafe {
            (device.raw.fp_v1_0().create_compute_pipelines)(
                device.raw.handle(),
                vk::PipelineCache::null(),
                1,
                &info,
                std::ptr::null(),
                &mut out.raw,
            )
        };
        result(code.result(), "vkCreateComputePipelines")?;
        Ok(out)
    }

    fn graphics_descriptors(
        &mut self,
        buffers: [Option<&'r Buffer<'d>>; 2],
        count: usize,
        sampled: SampledBindings<'d, 'r>,
        budget: &mut Budget,
    ) -> Result<(), VulkanError> {
        let mut sizes = [vk::DescriptorPoolSize::default(); 3];
        let mut pool_count = 0;
        for (kind, length) in [
            (vk::DescriptorType::STORAGE_BUFFER, count),
            (vk::DescriptorType::SAMPLED_IMAGE, sampled.images.len()),
            (vk::DescriptorType::SAMPLER, sampled.samplers.len()),
        ] {
            if length != 0 {
                sizes[pool_count] = vk::DescriptorPoolSize::default()
                    .ty(kind)
                    .descriptor_count(length as u32);
                pool_count += 1;
            }
        }
        budget.object()?;
        let info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&sizes[..pool_count]);
        // SAFETY: Exact bounded counts of the manifest's retained resources.
        self.pool = result(
            unsafe { self.device.raw.create_descriptor_pool(&info, None) },
            "vkCreateDescriptorPool(graphics)",
        )?;
        budget.object()?;
        let layouts = [self.set_layout];
        let info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.pool)
            .set_layouts(&layouts);
        // SAFETY: One owned output and the exact layout from this live pool.
        result(
            unsafe {
                (self.device.raw.fp_v1_0().allocate_descriptor_sets)(
                    self.device.raw.handle(),
                    &info,
                    &mut self.set,
                )
            }
            .result(),
            "vkAllocateDescriptorSets(graphics)",
        )?;
        let mut buffers_info = [vk::DescriptorBufferInfo::default(); 2];
        let mut buffer_bindings = [0; 2];
        let mut used = 0;
        for (stage, buffer) in buffers.into_iter().enumerate() {
            if let Some(buffer) = buffer {
                buffers_info[used] = vk::DescriptorBufferInfo::default()
                    .buffer(buffer.raw)
                    .range(buffer.length() as u64);
                buffer_bindings[used] = if stage == 0 { 0 } else { 57 };
                used += 1;
            }
        }
        let mut image_info = [vk::DescriptorImageInfo::default(); 32];
        for (index, image) in sampled.images.iter().enumerate() {
            image_info[index] = vk::DescriptorImageInfo::default()
                .image_view(image.view())
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        }
        for (index, sampler) in sampled.samplers.iter().enumerate() {
            image_info[16 + index] = vk::DescriptorImageInfo::default().sampler(sampler.raw);
        }
        let mut writes = [vk::WriteDescriptorSet::default(); 34];
        let mut written = 0;
        for index in 0..count {
            writes[written] = vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(buffer_bindings[index])
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .buffer_info(&buffers_info[index..index + 1]);
            written += 1;
        }
        for (index, binding) in sampled.image_bindings.iter().enumerate() {
            writes[written] = vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(*binding)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_info[index..index + 1]);
            written += 1;
        }
        for (index, binding) in sampled.sampler_bindings.iter().enumerate() {
            writes[written] = vk::WriteDescriptorSet::default()
                .dst_set(self.set)
                .dst_binding(*binding)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&image_info[16 + index..17 + index]);
            written += 1;
        }
        budget.work(written as u64)?;
        // SAFETY: Every descriptor is initialized for its matching bounded
        // owned resource. All owners outlive this pipeline and drained batch.
        unsafe {
            self.device
                .raw
                .update_descriptor_sets(&writes[..written], &[])
        };
        Ok(())
    }

    pub fn graphics(
        device: &'d Device,
        vertex: &CompiledShader,
        pixel: &CompiledShader,
        image: &'r Image<'d>,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        Self::graphics_target(
            device,
            [vertex, pixel],
            image,
            false,
            [None, None],
            SampledBindings::empty(),
            budget,
        )
    }

    /// Reuse the actual uploaded target instead of clearing diagnostic pixels.
    pub fn graphics_loaded(
        device: &'d Device,
        vertex: &CompiledShader,
        pixel: &CompiledShader,
        image: &'r Image<'d>,
        buffers: [Option<&'r Buffer<'d>>; 2],
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        Self::graphics_target(
            device,
            [vertex, pixel],
            image,
            true,
            buffers,
            SampledBindings::empty(),
            budget,
        )
    }

    pub fn graphics_loaded_sampled(
        device: &'d Device,
        vertex: &CompiledShader,
        pixel: &CompiledShader,
        image: &'r Image<'d>,
        buffers: [Option<&'r Buffer<'d>>; 2],
        sampled: SampledBindings<'d, 'r>,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        Self::graphics_target(
            device,
            [vertex, pixel],
            image,
            true,
            buffers,
            sampled,
            budget,
        )
    }

    fn graphics_target(
        device: &'d Device,
        shaders: [&CompiledShader; 2],
        image: &'r Image<'d>,
        load: bool,
        buffers: [Option<&'r Buffer<'d>>; 2],
        sampled: SampledBindings<'d, 'r>,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        let [vertex, pixel] = shaders;
        if !vertex.manifest().images().is_empty()
            || !vertex.manifest().samplers().is_empty()
            || sampled.images.len() > 16
            || sampled.samplers.len() > 16
            || sampled.images.len() != sampled.image_bindings.len()
            || sampled.samplers.len() != sampled.sampler_bindings.len()
            || sampled.images.len() != pixel.manifest().images().len()
            || sampled.samplers.len() != pixel.manifest().samplers().len()
            || pixel
                .manifest()
                .images()
                .iter()
                .map(|entry| entry.binding)
                .ne(sampled.image_bindings.iter().copied())
            || pixel
                .manifest()
                .samplers()
                .iter()
                .map(|entry| entry.binding)
                .ne(sampled.sampler_bindings.iter().copied())
        {
            return Err(VulkanError::InvalidCompilerContract(
                "graphics sampled descriptor resources",
            ));
        }
        let mut out = Self::empty(
            device,
            Resource::Graphics {
                image,
                buffers,
                sampled,
            },
        );
        let mut bindings = [vk::DescriptorSetLayoutBinding::default(); 34];
        let mut count = 0;
        for (index, shader) in [vertex, pixel].into_iter().enumerate() {
            let binding = shader.manifest().buffer_binding();
            if binding.is_some() != buffers[index].is_some()
                || shader.manifest().buffers().len() != usize::from(buffers[index].is_some())
            {
                return Err(VulkanError::InvalidCompilerContract(
                    "graphics descriptor resource",
                ));
            }
            if let Some(binding) = binding {
                let expected = if index == 0 { 0 } else { 57 };
                if binding != expected {
                    return Err(VulkanError::InvalidCompilerContract(
                        "graphics descriptor binding",
                    ));
                }
                bindings[count] = vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(if index == 0 {
                        vk::ShaderStageFlags::VERTEX
                    } else {
                        vk::ShaderStageFlags::FRAGMENT
                    });
                count += 1;
            }
        }
        let buffer_count = count;
        for (kind, numbers) in [
            (vk::DescriptorType::SAMPLED_IMAGE, sampled.image_bindings),
            (vk::DescriptorType::SAMPLER, sampled.sampler_bindings),
        ] {
            for binding in numbers {
                if bindings[..count].iter().any(|old| old.binding == *binding) {
                    return Err(VulkanError::InvalidCompilerContract(
                        "duplicate graphics descriptor binding",
                    ));
                }
                bindings[count] = vk::DescriptorSetLayoutBinding::default()
                    .binding(*binding)
                    .descriptor_type(kind)
                    .descriptor_count(1)
                    .stage_flags(vk::ShaderStageFlags::FRAGMENT);
                count += 1;
            }
        }
        out.layouts(
            &bindings[..count],
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            budget,
        )?;
        if count != 0 {
            out.graphics_descriptors(buffers, buffer_count, sampled, budget)?;
        }
        let attachments = [vk::AttachmentDescription::default()
            .format(vk::Format::R8G8B8A8_UNORM)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(if load {
                vk::AttachmentLoadOp::LOAD
            } else {
                vk::AttachmentLoadOp::CLEAR
            })
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(if load {
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
            } else {
                vk::ImageLayout::UNDEFINED
            })
            .final_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)];
        let colors = [vk::AttachmentReference::default()
            .attachment(0)
            .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)];
        let subpasses = [vk::SubpassDescription::default()
            .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
            .color_attachments(&colors)];
        let dependencies = [
            vk::SubpassDependency::default()
                .src_subpass(vk::SUBPASS_EXTERNAL)
                .dst_subpass(0)
                .src_stage_mask(if load {
                    vk::PipelineStageFlags::TRANSFER
                } else {
                    vk::PipelineStageFlags::TOP_OF_PIPE
                })
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(if load {
                    vk::AccessFlags::TRANSFER_WRITE
                } else {
                    vk::AccessFlags::empty()
                })
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                ),
            vk::SubpassDependency::default()
                .src_subpass(0)
                .dst_subpass(vk::SUBPASS_EXTERNAL)
                .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_stage_mask(vk::PipelineStageFlags::TRANSFER)
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ),
        ];
        let info = vk::RenderPassCreateInfo::default()
            .attachments(&attachments)
            .subpasses(&subpasses)
            .dependencies(&dependencies);
        budget.object()?;
        // SAFETY: Exact one-color subpass, no depth/input/resolve attachments;
        // explicit write->transfer dependency and final layout for real readback.
        out.render_pass = result(
            unsafe { device.raw.create_render_pass(&info, None) },
            "vkCreateRenderPass",
        )?;
        let views = [image.view];
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(out.render_pass)
            .attachments(&views)
            .width(image.width)
            .height(image.height)
            .layers(1);
        budget.object()?;
        // SAFETY: Compatible RGBA8 image view and checked extent, retained by
        // the resource borrow through framebuffer destruction.
        out.framebuffer = result(
            unsafe { device.raw.create_framebuffer(&info, None) },
            "vkCreateFramebuffer",
        )?;
        let vertex = Shader::new(device, vertex, budget)?;
        let pixel = Shader::new(device, pixel, budget)?;
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex.raw)
                .name(c"main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(pixel.raw)
                .name(c"main"),
        ];
        let input = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewports = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: image.width as f32,
            height: image.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissors = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: image.width,
                height: image.height,
            },
        }];
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let samples = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let color = [
            vk::PipelineColorBlendAttachmentState::default().color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B
                    | vk::ColorComponentFlags::A,
            ),
        ];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&color);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&samples)
            .color_blend_state(&blend)
            .layout(out.layout)
            .render_pass(out.render_pass)
            .subpass(0);
        budget.object()?;
        // SAFETY: Fixed triangle-list state, checked vertex/pixel manifests,
        // matching render pass and no optional features/dynamic state/derivative.
        // All borrowed create-info arrays live through the synchronous call.
        let code = unsafe {
            (device.raw.fp_v1_0().create_graphics_pipelines)(
                device.raw.handle(),
                vk::PipelineCache::null(),
                1,
                &info,
                std::ptr::null(),
                &mut out.raw,
            )
        };
        result(code.result(), "vkCreateGraphicsPipelines")?;
        Ok(out)
    }
}

impl Drop for Pipeline<'_, '_> {
    fn drop(&mut self) {
        // Keep explicit reads of retained resources: their borrows are the
        // authority for the descriptor/framebuffer handles, not their integers.
        match self.resource {
            Resource::Buffer(buffer) => {
                let _ = buffer.raw;
            }
            Resource::Graphics {
                image,
                buffers,
                sampled,
            } => {
                for image in sampled.images {
                    let _ = image.raw();
                }
                for sampler in sampled.samplers {
                    let _ = sampler.raw;
                }
                let _ = image.raw;
                for buffer in buffers.into_iter().flatten() {
                    let _ = buffer.raw;
                }
            }
        }
        // SAFETY: The batch completes/drains first, then unique children are
        // destroyed in dependency order. Null partial-construction handles are
        // skipped. The retained device and resource borrows are still alive.
        unsafe {
            if self.raw != vk::Pipeline::null() {
                self.device.raw.destroy_pipeline(self.raw, None);
            }
            if self.framebuffer != vk::Framebuffer::null() {
                self.device.raw.destroy_framebuffer(self.framebuffer, None);
            }
            if self.render_pass != vk::RenderPass::null() {
                self.device.raw.destroy_render_pass(self.render_pass, None);
            }
            if self.pool != vk::DescriptorPool::null() {
                self.device.raw.destroy_descriptor_pool(self.pool, None);
            }
            if self.layout != vk::PipelineLayout::null() {
                self.device.raw.destroy_pipeline_layout(self.layout, None);
            }
            if self.set_layout != vk::DescriptorSetLayout::null() {
                self.device
                    .raw
                    .destroy_descriptor_set_layout(self.set_layout, None);
            }
        }
    }
}
