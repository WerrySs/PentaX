// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: complete flat graphics submission admission.
// Source: pinned agc.cpp submit_command_buffer and graphicsRun.cpp Submit;
// draw/state contracts are documented by pentax-gpu/src/graphics/README.md.

use super::{Driver, Failure, NativeRange, PausedNativeMemory, add, compute, draw};
use std::mem::size_of;

impl super::Limits {
    fn graphics_backend_limits(self) -> pentax_platform::vulkan::GpuLimits {
        let mut limits = self.gpu;
        if let Some(work) = self.graphics_backend_work {
            limits.max_work = work;
        }
        limits
    }
}

impl Driver {
    #[cfg(test)]
    pub(super) fn submit_graphics(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        ranges: &[NativeRange],
    ) -> Result<u64, Failure> {
        self.submit_graphics_with_context(memory, ranges, None, None)
    }
    /// Snapshot and plan every root stream before executing any draw. The
    /// source has one graphics processor, independent of its 56 ACB queues.
    /// Unknown reset registers remain unknown, rather than invented defaults.
    pub(super) fn submit_graphics_with_context(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        ranges: &[NativeRange],
        mut kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<u64, Failure> {
        if self
            .graphics_owner
            .is_some_and(|owner| owner != memory.owner_id())
        {
            return Err(Failure::Memory);
        }
        let max_host_bytes = self.graphics_host_limit()?;
        let command_host_bytes = if self.consumer.is_some() {
            max_host_bytes
                .checked_sub(self.video.retained_frame_bytes())
                .ok_or(Failure::Limit)?
        } else {
            max_host_bytes
        };
        let presentation = self.presentation_limits()?;
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
            command_host_bytes,
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
                command_host_bytes,
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
                command_host_bytes,
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
        commands.max_host_bytes = commands
            .max_host_bytes
            .min(command_host_bytes.checked_sub(held).ok_or(Failure::Limit)?);
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
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            total,
            self.limits.max_preparation_work,
        )?;
        commands.max_work = commands.max_work.min(
            self.limits
                .max_preparation_work
                .saturating_sub(self.usage.preparation_work),
        );
        let mut ordered = false;
        for (buffer, bytes) in buffers.iter().enumerate() {
            ordered |= pentax_gpu::contains_indirect_registers(&[bytes]).map_err(|error| {
                Failure::GraphicsPreparation(match error {
                    compute::Error::Packet { dword, reason } => draw::Error::Packet {
                        buffer,
                        dword,
                        reason,
                    },
                    compute::Error::Limit(reason) => draw::Error::Limit(reason),
                    compute::Error::Allocation => draw::Error::Allocation,
                    compute::Error::Unsupported(reason) => draw::Error::Unsupported(reason),
                    _ => draw::Error::Unsupported("graphics packet classification"),
                })
            })?;
        }
        let mut plan = if ordered {
            GraphicsCommands::Ordered(
                draw::OrderedCommands::prepare(&buffers, &sources, initial, &commands)
                    .map_err(Failure::GraphicsPreparation)?,
            )
        } else {
            GraphicsCommands::Static {
                plan: draw::plan_commands(&buffers, initial, &commands)
                    .map_err(Failure::GraphicsPreparation)?,
                action: 0,
            }
        };
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
        if plan.usage().flips != 0 && (kernel.is_none() || clock.is_none()) {
            return Err(Failure::Kernel(pentax_kernel::Failure::Unsupported(
                "bound GPU flip process clock",
            )));
        }
        self.video.check_owner(memory.owner_id())?;
        // Pre-admit every pending slot and old+staged capture peak before any
        // draw can retire. Full queues stop explicitly; no consumer is invented.
        let mut planned = [0usize; 3];
        let mut frame_bytes = self.video.retained_frame_bytes();
        let mut capture_work = 0u64;
        for flip in plan.flips() {
            let surface = self.video.gpu_capture_surface(flip)?;
            let slot = usize::try_from(flip.handle() - 1)
                .ok()
                .filter(|i| *i < 3)
                .ok_or(Failure::UnsupportedVideo)?;
            planned[slot] += 1;
            if self
                .video
                .gpu_pending(flip.handle())?
                .checked_add(planned[slot])
                .is_none_or(|n| n > 16)
            {
                return Err(Failure::Waiting);
            }
            let tight = usize::try_from(u64::from(surface.width) * u64::from(surface.height) * 4)
                .map_err(|_| Failure::Limit)?;
            let source = usize::try_from(surface.data.size).map_err(|_| Failure::Limit)?;
            if source > presentation.max_frame_bytes || tight > presentation.max_frame_bytes {
                return Err(Failure::Limit);
            }
            let peak = frame_bytes
                .checked_add(source)
                .and_then(|n| n.checked_add(tight))
                .ok_or(Failure::Limit)?;
            if peak > presentation.max_held_bytes {
                return Err(Failure::Limit);
            }
            host_add(retained, peak, max_host_bytes)?;
            frame_bytes = frame_bytes.checked_add(tight).ok_or(Failure::Limit)?;
            capture_work = capture_work
                .checked_add(surface.data.size)
                .and_then(|n| n.checked_add(tight as u64 * 5 + 256))
                .ok_or(Failure::Limit)?;
        }
        self.video
            .admit_gpu_capture_budget(capture_work, &presentation)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            capture_work,
            self.limits.max_preparation_work,
        )?;
        let owner = memory.owner_id();
        let mut cursor = Cursor::Paused(memory);
        while let Some(action) = plan.next_step().map_err(Failure::GraphicsPreparation)? {
            let owned_draw;
            let command = match action {
                GraphicsStep::Array => {
                    plan.resolve_array(&super::super::Memory(cursor.paused_memory()))
                        .map_err(Failure::GraphicsPreparation)?;
                    continue;
                }
                GraphicsStep::StaticDraw(index) => plan.static_draw(index),
                GraphicsStep::OrderedDraw(draw) => {
                    owned_draw = draw;
                    &owned_draw
                }
                GraphicsStep::Memory(index) => {
                    // Consuming the current draw receipt invalidates it before
                    // every memory/barrier action, including empty writes. A
                    // later flip must follow a new actual owned draw.
                    let memory = cursor.into_pause();
                    match plan.memory_action(index) {
                        draw::MemoryAction::Write(write) => {
                            self.memory_write(memory, write, plan.command_ranges(&sources))?;
                        }
                        draw::MemoryAction::Wait(wait) => {
                            self.usage.preparation_work = add(
                                self.usage.preparation_work,
                                wait.byte_count() as u64 + 1,
                                self.limits.max_preparation_work,
                            )?;
                            self.usage.memory_commands =
                                self.usage.memory_commands.saturating_add(1);
                            if !wait
                                .satisfied(&super::super::Memory(memory))
                                .map_err(Failure::Preparation)?
                            {
                                return Err(Failure::Waiting);
                            }
                        }
                        draw::MemoryAction::Release(release) => {
                            self.retire_release(
                                memory,
                                release,
                                plan.command_ranges(&sources),
                                &mut kernel,
                                0,
                            )?;
                        }
                        draw::MemoryAction::Drain => self.drain_commands()?,
                    }
                    cursor = Cursor::Paused(memory);
                    continue;
                }
                GraphicsStep::Flip(flip) => {
                    let Cursor::Draw(current) = cursor else {
                        return Err(Failure::UnsupportedVideo);
                    };
                    let mut limits = presentation;
                    limits.max_held_bytes = limits
                        .max_held_bytes
                        .min(max_host_bytes.checked_sub(retained).ok_or(Failure::Limit)?);
                    let kernel = kernel.as_deref_mut().ok_or(Failure::UnsupportedVideo)?;
                    let clock = clock.ok_or(Failure::UnsupportedVideo)?;
                    let (memory, result) =
                        self.video
                            .submit_gpu_flip(current, flip, &limits, |memory| {
                                clock(kernel, memory)
                            });
                    result?;
                    cursor = Cursor::Paused(memory);
                    continue;
                }
            };
            let mut limits = self.limits.graphics;
            limits.max_work = limits.max_work.min(
                self.limits
                    .max_preparation_work
                    .saturating_sub(self.usage.preparation_work),
            );
            limits.max_vertices = limits
                .max_vertices
                .min(self.limits.max_vertices.saturating_sub(self.usage.vertices));
            limits.max_host_bytes = max_host_bytes
                .checked_sub(host_add(
                    retained,
                    self.video.retained_frame_bytes(),
                    max_host_bytes,
                )?)
                .ok_or(Failure::Limit)?;
            // Session staging covers simultaneous pixels and consumer memory,
            // not compiler IR/command plans. Both independent ceilings remain
            // enforced; previously captured Frames coexist with this draw.
            limits.max_pixel_bytes = limits.max_pixel_bytes.min(
                presentation
                    .max_held_bytes
                    .checked_sub(self.video.retained_frame_bytes())
                    .ok_or(Failure::Limit)?,
            );
            self.usage.draws = add(self.usage.draws, 1, self.limits.max_draws)?;
            let gpu_limits = self.limits.graphics_backend_limits();
            let current = super::super::graphics::execute_draw_current(
                cursor.into_pause(),
                command,
                &limits,
                &gpu_limits,
                plan.command_ranges(&sources),
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
                .saturating_add(current.written_bytes() as u64);
            cursor = Cursor::Draw(current);
        }
        self.graphics = plan.final_state().map_err(Failure::GraphicsPreparation)?;
        self.graphics_owner = Some(owner);
        self.frame.reset_before_graphics = false;
        self.poisoned = false;
        Ok(0)
    }
}

