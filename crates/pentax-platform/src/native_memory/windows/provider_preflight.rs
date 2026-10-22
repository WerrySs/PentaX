// SPDX-License-Identifier: GPL-2.0-only
//! Positive permission metadata only, bound to the actual paused address owner.
use super::*;

impl PausedNativeMemory<'_> {
    /// Exact full-span logical READ+EXECUTE with CPU WRITE forbidden. Physical
    /// pages remain NX under the address-space invariant. This retains the
    /// provider code profile; it never retains source bytes or a marker proof.
    pub fn preflight_provider_code(
        &self,
        address: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.provider_permission_span(address, size, false)
    }

    /// Exact full-span CPU READ with CPU WRITE, GPU WRITE and logical EXECUTE
    /// forbidden. No payload or provider identity is inferred from permissions.
    pub fn preflight_provider_metadata(
        &self,
        address: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.provider_permission_span(address, size, true)
    }

    fn provider_permission_span(
        &self,
        address: u64,
        size: u64,
        metadata: bool,
    ) -> Result<(), NativeMemoryError> {
        self.owner.live()?;
        if size == 0 {
            return Ok(());
        }
        let span = NativeRange { address, size };
        let slot = usize::from(metadata);
        let mut proofs = self.owner.provider_preflight.get();
        if proofs[slot] == Some(span) {
            return Ok(());
        }
        // A failure never retains the previous key as a negative/partial proof.
        proofs[slot] = None;
        self.owner.provider_preflight.set(proofs);
        #[cfg(test)]
        {
            let mut scans = self.owner.provider_preflight_scans.get();
            scans[slot] = scans[slot].saturating_add(1);
            self.owner.provider_preflight_scans.set(scans);
        }
        let (required, forbidden) = if metadata {
            (NativePermissions::READ, 2 | 4 | 0x20)
        } else {
            (NativePermissions::READ_EXECUTE, 2)
        };
        self.owner
            .check_permissions(address, size, Some(required), forbidden)?;
        proofs[slot] = Some(span);
        self.owner.provider_preflight.set(proofs);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
