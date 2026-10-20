// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from pinned libKernel.cpp and RuntimeLinker::SetApplicationHeapApi.
// See NOTICE and docs/upstream-provenance.md.

use crate::{Failure, GuestAccess, Kernel};

/// Application-owned allocator callback addresses retained as guest data.
/// These are not host pointers or execution authority. Any caller must admit
/// the live source and invoke through the owned guest runtime.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ApplicationHeapCallbacks {
    /// Source signature: void* malloc(uint64_t size).
    pub malloc: u64,
    /// Source signature: void free(void* allocation).
    pub free: u64,
    /// Source signature: int posix_memalign(void** output, uint64_t alignment, uint64_t size).
    pub posix_memalign: Option<u64>,
}

impl Kernel {
    /// Snapshot of the registered guest allocator, not a callable host interface.
    #[must_use]
    pub const fn application_heap_callbacks(&self) -> Option<ApplicationHeapCallbacks> {
        self.application_heap
    }

    pub(crate) fn set_application_heap_api(
        &mut self,
        address: u64,
        guest: &impl GuestAccess,
    ) -> Result<u64, Failure> {
        let next = if address == 0 {
            None
        } else {
            // The source library wrapper consumes a ten-pointer table. Read the
            // complete bounded extent before changing the existing registration.
            let mut table = [0; 10 * 8];
            self.budget.read(guest, address, &mut table)?;
            let word =
                |slot: usize| u64::from_le_bytes(table[slot * 8..slot * 8 + 8].try_into().unwrap());
            let malloc = word(0);
            let free = word(1);
            let memalign = word(6);
            if malloc == 0 || free == 0 {
                None
            } else {
                Some(ApplicationHeapCallbacks {
                    malloc,
                    free,
                    posix_memalign: (memalign != 0).then_some(memalign),
                })
            }
        };
        // Replacement and disabling are supported; no callback is invoked.
        self.application_heap = next;
        Ok(0) // The source library function returns void.
    }
}
