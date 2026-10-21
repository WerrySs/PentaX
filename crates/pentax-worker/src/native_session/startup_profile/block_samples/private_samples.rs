// SPDX-License-Identifier: GPL-2.0-only
//! Explicit private capture of recent successful samples. Never protocol data,
//! source authority, branch outcomes or a whole-run hotspot profile.

use super::{BlockSamples, CAPACITY};
use pentax_linker::SymbolicLinkPlan;

const HEADER: usize = 16;
const ROW: usize = 24;
const MAX_BYTES: usize = HEADER + CAPACITY * ROW;

/// Fixed private staging, initialized only after optional work admission.
/// Deliberately has no Debug implementation: the parent redacts every key.
pub(super) struct PrivateSamples {
    bytes: [u8; MAX_BYTES],
    length: usize,
}
impl PrivateSamples {
    pub(super) fn clear(&mut self) {
        self.length = 0;
    }
}

impl BlockSamples {
    // A private diagnostic refusal may discard only this optional artifact.
    // It cannot disable the ordinary observer or change execution admission.
    fn admit_private(&mut self, cost: u64) -> bool {
        let Some(next) = self
            .work
            .checked_add(cost)
            .filter(|n| *n <= self.maximum_work)
        else {
            self.private_samples = None;
            return false;
        };
        self.work = next;
        true
    }

    pub(in crate::native_session::startup_profile) fn enable_private_samples(&mut self) {
        if self.private_samples_requested {
            return;
        }
        self.private_samples_requested = true;
        if self.admit_private(MAX_BYTES as u64 + 1) {
            self.private_samples = Some(PrivateSamples {
                bytes: [0; MAX_BYTES],
                length: 0,
            });
        }
    }

    pub(in crate::native_session::startup_profile) fn private_samples_bytes(
        &self,
    ) -> Option<&[u8]> {
        self.private_samples
            .as_ref()
            .filter(|samples| samples.length != 0)
            .map(|samples| &samples.bytes[..samples.length])
    }

    pub(in crate::native_session::startup_profile) fn capture_private_samples(
        &mut self,
        source: &SymbolicLinkPlan<'_>,
    ) {
        let Some(samples) = &mut self.private_samples else {
            return;
        };
        samples.clear();
        if self.epoch != Some(source.identity()) {
            self.private_samples = None;
            return;
        }
        // First admit the metadata-count fold itself. Do not assume a fixed
        // number of segments or perform a metadata walk before its charge.
        let Some(fold_work) = (source.modules().len() as u64).checked_add(1) else {
            self.private_samples = None;
            return;
        };
        if !self.admit_private(fold_work) {
            return;
        }
        let Some(segments) = source.modules().iter().try_fold(0_u64, |count, module| {
            count.checked_add(module.runtime_plan().segments.len() as u64)
        }) else {
            self.private_samples = None;
            return;
        };
        // Each of <=64 keys can visit every segment in its selected module.
        // Eight units cover checked metadata geometry per visited segment;
        // every possible dedup comparison charges a complete 24-byte row.
        // Include validation, complete buffer zeroing and encoding before use.
        let cost = segments
            .checked_mul(8)
            .and_then(|n| n.checked_add(16))
            .and_then(|n| n.checked_mul(CAPACITY as u64))
            .and_then(|n| n.checked_add((CAPACITY * CAPACITY * ROW) as u64))
            .and_then(|n| n.checked_add(2 * MAX_BYTES as u64));
        let Some(cost) = cost else {
            self.private_samples = None;
            return;
        };
        if !self.admit_private(cost) {
            return;
        }
        let samples = self.private_samples.as_mut().expect("private admission");
        samples.bytes.fill(0);
        let mut rows = 0_usize;
        for entry in self.entries.iter().flatten() {
            // Do not normalize an old epoch, an invalid module or a PC outside
            // that module's initialized RX bytes into plausible private data.
            let normalized = (|| {
                if entry.key.epoch != source.identity() {
                    return None;
                }
                let module = source.modules().get(entry.key.module)?;
                let offset = entry.key.pc.checked_sub(module.load_bias())?;
                let mut initialized_rx = false;
                for segment in &module.runtime_plan().segments {
                    if !segment.flags.executable() || segment.flags.writable() {
                        continue;
                    }
                    let start = module.load_bias().checked_add(segment.vaddr)?;
                    let end = start.checked_add(segment.file_bytes.len() as u64)?;
                    initialized_rx |= start <= entry.key.pc && entry.key.pc < end;
                }
                if !initialized_rx {
                    return None;
                }
                Some((
                    u32::try_from(entry.key.module).ok()?,
                    u32::from(!entry.is_main),
                    offset,
                ))
            })();
            let Some((module, role, offset)) = normalized else {
                self.private_samples = None;
                return;
            };
            let found = (0..rows).find(|index| {
                let at = HEADER + index * ROW;
                samples.bytes[at..at + 4] == module.to_le_bytes()
                    && samples.bytes[at + 4..at + 8] == role.to_le_bytes()
                    && samples.bytes[at + 8..at + 16] == offset.to_le_bytes()
            });
            if let Some(index) = found {
                let at = HEADER + index * ROW + 16;
                let count =
                    u64::from_le_bytes(samples.bytes[at..at + 8].try_into().expect("fixed row"));
                // At most 64 actual retained records contribute; do not reuse
                // the public frequency, which deliberately combines roles.
                samples.bytes[at..at + 8].copy_from_slice(&(count + 1).to_le_bytes());
            } else {
                let at = HEADER + rows * ROW;
                samples.bytes[at..at + 4].copy_from_slice(&module.to_le_bytes());
                samples.bytes[at + 4..at + 8].copy_from_slice(&role.to_le_bytes());
                samples.bytes[at + 8..at + 16].copy_from_slice(&offset.to_le_bytes());
                samples.bytes[at + 16..at + 24].copy_from_slice(&1_u64.to_le_bytes());
                rows += 1;
            }
        }
        samples.bytes[..8].copy_from_slice(b"PXSAMP01");
        samples.bytes[8..12].copy_from_slice(&(rows as u32).to_le_bytes());
        // Bit0: ordinary observer disabled; bit1: its arithmetic overflow.
        // These describe the retained recent samples, never guest readiness.
        let flags = u32::from(self.disabled) | (u32::from(self.overflow) << 1);
        samples.bytes[12..16].copy_from_slice(&flags.to_le_bytes());
        samples.length = HEADER + rows * ROW;
    }
}

#[cfg(test)]
mod tests;
