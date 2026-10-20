// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: dedicated bounded allocations and RAII; see NOTICE.

use super::{
    Budget, VulkanError,
    device::{Device, result},
};
use ash::vk;
use std::ptr::NonNull;

pub(super) struct Buffer<'d> {
    device: &'d Device,
    pub raw: vk::Buffer,
    memory: vk::DeviceMemory,
    length: usize,
    coherent: bool,
}

impl<'d> Buffer<'d> {
    pub fn length(&self) -> usize {
        self.length
    }
    pub fn new(
        device: &'d Device,
        length: usize,
        usage: vk::BufferUsageFlags,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        if length == 0
            || length as u64 > u64::from(device.properties.limits.max_storage_buffer_range)
        {
            return Err(VulkanError::Unsupported("buffer size"));
        }
        budget.object()?;
        let info = vk::BufferCreateInfo::default()
            .size(length as u64)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: Finite checked size/usage and a live unique device. The owner
        // is initialized immediately so every later failure destroys the buffer.
        let raw = result(
            unsafe { device.raw.create_buffer(&info, None) },
            "vkCreateBuffer",
        )?;
        let mut out = Self {
            device,
            raw,
            memory: vk::DeviceMemory::null(),
            length,
            coherent: false,
        };
        // SAFETY: The buffer was just created on this device and is still owned.
        let requirements = unsafe { device.raw.get_buffer_memory_requirements(raw) };
        if requirements.size < length as u64 {
            return Err(VulkanError::Unsupported("buffer memory requirements"));
        }
        let (index, coherent) = device.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE,
        )?;
        budget.device(requirements.size)?;
        budget.object()?;
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index);
        // SAFETY: Exact requirements/type, dedicated allocation and no external
        // import or host pointer. No binding can outlive this memory owner.
        out.memory = result(
            unsafe { device.raw.allocate_memory(&allocation, None) },
            "vkAllocateMemory(buffer)",
        )?;
        out.coherent = coherent;
        // SAFETY: Fresh dedicated memory satisfying this buffer's requirements,
        // offset0 obeys alignment and the buffer is not in use or bound yet.
        let code = unsafe { device.raw.bind_buffer_memory(raw, out.memory, 0) };
        result(code, "vkBindBufferMemory")?;
        Ok(out)
    }

    fn mapped(&self) -> Result<Mapped<'_>, VulkanError> {
        // SAFETY: This private synchronous path never maps the same memory twice
        // or accesses it during GPU execution. The whole allocation is mapped.
        let pointer = result(
            unsafe {
                self.device.raw.map_memory(
                    self.memory,
                    0,
                    vk::WHOLE_SIZE,
                    vk::MemoryMapFlags::empty(),
                )
            },
            "vkMapMemory",
        )?;
        let Some(pointer) = NonNull::new(pointer.cast::<u8>()) else {
            // SAFETY: Successful map must be paired with unmap even when a
            // broken implementation returned a null pointer.
            unsafe { self.device.raw.unmap_memory(self.memory) };
            return Err(VulkanError::Unsupported("null mapped-memory pointer"));
        };
        Ok(Mapped {
            device: &self.device.raw,
            memory: self.memory,
            pointer,
        })
    }

    pub fn write(&mut self, bytes: &[u8], budget: &mut Budget) -> Result<(), VulkanError> {
        if bytes.len() != self.length {
            return Err(VulkanError::Unsupported("buffer upload length"));
        }
        budget.work(bytes.len() as u64)?;
        let mapped = self.mapped()?;
        // SAFETY: Length equals the buffer range within its dedicated mapped
        // allocation. Source is a separate Rust slice, not an exposed map alias.
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pointer.as_ptr(), bytes.len())
        };
        if !self.coherent {
            let ranges = [vk::MappedMemoryRange::default()
                .memory(self.memory)
                .offset(0)
                .size(vk::WHOLE_SIZE)];
            // SAFETY: Whole mapped allocation, atom-aligned offset0; WHOLE_SIZE
            // handles the noncoherent final atom without range rounding errors.
            let code = unsafe { self.device.raw.flush_mapped_memory_ranges(&ranges) };
            result(code, "vkFlushMappedMemoryRanges")?;
        }
        Ok(())
    }

    pub fn read_completed(&self, budget: &mut Budget) -> Result<Vec<u8>, VulkanError> {
        let mut bytes = budget.buffer(self.length)?;
        let mapped = self.mapped()?;
        if !self.coherent {
            let ranges = [vk::MappedMemoryRange::default()
                .memory(self.memory)
                .offset(0)
                .size(vk::WHOLE_SIZE)];
            // SAFETY: Caller waited for the producer fence and its host-read
            // barrier. This whole allocation is currently mapped at offset0.
            let code = unsafe { self.device.raw.invalidate_mapped_memory_ranges(&ranges) };
            result(code, "vkInvalidateMappedMemoryRanges")?;
        }
        // SAFETY: The same retained allocation remains mapped until Mapped drops;
        // fixed buffer length fits it, output is a distinct fully sized Vec.
        unsafe {
            std::ptr::copy_nonoverlapping(mapped.pointer.as_ptr(), bytes.as_mut_ptr(), bytes.len())
        };
        Ok(bytes)
    }
}
impl Drop for Buffer<'_> {
    fn drop(&mut self) {
        // SAFETY: Submission guards complete/drain before resources drop. This
        // owner uniquely destroys the buffer before its backing allocation.
        unsafe {
            self.device.raw.destroy_buffer(self.raw, None);
            if self.memory != vk::DeviceMemory::null() {
                self.device.raw.free_memory(self.memory, None);
            }
        }
    }
}
struct Mapped<'d> {
    device: &'d ash::Device,
    memory: vk::DeviceMemory,
    pointer: NonNull<u8>,
}
impl Drop for Mapped<'_> {
    fn drop(&mut self) {
        // SAFETY: Exactly one successful map, no pointer or borrow escapes it.
        unsafe { self.device.unmap_memory(self.memory) };
    }
}

