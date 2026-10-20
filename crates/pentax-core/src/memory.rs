// SPDX-License-Identifier: GPL-2.0-or-later
//! Default sparse checked guest memory, plus a separately owned native backing.
//! Sparse addresses never become pointers; native pointers stay inside platform.
use std::{collections::BTreeMap, fmt};

mod fallible;
pub mod native;
pub(crate) mod service;
pub(crate) use fallible::FallibleWriteError;

pub const PAGE_SIZE: u64 = pentax_loader::RUNTIME_PAGE_SIZE;
pub const ADDRESS_LIMIT: u64 = 1 << 47;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions(u8);

impl Permissions {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXECUTE: Self = Self(4);
    pub const READ_WRITE: Self = Self(3);
    pub const READ_EXECUTE: Self = Self(5);
    pub const fn new(read: bool, write: bool, execute: bool) -> Self {
        Self(read as u8 | ((write as u8) << 1) | ((execute as u8) << 2))
    }
    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryError {
    InvalidRange,
    Unaligned,
    Overlap,
    BudgetExceeded,
    Unmapped(u64),
    PermissionDenied(u64),
    Native(pentax_platform::native_memory::NativeMemoryError),
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange => write!(f, "guest address range is invalid"),
            Self::Unaligned => write!(f, "mapping must use 16 KiB page boundaries"),
            Self::Overlap => write!(f, "guest mapping overlaps existing pages"),
            Self::BudgetExceeded => write!(f, "guest resident memory budget exceeded"),
            Self::Unmapped(addr) => write!(f, "unmapped guest address {addr:#x}"),
            Self::PermissionDenied(addr) => write!(f, "guest access denied at {addr:#x}"),
            Self::Native(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for MemoryError {}

#[derive(Debug)]
struct Page {
    permissions: Permissions,
    // Zero-filled pages do not allocate a host buffer until written.
    // Length is exactly PAGE_SIZE. Capacity/allocator overhead is not part of
    // the logical resident-page counter, and backing never grows after setup.
    data: Option<Vec<u8>>,
}

#[derive(Debug)]
pub struct GuestMemory {
    pages: BTreeMap<u64, Page>,
    resident_bytes: u64,
    resident_limit: u64,
    mapped_limit: u64,
}

impl GuestMemory {
    pub fn new(resident_limit: u64, mapped_limit: u64) -> Self {
        Self {
            pages: BTreeMap::new(),
            resident_bytes: 0,
            resident_limit,
            mapped_limit,
        }
    }

    /// Logical backed page bytes, not exact allocator capacity or heap usage.
    pub fn resident_bytes(&self) -> u64 {
        self.resident_bytes
    }
    pub fn mapped_bytes(&self) -> u64 {
        self.pages.len() as u64 * PAGE_SIZE
    }

    pub fn map(
        &mut self,
        address: u64,
        size: u64,
        permissions: Permissions,
    ) -> Result<(), MemoryError> {
        let end = valid_range(address, size)?;
        if !address.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MemoryError::Unaligned);
        }
        let total = self
            .mapped_bytes()
            .checked_add(size)
            .ok_or(MemoryError::BudgetExceeded)?;
        if total > self.mapped_limit {
            return Err(MemoryError::BudgetExceeded);
        }
        if self.pages.range(address..end).next().is_some() {
            return Err(MemoryError::Overlap);
        }
        for base in (address..end).step_by(PAGE_SIZE as usize) {
            self.pages.insert(
                base,
                Page {
                    permissions,
                    data: None,
                },
            );
        }
        Ok(())
    }

    pub fn protect(
        &mut self,
        address: u64,
        size: u64,
        permissions: Permissions,
    ) -> Result<(), MemoryError> {
        let end = valid_range(address, size)?;
        if !address.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MemoryError::Unaligned);
        }
        self.check(address, size, None)?;
        for (_, page) in self.pages.range_mut(address..end) {
            page.permissions = permissions;
        }
        Ok(())
    }

    pub fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.copy_out(address, output, Permissions::READ)
    }

    pub fn fetch(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.copy_out(address, output, Permissions::EXECUTE)
    }

    fn copy_out(
        &self,
        address: u64,
        output: &mut [u8],
        permission: Permissions,
    ) -> Result<(), MemoryError> {
        if output.is_empty() {
            return Ok(());
        }
        self.check(address, output.len() as u64, Some(permission))?;
        let mut copied = 0;
        while copied < output.len() {
            let cursor = address + copied as u64;
            let base = page_base(cursor);
            let offset = (cursor - base) as usize;
            let count = (PAGE_SIZE as usize - offset).min(output.len() - copied);
            let page = &self.pages[&base];
            match &page.data {
                Some(data) => {
                    output[copied..copied + count].copy_from_slice(&data[offset..offset + count])
                }
                None => output[copied..copied + count].fill(0),
            }
            copied += count;
        }
        Ok(())
    }

    /// Writes are preflighted across the whole range, including the resident budget.
    /// Returned errors never partially change guest bytes. Page allocation in
    /// this public path remains infallible; it does not promise OOM recovery.
    pub fn write(&mut self, address: u64, input: &[u8]) -> Result<(), MemoryError> {
        if input.is_empty() {
            return Ok(());
        }
        self.preflight_write(address, input.len() as u64)?;
        let mut copied = 0;
        while copied < input.len() {
            let cursor = address + copied as u64;
            let base = page_base(cursor);
            let offset = (cursor - base) as usize;
            let count = (PAGE_SIZE as usize - offset).min(input.len() - copied);
            let page = self
                .pages
                .get_mut(&base)
                .expect("preflight checked mapping");
            if page.data.is_none() {
                page.data = Some(vec![0; PAGE_SIZE as usize]);
                self.resident_bytes += PAGE_SIZE;
            }
            page.data.as_mut().expect("page allocated")[offset..offset + count]
                .copy_from_slice(&input[copied..copied + count]);
            copied += count;
        }
        Ok(())
    }

    fn check(
        &self,
        address: u64,
        size: u64,
        permission: Option<Permissions>,
    ) -> Result<(), MemoryError> {
        let end = valid_range(address, size)?;
        for base in (page_base(address)..end).step_by(PAGE_SIZE as usize) {
            let page = self
                .pages
                .get(&base)
                .ok_or(MemoryError::Unmapped(base.max(address)))?;
            if permission.is_some_and(|p| !page.permissions.contains(p)) {
                return Err(MemoryError::PermissionDenied(base.max(address)));
            }
        }
        Ok(())
    }
}

