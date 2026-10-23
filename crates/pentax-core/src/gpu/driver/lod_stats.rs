// SPDX-License-Identifier: GPL-2.0-only
//! Ordered, same-owner LOD publication from actual fenced GPU counter snapshots.
//!
//! Public reference: Jetsku/KytyPS5 revision
//! b719d67ce6bcfd21d402bd415f3e4f68e4793534, LodStatsCounter::Report.
//! The first report resets real GPU storage without publishing. Later empty
//! probes preserve the snapshot, and a reset never creates sampled results.

use super::*;
use pentax_gpu::{
    lod_stats::{COUNTER_BYTES, Counters, REPORT_BYTES},
    lod_stats_request::Request,
};
use pentax_platform::native_memory::NativeOwnerId;

/// Only the GPU backend supplies this state's counter values. The owner also
/// survives the return of a submission and its selected-flip continuations.
pub(super) struct LodStatsState {
    owner: NativeOwnerId,
    counters: Counters,
}

impl LodStatsState {
    pub(super) fn counters(&self, owner: NativeOwnerId) -> Result<&Counters, Failure> {
        if self.owner != owner {
            return Err(Failure::Memory);
        }
        Ok(&self.counters)
    }

    /// Called only with the counter result retained by a successful actual
    /// draw after its GPU fence and original-owner target writeback.
    pub(super) fn accept_draw(
        &mut self,
        owner: NativeOwnerId,
        counters: Counters,
    ) -> Result<(), Failure> {
        if self.owner != owner {
            return Err(Failure::Memory);
        }
        self.counters = counters;
        Ok(())
    }
}

impl Driver {
    pub(super) fn lod_report(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        request: Request,
        commands: &[compute::CommandRange],
        held: usize,
        maximum: usize,
    ) -> Result<(), Failure> {
        let owner = memory.owner_id();
        if let Some(state) = &self.lod_stats {
            state.counters(owner)?;
        }
        // A probe after initialization neither writes nor clears accumulated
        // counters. It does not even inspect a non-publishing destination.
        if self.lod_stats.is_some() && request.is_empty_probe() {
            self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
            return Ok(());
        }
        let initialized = self.lod_stats.is_some();
        let write_length = if initialized {
            request.completion_bytes()
        } else {
            0
        };
        let reset = !initialized || request.reset_requested();
        // Fixed owned counter/report values coexist with backend allocations.
        // This allowance is reserved before reset or any guest-memory effect.
        let local = std::mem::size_of::<LodStatsState>()
            .checked_add(COUNTER_BYTES * 2)
            .and_then(|n| n.checked_add(REPORT_BYTES))
            .ok_or(Failure::Limit)?;
        let available = maximum
            .checked_sub(held)
            .and_then(|n| n.checked_sub(local))
            .ok_or(Failure::Limit)?;
        let remaining = self
            .limits
            .max_preparation_work
            .saturating_sub(self.usage.preparation_work);
        let mut work = 1u64;
        let address = request.destination();
        let size = write_length as u64;
        let written = add(
            self.usage.memory_written_bytes,
            size,
            self.limits.max_memory_write_bytes,
        )?;
        if write_length != 0 {
            work = work
                .checked_add(
                    memory
                        .preflight_distinct_backing(address, size, remaining)
                        .map_err(|_| Failure::Memory)?,
                )
                .ok_or(Failure::Limit)?;
            memory
                .preflight_gpu_write(address, size)
                .map_err(|_| Failure::Memory)?;
            for command in commands {
                let charged = memory
                    .preflight_disjoint_backing(
                        NativeRange { address, size },
                        NativeRange {
                            address: command.address,
                            size: command.size,
                        },
                        remaining.checked_sub(work).ok_or(Failure::Limit)?,
                    )
                    .map_err(|_| Failure::Memory)?;
                work = work.checked_add(charged).ok_or(Failure::Limit)?;
            }
        }
        // All draws already fenced and returned their real counter snapshot;
        // serializing that owned snapshot needs no second fabricated GPU job.
        let report = if write_length == 0 {
            None
        } else {
            Some(
                self.lod_stats
                    .as_ref()
                    .ok_or(Failure::UnsupportedQueue)?
                    .counters
                    .pack_report(),
            )
        };
        // Conservative fixed work admission includes serialization and the
        // bounded reset upload/readback. Backend limits remain independently
        // enforced and cannot be widened by this reservation.
        let operation_work = work
            .checked_add(REPORT_BYTES as u64)
            .and_then(|n| n.checked_add(if reset { (COUNTER_BYTES as u64) * 8 } else { 0 }))
            .ok_or(Failure::Limit)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            operation_work,
            self.limits.max_preparation_work,
        )?;
        self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
        let reset_counters = if reset {
            let mut limits = self.limits.gpu;
            if let Some(work) = self.limits.graphics_backend_work {
                limits.max_work = work;
            }
            limits.max_host_bytes = limits.max_host_bytes.min(available as u64);
            limits.max_work = limits.max_work.min((COUNTER_BYTES as u64) * 8);
            // A device effect followed by any failure must poison the stream.
            self.poisoned = true;
            Some(pentax_platform::vulkan::run_lod_reset(&limits).map_err(Failure::Vulkan)?)
        } else {
            None
        };
        if let Some(report) = report {
            self.poisoned = true;
            memory
                .apply_gpu_writes(
                    &[NativeMemoryWrite {
                        address,
                        bytes: &report.as_bytes()[..write_length],
                    }],
                    &NativeTransactionLimits {
                        max_outputs: 1,
                        max_output_bytes: size,
                        max_preflight_steps: work,
                        ..Default::default()
                    },
                )
                .map_err(|_| Failure::Memory)?;
            self.usage.memory_written_bytes = written;
            self.usage.written_bytes = self.usage.written_bytes.saturating_add(size);
        }
        if let Some(counters) = reset_counters {
            self.lod_stats = Some(LodStatsState { owner, counters });
        }
        Ok(())
    }
}
