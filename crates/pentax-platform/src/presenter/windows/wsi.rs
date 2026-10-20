// SPDX-License-Identifier: GPL-2.0-only

//! Owned FIFO transfer-only WSI. No shader, guest pointer, external HWND or
//! caller-selected driver/library path enters this private implementation.

use super::super::{
    CompletionRequirement, Config, Error, Limits, MAX_GPU, WindowChange, cpu_admit, frame_length,
    image_admit,
};
use ash::{Entry, Instance, vk};
use std::{
    ffi::{CStr, OsString},
    os::windows::ffi::OsStringExt,
    path::PathBuf,
    ptr::NonNull,
};
use windows_sys::Win32::{
    Foundation::{HINSTANCE, HWND},
    System::SystemInformation::GetSystemDirectoryW,
};

fn result<T>(value: Result<T, vk::Result>, operation: &'static str) -> Result<T, Error> {
    value.map_err(|code| Error::Vulkan(operation, code.as_raw()))
}

fn at_requirement(error: Error, requirement: CompletionRequirement) -> Error {
    if error == Error::UnavailableCompletion {
        Error::MissingCompletionRequirement(requirement)
    } else {
        error
    }
}

pub(super) struct Wsi {
    // Loader is kept alive until every object/function pointer is destroyed.
    _entry: Entry,
    instance: Instance,
    surface_api: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    physical: vk::PhysicalDevice,
    device: Option<ash::Device>,
    swapchain_api: Option<ash::khr::swapchain::Device>,
    wait_api: Option<ash::khr::present_wait::Device>,
    swapchain: vk::SwapchainKHR,
    images: [vk::Image; 8],
    requested_image_count: u32,
    image_count: usize,
    queue: vk::Queue,
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    acquire: vk::Semaphore,
    rendered: vk::Semaphore,
    fence: vk::Fence,
    present_fence: vk::Fence,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: Option<NonNull<u8>>,
    memory_bytes: u64,
    frame_bytes: usize,
    extent: vk::Extent2D,
    unresolved: bool,
    pending_id: u64,
}

