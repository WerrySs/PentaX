// SPDX-License-Identifier: GPL-2.0-only
//! An actual fragment-writable counter buffer; no CPU sampling model.
use super::*;
use pentax_gpu::lod_stats::{COUNTER_BYTES, Counters};

pub(super) struct Feedback<'d> {
    pub storage: Buffer<'d>,
}
impl<'d> Feedback<'d> {
    pub fn new(
        device: &'d Device,
        initial: &Counters,
        budget: &mut Budget,
    ) -> Result<Self, VulkanError> {
        budget.host(COUNTER_BYTES as u64)?;
        let mut bytes = [0; COUNTER_BYTES];
        for (raw, word) in bytes.chunks_exact_mut(4).zip(initial.as_words()) {
            raw.copy_from_slice(&word.to_le_bytes());
        }
        let mut storage = Buffer::new(
            device,
            COUNTER_BYTES,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            budget,
        )?;
        storage.write(&bytes, budget)?;
        Ok(Self { storage })
    }
    pub fn begin(
        &self,
        device: &Device,
        batch: &Batch<'_>,
        budget: &mut Budget,
    ) -> Result<(), VulkanError> {
        budget.command(1)?;
        let barrier = [vk::BufferMemoryBarrier::default()
            .buffer(self.storage.raw)
            .offset(0)
            .size(COUNTER_BYTES as u64)
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)];
        // SAFETY: The flushed upload is the exact owned514-word storage bound
        // at the compiler's fixed feedback binding. It outlives the batch.
        unsafe {
            device.raw.cmd_pipeline_barrier(
                batch.raw,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &barrier,
                &[],
            )
        };
        Ok(())
    }
    pub fn end(
        &self,
        device: &Device,
        batch: &Batch<'_>,
        budget: &mut Budget,
    ) -> Result<(), VulkanError> {
        buffer_host_barrier(
            device,
            batch.raw,
            self.storage.raw,
            COUNTER_BYTES as u64,
            vk::AccessFlags::SHADER_WRITE,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            budget,
        )
    }
    /// Caller must first complete this exact batch's submit_wait successfully.
    pub fn completed(&self, budget: &mut Budget) -> Result<Counters, VulkanError> {
        let bytes = self.storage.read_completed(budget)?;
        Counters::from_le_bytes(&bytes)
            .map_err(|_| VulkanError::InvalidCompilerContract("LOD counter readback extent"))
    }
}