enum Cursor<'borrow, 'owner> {
    Paused(&'borrow mut PausedNativeMemory<'owner>),
    Draw(super::super::graphics::CurrentDraw<'borrow, 'owner>),
}
impl<'borrow, 'owner> Cursor<'borrow, 'owner> {
    fn paused_memory(&self) -> &PausedNativeMemory<'owner> {
        match self {
            Self::Paused(memory) => memory,
            Self::Draw(current) => current.paused_memory(),
        }
    }
    fn into_pause(self) -> &'borrow mut PausedNativeMemory<'owner> {
        match self {
            Self::Paused(memory) => memory,
            Self::Draw(current) => current.into_pause(),
        }
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

// The static variant preserves the existing no-array planner and its complete
// immutable draw snapshots. Only array streams use source-ordered replay.
// Inline sealed state avoids a fallible allocation after an earlier effect.
#[allow(clippy::large_enum_variant)]
enum GraphicsCommands {
    Static {
        plan: draw::CommandPlan,
        action: usize,
    },
    Ordered(draw::OrderedCommands),
}
// Inline sealed state avoids a fallible allocation after an earlier effect.
#[allow(clippy::large_enum_variant)]
enum GraphicsStep {
    Array,
    StaticDraw(usize),
    OrderedDraw(draw::Draw),
    Memory(usize),
    Flip(draw::Flip),
}
impl GraphicsCommands {
    fn usage(&self) -> draw::CommandUsage {
        match self {
            Self::Static { plan, .. } => plan.usage(),
            Self::Ordered(plan) => plan.usage(),
        }
    }
    fn flips(&self) -> impl Iterator<Item = draw::Flip> + '_ {
        let actions = match self {
            Self::Static { plan, .. } => plan.actions(),
            Self::Ordered(plan) => plan.static_actions(),
        };
        actions.iter().filter_map(|action| match action {
            draw::Action::Flip(flip) => Some(*flip),
            _ => None,
        })
    }
    fn command_ranges<'a>(
        &'a self,
        sources: &'a [compute::CommandRange],
    ) -> &'a [compute::CommandRange] {
        match self {
            Self::Static { .. } => sources,
            Self::Ordered(plan) => plan.command_ranges(),
        }
    }
    fn resolve_array(&mut self, memory: &impl compute::Memory) -> Result<(), draw::Error> {
        match self {
            Self::Ordered(plan) => plan.resolve_array(memory),
            _ => Err(draw::Error::Unsupported("static register array")),
        }
    }
    fn memory_action(&self, index: usize) -> &draw::MemoryAction {
        match self {
            Self::Static { plan, .. } => plan.memory_action(index),
            Self::Ordered(plan) => plan.memory_action(index),
        }
    }
    fn static_draw(&self, index: usize) -> &draw::Draw {
        match self {
            Self::Static { plan, .. } => &plan.draws()[index],
            _ => unreachable!(),
        }
    }
    fn final_state(&self) -> Result<draw::GraphicsState, draw::Error> {
        match self {
            Self::Static { plan, .. } => Ok(plan.final_state()),
            Self::Ordered(plan) => plan.final_state(),
        }
    }
    fn next_step(&mut self) -> Result<Option<GraphicsStep>, draw::Error> {
        Ok(match self {
            Self::Static { plan, action } => {
                let Some(next) = plan.actions().get(*action) else {
                    return Ok(None);
                };
                *action += 1;
                Some(match *next {
                    draw::Action::Draw(index) => GraphicsStep::StaticDraw(index),
                    draw::Action::Memory(index) => GraphicsStep::Memory(index),
                    draw::Action::Flip(flip) => GraphicsStep::Flip(flip),
                })
            }
            Self::Ordered(plan) => plan.next_step()?.map(|next| match next {
                draw::OrderedStep::Array => GraphicsStep::Array,
                draw::OrderedStep::Draw(draw) => GraphicsStep::OrderedDraw(draw),
                draw::OrderedStep::Memory(index) => GraphicsStep::Memory(index),
                draw::OrderedStep::Flip(flip) => GraphicsStep::Flip(flip),
            }),
        })
    }
}