impl Wsi {
    pub fn open(
        hwnd: HWND,
        module: HINSTANCE,
        config: Config,
        limits: Limits,
        held: usize,
    ) -> Result<Self, Error> {
        let frame_bytes = frame_length(config.width, config.height, limits)?;
        image_admit(frame_bytes, limits)?;
        cpu_admit(held, frame_bytes as u64, limits)?;
        let mut directory = [0_u16; 512];
        // SAFETY: Fixed writable UTF-16 output and its exact length.
        let length =
            unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
        if length == 0 || length >= directory.len() {
            return Err(Error::MissingCompletionRequirement(
                CompletionRequirement::SystemLoader,
            ));
        }
        let mut path = PathBuf::from(OsString::from_wide(&directory[..length]));
        if !path.is_absolute() {
            return Err(Error::MissingCompletionRequirement(
                CompletionRequirement::SystemLoader,
            ));
        }
        path.push("vulkan-1.dll");
        // SAFETY: Only the absolute Windows system loader is loaded. Entry
        // remains alive until after instance/device/surface destruction.
        let entry = unsafe { Entry::load_from(path) }.map_err(|_| {
            Error::MissingCompletionRequirement(CompletionRequirement::SystemLoader)
        })?;
        // SAFETY: Read-only loader query, no application-provided pointer.
        let version = result(
            unsafe { entry.try_enumerate_instance_version() },
            "vkEnumerateInstanceVersion",
        )?
        .unwrap_or(vk::API_VERSION_1_0);
        if version < vk::API_VERSION_1_1 {
            return Err(Error::MissingCompletionRequirement(
                CompletionRequirement::Vulkan11,
            ));
        }
        let app = vk::ApplicationInfo::default()
            .application_name(c"PentaX")
            .engine_name(c"PentaX")
            .api_version(vk::API_VERSION_1_1);
        let extensions = [
            ash::khr::surface::NAME.as_ptr(),
            ash::khr::win32_surface::NAME.as_ptr(),
            ash::khr::get_surface_capabilities2::NAME.as_ptr(),
            ash::ext::surface_maintenance1::NAME.as_ptr(),
        ];
        let info = vk::InstanceCreateInfo::default()
            .application_info(&app)
            .enabled_extension_names(&extensions);
        // SAFETY: Stack-backed create info, fixed extension names, no callbacks
        // or custom allocator. A successful instance is immediately owned.
        let instance = result(
            unsafe { entry.create_instance(&info, None) },
            "vkCreateInstance",
        )?;
        let surface_api = ash::khr::surface::Instance::new(&entry, &instance);
        let win32 = ash::khr::win32_surface::Instance::new(&entry, &instance);
        let mut owner = Self {
            _entry: entry,
            instance,
            surface_api,
            surface: vk::SurfaceKHR::null(),
            physical: vk::PhysicalDevice::null(),
            device: None,
            swapchain_api: None,
            wait_api: None,
            swapchain: vk::SwapchainKHR::null(),
            images: [vk::Image::null(); 8],
            requested_image_count: 0,
            image_count: 0,
            queue: vk::Queue::null(),
            pool: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            acquire: vk::Semaphore::null(),
            rendered: vk::Semaphore::null(),
            fence: vk::Fence::null(),
            present_fence: vk::Fence::null(),
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            mapped: None,
            memory_bytes: 0,
            frame_bytes,
            extent: vk::Extent2D {
                width: config.width,
                height: config.height,
            },
            unresolved: false,
            pending_id: 0,
        };
        let info = vk::Win32SurfaceCreateInfoKHR::default()
            .hinstance(module as isize)
            .hwnd(hwnd as isize);
        // SAFETY: Both handles are this backend's live owned Win32 window/module.
        // No second surface is attached to eframe's launcher window.
        owner.surface = result(
            unsafe { win32.create_win32_surface(&info, None) },
            "vkCreateWin32SurfaceKHR",
        )?;
        let (physical, family) = owner
            .select_device()
            .map_err(|error| at_requirement(error, CompletionRequirement::PresentationQueue))?;
        owner.physical = physical;
        let priorities = [1.0];
        let queue_info = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let names = [
            ash::khr::swapchain::NAME.as_ptr(),
            ash::khr::present_id::NAME.as_ptr(),
            ash::khr::present_wait::NAME.as_ptr(),
            ash::ext::swapchain_maintenance1::NAME.as_ptr(),
        ];
        let mut present_id = vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
        let mut present_wait =
            vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(true);
        let mut maintenance = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default()
            .swapchain_maintenance1(true);
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_info)
            .enabled_extension_names(&names)
            .push_next(&mut present_id)
            .push_next(&mut present_wait)
            .push_next(&mut maintenance);
        // SAFETY: select_device verified both advertised extensions and feature
        // bits plus this exact queue family. All arrays/chains remain live.
        let device = result(
            unsafe { owner.instance.create_device(physical, &info, None) },
            "vkCreateDevice",
        )?;
        // SAFETY: Queue 0 was requested in this exact selected family.
        owner.queue = unsafe { device.get_device_queue(family, 0) };
        owner.swapchain_api = Some(ash::khr::swapchain::Device::new(&owner.instance, &device));
        owner.wait_api = Some(ash::khr::present_wait::Device::new(
            &owner.instance,
            &device,
        ));
        owner.device = Some(device);
        owner
            .create_swapchain(physical, limits)
            .map_err(|error| at_requirement(error, CompletionRequirement::SurfaceProfile))?;
        owner
            .create_upload(physical, limits, held)
            .map_err(|error| at_requirement(error, CompletionRequirement::UploadMemory))?;
        owner.create_commands(family)?;
        Ok(owner)
    }

    fn select_device(&self) -> Result<(vk::PhysicalDevice, u32), Error> {
        let mut unavailable = CompletionRequirement::Vulkan11;
        let mut count = 0;
        result(
            // SAFETY: Valid instance and scalar count output; no allocation.
            unsafe {
                (self.instance.fp_v1_0().enumerate_physical_devices)(
                    self.instance.handle(),
                    &mut count,
                    std::ptr::null_mut(),
                )
            }
            .result(),
            "vkEnumeratePhysicalDevices(count)",
        )?;
        if count == 0 || count > 32 {
            return Err(Error::UnavailableCompletion);
        }
        let mut physicals = [vk::PhysicalDevice::null(); 32];
        result(
            // SAFETY: The bounded count fits the fixed array. INCOMPLETE is refused.
            unsafe {
                (self.instance.fp_v1_0().enumerate_physical_devices)(
                    self.instance.handle(),
                    &mut count,
                    physicals.as_mut_ptr(),
                )
            }
            .result(),
            "vkEnumeratePhysicalDevices",
        )?;
        if count > 32 {
            return Err(Error::UnavailableCompletion);
        }
        for physical in physicals[..count as usize].iter().copied() {
            // SAFETY: Physical handle came from this live instance.
            let properties = unsafe { self.instance.get_physical_device_properties(physical) };
            if properties.api_version < vk::API_VERSION_1_1
                || properties.limits.max_image_dimension2_d
                    < self.extent.width.max(self.extent.height)
            {
                continue;
            }
            let mut extension_count = 0;
            result(
                // SAFETY: Count-only query for this physical device, no layer name.
                unsafe {
                    (self
                        .instance
                        .fp_v1_0()
                        .enumerate_device_extension_properties)(
                        physical,
                        std::ptr::null(),
                        &mut extension_count,
                        std::ptr::null_mut(),
                    )
                }
                .result(),
                "vkEnumerateDeviceExtensionProperties(count)",
            )?;
            if extension_count > 512 {
                continue;
            }
            let mut extensions = [vk::ExtensionProperties::default(); 512];
            result(
                // SAFETY: Count fits the fixed array; INCOMPLETE refuses races.
                unsafe {
                    (self
                        .instance
                        .fp_v1_0()
                        .enumerate_device_extension_properties)(
                        physical,
                        std::ptr::null(),
                        &mut extension_count,
                        extensions.as_mut_ptr(),
                    )
                }
                .result(),
                "vkEnumerateDeviceExtensionProperties",
            )?;
            if extension_count > 512 {
                return Err(Error::UnavailableCompletion);
            }
            let has = |name: &CStr| {
                extensions[..extension_count as usize]
                    .iter()
                    .any(|extension| {
                        // Vulkan extensionName is a fixed NUL-terminated array. Bound
                        // the search even for a malformed driver response.
                        let bytes = &extension.extension_name;
                        let Some(end) = bytes.iter().position(|value| *value == 0) else {
                            return false;
                        };
                        end == name.to_bytes().len()
                            && bytes[..end]
                                .iter()
                                .zip(name.to_bytes())
                                .all(|(a, b)| *a as u8 == *b)
                    })
            };
            let missing = [
                (ash::khr::swapchain::NAME, CompletionRequirement::Swapchain),
                (ash::khr::present_id::NAME, CompletionRequirement::PresentId),
                (
                    ash::khr::present_wait::NAME,
                    CompletionRequirement::PresentWait,
                ),
                (
                    ash::ext::swapchain_maintenance1::NAME,
                    CompletionRequirement::PresentationFence,
                ),
            ]
            .into_iter()
            .find(|(name, _)| !has(name));
            if let Some((_, requirement)) = missing {
                unavailable = unavailable.max(requirement);
                continue;
            }
            unavailable = unavailable.max(CompletionRequirement::FeatureBits);
            let mut id = vk::PhysicalDevicePresentIdFeaturesKHR::default();
            let mut wait = vk::PhysicalDevicePresentWaitFeaturesKHR::default();
            let mut maintenance = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default();
            let mut features = vk::PhysicalDeviceFeatures2::default()
                .push_next(&mut id)
                .push_next(&mut wait)
                .push_next(&mut maintenance);
            // SAFETY: Query only; both extension structures are stack-owned and
            // advertised by this physical device, with a core1.1 query entry.
            unsafe {
                self.instance
                    .get_physical_device_features2(physical, &mut features);
            }
            if id.present_id == 0
                || wait.present_wait == 0
                || maintenance.swapchain_maintenance1 == 0
            {
                continue;
            }
            unavailable = CompletionRequirement::PresentationQueue;
            let mut queue_count = 0;
            // SAFETY: Count-only query of the enumerated physical device.
            unsafe {
                (self
                    .instance
                    .fp_v1_0()
                    .get_physical_device_queue_family_properties)(
                    physical,
                    &mut queue_count,
                    std::ptr::null_mut(),
                );
            }
            if queue_count == 0 || queue_count > 64 {
                continue;
            }
            let mut queues = [vk::QueueFamilyProperties::default(); 64];
            // SAFETY: Fixed-array capacity was checked before the output call.
            unsafe {
                (self
                    .instance
                    .fp_v1_0()
                    .get_physical_device_queue_family_properties)(
                    physical,
                    &mut queue_count,
                    queues.as_mut_ptr(),
                );
            }
            if queue_count > 64 {
                return Err(Error::UnavailableCompletion);
            }
            for (index, queue) in queues[..queue_count as usize].iter().enumerate() {
                if queue.queue_count == 0 || !queue.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                    continue;
                }
                if result(
                    // SAFETY: Actual surface, queried physical and bounded family.
                    unsafe {
                        self.surface_api.get_physical_device_surface_support(
                            physical,
                            index as u32,
                            self.surface,
                        )
                    },
                    "vkGetPhysicalDeviceSurfaceSupportKHR",
                )? {
                    return Ok((physical, index as u32));
                }
            }
        }
        Err(Error::MissingCompletionRequirement(unavailable))
    }

    fn create_swapchain(
        &mut self,
        physical: vk::PhysicalDevice,
        limits: Limits,
    ) -> Result<(), Error> {
        // SAFETY: This surface/physical pair was admitted for presentation.
        let caps = result(
            unsafe {
                self.surface_api
                    .get_physical_device_surface_capabilities(physical, self.surface)
            },
            "vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
        )?;
        let extent = self.extent;
        let count = caps.min_image_count.checked_add(1).ok_or(Error::Limit)?;
        let count = if caps.max_image_count != 0 {
            count.min(caps.max_image_count)
        } else {
            count
        };
        surface_contract(caps, extent, count).map_err(|change| {
            Error::MissingCompletionRequirement(match change {
                WindowChange::SurfaceImageCount => CompletionRequirement::SurfaceImages,
                WindowChange::SurfaceExtent | WindowChange::SurfaceLimits => {
                    CompletionRequirement::SurfaceExtent
                }
                _ => CompletionRequirement::SurfaceUsage,
            })
        })?;
        let mut formats = [vk::SurfaceFormatKHR::default(); 32];
        let mut format_count = 0;
        result(
            // SAFETY: Count-only surface-format query.
            unsafe {
                (self
                    .surface_api
                    .fp()
                    .get_physical_device_surface_formats_khr)(
                    physical,
                    self.surface,
                    &mut format_count,
                    std::ptr::null_mut(),
                )
            }
            .result(),
            "vkGetPhysicalDeviceSurfaceFormatsKHR(count)",
        )?;
        if format_count == 0 || format_count > 32 {
            return Err(Error::MissingCompletionRequirement(
                CompletionRequirement::SurfaceFormats,
            ));
        }
        result(
            // SAFETY: Bounded fixed output, INCOMPLETE is rejected.
            unsafe {
                (self
                    .surface_api
                    .fp()
                    .get_physical_device_surface_formats_khr)(
                    physical,
                    self.surface,
                    &mut format_count,
                    formats.as_mut_ptr(),
                )
            }
            .result(),
            "vkGetPhysicalDeviceSurfaceFormatsKHR",
        )?;
        if format_count > 32 {
            return Err(Error::MissingCompletionRequirement(
                CompletionRequirement::SurfaceFormats,
            ));
        }
        let format = vk::Format::B8G8R8A8_UNORM;
        if !formats[..format_count as usize].iter().any(|f| {
            f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                && (f.format == format || format_count == 1 && f.format == vk::Format::UNDEFINED)
        }) {
            return Err(Error::MissingCompletionRequirement(
                CompletionRequirement::SurfaceFormats,
            ));
        }
        // FIFO is the required Vulkan baseline. No mailbox/immediate mode can
        // replace this owner's one inflight frame before its explicit wait.
        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(count)
            .image_format(format)
            .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::TRANSFER_DST)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true);
        let api = self
            .swapchain_api
            .as_ref()
            .ok_or(Error::UnavailableCompletion)?;
        // SAFETY: Admitted exact surface capabilities and fixed FIFO/format.
        self.swapchain = result(
            unsafe { api.create_swapchain(&info, None) },
            "vkCreateSwapchainKHR",
        )?;
        self.requested_image_count = count;
        let mut count = 0;
        result(
            // SAFETY: Count-only query of this live swapchain.
            unsafe {
                (api.fp().get_swapchain_images_khr)(
                    api.device(),
                    self.swapchain,
                    &mut count,
                    std::ptr::null_mut(),
                )
            }
            .result(),
            "vkGetSwapchainImagesKHR(count)",
        )?;
        returned_image_count(count, self.requested_image_count)?;
        result(
            // SAFETY: Fixed image-handle storage bounds checked; INCOMPLETE refused.
            unsafe {
                (api.fp().get_swapchain_images_khr)(
                    api.device(),
                    self.swapchain,
                    &mut count,
                    self.images.as_mut_ptr(),
                )
            }
            .result(),
            "vkGetSwapchainImagesKHR",
        )?;
        self.image_count = returned_image_count(count, self.requested_image_count)?;
        let bytes = (self.frame_bytes as u64)
            .checked_mul(u64::from(count))
            .ok_or(Error::Limit)?;
        if bytes > limits.max_device_bytes.min(MAX_GPU) {
            return Err(Error::Limit);
        }
        Ok(())
    }

    fn create_upload(
        &mut self,
        physical: vk::PhysicalDevice,
        limits: Limits,
        held: usize,
    ) -> Result<(), Error> {
        let device = self.device.as_ref().ok_or(Error::UnavailableCompletion)?;
        let info = vk::BufferCreateInfo::default()
            .size(self.frame_bytes as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: Checked finite length, no external memory or custom allocator.
        self.buffer = result(
            unsafe { device.create_buffer(&info, None) },
            "vkCreateBuffer",
        )?;
        // SAFETY: Newly owned buffer has not been bound or submitted.
        let requirements = unsafe { device.get_buffer_memory_requirements(self.buffer) };
        if requirements.size < self.frame_bytes as u64 {
            return Err(Error::Limit);
        }
        let total = (self.frame_bytes as u64)
            .checked_mul(self.image_count as u64)
            .and_then(|n| n.checked_add(requirements.size))
            .ok_or(Error::Limit)?;
        if total > limits.max_device_bytes.min(MAX_GPU) {
            return Err(Error::Limit);
        }
        cpu_admit(held, requirements.size, limits)?;
        // SAFETY: Read-only actual memory-property query for selected physical.
        let properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(physical)
        };
        if properties.memory_type_count > 32 {
            return Err(Error::UnavailableCompletion);
        }
        let flags = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let index = (0..properties.memory_type_count)
            .find(|index| {
                requirements.memory_type_bits & (1_u32 << index) != 0
                    && properties.memory_types[*index as usize]
                        .property_flags
                        .contains(flags)
            })
            .ok_or(Error::UnavailableCompletion)?;
        let allocation = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index);
        // SAFETY: Requirements-derived type/alignment/size, dedicated allocation.
        self.memory = result(
            unsafe { device.allocate_memory(&allocation, None) },
            "vkAllocateMemory",
        )?;
        self.memory_bytes = requirements.size;
        result(
            // SAFETY: Fresh compatible memory, offset0, buffer still unused.
            unsafe { device.bind_buffer_memory(self.buffer, self.memory, 0) },
            "vkBindBufferMemory",
        )?;
        // SAFETY: Whole owned host-visible coherent allocation, not GPU-active.
        let mapped = result(
            unsafe {
                device.map_memory(self.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            },
            "vkMapMemory",
        )?;
        if let Some(pointer) = NonNull::new(mapped.cast::<u8>()) {
            self.mapped = Some(pointer);
        } else {
            // SAFETY: Successful mapping must be unmapped even for bad NULL.
            unsafe {
                device.unmap_memory(self.memory);
            }
            return Err(Error::UnavailableCompletion);
        }
        Ok(())
    }

    fn create_commands(&mut self, family: u32) -> Result<(), Error> {
        let device = self.device.as_ref().ok_or(Error::UnavailableCompletion)?;
        let info = vk::CommandPoolCreateInfo::default().queue_family_index(family);
        // SAFETY: Actual admitted family; no command buffers are in use.
        self.pool = result(
            unsafe { device.create_command_pool(&info, None) },
            "vkCreateCommandPool",
        )?;
        let info = vk::CommandBufferAllocateInfo::default()
            .command_pool(self.pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // Call the raw entry point to avoid even a one-element infallible Vec.
        result(
            // SAFETY: Exact single command output and owned compatible pool.
            unsafe {
                (device.fp_v1_0().allocate_command_buffers)(
                    device.handle(),
                    &info,
                    &mut self.command,
                )
            }
            .result(),
            "vkAllocateCommandBuffers",
        )?;
        let info = vk::SemaphoreCreateInfo::default();
        // SAFETY: Fresh binary semaphore owned exclusively by this queue path.
        self.acquire = result(
            unsafe { device.create_semaphore(&info, None) },
            "vkCreateSemaphore(acquire)",
        )?;
        // SAFETY: Separate fresh binary semaphore for presentation consumption.
        self.rendered = result(
            unsafe { device.create_semaphore(&info, None) },
            "vkCreateSemaphore(rendered)",
        )?;
        // SAFETY: Fresh unsignaled fence, never shared with another submission.
        self.fence = result(
            unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) },
            "vkCreateFence",
        )?;
        // SAFETY: A separate unsignaled fence belongs exclusively to the single
        // present operation. It proves resource release, not display completion.
        self.present_fence = result(
            unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) },
            "vkCreateFence(present)",
        )?;
        Ok(())
    }

    pub fn staging_bytes(&self) -> u64 {
        self.memory_bytes
    }
    pub fn safe_to_destroy(&self) -> bool {
        !self.unresolved
    }

    pub fn check_surface(&self) -> Result<(), Error> {
        // SAFETY: Live physical/surface belonging to the same instance. This
        // read-only query does not acquire/present an image or sample a clock.
        let caps = result(
            unsafe {
                self.surface_api
                    .get_physical_device_surface_capabilities(self.physical, self.surface)
            },
            "vkGetPhysicalDeviceSurfaceCapabilitiesKHR(current)",
        )?;
        surface_contract(caps, self.extent, self.requested_image_count)
            .map_err(Error::WindowChanged)
    }

    pub fn submit(&mut self, id: u64, pixels: &[u8]) -> Result<(), Error> {
        if self.unresolved {
            return Err(Error::Busy);
        }
        if id == 0 || id <= self.pending_id {
            return Err(Error::InvalidSequence);
        }
        if pixels.len() != self.frame_bytes {
            return Err(Error::InvalidFrame);
        }
        let device = self.device.as_ref().ok_or(Error::UnavailableCompletion)?;
        let mapped = self.mapped.ok_or(Error::UnavailableCompletion)?;
        // SAFETY: Full requirements-backed coherent mapping, frame length
        // checked, and no prior submission/presentation is unresolved. The
        // source is a normal Rust slice; no guest pointer is dereferenced here.
        let target = unsafe { std::slice::from_raw_parts_mut(mapped.as_ptr(), self.frame_bytes) };
        for (source, dest) in pixels.chunks_exact(4).zip(target.chunks_exact_mut(4)) {
            dest.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
        }
        result(
            // SAFETY: Prior command buffer is no longer pending; reset has no GPU
            // work and the fresh recording contains only our checked full copy.
            unsafe { device.reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty()) },
            "vkResetCommandPool",
        )?;
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        result(
            // SAFETY: Owned reset primary command buffer, one exclusive thread.
            unsafe { device.begin_command_buffer(self.command, &begin) },
            "vkBeginCommandBuffer",
        )?;
        let api = self
            .swapchain_api
            .as_ref()
            .ok_or(Error::UnavailableCompletion)?;
        // SAFETY: Live nonretired swapchain, unused acquire semaphore, no fence,
        // timeout0. NOT_READY/TIMEOUT do not admit a frame or consume sequence.
        let acquired =
            unsafe { api.acquire_next_image(self.swapchain, 0, self.acquire, vk::Fence::null()) };
        let (index, suboptimal) = match acquired {
            Ok(value) => value,
            Err(vk::Result::NOT_READY | vk::Result::TIMEOUT) => return Err(Error::WouldBlock),
            Err(code) => return Err(Error::Vulkan("vkAcquireNextImageKHR", code.as_raw())),
        };
        // From acquisition onward any refusal cancels this backend. No unsafe
        // semaphore/image reuse and no unbounded device_wait_idle cleanup.
        self.unresolved = true;
        if suboptimal || index as usize >= self.image_count {
            return Err(Error::NeedsReopen);
        }
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let image = self.images[index as usize];
        let to_copy = [vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(range)];
        let region = [vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D {
                width: self.extent.width,
                height: self.extent.height,
                depth: 1,
            })];
        let to_present = [vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(range)];
        // SAFETY: Complete acquired image overwrite from a bounded coherent
        // staging buffer; no caller-selected regions/layouts or resource handles.
        // Acquire semaphore orders first transfer; signal orders presentation.
        unsafe {
            device.cmd_pipeline_barrier(
                self.command,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_copy,
            );
            device.cmd_copy_buffer_to_image(
                self.command,
                self.buffer,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &region,
            );
            device.cmd_pipeline_barrier(
                self.command,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &to_present,
            );
        }
        result(
            // SAFETY: Complete valid recording and owned nonpending fence.
            unsafe { device.end_command_buffer(self.command) },
            "vkEndCommandBuffer",
        )?;
        result(
            // SAFETY: No prior submission is unresolved; fence may be reset safely.
            unsafe { device.reset_fences(&[self.fence, self.present_fence]) },
            "vkResetFences",
        )?;
        let waits = [self.acquire];
        let stages = [vk::PipelineStageFlags::TRANSFER];
        let commands = [self.command];
        let signals = [self.rendered];
        let submit = [vk::SubmitInfo::default()
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages)
            .command_buffers(&commands)
            .signal_semaphores(&signals)];
        result(
            // SAFETY: All resources owned/live, single queue externally synchronized,
            // coherent upload finished before submit; arrays live for the call.
            unsafe { device.queue_submit(self.queue, &submit, self.fence) },
            "vkQueueSubmit",
        )?;
        let swapchains = [self.swapchain];
        let indices = [index];
        let ids = [id];
        let fences = [self.present_fence];
        let mut present_id = vk::PresentIdKHR::default().present_ids(&ids);
        let mut present_fence = vk::SwapchainPresentFenceInfoEXT::default().fences(&fences);
        let info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signals)
            .swapchains(&swapchains)
            .image_indices(&indices)
            .push_next(&mut present_id)
            .push_next(&mut present_fence);
        if result(
            // SAFETY: Enabled present-id/wait profile, id strictly increases and
            // arrays match one live swapchain. This is acceptance, not completion.
            unsafe { api.queue_present(self.queue, &info) },
            "vkQueuePresentKHR",
        )? {
            return Err(Error::NeedsReopen);
        }
        self.pending_id = id;
        Ok(())
    }

    pub fn poll(&mut self, id: u64) -> Result<bool, Error> {
        if id == 0 || id != self.pending_id {
            return Err(Error::InvalidSequence);
        }
        // A bounded message-pump refusal after the successful wait may delay
        // the public receipt. Retain that attested result for its exact id.
        if !self.unresolved {
            return Ok(true);
        }
        let device = self.device.as_ref().ok_or(Error::UnavailableCompletion)?;
        if !result(
            // SAFETY: Live owned fence from the exact current submission; polling
            // status does not wait or authorize presentation on its own.
            unsafe { device.get_fence_status(self.fence) },
            "vkGetFenceStatus",
        )? {
            return Ok(false);
        }
        if !result(
            // SAFETY: This exact enabled-maintenance present fence belongs to the
            // last queue-present call. Khronos specifies that signaling authorizes
            // semaphore/swapchain resource release; a submit fence alone does not.
            unsafe { device.get_fence_status(self.present_fence) },
            "vkGetFenceStatus(present)",
        )? {
            return Ok(false);
        }
        let api = self.wait_api.as_ref().ok_or(Error::UnavailableCompletion)?;
        // SAFETY: Exact submitted nonzero id on this nonretired FIFO swapchain;
        // presentWait feature enabled, one in flight, timeout0 only. Caller also
        // validates live window/profile generation before AND after success.
        match unsafe { api.wait_for_present(self.swapchain, id, 0) } {
            Ok(()) => {
                self.unresolved = false;
                Ok(true)
            }
            Err(vk::Result::TIMEOUT) => Ok(false),
            Err(code) => Err(Error::Vulkan("vkWaitForPresentKHR", code.as_raw())),
        }
    }
}

