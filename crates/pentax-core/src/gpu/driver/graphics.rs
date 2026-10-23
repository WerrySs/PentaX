// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: complete owned graphics submission admission.
// Source: pinned agc.cpp submit_command_buffer and graphicsRun.cpp Submit;
// draw/state contracts are documented by pentax-gpu/src/graphics/README.md.

use super::{Driver, Failure, NativeRange, PausedNativeMemory, add, compute, draw};
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

// Process-unique admission identities prevent cross-Driver and slot-reuse ABA.
// Like existing flip/submission identities, exhaustion refuses before effects.
static NEXT_GRAPHICS_REQUEST: AtomicU64 = AtomicU64::new(1);

impl super::Limits {
    fn graphics_backend_limits(self) -> pentax_platform::vulkan::GpuLimits {
        let mut limits = self.gpu;
        if let Some(work) = self.graphics_backend_work {
            limits.max_work = work;
        }
        limits
    }
}

pub(super) struct GraphicsExecution {
    pub(super) owner: pentax_platform::native_memory::NativeOwnerId,
    plan: GraphicsCommands,
    sources: Vec<compute::CommandRange>,
    staging: Vec<u8>,
    pub(super) retained: usize,
    max_host_bytes: usize,
    presentation: crate::gpu::video::PresentationLimits,
    deferred_flips: bool,
    pending: Option<SelectedWait>,
    pub(super) effect_started: bool,
    pub(super) generation: u64,
    pub(super) wait_generation: u64,
}
#[derive(Clone, Copy)]
struct SelectedWait {
    handle: i32,
    buffer: i32,
    port_generation: u64,
}
enum GraphicsAdvance {
    Complete(u64),
    Waiting,
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
        kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<u64, Failure> {
        self.graphics_check_owner(memory)?;
        if self.graphics_execution.is_some() || !self.graphics_admissions.is_empty() {
            return Err(Failure::Waiting);
        }
        let mut kernel = kernel;
        let Some(mut execution) =
            self.prepare_graphics_execution(memory, ranges, kernel.as_deref_mut(), clock)?
        else {
            return Ok(0);
        };
        // This legacy scalar-only route cannot publish a continuation ticket.
        // Refuse a wait stream before effects; native actions use the owned route.
        if execution.deferred_flips {
            return Err(Failure::UnsupportedQueue);
        }
        // Preserve the synchronous route for every existing no-wait stream.
        match self.advance_graphics_execution(memory, &mut execution, kernel, clock)? {
            GraphicsAdvance::Complete(value) => Ok(value),
            GraphicsAdvance::Waiting => {
                self.poisoned |= execution.effect_started;
                Err(Failure::Waiting)
            }
        }
    }

