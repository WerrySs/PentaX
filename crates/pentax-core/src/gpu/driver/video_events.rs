// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: actual consumer observations to retained queues.
// Pinned videoOut.cpp registration/filter contracts; see core NOTICE.
use super::*;
use pentax_kernel::VideoEventKind;

impl Driver {
    pub(super) fn video_event_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        stack: Option<u64>,
        kernel: Option<&mut pentax_kernel::Kernel>,
    ) -> Result<u64, Failure> {
        self.video.check_owner(memory.owner_id())?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            64,
            self.limits.max_preparation_work,
        )?;
        if matches!(
            function,
            Function::VideoGetEventId | Function::VideoGetEventData | Function::VideoGetEventCount
        ) {
            if args[0] == 0 || (function == Function::VideoGetEventData && args[1] == 0) {
                return Ok(0x8029_0002);
            }
            let mut filter = [0; 2];
            memory
                .read(args[0].checked_add(8).ok_or(Failure::Memory)?, &mut filter)
                .map_err(|_| Failure::Memory)?;
            if i16::from_le_bytes(filter) != -13 {
                return Ok(0x8029_000d);
            }
            let mut bytes = [0; 8];
            if function == Function::VideoGetEventId {
                memory
                    .read(args[0], &mut bytes)
                    .map_err(|_| Failure::Memory)?;
                return Ok(match u64::from_le_bytes(bytes) {
                    0 | 1 | 2 | 8 => u64::from_le_bytes(bytes),
                    _ => 0x8029_000d,
                });
            }
            memory
                .read(args[0].checked_add(16).ok_or(Failure::Memory)?, &mut bytes)
                .map_err(|_| Failure::Memory)?;
            let data = u64::from_le_bytes(bytes);
            if function == Function::VideoGetEventCount {
                return Ok((data >> 12) & 15);
            }
            memory
                .read(args[0], &mut bytes)
                .map_err(|_| Failure::Memory)?;
            let payload = (data >> 16)
                | if u64::from_le_bytes(bytes) == 0 && data >> 63 != 0 {
                    0xffff_0000_0000_0000
                } else {
                    0
                };
            memory
                .write(args[1], &payload.to_le_bytes())
                .map_err(|_| Failure::Memory)?;
            return Ok(0);
        }
        let kernel = kernel.ok_or(Failure::UnsupportedVideo)?;
        let handle = if function == Function::VideoClose {
            args[0]
        } else {
            args[1]
        } as u32 as i32;
        let Some(port) = self.video.port(handle) else {
            return Ok(0x8029_000b);
        };
        if function == Function::VideoClose {
            let cleanup = kernel
                .prepare_video_close(handle, port.generation)
                .map_err(Failure::Kernel)?;
            let result = self.video.invoke(
                function,
                args,
                self.limits.video_vblank_hz,
                self.limits.video_display,
                stack,
                memory,
            )?;
            if result == 0 {
                cleanup.complete();
                if let Some(slot) = usize::try_from(handle - 1).ok().filter(|i| *i < 3) {
                    self.video_ticks[slot] = None;
                }
            }
            return Ok(result);
        }
        let kind = if matches!(
            function,
            Function::VideoAddFlipEvent | Function::VideoDeleteFlipEvent
        ) {
            VideoEventKind::Flip
        } else {
            VideoEventKind::Vblank
        };
        let result = if matches!(
            function,
            Function::VideoAddFlipEvent | Function::VideoAddVblankEvent
        ) {
            kernel.add_video_event(args[0], handle, port.generation, kind, args[2])
        } else {
            kernel.delete_video_event(args[0], kind)
        };
        match result {
            Err(pentax_kernel::Failure::Errno(pentax_kernel::Errno::BadFd)) => Ok(0x8029_000c),
            Err(pentax_kernel::Failure::Errno(error)) => Ok(u64::from(error.kernel_code() as u32)),
            other => other.map_err(Failure::Kernel),
        }
    }

    /// The completion is unforgeable and consumed only after real queue
    /// publication. Failure leaves the same owned completion available.
    pub(crate) fn video_deliver_completion(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        completion: &mut Option<super::super::video::FlipCompletion>,
        tsc: u64,
    ) -> Result<(), Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        let Some(receipt) = completion.as_ref() else {
            return Err(Failure::UnsupportedVideo);
        };
        if receipt.owner() != memory.owner_id()
            || self
                .video
                .port(receipt.handle())
                .is_none_or(|p| p.generation != receipt.generation())
        {
            return Err(Failure::Memory);
        }
        kernel
            .prepare_video_interrupt(
                receipt.handle(),
                receipt.generation(),
                VideoEventKind::Flip,
                receipt.flip_arg() as u64,
                tsc,
            )
            .map_err(Failure::Kernel)?
            .complete();
        *completion = None;
        Ok(())
    }
    pub(crate) fn video_preflight_vblank(
        &self,
        memory: &PausedNativeMemory<'_>,
        handle: i32,
        generation: u64,
        tick: u64,
    ) -> Result<usize, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        if tick == 0
            || self
                .video
                .port(handle)
                .is_none_or(|p| p.generation != generation)
        {
            return Err(Failure::UnsupportedVideo);
        }
        let slot = usize::try_from(handle - 1)
            .ok()
            .filter(|i| *i < 3)
            .ok_or(Failure::UnsupportedVideo)?;
        if self.video_ticks[slot].is_some_and(|(old_generation, old_tick)| {
            old_generation == generation && tick <= old_tick
        }) {
            return Err(Failure::UnsupportedVideo);
        }
        Ok(slot)
    }
    pub(crate) fn video_observe_vblank(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        handle: i32,
        generation: u64,
        tick: u64,
        clock: pentax_kernel::DisplayClock,
    ) -> Result<(), Failure> {
        let slot = self.video_preflight_vblank(memory, handle, generation, tick)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            super::super::video::State::grid_work(),
            self.limits.max_preparation_work,
        )?;
        self.video_materialize(memory, kernel, clock, None)?;
        // Consumer tick numbers authenticate notification order only. Display
        // periods come from the same grid as headless scheduling/query pumps.
        self.video_ticks[slot] = Some((generation, tick));
        Ok(())
    }
}
