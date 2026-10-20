// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: prepared guest TLS identities and checked backing.
// Adapted from pinned tls_get_addr; see docs/upstream-provenance.md.

use super::{KernelMemory, error};
use pentax_kernel::{Failure, GuestError};
use pentax_platform::native_memory::NativeMemoryError;

pub(super) fn address(
    memory: &mut KernelMemory<'_, '_, '_>,
    descriptor: u64,
) -> Result<u64, Failure> {
    let source = memory
        .source
        .ok_or(Failure::Unsupported("image-bound TLS source"))?;
    let instance = memory
        .thread_tls
        .ok_or(Failure::Unsupported("admitted thread TLS"))?;
    let maximum = memory
        .lookup_limits
        .ok_or(Failure::Unsupported("TLS lookup budget"))?
        .max_work;
    let work = memory
        .lookup_work
        .as_deref_mut()
        .ok_or(Failure::Unsupported("TLS lookup budget"))?;
    *work = work
        .checked_add(instance.blocks.len() as u64 + source.modules().len() as u64 + 64)
        .filter(|work| *work <= maximum)
        .ok_or(Failure::Limit("TLS lookup work"))?;
    let mut bytes = [0; 16];
    memory
        .paused
        .read(descriptor, &mut bytes)
        .map_err(|e| Failure::Guest(error(e, false)))?;
    let key = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let offset = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    let block = instance
        .blocks
        .iter()
        .find(|block| block.runtime_id.get() == key)
        .ok_or(Failure::Unsupported("unknown prepared TLS identity"))?;
    let module = source
        .modules()
        .get(block.module.index())
        .ok_or(Failure::Unsupported("TLS source module"))?;
    if module
        .runtime_plan()
        .tls
        .as_ref()
        .is_none_or(|tls| tls.mem_size != block.size)
        || offset >= block.size
    {
        return Err(Failure::Unsupported("TLS descriptor extent"));
    }
    let address = block
        .address
        .checked_add(offset)
        .ok_or(Failure::Guest(GuestError::AddressOverflow))?;
    memory
        .paused
        .preflight_read(address, 1)
        .map_err(|e| Failure::Guest(error(e, false)))?;
    memory
        .paused
        .preflight_write(address, 1)
        .map_err(|e| Failure::Guest(error(e, true)))?;
    if !matches!(
        memory.paused.preflight_fetch_source(address, 1),
        Err(NativeMemoryError::PermissionDenied(_))
    ) {
        return Err(Failure::Unsupported("TLS block must remain NX"));
    }
    Ok(address)
}
