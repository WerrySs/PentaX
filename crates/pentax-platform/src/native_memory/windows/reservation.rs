// SPDX-License-Identifier: GPL-2.0-only

//! Exact owned placeholder slots. Every state transition retains its address.
use super::*;
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_PRESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER, MEM_RESERVE_PLACEHOLDER,
    MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile3, UnmapViewOfFile2, VirtualAlloc2,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegionKind {
    Placeholder,
    Private,
    Shared { physical: u64 },
    Released,
}

#[derive(Debug)]
pub(super) struct Region {
    pub(super) address: u64,
    size: u64,
    pub(super) kind: RegionKind,
}
impl Region {
    pub(super) fn map_private(
        &mut self,
        permissions: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        debug_assert_eq!(self.kind, RegionKind::Placeholder);
        debug_assert_eq!(self.size, NATIVE_GUEST_PAGE);
        // SAFETY: This exact placeholder is exclusively owned. Replacement
        // never releases the address, and the private allocation stays NX.
        let raw = unsafe {
            VirtualAlloc2(
                GetCurrentProcess(),
                self.address as usize as *const c_void,
                self.size as usize,
                MEM_RESERVE | MEM_COMMIT | MEM_REPLACE_PLACEHOLDER,
                physical(permissions),
                std::ptr::null_mut(),
                0,
            )
        };
        if raw.is_null() {
            return Err(os_error("VirtualAlloc2(private placeholder)", self.address));
        }
        self.kind = RegionKind::Private;
        Ok(())
    }
    pub(super) fn map_shared(
        &mut self,
        section: &mut Section,
        offset: u64,
        physical_address: u64,
        permissions: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        debug_assert_eq!(self.kind, RegionKind::Placeholder);
        debug_assert_eq!(self.size, NATIVE_GUEST_PAGE);
        // SAFETY: The owner admits the entire section offset before this call.
        // The exact 16 KiB owned placeholder is replaced, without an address
        // release window; the pagefile section and every view remain NX.
        let view = unsafe {
            MapViewOfFile3(
                section.handle,
                GetCurrentProcess(),
                self.address as usize as *const c_void,
                offset,
                self.size as usize,
                MEM_REPLACE_PLACEHOLDER,
                PAGE_READWRITE,
                std::ptr::null_mut(),
                0,
            )
        };
        if view.Value.is_null() {
            return Err(os_error("MapViewOfFile3(NX placeholder)", self.address));
        }
        self.kind = RegionKind::Shared {
            physical: physical_address,
        };
        // A view's initial mapping access is its maximum future protection.
        // Admit RW at creation, then restrict the unpublished NX view before
        // recording logical permissions or copying any outputs. This permits
        // later read-only/no-access -> RW protection changes without COW.
        let ready = section.commit_view(self.address, offset).and_then(|()| {
            if physical(permissions) != PAGE_READWRITE {
                protect_page(self.address, physical(permissions))
            } else {
                Ok(())
            }
        });
        if let Err(error) = ready {
            return Err(if self.unmap().is_ok() {
                error
            } else {
                NativeMemoryError::Poisoned
            });
        }
        Ok(())
    }
    pub(super) fn unmap(&mut self) -> Result<(), NativeMemoryError> {
        let success = match self.kind {
            RegionKind::Private => {
                // SAFETY: This is one exact private replacement allocation.
                // Restore its placeholder, retaining exclusive address ownership.
                (unsafe {
                    VirtualFree(
                        self.address as usize as *mut c_void,
                        self.size as usize,
                        MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                    )
                }) != 0
            }
            RegionKind::Shared { .. } => {
                // SAFETY: This is the exact base of one owned section view.
                // Preserve its placeholder and the separately retained section.
                (unsafe {
                    UnmapViewOfFile2(
                        GetCurrentProcess(),
                        MEMORY_MAPPED_VIEW_ADDRESS {
                            Value: self.address as usize as *mut c_void,
                        },
                        MEM_PRESERVE_PLACEHOLDER,
                    )
                }) != 0
            }
            _ => return Err(NativeMemoryError::Unmapped(self.address)),
        };
        if !success {
            return Err(os_error("restore owned placeholder", self.address));
        }
        self.kind = RegionKind::Placeholder;
        Ok(())
    }
    fn release(&mut self) -> bool {
        if self.kind == RegionKind::Released {
            return true;
        }
        if matches!(self.kind, RegionKind::Shared { .. }) && self.unmap().is_err() {
            return false;
        }
        // SAFETY: Each recorded region is an exact owned placeholder/private
        // allocation base. Successful release revokes only this region once.
        if unsafe { VirtualFree(self.address as usize as *mut c_void, 0, MEM_RELEASE) } == 0 {
            return false;
        }
        self.kind = RegionKind::Released;
        true
    }
}

