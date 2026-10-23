// SPDX-License-Identifier: GPL-2.0-only
//! Owned sampled images and samplers, retained through the enclosing batch.
use super::{Budget, Device, Image, VulkanError, result};
use ash::vk;

pub(in crate::vulkan) struct SampledImageInfo {
    pub width: u32,
    pub height: u32,
    pub levels: u32,
    pub base_level: u32,
    pub last_level: u32,
    pub components: vk::ComponentMapping,
    pub linear_filter: bool,
}

pub(in crate::vulkan) struct SampledImage<'d> {
    image: Image<'d>,
    pub levels: u32,
}
impl<'d> SampledImage<'d> {
    pub fn raw(&self) -> vk::Image {
        self.image.raw
    }
    pub fn view(&self) -> vk::ImageView {
        self.image.view
    }
    pub fn range(&self) -> vk::ImageSubresourceRange {
        vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: self.levels,
            base_array_layer: 0,
            layer_count: 1,
        }
    }
    pub fn rgba8(
        device: &'d Device,
        description: SampledImageInfo,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        let SampledImageInfo {
            width,
            height,
            levels,
            base_level,
            last_level,
            components,
            linear_filter,
        } = description;
        if width == 0
            || height == 0
            || levels == 0
            || levels > 16
            || levels > 32 - width.max(height).leading_zeros()
            || width > device.properties.limits.max_image_dimension2_d
            || height > device.properties.limits.max_image_dimension2_d
            || base_level > last_level
            || last_level >= levels
            || !device.sampled_rgba_supported(linear_filter)
        {
            return Err(VulkanError::Unsupported(
                "sampled RGBA8 mip geometry/format",
            ));
        }
        budget.object()?;
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(levels)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: Fixed checked format/type, bounded mips, one layer/sample and
        // no guest pointer, external memory, sparse binding or alias flags.
        let raw = result(
            unsafe { device.raw.create_image(&info, None) },
            "vkCreateImage(sampled)",
        )?;
        let mut image = Image {
            device,
            raw,
            view: vk::ImageView::null(),
            width,
            height,
            memory: vk::DeviceMemory::null(),
        };
        // SAFETY: Fresh uniquely owned image on the retained device.
        let requirements = unsafe { device.raw.get_image_memory_requirements(raw) };
        let (index, _) = device.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        budget.device(requirements.size)?;
        budget.object()?;
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index);
        // SAFETY: Dedicated allocation uses the queried compatible type/size.
        image.memory = result(
            unsafe { device.raw.allocate_memory(&allocation, None) },
            "vkAllocateMemory(sampled)",
        )?;
        result(
            // SAFETY: Aligned offset zero in the matching dedicated allocation.
            unsafe { device.raw.bind_image_memory(raw, image.memory, 0) },
            "vkBindImageMemory(sampled)",
        )?;
        budget.object()?;
        let view = vk::ImageViewCreateInfo::default()
            .image(raw)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .components(components)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: base_level,
                level_count: last_level - base_level + 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: Checked view bounds are wholly inside this owned image.
        image.view = result(
            unsafe { device.raw.create_image_view(&view, None) },
            "vkCreateImageView(sampled)",
        )?;
        Ok(Self { image, levels })
    }
}

pub(in crate::vulkan) struct Sampler<'d> {
    device: &'d Device,
    pub raw: vk::Sampler,
}
impl<'d> Sampler<'d> {
    pub fn new(
        device: &'d Device,
        info: &vk::SamplerCreateInfo<'_>,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        if !info.p_next.is_null()
            || info.flags != vk::SamplerCreateFlags::empty()
            || info.anisotropy_enable != vk::FALSE
            || info.compare_enable != vk::FALSE
            || info.unnormalized_coordinates != vk::FALSE
            || !info.min_lod.is_finite()
            || !info.max_lod.is_finite()
            || !info.mip_lod_bias.is_finite()
            || info.min_lod < 0.0
            || info.max_lod < info.min_lod
            || info.mip_lod_bias.abs() > device.properties.limits.max_sampler_lod_bias
        {
            return Err(VulkanError::Unsupported("sampled image sampler profile"));
        }
        budget.object()?;
        // SAFETY: Internally built finite noncompare normalized sampler. Its
        // format/filter compatibility was checked with the actual sampled image.
        let raw = result(
            unsafe { device.raw.create_sampler(info, None) },
            "vkCreateSampler",
        )?;
        Ok(Self { device, raw })
    }
}
impl Drop for Sampler<'_> {
    fn drop(&mut self) {
        // SAFETY: The pipeline/batch borrows have retired before this owner.
        unsafe { self.device.raw.destroy_sampler(self.raw, None) };
    }
}
