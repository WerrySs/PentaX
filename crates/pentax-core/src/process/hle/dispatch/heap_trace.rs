// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from pinned libC.cpp LibcInternalExt::LibcHeapGetTraceInfo; see NOTICE.

use super::*;
use crate::process::hle::provider::HEAP_TRACE_BYTES;

impl ServiceBudget {
    pub(super) fn heap_trace_info(
        &mut self,
        function: KnownFunction,
        output: u64,
        memory: &mut impl ServiceMemory,
    ) -> Result<ServiceReturn, MemoryServiceError> {
        // Fixed record extent; unused RSI/RDX never influence reads or work.
        self.admit(function, 32)?;
        check_pointer(output, 32, PointerArgument::First)?;
        let address = self
            .heap_trace_address
            .ok_or(MemoryServiceError::InvalidServiceState(
                "heap trace storage unavailable",
            ))?;
        let mut size = [0; 8];
        memory.read(output, &mut size)?;
        if u64::from_le_bytes(size) != 32 {
            return Err(MemoryServiceError::InvalidServiceState(
                "heap trace record size",
            ));
        }
        // The stored addresses belong to this exact prepared provider. Require
        // the full live writable data extent on every call; never reset it here.
        memory.preflight_read(address, HEAP_TRACE_BYTES)?;
        memory.preflight_write(address, HEAP_TRACE_BYTES)?;
        if !matches!(
            memory.fetch(address, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ) {
            return Err(MemoryServiceError::InvalidServiceState(
                "heap trace storage must be NX",
            ));
        }
        let mut pointers = [0; 16];
        pointers[..8].copy_from_slice(&address.to_le_bytes());
        pointers[8..].copy_from_slice(&(address + 8).to_le_bytes());
        // One complete write preserves size and both unknown 32-bit words,
        // and cannot partially publish one pointer on a boundary failure.
        memory.write_fallible(output + 16, &pointers)?;
        Ok(self.complete(function, 0)) // Source returns void.
    }
}
