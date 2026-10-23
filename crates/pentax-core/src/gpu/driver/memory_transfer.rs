// SPDX-License-Identifier: GPL-2.0-only
//! Source-ordered copy/fill using current paused-owner bytes, not an early image.
use super::*;

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
    ) -> Result<(), Failure> {
        let size = transfer.byte_count() as u64;
        if size == 0 {
            // Primary zero DMA has no backing access, fence, byte write or
            // event. Conservatively consumes graphics freshness like a wait.
            self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
            return Ok(());
        }
        let output = staging
            .get_mut(..transfer.byte_count())
            .ok_or(Failure::Limit)?;
        let address = transfer.destination();
        let bytes = add(
            self.usage.memory_written_bytes,
            size,
            self.limits.max_memory_write_bytes,
        )?;
        let remaining = self
            .limits
            .max_preparation_work
            .saturating_sub(self.usage.preparation_work);
        let mut work = memory
            .preflight_distinct_backing(address, size, remaining)
            .map_err(|_| Failure::Memory)?;
        memory
            .preflight_gpu_write(address, size)
            .map_err(|_| Failure::Memory)?;
        if let Some(source) = transfer.source_address() {
            memory
                .preflight_gpu_read(source, size)
                .map_err(|_| Failure::Memory)?;
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
        // The immutable plan already charged two visits for EVERY output byte
        // (current read/fill and copy-out). Charge only these live backing
        // proofs here; no refund or duplicate output-byte debit.
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        // No source bytes were read before complete live output/alias admission.
        // Every preceding GPU job fenced and wrote back before reaching here.
        if let Some(source) = transfer.source_address() {
            memory
                .read_gpu(source, output)
                .map_err(|_| Failure::Memory)?;
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
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "memory_transfer/hardware_tests.rs"]
mod hardware_tests;
