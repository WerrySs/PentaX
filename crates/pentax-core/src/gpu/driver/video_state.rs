// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: original-call vector ABI for owned VideoOut state.
use super::*;

impl Driver {
    pub(crate) fn submit_video_gamma(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
        vectors: Option<[[u8; 16]; 8]>,
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.usage.calls = add(self.usage.calls, 1, self.limits.max_calls)?;
        self.function_profile
            .record(Function::VideoColorSettingsSetGamma);
        self.video_state_call(memory, Function::VideoColorSettingsSetGamma, args, vectors)
    }
    pub(super) fn video_state_call(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        vectors: Option<[[u8; 16]; 8]>,
    ) -> Result<u64, Failure> {
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            128,
            self.limits.max_preparation_work,
        )?;
        self.video.color_or_vblank(function, args, vectors, memory)
    }
}

impl Driver {
    pub(crate) fn submit_video_clocked(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        kernel: &mut pentax_kernel::Kernel,
        clock: impl FnOnce(
            &mut pentax_kernel::Kernel,
            &mut PausedNativeMemory<'_>,
        ) -> Result<pentax_kernel::DisplayClock, Failure>,
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.usage.calls = add(self.usage.calls, 1, self.limits.max_calls)?;
        self.function_profile.record(function);
        self.charge_video_grid()?;
        if function != Function::VideoGetVblankStatus {
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                super::super::video::State::call_work(),
                self.limits.max_preparation_work,
            )?;
        }
        let call = self.video.prepare_clocked_call(
            function,
            args,
            self.limits.video_vblank_hz,
            self.limits.video_display,
            memory,
        )?;
        if (function == Function::VideoOpen && !(1..=3).contains(&call.result))
            || (function != Function::VideoOpen && call.result != 0)
        {
            return Ok(call.result);
        }
        let result = call.result;
        let clock = clock(kernel, memory)?;
        self.video_materialize(memory, kernel, clock, Some(call))?;
        Ok(result)
    }
    fn charge_video_grid(&mut self) -> Result<(), Failure> {
        // Covers fixed metadata staging/copying, all three arithmetic grids,
        // and the bounded retained-subscription scan. No per-period loop.
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            super::super::video::State::grid_work(),
            self.limits.max_preparation_work,
        )?;
        Ok(())
    }
    pub(super) fn video_materialize(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        clock: pentax_kernel::DisplayClock,
        call: Option<super::super::video::ClockedCall>,
    ) -> Result<(), Failure> {
        let update = self.video.prepare_grid(
            clock,
            self.limits.video_vblank_hz,
            self.limits.video_display,
            call,
        )?;
        let closing = update.close;
        if closing.is_none() && update.boundaries.iter().all(Option::is_none) {
            return self.video.commit_grid(update, memory);
        }
        let events = kernel
            .prepare_video_boundary_close(update.boundaries, closing)
            .map_err(Failure::Kernel)?;
        self.video.commit_grid(update, memory)?;
        events.complete();
        if let Some((handle, _)) = closing {
            self.video_ticks[(handle - 1) as usize] = None;
        }
        Ok(())
    }
    pub(crate) fn video_poll_clocked(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        clock: impl FnOnce(
            &mut pentax_kernel::Kernel,
            &mut PausedNativeMemory<'_>,
        ) -> Result<pentax_kernel::DisplayClock, Failure>,
    ) -> Result<Option<std::time::Duration>, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        if !self.video.has_clocked_port() {
            return Ok(None);
        }
        self.charge_video_grid()?;
        let subscriptions = kernel
            .video_vblank_mask(self.video.clocked_generations())
            .map_err(Failure::Kernel)?;
        let clock = clock(kernel, memory)?;
        self.video_materialize(memory, kernel, clock, None)?;
        self.video.grid_wait_delay(clock, subscriptions)
    }
}
