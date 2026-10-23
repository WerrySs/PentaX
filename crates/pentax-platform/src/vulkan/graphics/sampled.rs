// SPDX-License-Identifier: GPL-2.0-only
//! Exact manifest descriptors and uploads of the draw's owned sampled resources.
use super::*;
use crate::vulkan::{
    pipeline::SampledBindings,
    resources::{SampledImage, SampledImageInfo, Sampler},
};
use pentax_gpu::image_descriptor::{AddressMode, BorderColor, Component, Filter, MipFilter};

pub(super) struct SampledResources<'d> {
    images: Vec<SampledImage<'d>>,
    uploads: Vec<Buffer<'d>>,
    samplers: Vec<Sampler<'d>>,
    image_bindings: [u32; 16],
    sampler_bindings: [u32; 16],
}
fn filter(value: Filter) -> vk::Filter {
    match value {
        Filter::Nearest => vk::Filter::NEAREST,
        Filter::Linear => vk::Filter::LINEAR,
    }
}
fn address(value: AddressMode) -> vk::SamplerAddressMode {
    match value {
        AddressMode::Repeat => vk::SamplerAddressMode::REPEAT,
        AddressMode::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
        AddressMode::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        AddressMode::ClampToBorder => vk::SamplerAddressMode::CLAMP_TO_BORDER,
    }
}
fn component(value: Component) -> vk::ComponentSwizzle {
    match value {
        Component::Zero => vk::ComponentSwizzle::ZERO,
        Component::One => vk::ComponentSwizzle::ONE,
        Component::Red => vk::ComponentSwizzle::R,
        Component::Green => vk::ComponentSwizzle::G,
        Component::Blue => vk::ComponentSwizzle::B,
        Component::Alpha => vk::ComponentSwizzle::A,
    }
}
pub(super) fn upload_length(prepared: &PreparedDraw) -> Result<usize, VulkanError> {
    prepared
        .pixel_images()
        .iter()
        .try_fold(0usize, |length, texture| {
            length
                .checked_add(texture.pixels().len())
                .ok_or(VulkanError::Limit("sampled image upload bytes"))
        })
}
impl<'d> SampledResources<'d> {
    pub fn new(
        device: &'d Device,
        prepared: &PreparedDraw,
        allow_counter_images: bool,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        let textures = prepared.pixel_images();
        let samplers = prepared.pixel_samplers();
        let limits = &device.properties.limits;
        if textures.len() > 16
            || samplers.len() > 16
            || textures.len() > limits.max_per_stage_descriptor_sampled_images as usize
            || textures.len() > limits.max_descriptor_set_sampled_images as usize
            || samplers.len() > limits.max_per_stage_descriptor_samplers as usize
            || samplers.len() > limits.max_descriptor_set_samplers as usize
            || textures.len()
                + samplers.len()
                + usize::from(prepared.pixel_buffer().is_some())
                + usize::from(prepared.pixel().manifest().lod_feedback_binding().is_some())
                > limits.max_per_stage_resources as usize
        {
            return Err(VulkanError::Unsupported("sampled descriptor device limits"));
        }
        let linear = samplers.iter().any(|sampler| {
            let s = sampler.descriptor();
            s.min_filter == Filter::Linear
                || s.mag_filter == Filter::Linear
                || s.mip_filter == MipFilter::Linear
        });
        let mut out = Self {
            images: Vec::new(),
            uploads: Vec::new(),
            samplers: Vec::new(),
            image_bindings: [0; 16],
            sampler_bindings: [0; 16],
        };
        let initial_metadata = textures.len()
            * (std::mem::size_of::<SampledImage<'_>>() + std::mem::size_of::<Buffer<'_>>())
            + samplers.len() * std::mem::size_of::<Sampler<'_>>();
        budget.host(initial_metadata as u64)?;
        out.images
            .try_reserve_exact(textures.len())
            .map_err(|_| VulkanError::Allocation)?;
        out.uploads
            .try_reserve_exact(textures.len())
            .map_err(|_| VulkanError::Allocation)?;
        out.samplers
            .try_reserve_exact(samplers.len())
            .map_err(|_| VulkanError::Allocation)?;
        budget.host(
            (out.images.capacity() * std::mem::size_of::<SampledImage<'_>>()
                + out.uploads.capacity() * std::mem::size_of::<Buffer<'_>>()
                + out.samplers.capacity() * std::mem::size_of::<Sampler<'_>>()
                - initial_metadata) as u64,
        )?;
        for (index, texture) in textures.iter().enumerate() {
            let d = texture.descriptor();
            if d.counter_id.is_some() && !allow_counter_images {
                return Err(VulkanError::Unsupported("uninitialized image LOD feedback"));
            }
            let [r, g, b, a] = d.swizzle.map(component);
            let image = SampledImage::rgba8(
                device,
                SampledImageInfo {
                    width: d.width,
                    height: d.height,
                    levels: u32::from(d.level_count),
                    base_level: u32::from(d.base_level),
                    last_level: u32::from(d.last_level),
                    components: vk::ComponentMapping { r, g, b, a },
                    linear_filter: linear,
                },
                budget,
            )?;
            let mut upload = Buffer::new(
                device,
                texture.pixels().len(),
                vk::BufferUsageFlags::TRANSFER_SRC,
                budget,
            )?;
            upload.write(texture.pixels(), budget)?;
            out.images.push(image);
            out.uploads.push(upload);
            out.image_bindings[index] = texture.binding();
        }
        for (index, sampler) in samplers.iter().enumerate() {
            let d = sampler.descriptor();
            let pair = prepared
                .pixel()
                .manifest()
                .samplers()
                .get(index)
                .filter(|pair| pair.binding == sampler.binding())
                .ok_or(VulkanError::InvalidCompilerContract(
                    "paired sampler binding",
                ))?;
            let image = prepared
                .pixel()
                .manifest()
                .images()
                .iter()
                .find(|image| image.user_sgpr == pair.image_user_sgpr)
                .ok_or(VulkanError::InvalidCompilerContract("paired sampler image"))?;
            let texture = textures
                .iter()
                .find(|texture| texture.binding() == image.binding)
                .ok_or(VulkanError::InvalidCompilerContract(
                    "paired sampler texture",
                ))?;
            let source = texture.descriptor();
            let minimum = source
                .min_lod_u4_8
                .saturating_sub(u16::from(source.base_level) * 256);
            if image.min_lod_u4_8 != minimum
                || minimum > u16::from(source.last_level - source.base_level) * 256
                || d.min_lod_u4_8 < minimum
                || d.max_lod_u4_8 < minimum
                || minimum != 0 && d.min_filter != d.mag_filter
            {
                return Err(VulkanError::InvalidCompilerContract(
                    "paired image minimum LOD",
                ));
            }
            // Bias interaction with explicit-LOD instructions is outside this
            // initial closed profile, rather than a silently dropped S# field.
            if d.lod_bias_s6_8 != 0 {
                return Err(VulkanError::Unsupported("sampled LOD bias"));
            }
            let border = match d.border {
                BorderColor::TransparentBlack => vk::BorderColor::FLOAT_TRANSPARENT_BLACK,
                BorderColor::OpaqueBlack => vk::BorderColor::FLOAT_OPAQUE_BLACK,
                BorderColor::OpaqueWhite => vk::BorderColor::FLOAT_OPAQUE_WHITE,
            };
            let info = vk::SamplerCreateInfo::default()
                .min_filter(filter(d.min_filter))
                .mag_filter(filter(d.mag_filter))
                .mipmap_mode(if d.mip_filter == MipFilter::Linear {
                    vk::SamplerMipmapMode::LINEAR
                } else {
                    vk::SamplerMipmapMode::NEAREST
                })
                .address_mode_u(address(d.address_modes[0]))
                .address_mode_v(address(d.address_modes[1]))
                .address_mode_w(address(d.address_modes[2]))
                .min_lod(f32::from(d.min_lod_u4_8) / 256.0)
                .max_lod(f32::from(d.max_lod_u4_8) / 256.0)
                .border_color(border);
            out.samplers.push(Sampler::new(device, &info, budget)?);
            out.sampler_bindings[index] = sampler.binding();
        }
        Ok(out)
    }
    pub fn bindings(&self) -> SampledBindings<'d, '_> {
        SampledBindings {
            images: &self.images,
            image_bindings: &self.image_bindings[..self.images.len()],
            samplers: &self.samplers,
            sampler_bindings: &self.sampler_bindings[..self.samplers.len()],
            lod_feedback: None,
        }
    }
    pub fn record(
        &self,
        device: &Device,
        batch: &Batch<'_>,
        prepared: &PreparedDraw,
        budget: &mut Budget,
    ) -> Result<(), VulkanError> {
        budget.command(
            (self.images.len() as u32)
                .checked_mul(4)
                .ok_or(VulkanError::Limit("sampled upload commands"))?,
        )?;
        for ((image, upload), texture) in self
            .images
            .iter()
            .zip(&self.uploads)
            .zip(prepared.pixel_images())
        {
            let before = [vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image.raw())
                .subresource_range(image.range())];
            let host = [vk::BufferMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                .buffer(upload.raw)
                .size(upload.length() as u64)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)];
            let mut regions = [vk::BufferImageCopy::default(); 16];
            for (index, mip) in texture.mips().iter().enumerate() {
                regions[index] = vk::BufferImageCopy::default()
                    .buffer_offset(mip.offset as u64)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: index as u32,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: mip.width,
                        height: mip.height,
                        depth: 1,
                    });
            }
            let after = [vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .image(image.raw())
                .subresource_range(image.range())
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)];
            // SAFETY: Every region comes from the checked owned mip snapshot;
            // all resources and flushed uploads outlive the batch's fence/drain.
            unsafe {
                device.raw.cmd_pipeline_barrier(
                    batch.raw,
                    vk::PipelineStageFlags::HOST,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &host,
                    &[],
                );
                device.raw.cmd_pipeline_barrier(
                    batch.raw,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &before,
                );
                device.raw.cmd_copy_buffer_to_image(
                    batch.raw,
                    upload.raw,
                    image.raw(),
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &regions[..texture.mips().len()],
                );
                device.raw.cmd_pipeline_barrier(
                    batch.raw,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &after,
                );
            }
        }
        Ok(())
    }
}
