// SPDX-License-Identifier: GPL-2.0-only
// KytyPS5@7b7f9a70 src/libs/agc.cpp CommandBuffer/AgcDcbSetFlip.
// Modified for checked fit-only, atomic cursor/packet publication.
use super::*;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "flip_encoder/tests.rs"]
mod tests;

impl Driver {
    pub(super) fn encode_flip(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
    ) -> Result<u64, Failure> {
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            256,
            self.limits.max_preparation_work,
        )?;
        let buf = args[0];
        if buf == 0 {
            return Err(Failure::UnsupportedQueue);
        }
        let field = |offset| buf.checked_add(offset).ok_or(Failure::Memory);
        let mut bytes = [0; 8];
        memory
            .read(field(16)?, &mut bytes)
            .map_err(|_| Failure::Memory)?;
        let up = u64::from_le_bytes(bytes);
        memory
            .read(field(24)?, &mut bytes)
            .map_err(|_| Failure::Memory)?;
        let down = u64::from_le_bytes(bytes);
        let mut reserved = [0; 4];
        memory
            .read(field(48)?, &mut reserved)
            .map_err(|_| Failure::Memory)?;
        let reserved = u64::from(u32::from_le_bytes(reserved));
        let available = down
            .checked_sub(up)
            .filter(|n| *n <= i64::MAX as u64 && n.is_multiple_of(4))
            .map(|n| n / 4)
            .and_then(|n| n.checked_sub(reserved));
        if up == 0 || !up.is_multiple_of(4) || available.is_none_or(|n| n < 6) {
            // No callback is run or fabricated. Unsupported growth/fatal
            // allocation paths leave the source descriptor and packet intact.
            return Err(Failure::UnsupportedQueue);
        }
        let next = up.checked_add(24).ok_or(Failure::Memory)?;
        let written = add(
            self.usage.memory_written_bytes,
            32,
            self.limits.max_memory_write_bytes,
        )?;
        let mut packet = [0; 24];
        for (index, word) in [
            0xc004_105cu32,
            args[1] as u32,
            args[2] as u32,
            args[3] as u32,
            args[4] as u32,
            (args[4] >> 32) as u32,
        ]
        .into_iter()
        .enumerate()
        {
            packet[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        let remaining = self
            .limits
            .max_preparation_work
            .saturating_sub(self.usage.preparation_work);
        let work = memory
            .preflight_disjoint_backing(
                NativeRange {
                    address: buf,
                    size: 56,
                },
                NativeRange {
                    address: up,
                    size: 24,
                },
                remaining,
            )
            .map_err(|_| Failure::Memory)?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        memory
            .apply_transaction(
                pentax_platform::native_memory::NativeMemoryTransaction {
                    effect: pentax_platform::native_memory::NativeMemoryEffect::None,
                    writes: &[
                        NativeMemoryWrite {
                            address: field(16)?,
                            bytes: &next.to_le_bytes(),
                        },
                        NativeMemoryWrite {
                            address: up,
                            bytes: &packet,
                        },
                    ],
                },
                &NativeTransactionLimits {
                    max_outputs: 2,
                    max_output_bytes: 32,
                    max_preflight_steps: remaining,
                    ..Default::default()
                },
            )
            .map_err(|_| Failure::Memory)?;
        self.usage.memory_written_bytes = written;
        Ok(up)
    }
}
