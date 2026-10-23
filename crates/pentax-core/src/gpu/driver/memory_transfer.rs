// SPDX-License-Identifier: GPL-2.0-only
//! Source-ordered copy/fill using current paused-owner bytes, not an early image.
use super::*;

/// Public7b BufferCache zeroes one renderer-wide64KiB buffer at construction.
/// This driver-owned model survives queue/CP resets, and dies with the Driver.
/// It models DMA storage only: shader GDS instructions remain unsupported.
pub(super) struct GdsState {
    owner: pentax_platform::native_memory::NativeOwnerId,
    bytes: Vec<u8>,
}

impl Driver {
    pub(super) fn transfer_staging(
        &mut self,
        bytes: usize,
        output_bytes: usize,
        held: usize,
        host_limit: usize,
    ) -> Result<Vec<u8>, Failure> {
        // Streams without a nonzero new transfer retain their old admission
        // and failure-prefix behavior. New transfer streams admit aggregate
        // output separately from the maximum reusable staging extent.
        if bytes == 0 {
            return Ok(Vec::new());
        }
        add(
            self.usage.memory_written_bytes,
            output_bytes as u64,
            self.limits.max_memory_write_bytes,
        )?;
        held.checked_add(bytes)
            .filter(|n| *n <= host_limit)
            .ok_or(Failure::Limit)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            bytes as u64,
            self.limits.max_preparation_work,
        )?;
        let mut staging = Vec::new();
        staging
            .try_reserve_exact(bytes)
            .map_err(|_| Failure::Allocation)?;
        held.checked_add(staging.capacity())
            .filter(|n| *n <= host_limit)
            .ok_or(Failure::Limit)?;
        staging.resize(bytes, 0);
        Ok(staging)
    }

    pub(super) fn transfer_command(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        transfer: &compute::Transfer,
        commands: &[compute::CommandRange],
        staging: &mut [u8],
        transient_held: usize,
        transfer_host_limit: usize,
    ) -> Result<(), Failure> {
        let size = transfer.byte_count() as u64;
        if size == 0 {
            self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
            return Ok(());
        }
        let output = staging
            .get_mut(..transfer.byte_count())
            .ok_or(Failure::Limit)?;
        let address = transfer.destination();
        let destination_gds = transfer.destination_gds_offset();
        let bytes = add(
            self.usage.memory_written_bytes,
            size,
            self.limits.max_memory_write_bytes,
        )?;
        let remaining = self
            .limits
            .max_preparation_work
            .saturating_sub(self.usage.preparation_work);
        let mut work = 0u64;
        if destination_gds.is_none() {
            work = memory
                .preflight_distinct_backing(address, size, remaining)
                .map_err(|_| Failure::Memory)?;
            memory
                .preflight_gpu_write(address, size)
                .map_err(|_| Failure::Memory)?;
        }
        if let Some(source) = transfer.source_address() {
            memory
                .preflight_gpu_read(source, size)
                .map_err(|_| Failure::Memory)?;
            if destination_gds.is_none() {
                let charged = memory
                    .preflight_disjoint_backing(
                        NativeRange {
                            address: source,
                            size,
                        },
                        NativeRange { address, size },
                        remaining.checked_sub(work).ok_or(Failure::Limit)?,
                    )
                    .map_err(|_| Failure::Memory)?;
                work = work.checked_add(charged).ok_or(Failure::Limit)?;
            }
        }
        if destination_gds.is_none() {
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
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        if transfer.uses_gds() {
            self.ensure_gds(memory, transient_held, transfer_host_limit)?;
        }
        // All current RAM rights, complete destination and physical command
        // exclusions precede any source payload read. GDS never aliases RAM.
        if let Some(source) = transfer.source_address() {
            memory
                .read_gpu(source, output)
                .map_err(|_| Failure::Memory)?;
        } else if let Some(offset) = transfer.source_gds_offset() {
            let source = self
                .gds
                .as_ref()
                .ok_or(Failure::Memory)?
                .bytes
                .get(offset..offset.checked_add(output.len()).ok_or(Failure::Limit)?)
                .ok_or(Failure::Memory)?;
            output.copy_from_slice(source);
        } else if let Some(word) = transfer.fill_word() {
            for bytes in output.chunks_exact_mut(4) {
                bytes.copy_from_slice(&word.to_le_bytes());
            }
        } else {
            return Err(Failure::UnsupportedQueue);
        }
        self.usage.memory_written_bytes = bytes;
        self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
        self.poisoned = true;
        if let Some(offset) = destination_gds {
            let destination = self
                .gds
                .as_mut()
                .ok_or(Failure::Memory)?
                .bytes
                .get_mut(offset..offset.checked_add(output.len()).ok_or(Failure::Limit)?)
                .ok_or(Failure::Memory)?;
            destination.copy_from_slice(output);
            // Internal device-local writes spend the same output budget but
            // are not reported as writes to guest RAM.
            return Ok(());
        }
        memory
            .apply_gpu_writes(
                &[NativeMemoryWrite {
                    address,
                    bytes: output,
                }],
                &NativeTransactionLimits {
                    max_outputs: 1,
                    max_output_bytes: size,
                    max_preflight_steps: work,
                    ..Default::default()
                },
            )
            .map_err(|_| Failure::Memory)?;
        self.usage.written_bytes = self.usage.written_bytes.saturating_add(size);
        Ok(())
    }

    pub(super) fn gds_retained_bytes(&self) -> usize {
        self.gds.as_ref().map_or(0, |state| {
            state.bytes.capacity().saturating_add(size_of::<GdsState>())
        })
    }

    fn ensure_gds(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        transient_held: usize,
        transfer_host_limit: usize,
    ) -> Result<(), Failure> {
        // A zero-width permission proof invokes the paused owner's live check;
        // no RAM mapping, payload or numeric GDS offset is treated as authority.
        memory
            .preflight_gpu_read(0, 0)
            .map_err(|_| Failure::Memory)?;
        if self
            .gds
            .as_ref()
            .is_some_and(|state| state.owner != memory.owner_id())
        {
            return Err(Failure::Memory);
        }
        let held = self
            .graphics_held_bytes()?
            .checked_add(transient_held)
            .ok_or(Failure::Limit)?;
        let limit = self
            .graphics_host_ceiling()?
            .min(self.limits.compute.max_host_bytes);
        let extra = if self.gds.is_none() {
            compute::Transfer::GDS_BYTES
                .checked_add(size_of::<GdsState>())
                .ok_or(Failure::Limit)?
        } else {
            0
        };
        held.checked_add(extra)
            .filter(|total| *total <= limit)
            .ok_or(Failure::Limit)?;
        transient_held
            .checked_add(extra)
            .filter(|total| *total <= transfer_host_limit)
            .ok_or(Failure::Limit)?;
        if self.gds.is_none() {
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                compute::Transfer::GDS_BYTES as u64,
                self.limits.max_preparation_work,
            )?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(compute::Transfer::GDS_BYTES)
                .map_err(|_| Failure::Allocation)?;
            held.checked_add(bytes.capacity())
                .and_then(|total| total.checked_add(size_of::<GdsState>()))
                .filter(|total| *total <= limit)
                .ok_or(Failure::Limit)?;
            transient_held
                .checked_add(bytes.capacity())
                .and_then(|total| total.checked_add(size_of::<GdsState>()))
                .filter(|total| *total <= transfer_host_limit)
                .ok_or(Failure::Limit)?;
            bytes.resize(compute::Transfer::GDS_BYTES, 0);
            self.gds = Some(GdsState {
                owner: memory.owner_id(),
                bytes,
            });
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "memory_transfer/hardware_tests.rs"]
mod hardware_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "memory_transfer/gds_tests.rs"]
mod gds_tests;
