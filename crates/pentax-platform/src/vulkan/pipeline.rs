// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: manifest-bound descriptors and owned pipelines; see NOTICE.

use super::{
    Budget, VulkanError,
    device::{Device, result},
    resources::{Buffer, Image},
};
use ash::vk;
use pentax_shader_compiler::CompiledShader;

enum Resource<'d, 'r> {
    Buffer(&'r Buffer<'d>),
    Image(&'r Image<'d>),
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

    pub fn graphics(
        device: &'d Device,
        vertex: &CompiledShader,
        pixel: &CompiledShader,
        image: &'r Image<'d>,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        let mut out = Self::empty(device, Resource::Image(image));
        out.layouts(
            &[],
            vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
            budget,
        )?;
        let attachments = [vk::AttachmentDescription::default()
            .format(vk::Format::R8G8B8A8_UNORM)
            .samples(vk::SampleCountFlags::TYPE_1)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
            .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
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
                .src_stage_mask(vk::PipelineStageFlags::TOP_OF_PIPE)
                .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
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
        // SAFETY: Compatible4x4 RGBA8 image view, retained by resource borrow.
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
            Resource::Image(image) => {
                let _ = image.raw;
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
