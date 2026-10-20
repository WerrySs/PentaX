// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: complete flat graphics submission admission.
// Source: pinned agc.cpp submit_command_buffer and graphicsRun.cpp Submit;
// draw/state contracts are documented by pentax-gpu/src/graphics/README.md.

use super::{Driver, Failure, NativeRange, PausedNativeMemory, add, compute, draw};
use std::mem::size_of;

impl Driver {
    /// Snapshot and plan every root stream before executing any draw. The
    /// source has one graphics processor, independent of its 56 ACB queues.
    /// Unknown reset registers remain unknown, rather than invented defaults.
    pub(super) fn submit_graphics(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        ranges: &[NativeRange],
    ) -> Result<u64, Failure> {
        if self
            .graphics_owner
            .is_some_and(|owner| owner != memory.owner_id())
        {
            return Err(Failure::Memory);
        }
        let count = ranges
            .iter()
            .filter(|range| range.address != 0 && range.size != 0)
            .count();
        if count == 0 {
            return Ok(0);
        }
        let mut commands = self.limits.graphics_commands;
        commands.max_buffers = commands
            .max_buffers
            .min(self.limits.max_batch_buffers as usize);
        commands.max_draws = commands.max_draws.min(
            usize::try_from(self.limits.max_draws.saturating_sub(self.usage.draws))
                .unwrap_or(usize::MAX),
        );
        commands.max_vertices = commands
            .max_vertices
            .min(self.limits.max_vertices.saturating_sub(self.usage.vertices));
        if count > commands.max_buffers {
            return Err(Failure::Limit);
        }
        // Both bytes and descriptor storage are admitted before allocation or
        // copying. Capacity is checked again against the actual allocator result.
        let mut total = 0;
        for range in ranges
            .iter()
            .filter(|range| range.address != 0 && range.size != 0)
        {
            total = add(total, range.size, commands.max_command_bytes as u64)?;
        }
        let next_bytes = add(
            self.usage.command_bytes,
            total,
            self.limits.max_command_bytes,
        )?;
        let next_buffers = add(
            self.usage.command_buffers,
            count as u64,
            self.limits.max_command_buffers,
        )?;
        let metadata = count
            .checked_mul(
                size_of::<Vec<u8>>() + size_of::<&[u8]>() + size_of::<compute::CommandRange>(),
            )
            .and_then(|bytes| {
                // The caller retains its already-admitted range vector while
                // this helper runs; its capacity is bounded by max_batch_buffers.
                (self.limits.max_batch_buffers as usize)
                    .checked_mul(size_of::<NativeRange>())
                    .and_then(|ranges| bytes.checked_add(ranges))
            })
            .ok_or(Failure::Limit)?;
        let mut held = host_add(
            metadata,
            usize::try_from(total).map_err(|_| Failure::Limit)?,
            self.limits.graphics.max_host_bytes,
        )?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            total.checked_add(metadata as u64).ok_or(Failure::Limit)?,
            self.limits.max_preparation_work,
        )?;
        self.usage.command_bytes = next_bytes;
        self.usage.command_buffers = next_buffers;
        let mut snapshots = Vec::<Vec<u8>>::new();
        let mut sources = Vec::<compute::CommandRange>::new();
        let mut buffers = Vec::<&[u8]>::new();
        snapshots
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        sources
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        buffers
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        for (capacity, size) in [
            (snapshots.capacity(), size_of::<Vec<u8>>()),
            (sources.capacity(), size_of::<compute::CommandRange>()),
            (buffers.capacity(), size_of::<&[u8]>()),
        ] {
            held = host_add(
                held,
                capacity
                    .checked_sub(count)
                    .and_then(|n| n.checked_mul(size))
                    .ok_or(Failure::Limit)?,
                self.limits.graphics.max_host_bytes,
            )?;
        }
        for range in ranges
            .iter()
            .filter(|range| range.address != 0 && range.size != 0)
        {
            let size = usize::try_from(range.size).map_err(|_| Failure::Limit)?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(size)
                .map_err(|_| Failure::Allocation)?;
            held = host_add(
                held,
                bytes.capacity().checked_sub(size).ok_or(Failure::Limit)?,
                self.limits.graphics.max_host_bytes,
            )?;
            bytes.resize(size, 0);
            memory
                .read_gpu(range.address, &mut bytes)
                .map_err(|_| Failure::Memory)?;
            snapshots.push(bytes);
            sources.push(compute::CommandRange {
                address: range.address,
                size: range.size,
            });
        }
        for bytes in &snapshots {
            buffers.push(bytes);
        }
        commands.max_host_bytes = commands.max_host_bytes.min(
            self.limits
                .graphics
                .max_host_bytes
                .checked_sub(held)
                .ok_or(Failure::Limit)?,
        );
        commands.max_work = commands.max_work.min(
            self.limits
                .max_preparation_work
                .saturating_sub(self.usage.preparation_work),
        );
        let initial = if self.frame.reset_before_graphics {
            draw::GraphicsState::empty()
        } else {
            self.graphics
        };
        let plan = draw::plan_commands(&buffers, initial, &commands)
            .map_err(Failure::GraphicsPreparation)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            plan.usage().work,
            self.limits.max_preparation_work,
        )?;
        // The plan owns its state snapshots. Guest bytes are not retained as a
        // replay cache; all requested command bytes are freshly fetched per call.
        drop(buffers);
        drop(snapshots);
        let retained = plan
            .usage()
            .host_bytes
            .checked_add(
                sources
                    .capacity()
                    .checked_mul(size_of::<compute::CommandRange>())
                    .ok_or(Failure::Limit)?,
            )
            .and_then(|bytes| {
                (self.limits.max_batch_buffers as usize)
                    .checked_mul(size_of::<NativeRange>())
                    .and_then(|ranges| bytes.checked_add(ranges))
            })
            .ok_or(Failure::Limit)?;
        for command in plan.draws() {
            let mut limits = self.limits.graphics;
            limits.max_work = limits.max_work.min(
                self.limits
                    .max_preparation_work
                    .saturating_sub(self.usage.preparation_work),
            );
            limits.max_vertices = limits
                .max_vertices
                .min(self.limits.max_vertices.saturating_sub(self.usage.vertices));
            limits.max_host_bytes = limits
                .max_host_bytes
                .checked_sub(retained)
                .ok_or(Failure::Limit)?;
            self.usage.draws = add(self.usage.draws, 1, self.limits.max_draws)?;
            let report = super::super::graphics::execute_draw_admitted(
                memory,
                command,
                &limits,
                &self.limits.gpu,
                &sources,
                |prepared, preflight, may_execute| {
                    let vertices = add(
                        self.usage.vertices,
                        prepared.vertices,
                        self.limits.max_vertices,
                    )
                    .map_err(|_| super::super::graphics::Error::Limit)?;
                    let work = prepared
                        .work
                        .checked_add(preflight)
                        .ok_or(super::super::graphics::Error::Limit)?;
                    let work = add(
                        self.usage.preparation_work,
                        work,
                        self.limits.max_preparation_work,
                    )
                    .map_err(|_| super::super::graphics::Error::Limit)?;
                    self.usage.vertices = vertices;
                    self.usage.preparation_work = work;
                    // A possible GPU effect makes every subsequent error
                    // terminal. Only complete submission success clears this.
                    self.poisoned |= may_execute;
                    Ok(())
                },
            )
            .map_err(|error| match error {
                super::super::graphics::Error::Prepare(error) => {
                    Failure::GraphicsPreparation(error)
                }
                super::super::graphics::Error::Memory(_) => Failure::Memory,
                super::super::graphics::Error::Vulkan(error) => Failure::Vulkan(error),
                super::super::graphics::Error::Limit => Failure::Limit,
            })?;
            self.usage.written_bytes = self
                .usage
                .written_bytes
                .saturating_add(report.written_bytes as u64);
            // Linear draw completion cannot attest a tiled VideoOut target.
            // Presentation must establish its own actual layout/owner receipt.
        }
        self.graphics = plan.final_state();
        self.graphics_owner = Some(memory.owner_id());
        self.frame.reset_before_graphics = false;
        self.poisoned = false;
        Ok(0)
    }
}

fn host_add(held: usize, count: usize, maximum: usize) -> Result<usize, Failure> {
    held.checked_add(count)
        .filter(|bytes| *bytes <= maximum)
        .ok_or(Failure::Limit)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "graphics_tests.rs"]
mod tests;
