// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned headless device; see NOTICE.

use super::{Budget, VulkanError};
use ash::{Entry, Instance, vk};
use std::{
    ffi::OsString, marker::PhantomData, os::windows::ffi::OsStringExt, path::PathBuf, rc::Rc,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

pub(super) fn result<T>(
    value: Result<T, vk::Result>,
    operation: &'static str,
) -> Result<T, VulkanError> {
    value.map_err(|code| VulkanError::Driver {
        operation,
        code: code.as_raw(),
    })
}

struct InstanceOwner {
    _entry: Entry,
    raw: Instance,
}
impl Drop for InstanceOwner {
    fn drop(&mut self) {
        // SAFETY: DeviceOwner is destroyed first; this unique instance owns no
        // remaining device/surface/debug children and its loader remains live.
        unsafe { self.raw.destroy_instance(None) };
    }
}

/// The queue is private and all submissions are synchronous on this one thread.
/// Resources borrow this owner, so its device/instance/loader outlive them.
pub(super) struct Device {
    instance: InstanceOwner,
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub queue: vk::Queue,
    pub family: u32,
    pub properties: vk::PhysicalDeviceProperties,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    full_draw_index_uint32: bool,
    _thread: PhantomData<Rc<()>>,
}

// The reported fallback limit is still meaningful without the optional full
// range feature. Never infer full 32-bit indices from a buffer's element type.
// https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceFeatures.html
// https://docs.vulkan.org/refpages/latest/refpages/source/VkPhysicalDeviceLimits.html
fn check_index_value(full: bool, limit: u32, maximum: u32) -> Result<(), VulkanError> {
    if (full && limit != u32::MAX) || (!full && limit < 0x00ff_ffff) {
        return Err(VulkanError::Unsupported("indexed draw device limits"));
    }
    if maximum > limit {
        return Err(VulkanError::Unsupported("index value exceeds device limit"));
    }
    Ok(())
}

fn subgroup_supported(
    size: u32,
    stages: vk::ShaderStageFlags,
    ballot: vk::ShaderStageFlags,
    properties: &vk::PhysicalDeviceSubgroupProperties<'_>,
) -> bool {
    properties.subgroup_size == size
        && properties.supported_stages.contains(stages)
        && properties
            .supported_operations
            .contains(vk::SubgroupFeatureFlags::BASIC)
        && (ballot.is_empty()
            || properties
                .supported_operations
                .contains(vk::SubgroupFeatureFlags::BALLOT))
}

#[derive(Clone, Copy, Default)]
pub(super) struct DeviceRequirements {
    pub subgroup_stages: vk::ShaderStageFlags,
    pub ballot_stages: vk::ShaderStageFlags,
    pub arithmetic_stages: vk::ShaderStageFlags,
    pub fragment_stores_and_atomics: bool,
    // None preserves the existing native32 contract for graphics/old callers.
    pub required_subgroup_size: Option<u32>,
}

impl Device {
    pub fn open(
        subgroup_stages: vk::ShaderStageFlags,
        ballot_stages: vk::ShaderStageFlags,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        Self::open_required(
            DeviceRequirements {
                subgroup_stages,
                ballot_stages,
                ..Default::default()
            },
            budget,
        )
    }
    pub fn open_required(
        requirements: DeviceRequirements,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        let DeviceRequirements {
            subgroup_stages,
            ballot_stages,
            arithmetic_stages,
            fragment_stores_and_atomics,
            required_subgroup_size,
        } = requirements;
        let subgroup_size = required_subgroup_size.unwrap_or(32);
        if !matches!(subgroup_size, 32 | 64)
            || required_subgroup_size.is_some() && subgroup_stages.is_empty()
            || subgroup_size == 64 && subgroup_stages != vk::ShaderStageFlags::COMPUTE
        {
            return Err(VulkanError::InvalidCompilerContract(
                "required subgroup size",
            ));
        }
        if !subgroup_stages.contains(ballot_stages | arithmetic_stages) {
            return Err(VulkanError::InvalidCompilerContract(
                "subgroup requirement stages",
            ));
        }
        // Use the OS system directory, never PATH, current directory, guest data
        // or an environment-supplied loader location.
        let mut directory = [0_u16; 512];
        // SAFETY: Fixed writable UTF-16 output; the API receives its true length.
        let length =
            unsafe { GetSystemDirectoryW(directory.as_mut_ptr(), directory.len() as u32) } as usize;
        if length == 0 || length >= directory.len() {
            return Err(VulkanError::LoaderUnavailable);
        }
        let mut path = PathBuf::from(OsString::from_wide(&directory[..length]));
        if !path.is_absolute() {
            return Err(VulkanError::LoaderUnavailable);
        }
        path.push("vulkan-1.dll");
        budget.object()?;
        // SAFETY: Loads only the named absolute OS loader. It remains owned by
        // InstanceOwner until every function pointer/object has been destroyed.
        let entry =
            unsafe { Entry::load_from(path) }.map_err(|_| VulkanError::LoaderUnavailable)?;
        // SAFETY: This is the live loader's version query; no external pointer.
        let version = unsafe { entry.try_enumerate_instance_version() }
            .map_err(|_| VulkanError::LoaderUnavailable)?
            .unwrap_or(vk::API_VERSION_1_0);
        if version < vk::API_VERSION_1_2 {
            return Err(VulkanError::Unsupported(
                "Vulkan 1.2 float-controls baseline",
            ));
        }
        let application = vk::ApplicationInfo::default()
            .application_name(c"PentaX")
            .engine_name(c"PentaX")
            .api_version(vk::API_VERSION_1_2);
        let info = vk::InstanceCreateInfo::default().application_info(&application);
        budget.object()?;
        // SAFETY: All create-info borrows outlive this synchronous call; no
        // extensions, external callbacks or custom allocator are supplied.
        let raw = result(
            unsafe { entry.create_instance(&info, None) },
            "vkCreateInstance",
        )?;
        let instance = InstanceOwner { _entry: entry, raw };

        let mut count = 0;
        // SAFETY: Valid instance and scalar output; null means count query.
        let code = unsafe {
            (instance.raw.fp_v1_0().enumerate_physical_devices)(
                instance.raw.handle(),
                &mut count,
                std::ptr::null_mut(),
            )
        };
        result(code.result(), "vkEnumeratePhysicalDevices(count)")?;
        if count == 0 || count > 32 {
            return Err(VulkanError::Unsupported("physical-device count"));
        }
        budget.work(u64::from(count))?;
        let mut devices = [vk::PhysicalDevice::null(); 32];
        // SAFETY: Count is bounded by the fixed output array. INCOMPLETE is
        // refused rather than growing a driver-sized allocation or retry loop.
        let code = unsafe {
            (instance.raw.fp_v1_0().enumerate_physical_devices)(
                instance.raw.handle(),
                &mut count,
                devices.as_mut_ptr(),
            )
        };
        result(code.result(), "vkEnumeratePhysicalDevices")?;
        if count > devices.len() as u32 {
            return Err(VulkanError::Unsupported("physical-device count changed"));
        }

        let mut selected = None;
        for physical in devices[..count as usize].iter().copied() {
            // SAFETY: The immutable physical handle belongs to this instance.
            let base_properties = unsafe { instance.raw.get_physical_device_properties(physical) };
            if base_properties.api_version < vk::API_VERSION_1_2 {
                continue;
            }
            let mut floats = vk::PhysicalDeviceFloatControlsProperties::default();
            let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
            let mut properties = vk::PhysicalDeviceProperties2::default()
                .push_next(&mut floats)
                .push_next(&mut subgroup);
            // SAFETY: Physical handle belongs to this instance; output chain
            // consists of core 1.2 properties backed by live stack values.
            unsafe {
                instance
                    .raw
                    .get_physical_device_properties2(physical, &mut properties)
            };
            let properties = properties.properties;
            if properties.api_version < vk::API_VERSION_1_2
                || floats.shader_signed_zero_inf_nan_preserve_float32 == 0
            {
                continue;
            }
            budget.work(1)?;
            // SAFETY: Fixed core feature output for this retained physical
            // device. Only queried fullDrawIndexUint32 and explicitly requested,
            // supported fragmentStoresAndAtomics bits may be enabled.
            let supported = unsafe { instance.raw.get_physical_device_features(physical) };
            let full_draw_index_uint32 = supported.full_draw_index_uint32 != vk::FALSE;
            if fragment_stores_and_atomics && supported.fragment_stores_and_atomics == vk::FALSE {
                continue;
            }
            if !arithmetic_stages.is_empty()
                && (!subgroup.supported_stages.contains(arithmetic_stages)
                    || !subgroup
                        .supported_operations
                        .contains(vk::SubgroupFeatureFlags::ARITHMETIC))
            {
                continue;
            }
            if !subgroup_stages.is_empty()
                && !subgroup_supported(subgroup_size, subgroup_stages, ballot_stages, &subgroup)
            {
                continue;
            }
            let mut families = 0;
            // SAFETY: Valid physical handle, scalar count output, null array.
            unsafe {
                (instance
                    .raw
                    .fp_v1_0()
                    .get_physical_device_queue_family_properties)(
                    physical,
                    &mut families,
                    std::ptr::null_mut(),
                )
            };
            if families > 64 {
                continue;
            }
            budget.work(u64::from(families))?;
            let mut queues = [vk::QueueFamilyProperties::default(); 64];
            // SAFETY: Count was bounded by this fixed output array. The Vulkan
            // queue-family list is immutable over this physical device's life.
            unsafe {
                (instance
                    .raw
                    .fp_v1_0()
                    .get_physical_device_queue_family_properties)(
                    physical,
                    &mut families,
                    queues.as_mut_ptr(),
                )
            };
            if families > 64 {
                return Err(VulkanError::Unsupported("queue-family count changed"));
            }
            if let Some(index) = queues[..families as usize].iter().position(|q| {
                q.queue_count != 0
                    && q.queue_flags
                        .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
            }) {
                selected = Some((physical, index as u32, properties, full_draw_index_uint32));
                break;
            }
        }
        let (physical, family, properties, full_draw_index_uint32) = selected.ok_or(
            VulkanError::Unsupported("graphics+compute queue and float32 preservation"),
        )?;
        if properties.limits.max_push_constants_size < 128
            || properties.limits.max_compute_work_group_size[0] < 8
            || properties.limits.max_compute_work_group_invocations < 8
        {
            return Err(VulkanError::Unsupported("shader interface limits"));
        }
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(family)
            .queue_priorities(&priorities)];
        let features = vk::PhysicalDeviceFeatures::default()
            .full_draw_index_uint32(full_draw_index_uint32)
            .fragment_stores_and_atomics(fragment_stores_and_atomics);
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_features(&features);
        budget.object()?;
        // SAFETY: Selected nonempty queue family and one queue. The only
        // optional features are the just-queried index capability and explicitly
        // requested fragment storage atomics. Other features remain disabled.
        let raw = result(
            unsafe { instance.raw.create_device(physical, &info, None) },
            "vkCreateDevice",
        )?;
        // SAFETY: Queue0 was requested above; physical memory properties are a
        // fixed Vulkan struct and need no application-sized allocation.
        let (queue, memory) = unsafe {
            (
                raw.get_device_queue(family, 0),
                instance.raw.get_physical_device_memory_properties(physical),
            )
        };
        Ok(Self {
            instance,
            raw,
            physical,
            queue,
            family,
            properties,
            memory,
            full_draw_index_uint32,
            _thread: PhantomData,
        })
    }

    pub fn check_index_value(&self, maximum: u32) -> Result<(), VulkanError> {
        check_index_value(
            self.full_draw_index_uint32,
            self.properties.limits.max_draw_indexed_index_value,
            maximum,
        )
    }

    pub fn memory_type(
        &self,
        bits: u32,
        required: vk::MemoryPropertyFlags,
    ) -> Result<(u32, bool), VulkanError> {
        let mut fallback = None;
        for (index, memory) in self.memory.memory_types[..self.memory.memory_type_count as usize]
            .iter()
            .enumerate()
        {
            if bits & (1 << index) != 0 && memory.property_flags.contains(required) {
                let coherent = memory
                    .property_flags
                    .contains(vk::MemoryPropertyFlags::HOST_COHERENT);
                if coherent {
                    return Ok((index as u32, true));
                }
                fallback = Some((index as u32, false));
            }
        }
        fallback.ok_or(VulkanError::Unsupported("required memory type"))
    }

    pub fn rgba_supported(&self) -> bool {
        // SAFETY: This physical device belongs to the live retained instance.
        let props = unsafe {
            self.instance
                .raw
                .get_physical_device_format_properties(self.physical, vk::Format::R8G8B8A8_UNORM)
        };
        props.optimal_tiling_features.contains(
            vk::FormatFeatureFlags::COLOR_ATTACHMENT | vk::FormatFeatureFlags::TRANSFER_SRC,
        )
    }

    pub fn rgba_upload_supported(&self) -> bool {
        // SAFETY: Fixed core format on the selected retained physical device.
        let props = unsafe {
            self.instance
                .raw
                .get_physical_device_format_properties(self.physical, vk::Format::R8G8B8A8_UNORM)
        };
        props
            .optimal_tiling_features
            .contains(vk::FormatFeatureFlags::TRANSFER_DST)
    }

    pub fn sampled_rgba_supported(&self, linear: bool) -> bool {
        // SAFETY: Fixed core format on the selected retained physical device.
        let properties = unsafe {
            self.instance
                .raw
                .get_physical_device_format_properties(self.physical, vk::Format::R8G8B8A8_UNORM)
        };
        let mut required =
            vk::FormatFeatureFlags::SAMPLED_IMAGE | vk::FormatFeatureFlags::TRANSFER_DST;
        if linear {
            required |= vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
        }
        properties.optimal_tiling_features.contains(required)
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // SAFETY: All borrowed children are already dropped. Each submitted
        // Batch completed or proved device loss before permitting its children
        // to drop; an unproven cleanup instead terminates the isolated worker.
        unsafe { self.raw.destroy_device(None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_values_obey_the_enabled_feature_and_actual_reported_limit() {
        // A uint32 buffer can hold values above a device's supported range.
        // Its byte width does not authorize those values on a fallback host.
        assert_eq!(check_index_value(false, 0x00ff_ffff, 0x00ff_ffff), Ok(()));
        assert_eq!(
            check_index_value(false, 0x00ff_ffff, 0x0100_0000),
            Err(VulkanError::Unsupported("index value exceeds device limit"))
        );
        assert_eq!(check_index_value(false, 0x1234_5678, 0x1234_5678), Ok(()));
        assert!(check_index_value(false, 0x1234_5678, 0x1234_5679).is_err());
        assert_eq!(check_index_value(true, u32::MAX, u32::MAX), Ok(()));
        assert!(check_index_value(true, 0x00ff_ffff, 0).is_err());
        assert!(check_index_value(false, 0x00ff_fffe, 0).is_err());
    }
    #[test]
    fn native_subgroup_width_must_match_even_when_all_requested_operations_exist() {
        let compute = vk::ShaderStageFlags::COMPUTE;
        let fragment = vk::ShaderStageFlags::FRAGMENT;
        let mut reported = vk::PhysicalDeviceSubgroupProperties::default()
            .subgroup_size(32)
            .supported_stages(compute | fragment)
            .supported_operations(
                vk::SubgroupFeatureFlags::BASIC | vk::SubgroupFeatureFlags::BALLOT,
            );
        assert!(subgroup_supported(32, compute, compute, &reported));
        assert!(!subgroup_supported(64, compute, compute, &reported));
        reported.subgroup_size = 64;
        assert!(subgroup_supported(64, compute, compute, &reported));
        assert!(!subgroup_supported(32, fragment, fragment, &reported));
        reported.supported_operations = vk::SubgroupFeatureFlags::BASIC;
        assert!(!subgroup_supported(64, compute, compute, &reported));
        assert!(subgroup_supported(
            64,
            compute,
            vk::ShaderStageFlags::empty(),
            &reported
        ));
        reported.supported_stages = fragment;
        assert!(!subgroup_supported(
            64,
            compute,
            vk::ShaderStageFlags::empty(),
            &reported
        ));
    }
}