pub const fn page_base(address: u64) -> u64 {
    address & !(PAGE_SIZE - 1)
}

fn valid_range(address: u64, size: u64) -> Result<u64, MemoryError> {
    let end = address.checked_add(size).ok_or(MemoryError::InvalidRange)?;
    if size == 0 || end > ADDRESS_LIMIT {
        return Err(MemoryError::InvalidRange);
    }
    Ok(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_zero_pages_and_cross_page_write() {
        let mut mem = GuestMemory::new(PAGE_SIZE * 2, PAGE_SIZE * 2);
        mem.map(0x4000, PAGE_SIZE * 2, Permissions::READ_WRITE)
            .unwrap();
        assert_eq!(mem.resident_bytes(), 0);
        let mut bytes = [7; 8];
        mem.read(0x7ffc, &mut bytes).unwrap();
        assert_eq!(bytes, [0; 8]);
        mem.write(0x7ffc, &[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        mem.read(0x7ffc, &mut bytes).unwrap();
        assert_eq!(bytes, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(mem.resident_bytes(), PAGE_SIZE * 2);
    }

    #[test]
    fn cross_page_fault_is_transactional() {
        let mut mem = GuestMemory::new(PAGE_SIZE * 2, PAGE_SIZE * 2);
        mem.map(0x4000, PAGE_SIZE, Permissions::READ_WRITE).unwrap();
        mem.write(0x7fff, &[9]).unwrap();
        assert_eq!(
            mem.write(0x7fff, &[3, 4]),
            Err(MemoryError::Unmapped(0x8000))
        );
        let mut byte = [0];
        mem.read(0x7fff, &mut byte).unwrap();
        assert_eq!(byte, [9]);
        mem.map(0x8000, PAGE_SIZE, Permissions::READ).unwrap();
        assert_eq!(
            mem.write(0x7fff, &[3, 4]),
            Err(MemoryError::PermissionDenied(0x8000))
        );
        mem.read(0x7fff, &mut byte).unwrap();
        assert_eq!(byte, [9]);
    }

    #[test]
    fn budget_fault_does_not_allocate_or_write() {
        let mut mem = GuestMemory::new(PAGE_SIZE, PAGE_SIZE * 2);
        mem.map(0x4000, PAGE_SIZE * 2, Permissions::READ_WRITE)
            .unwrap();
        assert_eq!(mem.write(0x7fff, &[1, 2]), Err(MemoryError::BudgetExceeded));
        assert_eq!(mem.resident_bytes(), 0);
    }

    #[test]
    fn protections_overlaps_and_overflow_are_enforced() {
        let mut mem = GuestMemory::new(PAGE_SIZE, PAGE_SIZE);
        mem.map(0x4000, PAGE_SIZE, Permissions::READ_WRITE).unwrap();
        assert!(mem.map(0x4000, PAGE_SIZE, Permissions::READ).is_err());
        mem.protect(0x4000, PAGE_SIZE, Permissions::READ_EXECUTE)
            .unwrap();
        assert!(mem.write(0x4000, &[0]).is_err());
        assert!(mem.read(u64::MAX, &mut [0; 2]).is_err());
        assert!(
            mem.map(ADDRESS_LIMIT, PAGE_SIZE, Permissions::READ)
                .is_err()
        );
    }
}
