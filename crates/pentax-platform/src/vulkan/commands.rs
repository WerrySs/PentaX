// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: synchronous owned recording and completion; see NOTICE.

use super::{
    Budget, VulkanError,
    device::{Device, result},
};
use ash::vk;

/// Declared after all resources used by this batch. On any post-submit error
/// its destructor drains the device BEFORE those resources can be destroyed.
pub(super) struct Batch<'d> {
    device: &'d Device,
    pool: vk::CommandPool,
    pub raw: vk::CommandBuffer,
    fence: vk::Fence,
    submitted: bool,
}
impl<'d> Batch<'d> {
    pub fn begin(device: &'d Device, budget: &mut Budget) -> Result<Self, VulkanError> {
        budget.object()?;
        let info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(device.family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        // SAFETY: Unique command pool for the selected family; never shared or
        // reset while commands are pending, and no optional protected flags.
        let pool = result(
            unsafe { device.raw.create_command_pool(&info, None) },
            "vkCreateCommandPool",
        )?;
        let mut out = Self {
            device,
            pool,
            raw: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            submitted: false,
        };
        budget.object()?;
        let info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: Exact single output slot for count1, owned compatible pool.
        // Raw entry avoids a hidden infallible Vec allocation for one handle.
        let code = unsafe {
            (device.raw.fp_v1_0().allocate_command_buffers)(
                device.raw.handle(),
                &info,
                &mut out.raw,
            )
        };
        result(code.result(), "vkAllocateCommandBuffers")?;
        budget.object()?;
        let info = vk::FenceCreateInfo::default();
        // SAFETY: New unsignaled uniquely owned fence.
        out.fence = result(
            unsafe { device.raw.create_fence(&info, None) },
            "vkCreateFence",
        )?;
        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        budget.command(1)?;
        // SAFETY: Fresh primary command buffer, no simultaneous or inherited
        // rendering state; all subsequent recording is on this same thread.
        let code = unsafe { device.raw.begin_command_buffer(out.raw, &info) };
        result(code, "vkBeginCommandBuffer")?;
        Ok(out)
    }

    pub fn submit_wait(mut self, budget: &mut Budget) -> Result<(), VulkanError> {
        budget.command(2)?;
        // SAFETY: All rendering scopes are closed by the private recorder.
        let code = unsafe { self.device.raw.end_command_buffer(self.raw) };
        result(code, "vkEndCommandBuffer")?;
        let commands = [self.raw];
        let submits = [vk::SubmitInfo::default().command_buffers(&commands)];
        // Set BEFORE the driver call: any failure also takes the conservative
        // drain path; no uncertainty frees resources while they might be live.
        self.submitted = true;
        // SAFETY: Single-threaded private queue, one executable primary buffer,
        // unsignaled fence, no external wait/signal semaphores or ownership.
        let code = unsafe {
            self.device
                .raw
                .queue_submit(self.device.queue, &submits, self.fence)
        };
        result(code, "vkQueueSubmit")?;
        // SAFETY: The owned fence belongs to the submitted work. A successful
        // wait makes all its command/resource use complete before publication.
        let code = unsafe {
            self.device
                .raw
                .wait_for_fences(&[self.fence], true, budget.limits.fence_timeout_ns)
        };
        result(code, "vkWaitForFences")?;
        self.submitted = false;
        Ok(())
    }
}
impl Drop for Batch<'_> {
    fn drop(&mut self) {
        // SAFETY: This guard drops before every buffer/image/pipeline referenced
        // by its commands. Drain uncertain/pending work before freeing the fence
        // and pool. Driver hang cleanup is deliberately not claimed time bounded.
        unsafe {
            if self.submitted {
                let drained = self.device.raw.device_wait_idle();
                if !safe_to_destroy(drained) {
                    // An OOM/unknown cleanup failure is not completion evidence.
                    // This closed worker must not unwind and free live GPU
                    // resources. Device loss, unlike other errors, is explicitly
                    // completion-equivalent for object lifetime in Vulkan.
                    std::process::abort();
                }
            }
            if self.fence != vk::Fence::null() {
                self.device.raw.destroy_fence(self.fence, None);
            }
            self.device.raw.destroy_command_pool(self.pool, None);
        }
    }
}

fn safe_to_destroy(wait: Result<(), vk::Result>) -> bool {
    matches!(wait, Ok(()) | Err(vk::Result::ERROR_DEVICE_LOST))
}

pub(super) fn buffer_host_barrier(
    device: &Device,
    command: vk::CommandBuffer,
    buffer: vk::Buffer,
    length: u64,
    source: vk::AccessFlags,
    stage: vk::PipelineStageFlags,
    budget: &mut Budget,
) -> Result<(), VulkanError> {
    budget.command(1)?;
    let barriers = [vk::BufferMemoryBarrier::default()
        .src_access_mask(source)
        .dst_access_mask(vk::AccessFlags::HOST_READ)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer)
        .offset(0)
        .size(length)];
    // SAFETY: Range is the owned producer buffer, both stages belong to this
    // queue family, and recording remains outside a render pass. Host reads only
    // happen after the fence, with noncoherent invalidation when required.
    unsafe {
        device.raw.cmd_pipeline_barrier(
            command,
            stage,
            vk::PipelineStageFlags::HOST,
            vk::DependencyFlags::empty(),
            &[],
            &barriers,
            &[],
        )
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_requires_completion_or_explicit_device_loss() {
        assert!(safe_to_destroy(Ok(())));
        assert!(safe_to_destroy(Err(vk::Result::ERROR_DEVICE_LOST)));
        for failure in [
            vk::Result::ERROR_OUT_OF_HOST_MEMORY,
            vk::Result::ERROR_OUT_OF_DEVICE_MEMORY,
            vk::Result::ERROR_UNKNOWN,
            vk::Result::TIMEOUT,
        ] {
            assert!(!safe_to_destroy(Err(failure)));
        }
    }
}