    pub(super) fn prepare_graphics_execution(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        ranges: &[NativeRange],
        kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<Option<GraphicsExecution>, Failure> {
        if self
            .graphics_owner
            .is_some_and(|owner| owner != memory.owner_id())
        {
            return Err(Failure::Memory);
        }
        let max_host_bytes = self.graphics_host_limit()?;
        let command_host_bytes = max_host_bytes
            .checked_sub(self.video.retained_frame_bytes())
            .ok_or(Failure::Limit)?;
        let mut presentation = self.presentation_limits()?;
        let count = ranges
            .iter()
            .filter(|range| range.address != 0 && range.size != 0)
            .count();
        if count == 0 {
            return Ok(None);
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
        let plan = if captured.contains_arrays() || captured.contains_draw_arguments() {
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
        let staging = if plan.usage().staging_bytes == 0 {
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
        retained = host_add(retained, size_of::<GraphicsExecution>(), command_host_bytes)?;
        presentation.max_held_bytes = presentation
            .max_held_bytes
            .checked_sub(retained)
            .ok_or(Failure::Limit)?;
        let deferred_flips = plan.has_wait_flip();
        if !deferred_flips {
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
                let tight =
                    usize::try_from(u64::from(surface.width) * u64::from(surface.height) * 4)
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
        }
        Ok(Some(GraphicsExecution {
            owner: memory.owner_id(),
            plan,
            sources,
            staging,
            retained,
            max_host_bytes,
            presentation,
            deferred_flips,
            pending: None,
            effect_started: false,
            generation: 0,
            wait_generation: 0,
        }))
    }
    fn graphics_selected_wait(
        &self,
        memory: &PausedNativeMemory<'_>,
        wait: SelectedWait,
    ) -> Result<bool, Failure> {
        self.video.check_owner(memory.owner_id())?;
        if self
            .video
            .port(wait.handle)
            .is_none_or(|port| port.generation != wait.port_generation)
        {
            return Err(Failure::UnsupportedVideo);
        }
        Ok(self.video.buffer_pending(wait.handle, wait.buffer))
    }
    fn admit_graphics_flip_turn(
        &mut self,
        flip: draw::Flip,
        retained: usize,
        maximum: usize,
        limits: &crate::gpu::video::PresentationLimits,
    ) -> Result<(), Failure> {
        // Wait streams admit CURRENT queue occupancy at the actual flip turn.
        // This neither reserves a future slot nor claims a consumer completed.
        let surface = self.video.gpu_capture_surface(flip)?;
        if self.video.gpu_pending(flip.handle())? >= 16 {
            return Err(Failure::Waiting);
        }
        let tight = usize::try_from(u64::from(surface.width) * u64::from(surface.height) * 4)
            .map_err(|_| Failure::Limit)?;
        let source = usize::try_from(surface.data.size).map_err(|_| Failure::Limit)?;
        if source > limits.max_frame_bytes || tight > limits.max_frame_bytes {
            return Err(Failure::Limit);
        }
        let peak = self
            .video
            .retained_frame_bytes()
            .checked_add(source)
            .and_then(|n| n.checked_add(tight))
            .ok_or(Failure::Limit)?;
        if peak > limits.max_held_bytes {
            return Err(Failure::Limit);
        }
        host_add(retained, peak, maximum)?;
        let work = surface
            .data
            .size
            .checked_add(tight as u64 * 5 + 256)
            .ok_or(Failure::Limit)?;
        self.video.admit_gpu_capture_budget(work, limits)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        Ok(())
    }
    fn advance_graphics_execution(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        execution: &mut GraphicsExecution,
        mut kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<GraphicsAdvance, Failure> {
        let owner = execution.owner;
        if owner != memory.owner_id() {
            return Err(Failure::Memory);
        }
        if let Some(wait) = execution.pending {
            if self.graphics_selected_wait(memory, wait)? {
                return Ok(GraphicsAdvance::Waiting);
            }
            execution.pending = None;
        }
        let retained = execution.retained;
        let max_host_bytes = execution.max_host_bytes;
        let presentation = execution.presentation;
        let deferred_flips = execution.deferred_flips;
        let plan = &mut execution.plan;
        let sources = &execution.sources;
        let staging = &mut execution.staging;
        let mut cursor = Cursor::Paused(memory);
        while let Some(action) = plan.next_step().map_err(Failure::GraphicsPreparation)? {
            let owned_draw;
            let command = match action {
                GraphicsStep::WaitFlip(wait) => {
                    let memory = cursor.into_pause();
                    self.drain_commands()?;
                    self.video.check_owner(memory.owner_id())?;
                    if !(-2..16).contains(&wait.buffer()) {
                        return Err(Failure::UnsupportedVideo);
                    }
                    let port = self
                        .video
                        .port(wait.handle())
                        .ok_or(Failure::UnsupportedVideo)?;
                    let selected = SelectedWait {
                        handle: wait.handle(),
                        buffer: wait.buffer(),
                        port_generation: port.generation,
                    };
                    if self.graphics_selected_wait(memory, selected)? {
                        execution.wait_generation = execution
                            .wait_generation
                            .checked_add(1)
                            .ok_or(Failure::Limit)?;
                        execution.pending = Some(selected);
                        execution.effect_started |= self.poisoned;
                        return Ok(GraphicsAdvance::Waiting);
                    }
                    cursor = Cursor::Paused(memory);
                    continue;
                }
                GraphicsStep::LodStats(request) => {
                    let memory = cursor.into_pause();
                    // Earlier queued flips retain their owned Frames while
                    // this report/reset runs, just as in the ordinary draw path.
                    let held = retained
                        .checked_add(self.video.retained_frame_bytes())
                        .ok_or(Failure::Limit)?;
                    self.lod_report(
                        memory,
                        request,
                        plan.command_ranges(sources),
                        held,
                        max_host_bytes,
                    )?;
                    cursor = Cursor::Paused(memory);
                    continue;
                }
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
                            self.memory_write(memory, write, plan.command_ranges(sources))?;
                            cursor = Cursor::Paused(memory);
                        }
                        draw::MemoryAction::Transfer(transfer) => {
                            let memory = cursor.into_pause();
                            self.transfer_command(
                                memory,
                                transfer,
                                plan.command_ranges(sources),
                                staging,
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
                                plan.command_ranges(sources),
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
                    if deferred_flips {
                        self.admit_graphics_flip_turn(
                            flip,
                            retained,
                            max_host_bytes,
                            &presentation,
                        )?;
                    }
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
            let initial_lod = self
                .lod_stats
                .as_ref()
                .map(|state| state.counters(owner))
                .transpose()?;
            let mut current = super::super::graphics::execute_draw_current_with_lod(
                cursor.into_pause(),
                command,
                &limits,
                &gpu_limits,
                plan.command_ranges(sources),
                initial_lod,
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
            if let Some(counters) = current.take_lod_counters() {
                self.lod_stats
                    .as_mut()
                    .ok_or(Failure::UnsupportedQueue)?
                    .accept_draw(owner, counters)?;
            }
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
        Ok(GraphicsAdvance::Complete(0))
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

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "graphics_continuation_tests.rs"]
mod continuation_tests;

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
    WaitFlip(pentax_gpu::WaitFlip),
    LodStats(pentax_gpu::lod_stats_request::Request),
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
    fn has_wait_flip(&self) -> bool {
        let actions = match self {
            Self::Static { plan, .. } => plan.actions(),
            Self::Ordered(plan) => plan.static_actions(),
        };
        actions
            .iter()
            .any(|action| matches!(action, draw::Action::WaitFlip(_)))
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
                    draw::Action::WaitFlip(wait) => GraphicsStep::WaitFlip(wait),
                    draw::Action::LodStats(request) => GraphicsStep::LodStats(request),
                    draw::Action::Memory(index) => GraphicsStep::Memory(index),
                    draw::Action::Flip(flip) => GraphicsStep::Flip(flip),
                })
            }
            Self::Ordered(plan) => plan.next_step()?.map(|next| match next {
                draw::OrderedStep::Array => GraphicsStep::Array,
                draw::OrderedStep::WaitFlip(wait) => GraphicsStep::WaitFlip(wait),
                draw::OrderedStep::LodStats(request) => GraphicsStep::LodStats(request),
                draw::OrderedStep::IndirectDraw => GraphicsStep::IndirectDraw,
                draw::OrderedStep::Draw(draw) => GraphicsStep::OrderedDraw(draw),
                draw::OrderedStep::Memory(index) => GraphicsStep::Memory(index),
                draw::OrderedStep::Flip(flip) => GraphicsStep::Flip(flip),
            }),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphicsTicketKind {
    Admission,
    Selected,
}
/// Driver-only continuation proof. The native thread-wait wrapper binds its
/// exact original syscall/source/TLS token; these fields grant no memory authority.
#[derive(Clone, Copy)]
pub(crate) struct GraphicsTicket {
    owner: pentax_platform::native_memory::NativeOwnerId,
    generation: u64,
    wait_generation: u64,
    kind: GraphicsTicketKind,
}
impl std::fmt::Debug for GraphicsTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphicsTicket").finish_non_exhaustive()
    }
}
// Native HLE publishes a retained wait until the complete submission retires.
pub(crate) enum GraphicsProgress {
    Complete(u64),
    Waiting(GraphicsTicket),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GraphicsPhase {
    Idle,
    Running,
    Parked,
    Failed,
}
pub(super) struct GraphicsAdmission {
    owner: pentax_platform::native_memory::NativeOwnerId,
    generation: u64,
    // None denotes only the closed SuspendPoint boundary, never a guest opcode.
    ranges: Option<Vec<NativeRange>>,
}

impl Driver {
    fn graphics_check_owner(&self, memory: &PausedNativeMemory<'_>) -> Result<(), Failure> {
        let owner = memory.owner_id();
        self.video.check_owner(owner)?;
        if self.graphics_owner.is_some_and(|old| old != owner)
            || self
                .graphics_execution
                .as_ref()
                .is_some_and(|run| run.owner != owner)
            || self
                .graphics_admissions
                .first()
                .is_some_and(|item| item.owner != owner)
        {
            return Err(Failure::Memory);
        }
        Ok(())
    }
    pub(super) fn graphics_retained_bytes(&self) -> Result<usize, Failure> {
        let mut bytes = self
            .graphics_admissions
            .capacity()
            .checked_mul(size_of::<GraphicsAdmission>())
            .ok_or(Failure::Limit)?;
        for item in &self.graphics_admissions {
            let range_bytes = match &item.ranges {
                Some(ranges) => ranges
                    .capacity()
                    .checked_mul(size_of::<NativeRange>())
                    .ok_or(Failure::Limit)?,
                None => 0,
            };
            bytes = bytes.checked_add(range_bytes).ok_or(Failure::Limit)?;
        }
        bytes
            .checked_add(
                self.graphics_execution
                    .as_ref()
                    .map_or(0, |run| run.retained),
            )
            .ok_or(Failure::Limit)
    }
    fn graphics_held_bytes(&self) -> Result<usize, Failure> {
        self.graphics_retained_bytes()?
            .checked_add(self.video.retained_frame_bytes())
            .ok_or(Failure::Limit)
    }
    fn graphics_admission_storage(&mut self) -> Result<(), Failure> {
        if self.graphics_admissions.capacity() != 0 {
            return Ok(());
        }
        let count = (self.limits.max_batch_buffers as usize)
            .min(usize::try_from(self.limits.max_calls).map_err(|_| Failure::Limit)?);
        if count == 0 {
            return Err(Failure::Limit);
        }
        let bytes = count
            .checked_mul(size_of::<GraphicsAdmission>())
            .ok_or(Failure::Limit)?;
        let ceiling = self.graphics_host_ceiling()?;
        host_add(self.graphics_held_bytes()?, bytes, ceiling)?;
        let mut storage = Vec::new();
        storage
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        let actual = storage
            .capacity()
            .checked_mul(size_of::<GraphicsAdmission>())
            .ok_or(Failure::Limit)?;
        host_add(self.graphics_held_bytes()?, actual, ceiling)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            actual as u64,
            self.limits.max_preparation_work,
        )?;
        self.graphics_admissions = storage;
        Ok(())
    }
    fn graphics_enqueue(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        ranges: Option<&[NativeRange]>,
    ) -> Result<super::GraphicsTicket, Failure> {
        if self.poisoned || self.graphics_phase == GraphicsPhase::Failed {
            return Err(Failure::Poisoned);
        }
        self.graphics_check_owner(memory)?;
        self.graphics_admission_storage()?;
        if self.graphics_admissions.len() >= self.limits.max_batch_buffers as usize
            || self.graphics_admissions.len() == self.graphics_admissions.capacity()
        {
            return Err(Failure::Limit);
        }
        let generation = NEXT_GRAPHICS_REQUEST
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Failure::Limit)?;
        let ranges = if let Some(input) = ranges {
            if input.len() > self.limits.max_batch_buffers as usize {
                return Err(Failure::Limit);
            }
            let bytes = input
                .len()
                .checked_mul(size_of::<NativeRange>())
                .ok_or(Failure::Limit)?;
            host_add(
                self.graphics_held_bytes()?,
                bytes,
                self.graphics_host_ceiling()?,
            )?;
            let mut values = Vec::new();
            values
                .try_reserve_exact(input.len())
                .map_err(|_| Failure::Allocation)?;
            let actual = values
                .capacity()
                .checked_mul(size_of::<NativeRange>())
                .ok_or(Failure::Limit)?;
            host_add(
                self.graphics_held_bytes()?,
                actual,
                self.graphics_host_ceiling()?,
            )?;
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                actual as u64,
                self.limits.max_preparation_work,
            )?;
            values.extend_from_slice(input);
            Some(values)
        } else {
            None
        };
        self.graphics_admissions.push(GraphicsAdmission {
            owner: memory.owner_id(),
            generation,
            ranges,
        });
        Ok(GraphicsTicket {
            owner: memory.owner_id(),
            generation,
            wait_generation: 0,
            kind: GraphicsTicketKind::Admission,
        })
    }
    /// Begin one owned execution after exact native call admission.
    pub(crate) fn begin_graphics_execution(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        ranges: &[NativeRange],
        kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<super::GraphicsProgress, Failure> {
        let ticket = self.graphics_enqueue(memory, Some(ranges))?;
        self.resume_graphics_execution(memory, ticket, kernel, clock)
    }
    /// A serialized source SuspendPoint boundary under the original native token.
    pub(crate) fn queue_graphics_suspend(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<super::GraphicsProgress, Failure> {
        let ticket = self.graphics_enqueue(memory, None)?;
        self.resume_graphics_execution(memory, ticket, kernel, clock)
    }
    pub(crate) fn graphics_ticket_ready(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        ticket: &super::GraphicsTicket,
    ) -> Result<bool, Failure> {
        // A peer's actual terminal failure always wins over saved prefix state.
        if self.poisoned || self.graphics_phase == GraphicsPhase::Failed {
            return Err(Failure::Poisoned);
        }
        if ticket.owner != memory.owner_id() {
            return Err(Failure::Memory);
        }
        self.graphics_check_owner(memory)?;
        // Authenticate before any budget/predicate error may fail this queue.
        // An old, foreign or already-completed ticket cannot poison its owner.
        match ticket.kind {
            GraphicsTicketKind::Admission => {
                if ticket.wait_generation != 0
                    || !self.graphics_admissions.iter().any(|item| {
                        item.generation == ticket.generation && item.owner == ticket.owner
                    })
                {
                    return Err(Failure::UnsupportedQueue);
                }
            }
            GraphicsTicketKind::Selected => {
                if !self.graphics_execution.as_ref().is_some_and(|run| {
                    run.owner == ticket.owner
                        && run.generation == ticket.generation
                        && run.wait_generation == ticket.wait_generation
                        && run.pending.is_some()
                }) {
                    return Err(Failure::UnsupportedQueue);
                }
            }
        }
        let result = (|| {
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                64 + self.graphics_admissions.len() as u64,
                self.limits.max_preparation_work,
            )?;
            match ticket.kind {
                GraphicsTicketKind::Admission => Ok(self.graphics_execution.is_none()
                    && self.graphics_phase == GraphicsPhase::Idle
                    && self
                        .graphics_admissions
                        .first()
                        .is_some_and(|item| item.generation == ticket.generation)),
                GraphicsTicketKind::Selected => {
                    let wait = self
                        .graphics_execution
                        .as_ref()
                        .and_then(|run| run.pending)
                        .ok_or(Failure::UnsupportedQueue)?;
                    Ok(!self.graphics_selected_wait(memory, wait)?)
                }
            }
        })();
        if result.is_err() {
            // The exact admitted request failed, retaining any active prefix
            // and allocations until authenticated owner teardown.
            self.graphics_phase = GraphicsPhase::Failed;
            self.poisoned = true;
        }
        result
    }

    fn fail_graphics_admission(&mut self, error: Failure) -> Failure {
        self.graphics_phase = GraphicsPhase::Failed;
        self.poisoned = true;
        error
    }
    pub(crate) fn resume_graphics_execution(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        ticket: super::GraphicsTicket,
        mut kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<super::GraphicsClock>,
    ) -> Result<super::GraphicsProgress, Failure> {
        if !self.graphics_ticket_ready(memory, &ticket)? {
            return Ok(GraphicsProgress::Waiting(ticket));
        }
        let mut execution = match ticket.kind {
            GraphicsTicketKind::Selected => self
                .graphics_execution
                .take()
                .ok_or(Failure::UnsupportedQueue)
                .map_err(|error| self.fail_graphics_admission(error))?,
            GraphicsTicketKind::Admission => {
                // Readiness proved this exact FIFO head. Charge all bounded
                // shifts before mutation; never let another request overtake.
                self.usage.preparation_work = add(
                    self.usage.preparation_work,
                    (self.graphics_admissions.len() * size_of::<GraphicsAdmission>()) as u64,
                    self.limits.max_preparation_work,
                )
                .map_err(|error| self.fail_graphics_admission(error))?;
                let boundary = if self.graphics_admissions[0].ranges.is_none() {
                    Some(
                        self.frame
                            .boundaries
                            .checked_add(1)
                            .ok_or(Failure::Limit)
                            .map_err(|error| self.fail_graphics_admission(error))?,
                    )
                } else {
                    None
                };
                let admission = self.graphics_admissions.remove(0);
                if let Some(boundary) = boundary {
                    self.frame.boundaries = boundary;
                    self.frame.reset_before_graphics = true;
                    return Ok(GraphicsProgress::Complete(0));
                }
                let prepared = self.prepare_graphics_execution(
                    memory,
                    admission.ranges.as_ref().unwrap(),
                    kernel.as_deref_mut(),
                    clock,
                );
                let mut run = match prepared {
                    Ok(Some(run)) => run,
                    Ok(None) => return Ok(GraphicsProgress::Complete(0)),
                    Err(error) => {
                        self.graphics_phase = GraphicsPhase::Failed;
                        self.poisoned = true;
                        return Err(error);
                    }
                };
                run.generation = admission.generation;
                run
            }
        };
        self.graphics_phase = GraphicsPhase::Running;
        // Restore only the currently authenticated prefix after checking global
        // terminal poison above. No peer failure is overwritten on resume.
        self.poisoned = execution.effect_started;
        let outcome = (|| {
            execution.max_host_bytes = execution.max_host_bytes.min(self.graphics_host_limit()?);
            if execution.retained > execution.max_host_bytes {
                return Err(Failure::Limit);
            }
            let current = self.presentation_limits()?;
            execution.presentation.max_frame_bytes = execution
                .presentation
                .max_frame_bytes
                .min(current.max_frame_bytes);
            execution.presentation.max_held_bytes = execution.presentation.max_held_bytes.min(
                current
                    .max_held_bytes
                    .checked_sub(execution.retained)
                    .ok_or(Failure::Limit)?,
            );
            execution.presentation.max_work = execution.presentation.max_work.min(current.max_work);
            self.advance_graphics_execution(memory, &mut execution, kernel, clock)
        })();
        execution.effect_started |= self.poisoned;
        match outcome {
            Ok(GraphicsAdvance::Complete(value)) => {
                self.graphics_phase = GraphicsPhase::Idle;
                self.poisoned = false;
                Ok(GraphicsProgress::Complete(value))
            }
            Ok(GraphicsAdvance::Waiting) => {
                let next = GraphicsTicket {
                    owner: execution.owner,
                    generation: execution.generation,
                    wait_generation: execution.wait_generation,
                    kind: GraphicsTicketKind::Selected,
                };
                // Infallible publication comes BEFORE clearing the transient
                // latch. The exact prefix/plan is always InFlight or Failed.
                self.graphics_execution = Some(execution);
                self.graphics_phase = GraphicsPhase::Parked;
                self.poisoned = false;
                Ok(GraphicsProgress::Waiting(next))
            }
            Err(error) => {
                self.graphics_execution = Some(execution);
                self.graphics_phase = GraphicsPhase::Failed;
                self.poisoned = true;
                Err(error)
            }
        }
    }
    pub(crate) fn has_pending_graphics_continuation(&self) -> bool {
        self.graphics_execution.is_some() || !self.graphics_admissions.is_empty()
    }

    /// Failure cleanup is independent of remaining work/diagnostic budgets.
    /// Only the actual owner teardown may release its retained capacities.
    pub(crate) fn revoke_graphics_owner(
        &mut self,
        owner: pentax_platform::native_memory::NativeOwnerId,
    ) -> Result<(), Failure> {
        if self
            .graphics_execution
            .as_ref()
            .is_some_and(|run| run.owner != owner)
            || self
                .graphics_admissions
                .iter()
                .any(|item| item.owner != owner)
            || self.graphics_owner.is_some_and(|old| old != owner)
        {
            return Err(Failure::Memory);
        }
        self.video.check_owner(owner)?;
        self.graphics_execution = None;
        self.graphics_admissions = Vec::new();
        self.graphics_phase = GraphicsPhase::Failed;
        self.poisoned = true;
        Ok(())
    }
}
