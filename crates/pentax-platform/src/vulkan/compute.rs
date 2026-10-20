// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: manifest-bound guest compute execution; see NOTICE.

use super::{
    Budget, ComputeReadback, GpuLimits, VulkanError,
    commands::{Batch, buffer_host_barrier},
    device::Device,
    pipeline::Pipeline,
    programs::check_contract,
    resources::Buffer,
};
use ash::vk;
use pentax_gpu::compute::PreparedCompute;

pub(super) fn run(
    prepared: &PreparedCompute,
    limits: GpuLimits,
) -> Result<ComputeReadback, VulkanError> {
    let mut budget = Budget::new(limits);
    let shader = prepared.shader();
    check_contract(shader, pentax_shader::ShaderStage::Compute)?;
    budget.shader(shader.spirv().len())?;
    budget.work(
        prepared
            .usage()
            .invocations
            .checked_mul(shader.spirv().len() as u64)
            .ok_or(VulkanError::Limit("compute execution work"))?,
    )?;
    if shader.manifest().buffers().len() != 1 {
        return Err(VulkanError::InvalidCompilerContract("compute buffers"));
    }
    let required = shader.manifest().required_features();
    let subgroup = if required.subgroup_basic() || required.subgroup_ballot() {
        vk::ShaderStageFlags::COMPUTE
    } else {
        vk::ShaderStageFlags::empty()
    };
    let ballot = if required.subgroup_ballot() {
        vk::ShaderStageFlags::COMPUTE
    } else {
        vk::ShaderStageFlags::empty()
    };
    let device = Device::open(subgroup, ballot, &mut budget)?;
    let local = shader
        .manifest()
        .workgroup_size()
        .ok_or(VulkanError::InvalidCompilerContract("compute local size"))?;
    let groups = prepared.groups();
    let host = device.properties.limits;
    let mut local_count = 1_u64;
    for axis in 0..3 {
        if local[axis] == 0
            || local[axis] > host.max_compute_work_group_size[axis]
            || groups[axis] == 0
            || groups[axis] > host.max_compute_work_group_count[axis]
        {
            return Err(VulkanError::Unsupported("compute group geometry"));
        }
        local_count = local_count
            .checked_mul(u64::from(local[axis]))
            .ok_or(VulkanError::Limit("compute local size"))?;
    }
    if local_count > u64::from(host.max_compute_work_group_invocations) {
        return Err(VulkanError::Unsupported("compute local invocations"));
    }
    let length = prepared.input().len();
    let mut buffer = Buffer::new(
        &device,
        length,
        vk::BufferUsageFlags::STORAGE_BUFFER,
        &mut budget,
    )?;
    buffer.write(prepared.input(), &mut budget)?;
    let pipeline = Pipeline::compute(&device, shader, &buffer, &mut budget)?;
    // The batch is declared after all resources. Its Drop drains uncertain
    // submissions before any pipeline/buffer/device dependency can be destroyed.
    let batch = Batch::begin(&device, &mut budget)?;
    budget.command(5)?;
    let barrier = [vk::BufferMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::HOST_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .buffer(buffer.raw)
        .offset(0)
        .size(length as u64)];
    // SAFETY: Opaque compiler product and full manifest/features/device geometry
    // were checked. The exact owned uploaded buffer, descriptor set, pipeline,
    // 128-byte initialized push block and recording batch remain borrowed/live.
    // No host access overlaps this submission; the barrier and fence below
    // complete all shader writes before readback or dependent owner destruction.
    unsafe {
        device.raw.cmd_pipeline_barrier(
            batch.raw,
            vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &barrier,
            &[],
        );
        device
            .raw
            .cmd_bind_pipeline(batch.raw, vk::PipelineBindPoint::COMPUTE, pipeline.raw);
        device.raw.cmd_bind_descriptor_sets(
            batch.raw,
            vk::PipelineBindPoint::COMPUTE,
            pipeline.layout,
            0,
            &[pipeline.set],
            &[],
        );
        device.raw.cmd_push_constants(
            batch.raw,
            pipeline.layout,
            vk::ShaderStageFlags::COMPUTE,
            0,
            prepared.push_data(),
        );
        device
            .raw
            .cmd_dispatch(batch.raw, groups[0], groups[1], groups[2]);
    }
    buffer_host_barrier(
        &device,
        batch.raw,
        buffer.raw,
        length as u64,
        vk::AccessFlags::SHADER_WRITE,
        vk::PipelineStageFlags::COMPUTE_SHADER,
        &mut budget,
    )?;
    batch.submit_wait(&mut budget)?;
    let bytes = buffer.read_completed(&mut budget)?;
    Ok(ComputeReadback {
        bytes,
        usage: budget.usage,
    })
}