#[derive(Debug)]
pub(super) struct Reservation {
    base: NonNull<c_void>,
    pub(super) size: usize,
    regions: Vec<Region>,
    released: bool,
}
impl Reservation {
    pub(super) fn address(&self) -> u64 {
        self.base.as_ptr() as u64
    }
    pub(super) fn end(&self) -> u64 {
        self.address() + self.size as u64
    }
    pub(super) fn region(&self, address: u64) -> &Region {
        let index = self.regions.partition_point(|r| r.address <= address) - 1;
        &self.regions[index]
    }
    pub(super) fn region_mut(&mut self, address: u64) -> &mut Region {
        let index = self.regions.partition_point(|r| r.address <= address) - 1;
        &mut self.regions[index]
    }
    /// Split only a range about to be mapped. Unused multi-GiB windows retain a
    /// single placeholder. Successful splits never release addresses or change
    /// guest mappings; a later failure leaves every exact region owned.
    pub(super) fn prepare_pages(&mut self, start: u64, end: u64) -> Result<(), NativeMemoryError> {
        debug_assert!(start >= self.address() && end <= self.end() && start < end);
        debug_assert!(start.is_multiple_of(NATIVE_GUEST_PAGE));
        debug_assert!(end.is_multiple_of(NATIVE_GUEST_PAGE));
        self.regions
            .try_reserve((end - start) as usize / NATIVE_GUEST_PAGE as usize + 1)
            .map_err(|_| NativeMemoryError::Allocation)?;
        for boundary in (start..=end).step_by(NATIVE_GUEST_PAGE as usize) {
            if boundary == self.end() {
                break;
            }
            let index = self.regions.partition_point(|r| r.address <= boundary) - 1;
            let region = &self.regions[index];
            if region.address == boundary {
                continue;
            }
            debug_assert_eq!(region.kind, RegionKind::Placeholder);
            let left = boundary - region.address;
            let right = Region {
                address: boundary,
                size: region.size - left,
                kind: RegionKind::Placeholder,
            };
            // SAFETY: Split one exact owned placeholder at an interior aligned
            // boundary. Both halves stay reserved and NX. Descriptor capacity
            // was admitted above, so publishing the split cannot allocate.
            if unsafe {
                VirtualFree(
                    region.address as usize as *mut c_void,
                    left as usize,
                    MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                )
            } == 0
            {
                return Err(os_error("split owned placeholder", region.address));
            }
            self.regions[index].size = left;
            self.regions.insert(index + 1, right);
        }
        Ok(())
    }
    pub(super) fn reserve(address: Option<u64>, size: u64) -> Result<Self, NativeMemoryError> {
        let mut regions = Vec::new();
        regions
            .try_reserve_exact(1)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let target = address.map_or(std::ptr::null(), |v| v as usize as *const c_void);
        // SAFETY: Fresh reservation cannot replace anyone else's allocation.
        // The caller admits aligned size/address and descriptor capacity first.
        let raw = unsafe {
            VirtualAlloc2(
                std::ptr::null_mut(),
                target,
                size as usize,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                std::ptr::null_mut(),
                0,
            )
        };
        let base = NonNull::new(raw)
            .ok_or_else(|| os_error("VirtualAlloc2(placeholder)", address.unwrap_or(0)))?;
        regions.push(Region {
            address: base.as_ptr() as u64,
            size,
            kind: RegionKind::Placeholder,
        });
        let mut owned = Self {
            base,
            size: size as usize,
            regions,
            released: false,
        };
        let result = (|| {
            if owned.address() == 0
                || owned.end() > NATIVE_ADDRESS_LIMIT
                || address.is_some_and(|expected| expected != owned.address())
            {
                return Err(NativeMemoryError::InvalidRange);
            }
            Ok(())
        })();
        if let Err(error) = result {
            return Err(if owned.release() {
                error
            } else {
                NativeMemoryError::Poisoned
            });
        }
        Ok(owned)
    }
    pub(super) fn release(&mut self) -> bool {
        if self.released {
            return true;
        }
        let mut success = true;
        for region in &mut self.regions {
            success &= region.release();
        }
        #[cfg(test)]
        let _ = LAST_RELEASE.try_with(|slot| slot.set(Some((self.address(), success))));
        self.released = success;
        success
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_reservation_splits_only_used_pages_and_releases_owned_regions() {
        let mut reservation = Reservation::reserve(None, 32 * 1024 * 1024 * 1024).unwrap();
        assert_eq!(reservation.regions.len(), 1);
        let middle = reservation.address() + 8 * 1024 * 1024 * 1024;
        reservation
            .prepare_pages(middle, middle + 3 * NATIVE_GUEST_PAGE)
            .unwrap();
        assert_eq!(reservation.regions.len(), 5);
        reservation
            .prepare_pages(middle + NATIVE_GUEST_PAGE, middle + 2 * NATIVE_GUEST_PAGE)
            .unwrap();
        assert_eq!(reservation.regions.len(), 5);
        for address in [
            reservation.address(),
            middle,
            reservation.end() - NATIVE_GUEST_PAGE,
        ] {
            reservation
                .prepare_pages(address, address + NATIVE_GUEST_PAGE)
                .unwrap();
            assert_eq!(reservation.region(address).size, NATIVE_GUEST_PAGE);
            reservation
                .region_mut(address)
                .map_private(NativePermissions::READ)
                .unwrap();
        }
        assert_eq!(reservation.regions.len(), 7);
        reservation.region_mut(middle).unmap().unwrap();
        reservation
            .region_mut(middle)
            .map_private(NativePermissions::READ)
            .unwrap();
        assert!(reservation.release());
        assert!(
            reservation
                .regions
                .iter()
                .all(|r| r.kind == RegionKind::Released)
        );
        assert!(reservation.release());
    }
}