/// Revalidate the immutable selected swapchain contract, not every advertised
/// option. Unused flags may change; a required option may never be recaptured
/// or silently replaced. Window/display identity and actual completion remain
/// independently checked by the owner around this capability query.
///
/// Vulkan VkSwapchainCreateInfoKHR VUIDs 01272/02839 (requested count), 07781
/// (extent bounds), 01275 (layers), 01427 (usage), 01279 (transform), 01280
/// (alpha): https://docs.vulkan.org/refpages/latest/refpages/source/VkSwapchainCreateInfoKHR.html
fn surface_contract(
    caps: vk::SurfaceCapabilitiesKHR,
    extent: vk::Extent2D,
    requested_image_count: u32,
) -> Result<(), WindowChange> {
    if requested_image_count == 0
        || requested_image_count > 8
        || requested_image_count < caps.min_image_count
        || caps.max_image_count != 0 && requested_image_count > caps.max_image_count
    {
        return Err(WindowChange::SurfaceImageCount);
    }
    let undefined = vk::Extent2D {
        width: u32::MAX,
        height: u32::MAX,
    };
    if extent.width == 0
        || extent.height == 0
        || caps.current_extent != undefined && caps.current_extent != extent
    {
        return Err(WindowChange::SurfaceExtent);
    }
    if extent.width < caps.min_image_extent.width
        || extent.width > caps.max_image_extent.width
        || extent.height < caps.min_image_extent.height
        || extent.height > caps.max_image_extent.height
        || caps.max_image_array_layers < 1
    {
        return Err(WindowChange::SurfaceLimits);
    }
    if !caps
        .supported_transforms
        .contains(vk::SurfaceTransformFlagsKHR::IDENTITY)
        || caps.current_transform != vk::SurfaceTransformFlagsKHR::IDENTITY
        || !caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
        || !caps
            .supported_usage_flags
            .contains(vk::ImageUsageFlags::TRANSFER_DST)
    {
        return Err(WindowChange::SurfaceProfile);
    }
    Ok(())
}

