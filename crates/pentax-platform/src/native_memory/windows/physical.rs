// SPDX-License-Identifier: GPL-2.0-only

//! Private pagefile sections retain physical bytes independently of their views.
//! Eager fixtures and explicit first-view commitment share the same capacity cap.
use super::*;
use std::os::windows::io::AsRawHandle;
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::Memory::{CreateFileMappingW, MEM_COMMIT, SEC_RESERVE, VirtualAlloc},
};

#[derive(Debug, Clone, Copy)]
pub(super) struct PhysicalAllocation {
    pub(super) start: u64,
    pub(super) size: u64,
    pub(super) section: usize,
    pub(super) offset: u64,
}
#[derive(Debug)]
pub(super) struct Section {
    pub(super) handle: HANDLE,
    closed: bool,
    size: u64,
    committed: u64,
    deferred_pages: Option<Vec<u64>>,
    backing_file: Option<std::fs::File>,
}
impl Section {
    fn new(size: u64, deferred: bool) -> Result<Self, NativeMemoryError> {
        // One bit per guest page. Admit all bookkeeping before creating the OS
        // section, so successful commits never need another allocation.
        let deferred_pages = if deferred {
            let count = usize::try_from((size / NATIVE_GUEST_PAGE).div_ceil(64))
                .map_err(|_| NativeMemoryError::Allocation)?;
            let mut pages = Vec::new();
            pages
                .try_reserve_exact(count)
                .map_err(|_| NativeMemoryError::Allocation)?;
            pages.resize(count, 0);
            Some(pages)
        } else {
            None
        };
        // SAFETY: An unnamed, noninheritable pagefile section owns exactly the
        // admitted size. Optional SEC_RESERVE defers commitment to commit_view;
        // PAGE_READWRITE permits data views only, never execute.
        // Null security/name pointers request the default private object.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE | if deferred { SEC_RESERVE } else { 0 },
                (size >> 32) as u32,
                size as u32,
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(os_error("CreateFileMappingW(NX pagefile)", 0));
        }
        Ok(Self {
            handle,
            closed: false,
            size,
            committed: if deferred { 0 } else { size },
            deferred_pages,
            backing_file: None,
        })
    }
    fn new_file(size: u64, backing: &NativeFileBacking) -> Result<Self, NativeMemoryError> {
        let file = backing.create(size)?;
        #[cfg(test)]
        backing.check_stage(5)?;
        // SAFETY: Exclusively owned, zero-initialized sparse file at exactly
        // the admitted size. The unnamed, noninheritable section permits NX
        // shared data views only; retain the file until this section closes.
        let handle = unsafe {
            CreateFileMappingW(
                file.as_raw_handle(),
                std::ptr::null(),
                PAGE_READWRITE,
                (size >> 32) as u32,
                size as u32,
                std::ptr::null(),
            )
        };
        if handle.is_null() {
            return Err(os_error("backing file section create", 0));
        }
        Ok(Self {
            handle,
            closed: false,
            size,
            committed: 0,
            deferred_pages: None,
            backing_file: Some(file),
        })
    }
    pub(super) fn file_backed_bytes(&self) -> u64 {
        if self.backing_file.is_some() {
            self.size
        } else {
            0
        }
    }
    pub(super) fn committed_bytes(&self) -> u64 {
        self.committed
    }
    pub(super) fn commit_view(
        &mut self,
        address: u64,
        offset: u64,
    ) -> Result<(), NativeMemoryError> {
        let Some(pages) = &mut self.deferred_pages else {
            return Ok(());
        };
        debug_assert!(offset.is_multiple_of(NATIVE_GUEST_PAGE));
        debug_assert!(offset + NATIVE_GUEST_PAGE <= self.size);
        let page = (offset / NATIVE_GUEST_PAGE) as usize;
        let bit = 1_u64 << (page % 64);
        if pages[page / 64] & bit != 0 {
            return Ok(());
        }
        // SAFETY: The caller has just replaced its exact owned placeholder
        // with a view of this section at this offset. Commit only that 16 KiB
        // view, still NX. Committing a section offset is shared by all aliases.
        // Windows retains this commitment until the section's last reference
        // goes away, even when a later mapping/protection step rolls back.
        let committed = unsafe {
            VirtualAlloc(
                address as usize as *const c_void,
                NATIVE_GUEST_PAGE as usize,
                MEM_COMMIT,
                PAGE_READWRITE,
            )
        };
        if committed.is_null() {
            return Err(os_error("VirtualAlloc(NX section commit)", address));
        }
        pages[page / 64] |= bit;
        self.committed += NATIVE_GUEST_PAGE;
        Ok(())
    }
    fn close(&mut self) -> bool {
        if self.closed {
            return true;
        }
        // SAFETY: Only this owner has the section handle. Successful close is
        // recorded once. Normal release excludes live views. During poisoned
        // destruction any OS view left by failed cleanup retains its own section
        // reference; closing this handle cannot create a dangling mapped view.
        self.closed = unsafe { CloseHandle(self.handle) } != 0;
        if self.closed {
            self.backing_file = None;
        }
        self.closed
    }
}
impl Drop for Section {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl NativeAddressSpace {
    pub(super) fn preflight_physical_allocation(
        &self,
        physical: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.live()?;
        let end = range(physical, size)?;
        if !physical.is_multiple_of(NATIVE_GUEST_PAGE) || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        if self
            .backing_bytes()
            .checked_add(size)
            .is_none_or(|v| v > self.limits.mapped_bytes)
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        if self.file_backing.as_ref().is_some_and(|backing| {
            self.physical_bytes
                .checked_add(size)
                .is_none_or(|n| n > backing.capacity_bytes())
        }) {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        if self
            .physical_allocations
            .iter()
            .any(|a| physical < a.start + a.size && a.start < end)
        {
            return Err(NativeMemoryError::Overlap);
        }
        Ok(())
    }
    pub(super) fn allocate_physical(
        &mut self,
        physical: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.preflight_physical_allocation(physical, size)?;
        self.physical_allocations
            .try_reserve_exact(1)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let slot = self
            .physical_sections
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.physical_sections.len());
        if slot == self.physical_sections.len() {
            self.physical_sections
                .try_reserve_exact(1)
                .map_err(|_| NativeMemoryError::Allocation)?;
        }
        self.operation()?;
        let section = match &self.file_backing {
            Some(backing) => Section::new_file(size, backing)?,
            None => Section::new(size, self.defer_physical_commit)?,
        };
        if slot == self.physical_sections.len() {
            self.physical_sections.push(Some(section));
        } else {
            self.physical_sections[slot] = Some(section);
        }
        self.physical_allocations.push(PhysicalAllocation {
            start: physical,
            size,
            section: slot,
            offset: 0,
        });
        self.physical_allocations.sort_unstable_by_key(|a| a.start);
        self.physical_bytes += size;
        Ok(())
    }
    pub(super) fn physical_allocation(
        &self,
        physical: u64,
        size: u64,
    ) -> Result<&PhysicalAllocation, NativeMemoryError> {
        let end = range(physical, size)?;
        let at = self
            .physical_allocations
            .partition_point(|a| a.start <= physical);
        self.physical_allocations
            .get(at.wrapping_sub(1))
            .filter(|a| end <= a.start + a.size)
            .ok_or(NativeMemoryError::InvalidRange)
    }
    pub(super) fn preflight_physical_map(
        &self,
        address: u64,
        size: u64,
        physical: u64,
        permissions: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.live()?;
        if permissions.executable() {
            return Err(NativeMemoryError::PermissionDenied(address));
        }
        if !physical.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        self.preflight_physical_release(physical, size)?;
        self.preflight_map(address, size, false)?;
        Ok(())
    }
    pub(super) fn map_physical(
        &mut self,
        address: u64,
        size: u64,
        physical: u64,
        permissions: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.preflight_physical_map(address, size, physical, permissions)?;
        self.map_backing_at(address, size, permissions, Some(physical))
    }
    pub(super) fn preflight_physical_release(
        &self,
        physical: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.live()?;
        let end = range(physical, size)?;
        if !physical.is_multiple_of(NATIVE_GUEST_PAGE) || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        let mut cursor = physical;
        while cursor < end {
            let a = self.physical_allocation(cursor, NATIVE_GUEST_PAGE)?;
            cursor = end.min(a.start + a.size);
        }
        Ok(())
    }
}

mod release;
