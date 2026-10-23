// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from public KytyPS5 7b7f9a70 agc.cpp; see NOTICE.
//! Packet metadata pointers are returned data, never execution authority.
use super::*;
use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};

impl Driver {
    pub(super) fn packet_payload_address(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
    ) -> Result<u64, Failure> {
        let [output, command, kind, ..] = args;
        if output == 0 {
            return Err(Failure::Memory);
        }
        // At most one four-byte read (two pages) and one eight-byte None-effect
        // transaction (two pages, thirteen conservative preflight steps).
        // Fixed admission also covers arithmetic, local copies and bookkeeping.
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            64,
            self.limits.max_preparation_work,
        )?;
        let value = if kind as u32 != 0 {
            // Source int is low32. This branch does not access command at all,
            // including NULL or unmapped command data and the returned pointee.
            command.checked_add(8).ok_or(Failure::Memory)?
        } else {
            let mut header = [0; 4];
            memory
                .read(command, &mut header)
                .map_err(|_| Failure::Memory)?;
            if !u32::from_le_bytes(header) & 0x3fff_0000 != 0 {
                command.checked_add(4).ok_or(Failure::Memory)?
            } else {
                0
            }
        };
        let written = add(
            self.usage.memory_written_bytes,
            8,
            self.limits.max_memory_write_bytes,
        )?;
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::None,
                    writes: &[NativeMemoryWrite {
                        address: output,
                        bytes: &value.to_le_bytes(),
                    }],
                },
                &NativeTransactionLimits {
                    max_outputs: 1,
                    max_output_bytes: 8,
                    max_preflight_steps: 13,
                    ..Default::default()
                },
            )
            .map_err(|_| Failure::Memory)?;
        self.usage.memory_written_bytes = written;
        Ok(0)
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "packet_metadata/tests.rs"]
mod tests;
