// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned guarded fixed-profile argument/stack storage.
// Entry placement is derived from WerrySs/KytyPS5-Performance, commit
// 209dffe9032d72749257ce18680d506b891284de, src/loader/runtimeLinker.cpp,
// Execute/RunEntry. Preserve LICENSES/Kyty-MIT.txt with the upstream GPLv2 grant.

use super::Region;
use crate::{GUEST_PAGE_SIZE, PlatformError};

const USABLE_BYTES: usize = 16 * GUEST_PAGE_SIZE;
const RESERVATION_BYTES: usize = USABLE_BYTES + 2 * GUEST_PAGE_SIZE;

/// The fork's fixed 32-byte prefix/layout; all three pointers are native owned
/// addresses. Natural C layout has the same offsets as the packed source object.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct EntryParams {
    pub argc: i32,
    pub pad: u32,
    pub argv: [u64; 3],
}

const _: () = assert!(std::mem::size_of::<EntryParams>() == 32);
const _: () = assert!(std::mem::offset_of!(EntryParams, argv) == 8);

/// A fully committed, non-growing stack between two inaccessible reservations.
/// No pointer/slice or execution capability escapes the platform implementation.
pub(super) struct GuestStack {
    _region: Region,
    pub params: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub base: u64,
    pub limit: u64,
}

impl GuestStack {
    pub fn new() -> Result<Self, PlatformError> {
        let mut region = Region::reserve(RESERVATION_BYTES)?;
        region.commit_writable_range(GUEST_PAGE_SIZE, USABLE_BYTES)?;
        let limit = region.address() + GUEST_PAGE_SIZE as u64;
        let base = limit + USABLE_BYTES as u64;
        let params = (base - 0x100) & !15;
        let argument = params + std::mem::size_of::<EntryParams>() as u64;
        let object = EntryParams {
            argc: 1,
            pad: 0,
            argv: [argument, 0, 0],
        };
        let params_offset = (params - region.address()) as usize;
        let mut encoded = [0_u8; 32];
        encoded[..4].copy_from_slice(&object.argc.to_le_bytes());
        for (index, value) in object.argv.iter().enumerate() {
            encoded[8 + index * 8..16 + index * 8].copy_from_slice(&value.to_le_bytes());
        }
        crate::checked_range(RESERVATION_BYTES, params_offset, encoded.len() + 7)?;
        region.write(params_offset, &encoded)?;
        region.write(params_offset + encoded.len(), b"PentaX\0")?;
        let rsp = ((params - 0x1000) & !15) - 16;
        let root_offset = (rsp - region.address()) as usize;
        crate::checked_range(RESERVATION_BYTES, root_offset, 16)?;
        region.write(root_offset, &[0; 16])?;
        Ok(Self {
            _region: region,
            params,
            rsp,
            rbp: rsp,
            base,
            limit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RESERVE, PAGE_READWRITE};

    #[test]
    fn owned_stack_has_inaccessible_guards_rw_nx_storage_and_fixed_root_layout() {
        let stack = GuestStack::new().unwrap();
        let low_guard = stack.limit - GUEST_PAGE_SIZE as u64;
        for address in [low_guard, stack.base] {
            let information = super::super::tests::query(address);
            assert_eq!(information.State, MEM_RESERVE);
        }
        for address in (stack.limit..stack.base).step_by(GUEST_PAGE_SIZE) {
            let information = super::super::tests::query(address);
            assert_eq!(information.State, MEM_COMMIT);
            assert_eq!(information.Protect, PAGE_READWRITE);
        }
        assert_eq!(stack.rsp % 16, 0);
        assert_eq!(stack.rbp, stack.rsp);
        assert!(stack.limit + 128 < stack.rsp);
        assert!(stack.rsp + 16 < stack.params);
        // SAFETY: Both pointers name aligned initialized objects inside stack's
        // live uniquely owned committed allocation; this test only reads them.
        let (params, root) = unsafe {
            (
                (stack.params as *const EntryParams).read(),
                (stack.rbp as *const [u64; 2]).read(),
            )
        };
        assert_eq!(params.argc, 1);
        assert_eq!(params.pad, 0);
        assert_eq!(&params.argv[1..], &[0, 0]);
        assert_eq!(root, [0, 0]);
        super::super::LAST_RELEASE.with(|slot| slot.set(None));
        drop(stack);
        assert_eq!(
            super::super::LAST_RELEASE.with(std::cell::Cell::take),
            Some((low_guard, true))
        );
    }
}
