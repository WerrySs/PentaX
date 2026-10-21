// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded same-owner CPU flip and frame hand-off.
// Adapted from pinned videoOut.cpp/libVideoOut.cpp; see core NOTICE.

use super::*;
use crate::gpu::video::{FlipCompletion, FlipSource, FlipStatus, FlipSubmission, Frame};

const INVALID_ADDRESS: u64 = 0x8029_0002;
const INVALID_HANDLE: u64 = 0x8029_000b;

impl Driver {
    /// Accounting policy only. All inputs can only tighten configured bounds;
    /// numeric byte counts confer no access or frame ownership.
    pub(crate) fn video_consumer_admit(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        max_frame_bytes: usize,
        max_held_bytes: usize,
        external_held: usize,
    ) -> Result<usize, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        let original = self.limits.video_presentation;
        let frame = original.max_frame_bytes.min(max_frame_bytes);
        let held = original.max_held_bytes.min(max_held_bytes);
        if frame == 0
            || held == 0
            || external_held
                .checked_add(self.video.retained_frame_bytes())
                .is_none_or(|n| n > held)
        {
            return Err(Failure::Limit);
        }
        if let Some(old) = self.consumer
            && (frame > old.max_frame_bytes || held > old.max_held_bytes)
        {
            return Err(Failure::Limit);
        }
        self.consumer = Some(ConsumerBudget {
            max_frame_bytes: frame,
            max_held_bytes: held,
            external_held,
        });
        // Census only: no numeric capacity grants access or ownership. The
        // non-guest transport pump includes these still-queued allocations
        // in every decoder/backend growth admission until the next census.
        Ok(self.video.retained_frame_bytes())
    }
    pub(super) fn presentation_limits(
        &self,
    ) -> Result<crate::gpu::video::PresentationLimits, Failure> {
        let mut limits = self.limits.video_presentation;
        if let Some(consumer) = self.consumer {
            limits.max_frame_bytes = limits.max_frame_bytes.min(consumer.max_frame_bytes);
            limits.max_held_bytes = limits
                .max_held_bytes
                .min(consumer.max_held_bytes)
                .checked_sub(consumer.external_held)
                .ok_or(Failure::Limit)?;
        }
        Ok(limits)
    }
    pub(super) fn graphics_host_limit(&self) -> Result<usize, Failure> {
        match self.consumer {
            Some(consumer) => self
                .limits
                .graphics
                .max_host_bytes
                .checked_sub(consumer.external_held)
                .ok_or(Failure::Limit),
            None => Ok(self.limits.graphics.max_host_bytes),
        }
    }
    pub(crate) fn video_peek_ready_frame(
        &self,
        memory: &PausedNativeMemory<'_>,
    ) -> Result<Option<crate::gpu::video::ReadyFrameMetadata>, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        self.video.peek_ready_frame()
    }
    /// Guest-call wrapper: submit_impl has already admitted exactly one call.
    /// The lazy clock supplies the actual kernel process-counter domain only
    /// after source metadata admission; raw host ticks are never substituted.
    pub(super) fn video_submit_flip(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
        clock: impl FnOnce(&mut PausedNativeMemory<'_>) -> Result<u64, Failure>,
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            128,
            self.limits.max_preparation_work,
        )?;
        let handle = args[0] as u32 as i32;
        let buffer = args[1] as u32 as i32;
        let owner = memory.owner_id();
        let submission = self.video.reserve_flip_with_clock(
            handle,
            buffer,
            args[2] as u32 as i32,
            args[3] as i64,
            FlipSource::Cpu,
            owner,
            || clock(memory),
        )?;
        let ticket = match submission {
            FlipSubmission::Rejected(code) => return Ok(code),
            FlipSubmission::Accepted(ticket) => ticket,
        };
        let result = (|| {
            let surface = self
                .video
                .surface(handle, buffer)
                .ok_or(Failure::UnsupportedVideo)?;
            // Conservative admission includes the padded source read, detile
            // work and optional channel conversion. State also caps actual
            // simultaneous capacities before publishing any captured pixels.
            let pixel_work = u64::from(surface.width)
                .checked_mul(u64::from(surface.height))
                .and_then(|v| v.checked_mul(20))
                .ok_or(Failure::Limit)?;
            let work = surface
                .data
                .size
                .checked_add(pixel_work)
                .and_then(|v| v.checked_add(128))
                .ok_or(Failure::Limit)?;
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                work,
                self.limits.max_preparation_work,
            )?;
            let limits = self.presentation_limits()?;
            self.video.prepare_flip(ticket, memory, &limits)?;
            self.video.complete_flip(ticket)?; // CPU only; no GPU-EOP completion bypass.
            Ok(0)
        })();
        // Reservation is already observable pending state. Any later refusal
        // is terminal, preventing retry from silently enrolling a second flip.
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    /// Exact source null-before-handle ordering and one128-byte output write.
    /// This helper performs no timestamp sampling or hidden kernel service call.
    pub(super) fn video_flip_query(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        if !matches!(
            function,
            Function::VideoGetFlipStatus | Function::VideoIsFlipPending
        ) {
            return Err(Failure::UnsupportedVideo);
        }
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            128,
            self.limits.max_preparation_work,
        )?;
        if function == Function::VideoGetFlipStatus && args[1] == 0 {
            return Ok(INVALID_ADDRESS);
        }
        let Some(status) = self.video.flip_status(args[0] as u32 as i32) else {
            return Ok(INVALID_HANDLE);
        };
        if function == Function::VideoIsFlipPending {
            return u64::try_from(status.pending).map_err(|_| Failure::UnsupportedVideo);
        }
        let bytes = status.bytes();
        memory
            .apply_transaction(
                pentax_platform::native_memory::NativeMemoryTransaction {
                    effect: pentax_platform::native_memory::NativeMemoryEffect::None,
                    writes: &[NativeMemoryWrite {
                        address: args[1],
                        bytes: &bytes,
                    }],
                },
                &NativeTransactionLimits::default(),
            )
            .map_err(|_| Failure::Memory)?;
        Ok(0)
    }

    /// Metadata snapshot on the actual owner. No lazy foreign-owner binding.
    pub(crate) fn video_flip_status(
        &self,
        memory: &PausedNativeMemory<'_>,
        handle: i32,
    ) -> Result<Option<FlipStatus>, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        Ok(self.video.flip_status(handle))
    }

    pub(crate) fn video_buffer_pending(
        &self,
        memory: &PausedNativeMemory<'_>,
        handle: i32,
        buffer: i32,
    ) -> Result<bool, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        if self.video.port(handle).is_none() {
            return Err(Failure::UnsupportedVideo);
        }
        if !(-2..16).contains(&buffer) {
            return Err(Failure::UnsupportedVideo);
        }
        Ok(self.video.buffer_pending(handle, buffer))
    }

    /// Root transport must supply an actual vblank tick and independently bound
    /// its outstanding owned Frame. Charge BOTH hand-off and acknowledgement
    /// before external presentation: no new quota/allocation follows display.
    pub(crate) fn video_take_frame(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        vblank_count: u64,
    ) -> Result<Option<Frame>, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        // A handoff moves an existing allocation from queue to transport. Admit
        // the external counter before mutation; no quota follows a real take.
        let external = if let Some(consumer) = self.consumer {
            let capacity = self
                .video
                .peek_ready_frame()?
                .map_or(0, |frame| frame.capacity());
            Some(
                consumer
                    .external_held
                    .checked_add(capacity)
                    .ok_or(Failure::Limit)?,
            )
        } else {
            None
        };
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            256,
            self.limits.max_preparation_work,
        )?;
        let frame = self.video.take_frame(vblank_count)?;
        if frame.is_some()
            && let (Some(consumer), Some(external)) = (&mut self.consumer, external)
        {
            consumer.external_held = external;
        }
        Ok(frame)
    }

    /// Actual consumer acknowledgement, not an export or reservation success.
    /// The exact opaque Frame supplies ticket/generation proof. Root event
    /// delivery must pre-admit real retained equeue owners before presentation.
    pub(crate) fn video_acknowledge_frame(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        frame: &Frame,
        clock: impl FnOnce(&mut PausedNativeMemory<'_>) -> Result<(u64, u64), Failure>,
    ) -> Result<FlipCompletion, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.video.check_owner(memory.owner_id())?;
        self.video
            .acknowledge_frame_with_clock(frame, || clock(memory))
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

    fn owner() -> (NativeAddressSpace, u64) {
        for index in 0..16 {
            let base = 0x60_0000_0000 + index * 0x100_0000;
            let mut memory = NativeAddressSpace::new(Default::default()).unwrap();
            if memory
                .map_at(base, 16384, NativePermissions::READ_WRITE)
                .is_err()
            {
                continue;
            }
            if memory
                .map_at(
                    base + 65536,
                    65536,
                    NativePermissions::READ_WRITE.with_gpu(true, true),
                )
                .is_err()
            {
                continue;
            }
            return (memory, base);
        }
        panic!("original bounded video address candidates unavailable");
    }

    #[test]
    fn driver_cpu_flip_uses_registered_pixels_and_defers_status_until_real_acknowledgement() {
        let (mut owner, base) = owner();
        let mut memory = owner.pause().unwrap();
        let mut driver = Driver::new(Default::default());
        // Fresh state has no owner yet. Source null-before-handle ordering
        // still applies, and invalid handles cannot inspect output or clocks.
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::VideoGetFlipStatus,
                [99, 0, 0, 0, 0, 0]
            ),
            Ok(INVALID_ADDRESS)
        );
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::VideoGetFlipStatus,
                [99, u64::MAX, 0, 0, 0, 0]
            ),
            Ok(INVALID_HANDLE)
        );
        let fresh_samples = std::cell::Cell::new(0);
        assert_eq!(
            driver.submit_video_flip_with_clock(&mut memory, [99, 0, 1, 0, 0, 0], |_| {
                fresh_samples.set(fresh_samples.get() + 1);
                Err(Failure::Memory)
            }),
            Ok(INVALID_HANDLE)
        );
        assert_eq!(fresh_samples.get(), 0);
        assert_eq!(
            driver.function_profile().count(Function::VideoSubmitFlip),
            1
        );
        assert_eq!(
            driver
                .function_profile()
                .count(Function::VideoGetFlipStatus),
            2
        );
        assert_eq!(driver.function_profile().last(), "VideoSubmitFlip");
        let profile = driver.function_profile();
        let maximum = driver.limits.max_calls;
        driver.limits.max_calls = driver.usage().calls;
        assert_eq!(
            driver.submit_video_flip_with_clock(&mut memory, [99, 0, 1, 0, 0, 0], |_| {
                fresh_samples.set(fresh_samples.get() + 1);
                Err(Failure::Memory)
            }),
            Err(Failure::Limit)
        );
        assert_eq!(driver.function_profile(), profile);
        assert_eq!(fresh_samples.get(), 0);
        driver.limits.max_calls = maximum;
        assert_eq!(
            driver.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0]),
            Ok(1)
        );
        let mut attribute = [0; 80];
        attribute[12..16].copy_from_slice(&2_u32.to_le_bytes());
        attribute[16..20].copy_from_slice(&2_u32.to_le_bytes());
        attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000_u64.to_le_bytes());
        memory.write(base + 128, &attribute).unwrap();
        let mut descriptor = [0; 32];
        descriptor[..8].copy_from_slice(&(base + 65536).to_le_bytes());
        memory.write(base + 256, &descriptor).unwrap();
        memory.write(base + 8, &[0; 16]).unwrap();
        assert_eq!(
            driver.submit_with_stack(
                &mut memory,
                Function::VideoRegisterBuffers2,
                [1, 0, 0, base + 256, 1, base + 128],
                Some(base)
            ),
            Ok(0)
        );
        let mut original = Vec::new();
        for (index, offset) in [0, 4, 16, 20].into_iter().enumerate() {
            let pixel = [index as u8 + 1, 20, 30, 255];
            memory.write(base + 65536 + offset, &pixel).unwrap();
            original.extend(pixel);
        }
        let calls = driver.usage().calls;
        let samples = std::cell::Cell::new(0);
        for (args, error) in [
            ([99, 0, 1, 0, 0, 0], INVALID_HANDLE),
            ([1, 0, 99, 0, 0, 0], 0x8029_0006),
            ([1, 99, 1, 0, 0, 0], 0x8029_000a),
        ] {
            assert_eq!(
                driver.video_submit_flip(&mut memory, args, |_| {
                    samples.set(samples.get() + 1);
                    Err(Failure::Memory)
                }),
                Ok(error)
            );
        }
        assert_eq!(samples.get(), 0); // Invalid requests never initialize a clock epoch.
        assert_eq!(
            driver.video_submit_flip(&mut memory, [1, 0, 1, 0, 0, 0], |_| {
                samples.set(samples.get() + 1);
                Err(Failure::Memory)
            }),
            Err(Failure::Memory)
        );
        assert_eq!(samples.get(), 1);
        assert_eq!(
            driver
                .video_flip_status(&memory, 1)
                .unwrap()
                .unwrap()
                .pending,
            0
        );
        assert_eq!(
            driver.video_submit_flip(
                &mut memory,
                [0xa5a5_0000_0000_0001, 0, 1, (-7_i64) as u64, 0, 0],
                |_| {
                    samples.set(samples.get() + 1);
                    Ok(100)
                }
            ),
            Ok(0)
        );
        assert_eq!(samples.get(), 2);
        assert_eq!(driver.usage().calls, calls); // Caller already owns guest-call admission.
        assert_eq!(
            driver.function_profile().count(Function::VideoSubmitFlip),
            1
        );
        assert_eq!(
            driver.function_profile().top(),
            [
                Some(("VideoGetFlipStatus", 2)),
                Some(("VideoOpen", 1)),
                Some(("VideoRegisterBuffers2", 1)),
                Some(("VideoSubmitFlip", 1)),
            ]
        ); // Direct helpers above must not double-count the outer admission.
        let before_peek = driver.video_flip_status(&memory, 1).unwrap();
        let metadata = driver.video_peek_ready_frame(&memory).unwrap().unwrap();
        assert_eq!(
            (
                metadata.width(),
                metadata.height(),
                metadata.source_format(),
                metadata.premultiplied_alpha()
            ),
            (2, 2, crate::gpu::video::PixelFormat::Rgba8Srgb, false)
        );
        assert_eq!(
            metadata.generation(),
            driver.video_port(1).unwrap().generation
        );
        assert!(metadata.capacity() >= 16);
        assert_eq!(driver.video_flip_status(&memory, 1).unwrap(), before_peek);
        // Consumer metadata can only tighten policy; it grants no dequeue or
        // acknowledgment. Existing captured capacity participates immediately.
        let max_frame = driver.limits.video_presentation.max_frame_bytes;
        let max_held = driver
            .limits
            .video_presentation
            .max_held_bytes
            .min(64 * 1024 * 1024);
        assert_eq!(
            driver.video_consumer_admit(&memory, max_frame, max_held, max_held),
            Err(Failure::Limit)
        );
        let queued = driver
            .video_consumer_admit(&memory, max_frame, max_held, 1024)
            .unwrap();
        assert_eq!(queued, metadata.capacity());
        assert_eq!(
            driver.presentation_limits().unwrap().max_held_bytes,
            max_held - 1024
        );
        // The session pixel ceiling does not replace the independent graphics
        // compiler/workspace ceiling. Actual external memory counts in both.
        assert_eq!(
            driver.graphics_host_limit().unwrap(),
            driver.limits.graphics.max_host_bytes - 1024
        );
        let mut foreign = NativeAddressSpace::reserve_any(Default::default(), 65536).unwrap();
        assert!(matches!(
            driver.video_peek_ready_frame(&foreign.pause().unwrap()),
            Err(Failure::Memory)
        ));
        assert!(driver.video_buffer_pending(&memory, 1, 0).unwrap());
        assert_eq!(
            driver.video_flip_status(&memory, 1).unwrap().unwrap().count,
            0
        );
        assert_eq!(
            driver.video_flip_query(
                &mut memory,
                Function::VideoGetFlipStatus,
                [99, 0, 0, 0, 0, 0]
            ),
            Ok(INVALID_ADDRESS)
        );
        let boundary = base + 16384 - 127;
        memory.write(boundary, &[0xa5; 127]).unwrap();
        assert_eq!(
            driver.video_flip_query(
                &mut memory,
                Function::VideoGetFlipStatus,
                [1, boundary, 0, 0, 0, 0]
            ),
            Err(Failure::Memory)
        );
        let mut untouched = [0; 127];
        memory.read(boundary, &mut untouched).unwrap();
        assert_eq!(untouched, [0xa5; 127]);
        memory.write(base + 512, &[0xa5; 136]).unwrap();
        assert_eq!(
            driver.video_flip_query(
                &mut memory,
                Function::VideoGetFlipStatus,
                [1, base + 512, 0, 0, 0, 0]
            ),
            Ok(0)
        );
        let mut bytes = [0; 136];
        memory.read(base + 512, &mut bytes).unwrap();
        assert_eq!(u64::from_le_bytes(bytes[..8].try_into().unwrap()), 0);
        assert_eq!(i32::from_le_bytes(bytes[52..56].try_into().unwrap()), 1);
        assert_eq!(&bytes[128..], &[0xa5; 8]);
        assert_eq!(
            driver.video_flip_query(
                &mut memory,
                Function::VideoIsFlipPending,
                [1, 0, 0, 0, 0, 0]
            ),
            Ok(1)
        );
        assert!(driver.video_take_frame(&memory, 0).unwrap().is_none());
        let frame = driver.video_take_frame(&memory, 1).unwrap().unwrap();
        assert!(driver.video_peek_ready_frame(&memory).unwrap().is_none());
        assert_eq!(
            driver.consumer.unwrap().external_held,
            1024 + frame.capacity()
        );
        assert_eq!(
            driver.presentation_limits().unwrap().max_held_bytes,
            max_held - 1024 - frame.capacity()
        );
        assert_eq!(frame.rgba8(), original);
        memory.write(base + 65536, &[0; 4]).unwrap();
        assert_eq!(frame.rgba8(), original); // Immutable owned snapshot.
        let work = driver.usage().preparation_work;
        driver.limits.max_preparation_work = work; // Already pre-admitted hand-off/ack.
        let before = driver.video_flip_status(&memory, 1).unwrap();
        assert_eq!(
            driver.video_acknowledge_frame(&mut memory, &frame, |_| Err(Failure::Memory)),
            Err(Failure::Memory)
        );
        assert_eq!(driver.video_flip_status(&memory, 1).unwrap(), before);
        let completion = driver
            .video_acknowledge_frame(&mut memory, &frame, |_| Ok((25, 200)))
            .unwrap();
        assert_eq!(
            (
                completion.flip_arg(),
                completion.count(),
                completion.buffer()
            ),
            (-7, 1, 0)
        );
        assert_eq!(driver.usage().preparation_work, work);
        assert!(!driver.video_buffer_pending(&memory, 1, 0).unwrap());
        let status = driver.video_flip_status(&memory, 1).unwrap().unwrap();
        assert_eq!(
            (
                status.pending,
                status.process_time,
                status.process_counter,
                status.submit_counter
            ),
            (0, 25, 200, 100)
        );
        let stale_samples = std::cell::Cell::new(0);
        assert!(
            driver
                .video_acknowledge_frame(&mut memory, &frame, |_| {
                    stale_samples.set(stale_samples.get() + 1);
                    Ok((26, 201))
                })
                .is_err()
        );
        assert_eq!(stale_samples.get(), 0);
        assert_eq!(
            driver.video_flip_query(
                &mut memory,
                Function::VideoIsFlipPending,
                [1, 0, 0, 0, 0, 0]
            ),
            Err(Failure::Limit)
        );
        assert_eq!(
            driver.video_flip_status(&memory, 1).unwrap().unwrap().count,
            1
        );
        let (mut foreign, _) = self::owner();
        assert_eq!(
            driver.video_flip_status(&foreign.pause().unwrap(), 1),
            Err(Failure::Memory)
        );
        assert_eq!(
            driver.video_acknowledge_frame(&mut foreign.pause().unwrap(), &frame, |_| {
                stale_samples.set(stale_samples.get() + 1);
                Ok((26, 201))
            }),
            Err(Failure::Memory)
        );
        assert_eq!(stale_samples.get(), 0);
        assert_eq!(driver.usage().dispatches, 0);
        assert_eq!(driver.frame_state().boundaries, 0);
        let mut limited = Driver::new(Default::default());
        assert_eq!(
            limited.submit(&mut memory, Function::VideoOpen, [255, 0, 0, 0, 0, 0]),
            Ok(1)
        );
        assert_eq!(
            limited.submit_with_stack(
                &mut memory,
                Function::VideoRegisterBuffers2,
                [1, 0, 0, base + 256, 1, base + 128],
                Some(base)
            ),
            Ok(0)
        );
        limited.limits.max_preparation_work = limited.usage().preparation_work + 128;
        let calls = std::cell::Cell::new(0);
        assert_eq!(
            limited.video_submit_flip(&mut memory, [1, 0, 1, 0, 0, 0], |_| {
                calls.set(calls.get() + 1);
                Ok(1)
            }),
            Err(Failure::Limit)
        );
        assert!(limited.poisoned);
        assert_eq!(limited.video.flip_status(1).unwrap().pending, 1);
        assert_eq!(
            limited.video_submit_flip(&mut memory, [1, 0, 1, 0, 0, 0], |_| {
                calls.set(calls.get() + 1);
                Ok(2)
            }),
            Err(Failure::Poisoned)
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(limited.video.flip_status(1).unwrap().pending, 1); // No duplicate retry.
    }
}
