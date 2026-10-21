// SPDX-License-Identifier: GPL-2.0-only
//! Explicit private terminal CPU observations. Never execution authority or
//! public protocol data. Guest register values may themselves contain pointers.

use super::NativeCpuState;
use pentax_linker::{SourceIdentity, SymbolicLinkPlan};

const HEADER: usize = 16;
const ROW: usize = 160;
const MAX_BYTES: usize = HEADER + 2 * ROW;
const MAX_WORK: u64 = 64 * 1024 * 1024;

pub(super) struct PrivateThreadStates {
    requested: bool,
    epoch: Option<SourceIdentity>,
    work: u64,
    bytes: [u8; MAX_BYTES],
    length: usize,
}

impl Default for PrivateThreadStates {
    fn default() -> Self {
        Self {
            requested: false,
            epoch: None,
            work: 0,
            bytes: [0; MAX_BYTES],
            length: 0,
        }
    }
}

impl std::fmt::Debug for PrivateThreadStates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PrivateThreadStates { payload: redacted }")
    }
}

impl PrivateThreadStates {
    pub(super) fn enable(&mut self) {
        self.requested = true;
    }

    pub(super) fn begin_epoch(&mut self, epoch: SourceIdentity) {
        if self.requested {
            self.epoch = Some(epoch);
            self.length = 0;
        }
    }

    pub(super) fn bytes(&self) -> Option<&[u8]> {
        (self.length != 0).then_some(&self.bytes[..self.length])
    }

    fn admit(&mut self, cost: u64) -> bool {
        let Some(next) = self.work.checked_add(cost).filter(|n| *n <= MAX_WORK) else {
            return false;
        };
        self.work = next;
        true
    }

    /// States are borrowed from the actual current executor and its scheduler,
    /// while both remain alive in this exact epoch. Slot zero is main; slot one
    /// is the active child, if any. No wait predicate or guest memory is read.
    pub(super) fn capture(
        &mut self,
        source: &SymbolicLinkPlan<'_>,
        states: [Option<&NativeCpuState>; 2],
    ) {
        if !self.requested {
            return;
        }
        self.length = 0;
        if self.epoch != Some(source.identity()) {
            return;
        }
        // Admit the metadata-count fold before walking it. The separate finite
        // observer allowance never debits or changes guest execution budgets.
        let Some(fold) = (source.modules().len() as u64).checked_add(1) else {
            return;
        };
        if !self.admit(fold) {
            return;
        }
        let Some(segments) = source.modules().iter().try_fold(0_u64, |n, module| {
            n.checked_add(module.runtime_plan().segments.len() as u64)
        }) else {
            return;
        };
        // Two complete checked geometry scans, fixed staging reset and every
        // possible encoded byte are admitted before any scan or register copy.
        let cost = segments
            .checked_mul(16)
            .and_then(|n| n.checked_add(fold.checked_mul(4)?))
            .and_then(|n| n.checked_add((2 * MAX_BYTES) as u64));
        let Some(cost) = cost else { return };
        if !self.admit(cost) {
            return;
        }
        self.bytes.fill(0);
        let mut rows = 0;
        for (role, state) in states.into_iter().enumerate() {
            let Some(state) = state else { continue };
            let Some((module, offset)) = normalize(source, state.guest_pc) else {
                return;
            };
            let at = HEADER + rows * ROW;
            self.bytes[at..at + 4].copy_from_slice(&module.to_le_bytes());
            self.bytes[at + 4..at + 8].copy_from_slice(&(role as u32).to_le_bytes());
            self.bytes[at + 8..at + 16].copy_from_slice(&offset.to_le_bytes());
            for (index, value) in state.gpr.iter().enumerate() {
                let start = at + 16 + index * 8;
                self.bytes[start..start + 8].copy_from_slice(&value.to_le_bytes());
            }
            self.bytes[at + 144..at + 152].copy_from_slice(&state.rflags.to_le_bytes());
            // Last eight bytes reserved zero. No absolute PC or host state.
            rows += 1;
        }
        if rows == 0 {
            return;
        }
        self.bytes[..8].copy_from_slice(b"PXCTX001");
        self.bytes[8..12].copy_from_slice(&(rows as u32).to_le_bytes());
        // Header flags reserved zero; availability is expressed by row roles.
        self.length = HEADER + rows * ROW;
    }
}

fn normalize(source: &SymbolicLinkPlan<'_>, pc: u64) -> Option<(u32, u64)> {
    let mut found = None;
    for (index, module) in source.modules().iter().enumerate() {
        let mut initialized_rx = false;
        for segment in &module.runtime_plan().segments {
            if !segment.flags.readable() || !segment.flags.executable() || segment.flags.writable()
            {
                continue;
            }
            let start = module.load_bias().checked_add(segment.vaddr)?;
            let end = start.checked_add(segment.file_bytes.len() as u64)?;
            initialized_rx |= start <= pc && pc < end;
        }
        if initialized_rx {
            if found.is_some() {
                return None; // An ambiguous module mapping is not guessed.
            }
            found = Some((
                u32::try_from(index).ok()?,
                pc.checked_sub(module.load_bias())?,
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests;
