// SPDX-License-Identifier: GPL-2.0-only
//! Actual GPU initialization and fenced readback of the fixed feedback storage.
use super::{
    Budget, GpuLimits, VulkanError,
    commands::{Batch, buffer_host_barrier},
    device::Device,
    resources::Buffer,
};
use ash::vk;
use pentax_gpu::lod_stats::{COUNTER_BYTES, Counters, STORAGE_ENTRIES};

pub(super) fn reset(limits: GpuLimits) -> Result<Counters, VulkanError> {
    if limits.max_host_bytes < COUNTER_BYTES as u64 {
        return Err(VulkanError::Limit("LOD counter readback bytes"));
    }
    let mut budget = Budget::new(limits);
    budget.work(
        (COUNTER_BYTES as u64)
            .checked_mul(2)
            .ok_or(VulkanError::Limit("LOD reset work"))?,
    )?;
    let device = Device::open(
        vk::ShaderStageFlags::empty(),
        vk::ShaderStageFlags::empty(),
        &mut budget,
    )?;
    let counters = Buffer::new(
        &device,
        COUNTER_BYTES,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        &mut budget,
    )?;
    // Declared after the buffer: uncertain submission cleanup drains before any
    // resource can be destroyed. No guest address or CPU-generated sample enters.
    let batch = Batch::begin(&device, &mut budget)?;
    budget.command(2)?;
    let half = (STORAGE_ENTRIES * 4) as u64;
    // SAFETY: Two disjoint complete, four-byte aligned fills cover the exact
    // owned514-word buffer. This initializes storage; it publishes no report.
    unsafe {
        device
            .raw
            .cmd_fill_buffer(batch.raw, counters.raw, 0, half, u32::MAX);
        device
            .raw
            .cmd_fill_buffer(batch.raw, counters.raw, half, half, 0);
    }
    buffer_host_barrier(
        &device,
        batch.raw,
        counters.raw,
        COUNTER_BYTES as u64,
        vk::AccessFlags::TRANSFER_WRITE,
        vk::PipelineStageFlags::TRANSFER,
        &mut budget,
    )?;
    batch.submit_wait(&mut budget)?;
    let bytes = counters.read_completed(&mut budget)?;
    Counters::from_le_bytes(&bytes)
        .map_err(|_| VulkanError::InvalidCompilerContract("LOD reset counter extent"))
}