fn returned_image_count(count: u32, requested: u32) -> Result<usize, Error> {
    if requested == 0 || requested > 8 || count < requested || count > 8 {
        return Err(Error::UnavailableCompletion);
    }
    Ok(count as usize)
}

#[cfg(test)]
#[path = "wsi/contract_tests.rs"]
mod contract_tests;

impl Drop for Wsi {
    fn drop(&mut self) {
        // Private invariant: Presenter forgets the WHOLE backend (including
        // HWND and slot) if unresolved. No destructor can wait indefinitely.
        debug_assert!(!self.unresolved);
        if let Some(device) = self.device.as_ref() {
            // SAFETY: No queued work exists during construction failure; normal
            // destruction follows transfer fence, presentation-resource fence,
            // and the exact presentation-id completion.
            // All nullable handles belong to this device and are destroyed once.
            unsafe {
                if self.mapped.is_some() {
                    device.unmap_memory(self.memory);
                }
                if self.buffer != vk::Buffer::null() {
                    device.destroy_buffer(self.buffer, None);
                }
                if self.memory != vk::DeviceMemory::null() {
                    device.free_memory(self.memory, None);
                }
                if self.fence != vk::Fence::null() {
                    device.destroy_fence(self.fence, None);
                }
                if self.present_fence != vk::Fence::null() {
                    device.destroy_fence(self.present_fence, None);
                }
                if self.acquire != vk::Semaphore::null() {
                    device.destroy_semaphore(self.acquire, None);
                }
                if self.rendered != vk::Semaphore::null() {
                    device.destroy_semaphore(self.rendered, None);
                }
                if self.pool != vk::CommandPool::null() {
                    device.destroy_command_pool(self.pool, None);
                }
                if let Some(api) = self.swapchain_api.as_ref()
                    && self.swapchain != vk::SwapchainKHR::null()
                {
                    api.destroy_swapchain(self.swapchain, None);
                }
                device.destroy_device(None);
            }
        }
        // SAFETY: Device/swapchain are gone, HWND remains live in Backend until
        // after this destructor. Instance has no remaining live child object.
        unsafe {
            if self.surface != vk::SurfaceKHR::null() {
                self.surface_api.destroy_surface(self.surface, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}