pub(super) struct Image<'d> {
    device: &'d Device,
    pub raw: vk::Image,
    pub view: vk::ImageView,
    pub width: u32,
    pub height: u32,
    memory: vk::DeviceMemory,
}
impl<'d> Image<'d> {
    pub fn rgba(
        device: &'d Device,
        width: u32,
        height: u32,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        Self::rgba_target(device, width, height, false, budget)
    }

    pub fn rgba_uploaded(
        device: &'d Device,
        width: u32,
        height: u32,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        Self::rgba_target(device, width, height, true, budget)
    }

    fn rgba_target(
        device: &'d Device,
        width: u32,
        height: u32,
        upload: bool,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        if width == 0
            || height == 0
            || width > device.properties.limits.max_image_dimension2_d
            || height > device.properties.limits.max_image_dimension2_d
            || !device.rgba_supported()
            || (upload && !device.rgba_upload_supported())
        {
            return Err(VulkanError::Unsupported(
                "RGBA8 color attachment/transfer source",
            ));
        }
        width
            .checked_mul(height)
            .and_then(|v| v.checked_mul(4))
            .ok_or(VulkanError::Limit("image extent"))?;
        budget.object()?;
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_SRC
                    | if upload {
                        vk::ImageUsageFlags::TRANSFER_DST
                    } else {
                        vk::ImageUsageFlags::empty()
                    },
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: Checked core format/dimensions, one layer/mip/sample, no alias,
        // external ownership, sparse binding, host pointer or optional flags.
        let raw = result(
            unsafe { device.raw.create_image(&info, None) },
            "vkCreateImage",
        )?;
        let mut out = Self {
            device,
            raw,
            view: vk::ImageView::null(),
            width,
            height,
            memory: vk::DeviceMemory::null(),
        };
        // SAFETY: Fresh owned image on this live device.
        let requirements = unsafe { device.raw.get_image_memory_requirements(raw) };
        let (index, _) = device.memory_type(
            requirements.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        budget.device(requirements.size)?;
        budget.object()?;
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index);
        // SAFETY: Dedicated compatible memory matching queried requirements.
        out.memory = result(
            unsafe { device.raw.allocate_memory(&info, None) },
            "vkAllocateMemory(image)",
        )?;
        // SAFETY: Required aligned offset0 in fresh dedicated allocation.
        let code = unsafe { device.raw.bind_image_memory(raw, out.memory, 0) };
        result(code, "vkBindImageMemory")?;
        budget.object()?;
        let view = vk::ImageViewCreateInfo::default()
            .image(raw)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(color_range());
        // SAFETY: Compatible format/type and exact existing one-layer/mip range.
        out.view = result(
            unsafe { device.raw.create_image_view(&view, None) },
            "vkCreateImageView",
        )?;
        Ok(out)
    }
}
pub(super) fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
impl Drop for Image<'_> {
    fn drop(&mut self) {
        // SAFETY: Framebuffers and completed/drained submissions drop first.
        // Each successfully created handle is destroyed once in dependency order.
        unsafe {
            if self.view != vk::ImageView::null() {
                self.device.raw.destroy_image_view(self.view, None);
            }
            self.device.raw.destroy_image(self.raw, None);
            if self.memory != vk::DeviceMemory::null() {
                self.device.raw.free_memory(self.memory, None);
            }
        }
    }
}
