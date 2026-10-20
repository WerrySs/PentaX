// SPDX-License-Identifier: GPL-2.0-only

//! Sealed service/staging operations: implementations retain their actual
//! backing owner. No caller-defined trait implementation supplies fake checks.

use super::{FallibleWriteError, GuestMemory, MemoryError, Permissions};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryError, NativePermissions, PausedNativeMemory,
};

pub(super) mod sealed {
    pub trait Sealed {}
}
pub(crate) trait ServiceMemory: sealed::Sealed {
    fn preflight_read(&self, address: u64, size: u64) -> Result<(), MemoryError>;
    fn preflight_write(&self, address: u64, size: u64) -> Result<(), MemoryError>;
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError>;
    fn fetch(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError>;
    fn write_fallible(&mut self, address: u64, bytes: &[u8]) -> Result<(), FallibleWriteError>;
}
impl sealed::Sealed for GuestMemory {}
impl ServiceMemory for GuestMemory {
    fn preflight_read(&self, a: u64, n: u64) -> Result<(), MemoryError> {
        self.preflight_read(a, n)
    }
    fn preflight_write(&self, a: u64, n: u64) -> Result<(), MemoryError> {
        self.preflight_write(a, n)
    }
    fn read(&self, a: u64, b: &mut [u8]) -> Result<(), MemoryError> {
        self.read(a, b)
    }
    fn fetch(&self, a: u64, b: &mut [u8]) -> Result<(), MemoryError> {
        self.fetch(a, b)
    }
    fn write_fallible(&mut self, a: u64, b: &[u8]) -> Result<(), FallibleWriteError> {
        self.write_fallible(a, b)
    }
}
impl sealed::Sealed for PausedNativeMemory<'_> {}
impl ServiceMemory for PausedNativeMemory<'_> {
    fn preflight_read(&self, a: u64, n: u64) -> Result<(), MemoryError> {
        self.preflight_read(a, n).map_err(convert)
    }
    fn preflight_write(&self, a: u64, n: u64) -> Result<(), MemoryError> {
        self.preflight_write(a, n).map_err(convert)
    }
    fn read(&self, a: u64, b: &mut [u8]) -> Result<(), MemoryError> {
        self.read(a, b).map_err(convert)
    }
    fn fetch(&self, a: u64, b: &mut [u8]) -> Result<(), MemoryError> {
        self.fetch_source(a, b).map_err(convert)
    }
    fn write_fallible(&mut self, a: u64, b: &[u8]) -> Result<(), FallibleWriteError> {
        self.write(a, b)
            .map_err(|e| FallibleWriteError::Memory(convert(e)))
    }
}

pub(crate) trait StagingMemory {
    fn owns_guard_reservation(&self, _address: u64, _size: u64) -> bool {
        true
    }
    fn read(&mut self, address: u64, bytes: &mut [u8]) -> Result<(), MemoryError>;
    fn map(&mut self, address: u64, size: u64, permissions: Permissions)
    -> Result<(), MemoryError>;
    fn protect(
        &mut self,
        address: u64,
        size: u64,
        permissions: Permissions,
    ) -> Result<(), MemoryError>;
    fn write(&mut self, address: u64, bytes: &[u8]) -> Result<(), MemoryError>;
}
impl StagingMemory for GuestMemory {
    fn read(&mut self, a: u64, b: &mut [u8]) -> Result<(), MemoryError> {
        GuestMemory::read(self, a, b)
    }
    fn map(&mut self, a: u64, n: u64, p: Permissions) -> Result<(), MemoryError> {
        self.map(a, n, p)
    }
    fn protect(&mut self, a: u64, n: u64, p: Permissions) -> Result<(), MemoryError> {
        self.protect(a, n, p)
    }
    fn write(&mut self, a: u64, b: &[u8]) -> Result<(), MemoryError> {
        self.write(a, b)
    }
}
impl StagingMemory for NativeAddressSpace {
    fn owns_guard_reservation(&self, address: u64, size: u64) -> bool {
        address.checked_add(size).is_some_and(|end| {
            self.reserved_ranges()
                .any(|range| range.address <= address && end <= range.address + range.size)
        })
    }
    fn read(&mut self, a: u64, b: &mut [u8]) -> Result<(), MemoryError> {
        self.pause().map_err(convert)?.read(a, b).map_err(convert)
    }
    fn map(&mut self, a: u64, n: u64, p: Permissions) -> Result<(), MemoryError> {
        self.map_at(a, n, native_permissions(p)).map_err(convert)
    }
    fn protect(&mut self, a: u64, n: u64, p: Permissions) -> Result<(), MemoryError> {
        self.protect(a, n, native_permissions(p)).map_err(convert)
    }
    fn write(&mut self, a: u64, b: &[u8]) -> Result<(), MemoryError> {
        self.pause().map_err(convert)?.write(a, b).map_err(convert)
    }
}
pub(crate) fn native_permissions(p: Permissions) -> NativePermissions {
    NativePermissions::new(
        p.contains(Permissions::READ),
        p.contains(Permissions::WRITE),
        p.contains(Permissions::EXECUTE),
    )
}
pub(crate) fn convert(error: NativeMemoryError) -> MemoryError {
    match error {
        NativeMemoryError::InvalidRange => MemoryError::InvalidRange,
        NativeMemoryError::Unaligned => MemoryError::Unaligned,
        NativeMemoryError::Overlap => MemoryError::Overlap,
        NativeMemoryError::BudgetExceeded => MemoryError::BudgetExceeded,
        NativeMemoryError::Unmapped(a) => MemoryError::Unmapped(a),
        NativeMemoryError::PermissionDenied(a) => MemoryError::PermissionDenied(a),
        other => MemoryError::Native(other),
    }
}
