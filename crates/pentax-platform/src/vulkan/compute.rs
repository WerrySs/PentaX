// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: manifest-bound guest compute execution; see NOTICE.

use super::{
    Budget, ComputeBufferReadback, ComputeReadback, GpuLimits, VulkanError,
    commands::{Batch, buffer_host_barrier},
    device::{Device, DeviceRequirements},
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
    if shader.manifest().buffers().len() != prepared.buffers().len() || prepared.buffers().len() > 4
    {
        return Err(VulkanError::InvalidCompilerContract("compute buffers"));
    }
    for (resource, element) in prepared.buffers().iter().zip(shader.manifest().buffers()) {
        if resource.read() != element.read
            || resource.written() != element.written
            || element.coherent
            || resource.input().is_empty()
            || resource.input().len() < element.max_byte_extent as usize
        {
            return Err(VulkanError::InvalidCompilerContract(
                "compute buffer access/extent",
            ));
        }
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
    // No varying-subgroup-size pipeline flag or size-control extension is used.
    // Actual subgroup operations require the manifest's exact native size.
    // Logical EXEC/lane indexing and LDS Workgroup barriers do not impersonate
    // native subgroup64 and need no such requirement by themselves.
    let device = Device::open_required(
        DeviceRequirements {
            subgroup_stages: subgroup,
            ballot_stages: ballot,
            required_subgroup_size: shader.manifest().required_subgroup_size(),
            ..Default::default()
        },
        &mut budget,
    )?;
    let local = shader
        .manifest()
        .workgroup_size()
        .ok_or(VulkanError::InvalidCompilerContract("compute local size"))?;
    let groups = prepared.groups();
    let host = device.properties.limits;
    if shader.manifest().shared_memory_bytes() > host.max_compute_shared_memory_size {
        return Err(VulkanError::Unsupported("compute shared memory size"));
    }
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
    let count = prepared.buffers().len();
    budget.host((count * std::mem::size_of::<Buffer<'_>>()) as u64)?;
    let mut buffers = Vec::new();
    buffers
        .try_reserve_exact(count)
        .map_err(|_| VulkanError::Allocation)?;
    budget.host(((buffers.capacity() - count) * std::mem::size_of::<Buffer<'_>>()) as u64)?;
    let written = prepared
        .buffers()
        .iter()
        .filter(|buffer| buffer.written())
        .count();
    budget.host((written * std::mem::size_of::<ComputeBufferReadback>()) as u64)?;
    let mut outputs = Vec::new();
    outputs
        .try_reserve_exact(written)
        .map_err(|_| VulkanError::Allocation)?;
    budget.host(
        ((outputs.capacity() - written) * std::mem::size_of::<ComputeBufferReadback>()) as u64,
    )?;
    for (index, input) in prepared.buffers().iter().enumerate() {
        if input.input().len() as u64 > u64::from(host.max_storage_buffer_range) {
            return Err(VulkanError::Unsupported("compute storage range"));
        }
        let mut buffer = Buffer::new(
            &device,
            input.input().len(),
            vk::BufferUsageFlags::STORAGE_BUFFER,
            &mut budget,
        )?;
        buffer.write(input.input(), &mut budget)?;
        buffers.push(buffer);
        if input.written() {
            // Allocate and admit every output before any GPU effect. These
            // bytes are never exposed until actual fenced readback overwrites them.
            outputs.push(ComputeBufferReadback {
                resource_index: index,
                bytes: budget.buffer(input.input().len())?,
            });
        }
    }
    let pipeline = Pipeline::compute_many(&device, shader, &buffers, &mut budget)?;
    // The batch is declared after all resources. Its Drop drains uncertain
    // submissions before any pipeline/buffer/device dependency can be destroyed.
    let batch = Batch::begin(&device, &mut budget)?;
    budget.command(if count == 0 { 3 } else { 5 })?;
    let mut barriers = [vk::BufferMemoryBarrier::default(); 4];
    for (barrier, buffer) in barriers.iter_mut().zip(&buffers) {
        *barrier = vk::BufferMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(buffer.raw)
            .offset(0)
            .size(buffer.length() as u64);
    }
    // SAFETY: Opaque compiler product and full manifest/features/device geometry
    // were checked. The exact owned uploaded buffer, descriptor set, pipeline,
    // 128-byte initialized push block and recording batch remain borrowed/live.
    // No host access overlaps this submission; the barrier and fence below
    // complete all shader writes before readback or dependent owner destruction.
    unsafe {
        if count != 0 {
            device.raw.cmd_pipeline_barrier(
                batch.raw,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::COMPUTE_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &barriers[..count],
                &[],
            );
        }
        device
            .raw
            .cmd_bind_pipeline(batch.raw, vk::PipelineBindPoint::COMPUTE, pipeline.raw);
        if count != 0 {
            device.raw.cmd_bind_descriptor_sets(
                batch.raw,
                vk::PipelineBindPoint::COMPUTE,
                pipeline.layout,
                0,
                &[pipeline.set],
                &[],
            );
        }
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
    for output in &outputs {
        let buffer = &buffers[output.resource_index];
        buffer_host_barrier(
            &device,
            batch.raw,
            buffer.raw,
            buffer.length() as u64,
            vk::AccessFlags::SHADER_WRITE,
            vk::PipelineStageFlags::COMPUTE_SHADER,
            &mut budget,
        )?;
    }
    batch.submit_wait(&mut budget)?;
    for output in &mut outputs {
        buffers[output.resource_index].read_completed_into(&mut output.bytes)?;
    }
    Ok(ComputeReadback {
        resource_count: count,
        buffers: outputs,
        usage: budget.usage,
    })
}
