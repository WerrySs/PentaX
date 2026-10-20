// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: owner-bound graphics events and ordered retirement.
// Pinned agc.cpp, renderer/sync.cpp, eventQueueFilters.h; see NOTICE.

use super::*;

impl Driver {
    pub(super) fn event_queue(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        kernel: Option<&mut pentax_kernel::Kernel>,
    ) -> Result<u64, Failure> {
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            32,
            self.limits.max_preparation_work,
        )?;
        if matches!(
            function,
            Function::AgcAddEqEvent | Function::AgcDeleteEqEvent
        ) {
            if args[0] == 0 {
                return Ok(0x8002_0009);
            }
            let kernel = kernel.ok_or(Failure::Kernel(pentax_kernel::Failure::Unsupported(
                "graphics event queue owner",
            )))?;
            let result = if function == Function::AgcAddEqEvent {
                kernel.add_graphics_event(args[0], args[1] as i32, args[2])
            } else {
                kernel.delete_graphics_event(args[0], args[1] as i32)
            };
            return match result {
                Err(pentax_kernel::Failure::Errno(error)) => {
                    Ok(u64::from(error.kernel_code() as u32))
                }
                result => result.map_err(Failure::Kernel),
            };
        }
        if args[0] == 0 {
            return Ok(0);
        }
        let mut filter = [0; 2];
        memory
            .read(args[0].checked_add(8).ok_or(Failure::Memory)?, &mut filter)
            .map_err(|_| Failure::Memory)?;
        let graphics = i16::from_le_bytes(filter) == -14;
        let offset = if (function == Function::AgcGetEqContextId) == graphics {
            16
        } else {
            0
        };
        let mut value = [0; 4];
        memory
            .read(
                args[0].checked_add(offset).ok_or(Failure::Memory)?,
                &mut value,
            )
            .map_err(|_| Failure::Memory)?;
        Ok(u64::from(u32::from_le_bytes(value)))
    }

    pub(super) fn retire_release(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        release: &compute::Release,
        commands: &[compute::CommandRange],
        kernel: &mut Option<&mut pentax_kernel::Kernel>,
        queue: u32,
    ) -> Result<(), Failure> {
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            32,
            self.limits.max_preparation_work,
        )?;
        let clock = if release.reference_clock() && release.write().is_some() {
            let sample = pentax_platform::time_counter::sample().map_err(|_| Failure::Memory)?;
            Some(reference_clock(sample.ticks, sample.frequency)?.to_le_bytes())
        } else {
            None
        };
        let next_interrupt = self
            .usage
            .interrupts
            .checked_add(u64::from(release.interrupt_context().is_some()))
            .ok_or(Failure::Limit)?;
        let pending = if let Some(context) = release.interrupt_context() {
            Some(
                kernel
                    .as_deref_mut()
                    .ok_or(Failure::Kernel(pentax_kernel::Failure::Unsupported(
                        "graphics interrupt owner",
                    )))?
                    .prepare_graphics_interrupt(queue as i32, context)
                    .map_err(Failure::Kernel)?,
            )
        } else {
            None
        };
        // Every preceding compute dispatch completed its Vulkan fence and
        // same-owner writeback. The admitted label store must succeed before
        // its notification is committed. Failure drops the exclusive ticket.
        if let Some(write) = release.write() {
            self.memory_write_bytes(
                memory,
                write,
                commands,
                clock.as_ref().map_or(write.bytes(), |bytes| &bytes[..]),
            )?;
        }
        if let Some(pending) = pending {
            self.poisoned = true;
            pending.complete();
            self.usage.interrupts = next_interrupt;
        }
        Ok(())
    }
}

fn reference_clock(ticks: u64, frequency: u64) -> Result<u64, Failure> {
    const RATE: u64 = 100_000_000;
    if frequency == 0 {
        return Err(Failure::Memory);
    }
    let whole = (ticks / frequency)
        .checked_mul(RATE)
        .ok_or(Failure::Limit)?;
    let part = (ticks % frequency)
        .checked_mul(RATE)
        .ok_or(Failure::Limit)?
        / frequency;
    whole.checked_add(part).ok_or(Failure::Limit)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_clock_retains_source_truncation_and_overflow_domains() {
        assert_eq!(reference_clock(11, 3), Ok(366_666_666));
        assert_eq!(reference_clock(0, 1), Ok(0));
        assert_eq!(reference_clock(u64::MAX, 100_000_000), Ok(u64::MAX));
        assert_eq!(reference_clock(1, 0), Err(Failure::Memory));
        assert_eq!(reference_clock(u64::MAX, 1), Err(Failure::Limit));
        assert_eq!(reference_clock(u64::MAX - 1, u64::MAX), Err(Failure::Limit));
    }
}
