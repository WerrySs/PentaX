// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: complete owned graphics submission admission.
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
    /// Snapshot and plan every root and nested stream before executing any draw. The
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
        // The GPU capture admits the entire root/IB closure against the
        // remaining session bytes and buffers before any command effect.
        commands.max_command_bytes = commands.max_command_bytes.min(
            usize::try_from(
                self.limits
                    .max_command_bytes
                    .saturating_sub(self.usage.command_bytes),
            )
            .unwrap_or(usize::MAX),
        );
        commands.max_buffers = commands.max_buffers.min(
            usize::try_from(
                self.limits
                    .max_command_buffers
                    .saturating_sub(self.usage.command_buffers),
            )
            .unwrap_or(usize::MAX),
        );
        if count > commands.max_buffers {
            return Err(Failure::Limit);
        }
        // The caller's admitted NativeRange vector coexists with capture and
        // planning. The temporary root descriptors coexist with capture only.
        let caller_bytes = (self.limits.max_batch_buffers as usize)
            .checked_mul(size_of::<NativeRange>())
            .ok_or(Failure::Limit)?;
        let metadata = count
            .checked_mul(size_of::<compute::CommandRange>())
            .and_then(|bytes| bytes.checked_add(caller_bytes))
            .ok_or(Failure::Limit)?;
        let mut held = host_add(0, metadata, command_host_bytes)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            metadata as u64,
            self.limits.max_preparation_work,
        )?;
        let mut roots = Vec::<compute::CommandRange>::new();
        roots
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        held = host_add(
            held,
            roots
                .capacity()
                .checked_sub(count)
                .and_then(|n| n.checked_mul(size_of::<compute::CommandRange>()))
                .ok_or(Failure::Limit)?,
            command_host_bytes,
        )?;
        for range in ranges
            .iter()
            .filter(|range| range.address != 0 && range.size != 0)
        {
            roots.push(compute::CommandRange {
                address: range.address,
                size: range.size,
            });
        }
        let mut capture_limits = commands;
        capture_limits.max_host_bytes = capture_limits
            .max_host_bytes
            .min(command_host_bytes.checked_sub(held).ok_or(Failure::Limit)?);
        capture_limits.max_work = capture_limits.max_work.min(
            self.limits
                .max_preparation_work
                .saturating_sub(self.usage.preparation_work),
        );
        let captured_attempt = draw::CapturedCommands::capture_attempt(
            &roots,
            &super::super::Memory(memory),
            &capture_limits,
        );
        // A refused nested capture has already read/parsed an admitted prefix.
        // Preserve its attempted usage before returning the causal failure;
        // retrying malformed input cannot reset cumulative session budgets.
        let capture = match &captured_attempt {
            Ok(captured) => captured.usage(),
            Err(failure) => failure.usage,
        };
        self.usage.command_bytes = add(
            self.usage.command_bytes,
            capture.command_bytes as u64,
            self.limits.max_command_bytes,
        )?;
        self.usage.command_buffers = add(
            self.usage.command_buffers,
            capture.captured_buffers as u64,
            self.limits.max_command_buffers,
        )?;
        self.usage.indirect_buffers = self
            .usage
            .indirect_buffers
            .checked_add(capture.indirect_buffers as u64)
            .ok_or(Failure::Limit)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            capture.work,
            self.limits.max_preparation_work,
        )?;
        let mut captured = captured_attempt.map_err(|failure| match failure.error {
            draw::Error::Memory(_) => Failure::Memory,
            error => Failure::GraphicsPreparation(error),
        })?;
        drop(roots);
        // Capture storage and the returned plan coexist during preparation.
        // Its bounded peak includes immutable expanded bytes and provenance;
        // planning receives only the remaining independent host/work envelope.
        let planning_held = host_add(caller_bytes, capture.host_bytes, command_host_bytes)?;
        commands.max_host_bytes = commands
            .max_host_bytes
            .checked_sub(capture.host_bytes)
            .ok_or(Failure::Limit)?
            .min(
                command_host_bytes
                    .checked_sub(planning_held)
                    .ok_or(Failure::Limit)?,
            );
        commands.max_work = commands
            .max_work
            .checked_sub(capture.work)
            .ok_or(Failure::Limit)?
            .min(
                self.limits
                    .max_preparation_work
                    .saturating_sub(self.usage.preparation_work),
            );
        let initial = if self.frame.reset_before_graphics {
            draw::GraphicsState::empty()
        } else {
            self.graphics
        };
        let mut plan = if captured.contains_arrays() || captured.contains_draw_arguments() {
            GraphicsCommands::Ordered(
                captured
                    .prepare_ordered(initial, &commands)
                    .map_err(Failure::GraphicsPreparation)?,
            )
        } else {
            GraphicsCommands::Static {
                plan: captured
                    .plan(initial, &commands)
                    .map_err(Failure::GraphicsPreparation)?,
                action: 0,
            }
        };
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            plan.usage().work,
            self.limits.max_preparation_work,
        )?;
        // Moving the complete captured source ranges allocates nothing. Static
        // execution keeps them for write exclusions; ordered execution owns
        // its independent ranges, including dynamically resolved arrays.
        let sources = captured.into_command_ranges();
        let mut retained = plan
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
        let mut staging = if plan.usage().staging_bytes == 0 {
            Vec::new()
        } else {
            let staging_limit = retained
                .checked_add(
                    commands
                        .max_host_bytes
                        .checked_sub(plan.usage().host_bytes)
                        .ok_or(Failure::Limit)?,
                )
                .ok_or(Failure::Limit)?
                .min(command_host_bytes);
            let staging = self.transfer_staging(
                plan.usage().staging_bytes,
                plan.usage().write_bytes,
                retained,
                staging_limit,
            )?;
            retained = host_add(retained, staging.capacity(), command_host_bytes)?;
            staging
        };
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
                GraphicsStep::IndirectDraw => {
                    let memory = cursor.into_pause();
                    let resolved = plan
                        .resolve_indirect(&super::super::Memory(memory))
                        .map_err(Failure::GraphicsPreparation)?;
                    // Source patches/NUM_INSTANCES are a real state transition,
                    // including zero work; no prior draw receipt survives it.
                    self.poisoned = true;
                    cursor = Cursor::Paused(memory);
                    let Some(draw) = resolved else {
                        continue;
                    };
                    owned_draw = draw;
                    &owned_draw
                }
                GraphicsStep::StaticDraw(index) => plan.static_draw(index),
                GraphicsStep::OrderedDraw(draw) => {
                    owned_draw = draw;
                    &owned_draw
                }
                GraphicsStep::Memory(index) => {
                    match plan.memory_action(index) {
                        draw::MemoryAction::Synchronize(sync) => {
                            // The admitted pure barrier preserves the current
                            // fenced target and the unique pause loan.
                            self.synchronize_commands(sync)?;
                        }
                        draw::MemoryAction::Annotation(marker) => {
                            self.annotation_command(marker)?;
                        }
                        draw::MemoryAction::Drain => {
                            // All prior work already fenced and wrote back.
                            // Counter admission cannot alter guest backing,
                            // target/configuration or the unique pause loan.
                            self.drain_commands()?;
                        }
                        draw::MemoryAction::Write(write) => {
                            let memory = cursor.into_pause();
                            self.memory_write(memory, write, plan.command_ranges(&sources))?;
                            cursor = Cursor::Paused(memory);
                        }
                        draw::MemoryAction::Transfer(transfer) => {
                            let memory = cursor.into_pause();
                            self.transfer_command(
                                memory,
                                transfer,
                                plan.command_ranges(&sources),
                                &mut staging,
                            )?;
                            cursor = Cursor::Paused(memory);
                        }
                        draw::MemoryAction::Wait(wait) => {
                            let memory = cursor.into_pause();
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
                            cursor = Cursor::Paused(memory);
                        }
                        draw::MemoryAction::Release(release) => {
                            let memory = cursor.into_pause();
                            self.retire_release(
                                memory,
                                release,
                                plan.command_ranges(&sources),
                                &mut kernel,
                                0,
                            )?;
                            cursor = Cursor::Paused(memory);
                        }
                    }
                    // Every non-barrier action consumed the prior draw receipt,
                    // including empty writes and read-only waits. A failed
                    // action returns before any later flip can consume it.
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
#[path = "graphics_indirect_tests.rs"]
mod indirect_tests;
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
    IndirectDraw,
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
    fn resolve_indirect(
        &mut self,
        memory: &impl compute::Memory,
    ) -> Result<Option<draw::Draw>, draw::Error> {
        match self {
            Self::Ordered(plan) => plan.resolve_indirect(memory),
            _ => Err(draw::Error::Unsupported("static indirect draw")),
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
                    draw::Action::IndirectDraw(_) | draw::Action::LiveDraw => {
                        return Err(draw::Error::Unsupported("static live draw action"));
                    }
                    draw::Action::Memory(index) => GraphicsStep::Memory(index),
                    draw::Action::Flip(flip) => GraphicsStep::Flip(flip),
                })
            }
            Self::Ordered(plan) => plan.next_step()?.map(|next| match next {
                draw::OrderedStep::Array => GraphicsStep::Array,
                draw::OrderedStep::IndirectDraw => GraphicsStep::IndirectDraw,
                draw::OrderedStep::Draw(draw) => GraphicsStep::OrderedDraw(draw),
                draw::OrderedStep::Memory(index) => GraphicsStep::Memory(index),
                draw::OrderedStep::Flip(flip) => GraphicsStep::Flip(flip),
            }),
        })
    }
}
