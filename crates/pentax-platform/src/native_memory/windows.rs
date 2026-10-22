// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use std::{cell::Cell, ffi::c_void, marker::PhantomData, ptr::NonNull, rc::Rc};
use windows_sys::Win32::{
    Foundation::GetLastError,
    System::Memory::{
        MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, VirtualFree,
        VirtualProtect,
    },
};

mod reservation;
use reservation::{Region, RegionKind, Reservation};
mod physical;
use physical::{PhysicalAllocation, Section};
mod file_backing;
pub use file_backing::NativeFileBacking;
mod names;
use names::NameEntry;

#[derive(Debug, Clone, Copy)]
struct Page {
    address: u64,
    permissions: NativePermissions,
    lifetime: u64,
}

struct MappingPreflight {
    end: u64,
    first: u64,
    stop: u64,
    missing: usize,
    reserved: u64,
    count: usize,
}

/// Unique address-space owner; deliberately neither Send nor Sync. There is
/// no execution entry point. Safe borrows cannot race a read/write/protect.
///
/// ```compile_fail
/// use pentax_platform::native_memory::NativeAddressSpace;
/// fn require_send<T: Send>() {}
/// require_send::<NativeAddressSpace>();
/// ```
#[derive(Debug)]
pub struct NativeAddressSpace {
    owner_id: NativeOwnerId,
    reservations: Vec<Reservation>,
    pages: Vec<Page>,
    // One successful exact full-span logical CPU RW+NX proof. It belongs to
    // this owner, contains no payload/pointer and never bypasses live(). Every
    // attempted mapping/protection change clears it before any OS effect.
    stack_preflight: Cell<Option<NativeRange>>,
    // Two exact positive provider permission spans: logical code and metadata.
    // These owner-local proofs contain no payload or execution authority and
    // share the existing before-mutation invalidation point with stack proofs.
    provider_preflight: Cell<[Option<NativeRange>; 2]>,
    names: Vec<NameEntry>,
    name_work: u64,
    next_mapping_lifetime: u64,
    physical_allocations: Vec<PhysicalAllocation>,
    physical_sections: Vec<Option<Section>>,
    physical_bytes: u64,
    defer_physical_commit: bool,
    file_backing: Option<NativeFileBacking>,
    private_commit_limit: u64,
    last_host_failure: Option<NativeHostMemoryFailure>,
    private_bytes: u64,
    limits: NativeMemoryLimits,
    granularity: u64,
    reserved: u64,
    poisoned: bool,
    _thread: PhantomData<Rc<()>>,
    #[cfg(test)]
    gate: Option<usize>,
    #[cfg(test)]
    operations: usize,
    #[cfg(test)]
    rollback_gate: Option<usize>,
    #[cfg(test)]
    rollback_operations: usize,
    #[cfg(test)]
    stack_preflight_scans: Cell<u64>,
    #[cfg(test)]
    provider_preflight_scans: Cell<[u64; 2]>,
}

impl NativeAddressSpace {
    pub fn new(limits: NativeMemoryLimits) -> Result<Self, NativeMemoryError> {
        // Claim identity before any host query/allocation. Failed constructors
        // consume their identity too: neither OS address reuse nor failure can
        // make a later owner compare equal to an earlier one.
        let owner_id = allocate_owner_id(&NEXT_OWNER_ID)?;
        let layout = crate::host_layout().map_err(|_| NativeMemoryError::UnsupportedPageLayout)?;
        if !(layout.allocation_granularity as u64).is_multiple_of(NATIVE_GUEST_PAGE)
            || !NATIVE_GUEST_PAGE.is_multiple_of(layout.page_size as u64)
        {
            return Err(NativeMemoryError::UnsupportedPageLayout);
        }
        Ok(Self {
            owner_id,
            reservations: Vec::new(),
            pages: Vec::new(),
            stack_preflight: Cell::new(None),
            provider_preflight: Cell::new([None; 2]),
            names: Vec::new(),
            name_work: 0,
            next_mapping_lifetime: 1,
            physical_allocations: Vec::new(),
            physical_sections: Vec::new(),
            physical_bytes: 0,
            defer_physical_commit: false,
            file_backing: None,
            private_commit_limit: limits.mapped_bytes,
            last_host_failure: None,
            private_bytes: 0,
            limits,
            granularity: layout.allocation_granularity as u64,
            reserved: 0,
            poisoned: false,
            _thread: PhantomData,
            #[cfg(test)]
            gate: None,
            #[cfg(test)]
            operations: 0,
            #[cfg(test)]
            rollback_gate: None,
            #[cfg(test)]
            rollback_operations: 0,
            #[cfg(test)]
            stack_preflight_scans: Cell::new(0),
            #[cfg(test)]
            provider_preflight_scans: Cell::new([0; 2]),
        })
    }
    /// Stable identity of this exact owner, including after moves. This getter
    /// is diagnostic only; obtain a live pause for any memory capability.
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner_id
    }
    /// Reserve fresh OS-selected guest address space without mapping pages.
    /// Useful when generating original profiles whose bias is not fixed yet.
    pub fn reserve_any(limits: NativeMemoryLimits, size: u64) -> Result<Self, NativeMemoryError> {
        let mut owner = Self::new(limits)?;
        if size == 0
            || !size.is_multiple_of(owner.granularity)
            || size > limits.reserved_bytes
            || size >= NATIVE_ADDRESS_LIMIT
        {
            return Err(NativeMemoryError::InvalidRange);
        }
        owner
            .reservations
            .try_reserve_exact(1)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let reservation = Reservation::reserve(None, size)?;
        owner.reservations.push(reservation);
        owner.reserved = size;
        Ok(owner)
    }
    /// Reserve one fresh OS-selected placeholder wholly inside `window`.
    /// The window has an exclusive end and allocation-granularity alignment.
    /// No pages are committed and no payload/access/execution authority is
    /// created. Later explicit mappings still use the ordinary owner checks.
    /// An unavailable or incompatible host range is refused, never broadened.
    pub fn reserve_within(
        limits: NativeMemoryLimits,
        size: u64,
        window: NativeRange,
    ) -> Result<Self, NativeMemoryError> {
        let mut owner = Self::new(limits)?;
        checked_reservation_window(size, window, owner.granularity, limits.reserved_bytes)?;
        owner
            .reservations
            .try_reserve_exact(1)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let reservation = Reservation::reserve_within(size, window, owner.granularity)?;
        owner.reservations.push(reservation);
        owner.reserved = size;
        Ok(owner)
    }
    pub fn reserved_ranges(&self) -> impl Iterator<Item = NativeRange> + '_ {
        self.reservations.iter().map(|r| NativeRange {
            address: r.address(),
            size: r.size as u64,
        })
    }
    pub fn mapped_bytes(&self) -> u64 {
        self.pages.len() as u64 * NATIVE_GUEST_PAGE
    }
    /// Charge physical pages when their first view is mapped. Whole section
    /// capacity still consumes the backing budget at allocation. This opt-in
    /// must precede all physical allocations; compact owners retain eager mode.
    pub fn defer_physical_commit(&mut self) -> Result<(), NativeMemoryError> {
        self.live()?;
        if self.physical_sections.iter().any(Option::is_some) || self.file_backing.is_some() {
            return Err(NativeMemoryError::InvalidRange);
        }
        self.defer_physical_commit = true;
        Ok(())
    }
    /// Select explicit private file storage before physical allocations. File
    /// capacity and all aliases still consume the ordinary mapped/backing cap;
    /// private pages additionally obey an independent pagefile-commit ceiling.
    pub fn use_file_backing(
        &mut self,
        backing: NativeFileBacking,
        private_commit_limit: u64,
    ) -> Result<(), NativeMemoryError> {
        self.live()?;
        if self.physical_sections.iter().any(Option::is_some)
            || self.file_backing.is_some()
            || private_commit_limit < self.private_bytes
            || private_commit_limit > self.limits.mapped_bytes
        {
            return Err(NativeMemoryError::InvalidRange);
        }
        self.file_backing = Some(backing);
        self.private_commit_limit = self.private_commit_limit.min(private_commit_limit);
        self.defer_physical_commit = false;
        Ok(())
    }
    /// Owned committed private pages and unique section offsets, not RSS.
    /// Unmapping or rolling back views never credits retained section commits.
    pub fn committed_bytes(&self) -> u64 {
        self.private_bytes
            + self
                .physical_sections
                .iter()
                .flatten()
                .map(Section::committed_bytes)
                .sum::<u64>()
    }
    /// Admitted private pages plus entire retained physical-section capacity.
    /// Shared aliases count once, including reserved but uncommitted offsets.
    pub fn backing_bytes(&self) -> u64 {
        self.private_bytes + self.physical_bytes
    }
    pub const fn reserved_bytes(&self) -> u64 {
        self.reserved
    }
    pub fn statistics(&self) -> NativeMemoryStatistics {
        NativeMemoryStatistics {
            mapped_bytes: self.mapped_bytes(),
            committed_bytes: self.committed_bytes(),
            backing_bytes: self.backing_bytes(),
            file_backed_bytes: self
                .physical_sections
                .iter()
                .flatten()
                .map(Section::file_backed_bytes)
                .sum(),
            private_commit_limit_bytes: self.private_commit_limit,
            reserved_bytes: self.reserved,
            limits: self.limits,
            host_failure: self.last_host_failure,
        }
    }
    /// Tighten an owned budget without changing mappings or raising limits.
    pub fn restrict_limits(&mut self, limits: NativeMemoryLimits) -> Result<(), NativeMemoryError> {
        self.live()?;
        if self.mapped_bytes() > limits.mapped_bytes
            || self.backing_bytes() > limits.mapped_bytes
            || self.reserved > limits.reserved_bytes
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        self.limits.mapped_bytes = self.limits.mapped_bytes.min(limits.mapped_bytes);
        self.private_commit_limit = self.private_commit_limit.min(limits.mapped_bytes);
        self.limits.reserved_bytes = self.limits.reserved_bytes.min(limits.reserved_bytes);
        Ok(())
    }
    pub const fn allocation_granularity(&self) -> u64 {
        self.granularity
    }
    fn live(&self) -> Result<(), NativeMemoryError> {
        if self.poisoned {
            Err(NativeMemoryError::Poisoned)
        } else {
            Ok(())
        }
    }
    fn invalidate_stack_preflight(&mut self) {
        // Mutation entry points: map_backing_at (private/shared), map_physical
        // (including its early preflight), protect, unmap, release_physical,
        // compare_replace_words (temporary physical rights), and abandon.
        // Nested region preparation, alias restoration and protection rollback
        // run while empty and never publish a new proof. Future mapping/rights
        // mutators must enter here before validation or any possible OS effect.
        // Ordinary checked CPU/GPU payload writes do not change these metadata.
        self.stack_preflight.set(None);
        self.provider_preflight.set([None; 2]);
    }
    fn operation(&mut self) -> Result<(), NativeMemoryError> {
        #[cfg(test)]
        {
            let index = self.operations;
            self.operations += 1;
            if self.gate == Some(index) {
                return Err(NativeMemoryError::OperatingSystem {
                    operation: "injected",
                    address: 0,
                    code: 8,
                });
            }
        }
        Ok(())
    }
    fn rollback_operation(&mut self) -> bool {
        #[cfg(test)]
        {
            let index = self.rollback_operations;
            self.rollback_operations += 1;
            if self.rollback_gate == Some(index) {
                return false;
            }
        }
        true
    }
    fn rollback_reservations(&mut self, pending: Vec<Reservation>) {
        for mut reservation in pending {
            if self.rollback_operation() && reservation.release() {
                continue;
            }
            // Keep ownership even when release failed. Descriptor capacity was
            // reserved before the first OS mutation, and the owner is fail-stop.
            self.poisoned = true;
            self.reserved += reservation.size as u64;
            self.reservations.push(reservation);
        }
        self.reservations.sort_unstable_by_key(Reservation::address);
    }
    fn index(&self, address: u64) -> Result<usize, usize> {
        self.pages.binary_search_by_key(&address, |p| p.address)
    }
    fn region(&self, address: u64) -> &Region {
        let at = self
            .reservations
            .partition_point(|r| r.address() <= address);
        self.reservations[at - 1].region(address)
    }
    fn region_mut(&mut self, address: u64) -> &mut Region {
        let at = self
            .reservations
            .partition_point(|r| r.address() <= address);
        self.reservations[at - 1].region_mut(address)
    }
    fn contains_reservation(&self, address: u64) -> bool {
        let at = self
            .reservations
            .partition_point(|r| r.address() <= address);
        at != 0 && address + self.granularity <= self.reservations[at - 1].end()
    }

    fn preflight_map(
        &self,
        address: u64,
        size: u64,
        private: bool,
    ) -> Result<MappingPreflight, NativeMemoryError> {
        self.live()?;
        let end = range(address, size)?;
        if !address.is_multiple_of(NATIVE_GUEST_PAGE) || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        if self
            .mapped_bytes()
            .checked_add(size)
            .is_none_or(|n| n > self.limits.mapped_bytes)
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        if private
            && self
                .backing_bytes()
                .checked_add(size)
                .is_none_or(|n| n > self.limits.mapped_bytes)
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        if private
            && self
                .private_bytes
                .checked_add(size)
                .is_none_or(|n| n > self.private_commit_limit)
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        let first_page = self.pages.partition_point(|p| p.address < address);
        if self.pages.get(first_page).is_some_and(|p| p.address < end) {
            return Err(NativeMemoryError::Overlap);
        }
        let first = address / self.granularity * self.granularity;
        if first == 0 {
            return Err(NativeMemoryError::InvalidRange);
        }
        let stop = round(end, self.granularity)?;
        let missing = (first..stop)
            .step_by(self.granularity as usize)
            .filter(|base| !self.contains_reservation(*base))
            .count();
        let needed = (missing as u64)
            .checked_mul(self.granularity)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let reserved = self
            .reserved
            .checked_add(needed)
            .filter(|n| *n <= self.limits.reserved_bytes)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let count = usize::try_from(size / NATIVE_GUEST_PAGE)
            .map_err(|_| NativeMemoryError::InvalidRange)?;
        Ok(MappingPreflight {
            end,
            first,
            stop,
            missing,
            reserved,
            count,
        })
    }

    /// Map new 16KiB pages at exact guest VAs. Recoverable failures roll back
    /// reservations/commits; failed rollback retains ownership and poisons it.
    pub fn map_at(
        &mut self,
        address: u64,
        size: u64,
        permissions: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.map_backing_at(address, size, permissions, None)
    }
    fn map_backing_at(
        &mut self,
        address: u64,
        size: u64,
        permissions: NativePermissions,
        shared: Option<u64>,
    ) -> Result<(), NativeMemoryError> {
        self.invalidate_stack_preflight();
        // Lifetime identity must be available before any host/backing effect.
        let next_lifetime = self
            .next_mapping_lifetime
            .checked_add(1)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let MappingPreflight {
            end,
            first,
            stop,
            missing,
            reserved,
            count,
        } = self.preflight_map(address, size, shared.is_none())?;
        // Reserve every descriptor before any OS mutation. Append/sort commits
        // below cannot allocate, invoke user code or fail recoverably.
        self.pages
            .try_reserve_exact(count)
            .map_err(|_| NativeMemoryError::Allocation)?;
        self.reservations
            .try_reserve_exact(missing)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let mut pending = Vec::new();
        pending
            .try_reserve_exact(missing)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let reservations = (|| {
            for base in (first..stop).step_by(self.granularity as usize) {
                if !self.contains_reservation(base) {
                    self.operation()?;
                    pending.push(Reservation::reserve(Some(base), self.granularity)?);
                }
            }
            Ok(())
        })();
        if let Err(error) = reservations {
            if error == NativeMemoryError::Poisoned {
                self.poisoned = true;
            }
            self.rollback_reservations(pending);
            return if self.poisoned {
                Err(NativeMemoryError::Poisoned)
            } else {
                Err(error)
            };
        }
        let mut committed = 0_u64;
        let result = (|| {
            for reservation in self.reservations.iter_mut().chain(pending.iter_mut()) {
                let start = address.max(reservation.address());
                let stop = end.min(reservation.end());
                if start < stop {
                    reservation.prepare_pages(start, stop)?;
                }
            }
            for base in (address..end).step_by(NATIVE_GUEST_PAGE as usize) {
                self.operation()?;
                let backing = if let Some(physical) = shared {
                    let at = physical + base - address;
                    let span = self.physical_allocation(at, NATIVE_GUEST_PAGE)?;
                    Some((span.section, span.offset + at - span.start, at))
                } else {
                    None
                };
                let region = mapping_region_mut(&mut self.reservations, &mut pending, base);
                if let Some((section, offset, physical)) = backing {
                    region.map_shared(
                        self.physical_sections[section].as_mut().unwrap(),
                        offset,
                        physical,
                        permissions,
                    )?;
                } else {
                    region.map_private(permissions)?;
                }
                committed += NATIVE_GUEST_PAGE;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if error == NativeMemoryError::Poisoned {
                self.poisoned = true;
            }
            for base in (address..address + committed).step_by(NATIVE_GUEST_PAGE as usize) {
                let decommitted = if self.rollback_operation() {
                    mapping_region_mut(&mut self.reservations, &mut pending, base)
                        .unmap()
                        .is_ok()
                } else {
                    false
                };
                if !decommitted {
                    self.poisoned = true;
                }
            }
            self.rollback_reservations(pending);
            return if self.poisoned {
                Err(NativeMemoryError::Poisoned)
            } else {
                Err(error)
            };
        }
        self.reservations.extend(pending);
        self.reservations.sort_unstable_by_key(Reservation::address);
        self.reserved = reserved;
        for base in (address..end).step_by(NATIVE_GUEST_PAGE as usize) {
            self.pages.push(Page {
                address: base,
                permissions,
                lifetime: self.next_mapping_lifetime,
            });
        }
        self.next_mapping_lifetime = next_lifetime;
        self.pages.sort_unstable_by_key(|p| p.address);
        if shared.is_none() {
            self.private_bytes += size;
        }
        Ok(())
    }

    pub fn protect(
        &mut self,
        address: u64,
        size: u64,
        permissions: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.invalidate_stack_preflight();
        self.live()?;
        let end = range(address, size)?;
        if !address.is_multiple_of(NATIVE_GUEST_PAGE) || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        self.check(address, size, None)?;
        let start = self
            .index(address)
            .map_err(|_| NativeMemoryError::Unmapped(address))?;
        let count = (size / NATIVE_GUEST_PAGE) as usize;
        let mut changed = 0_usize;
        let result = (|| {
            for index in start..start + count {
                self.operation()?;
                protect_page(self.pages[index].address, physical(permissions))?;
                changed += 1;
            }
            Ok(())
        })();
        if let Err(error) = result {
            for index in start..start + changed {
                if !self.rollback_operation()
                    || protect_page(
                        self.pages[index].address,
                        physical(self.pages[index].permissions),
                    )
                    .is_err()
                {
                    self.poisoned = true;
                }
            }
            return if self.poisoned {
                Err(NativeMemoryError::Poisoned)
            } else {
                Err(error)
            };
        }
        for page in &mut self.pages[start..start + count] {
            debug_assert!(page.address < end);
            page.permissions = permissions;
        }
        Ok(())
    }

    /// Restore owned mapped pages to placeholders while retaining their addresses.
    /// Private contents are discarded; shared section contents remain retained.
    /// Full-span validation precedes effects. Once any decommit occurred, an OS
    /// or later injected failure poisons the owner rather than exposing partial
    /// mappings as a recoverable operation. No physical view becomes executable.
    pub fn unmap(&mut self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.invalidate_stack_preflight();
        self.live()?;
        range(address, size)?;
        if !address.is_multiple_of(NATIVE_GUEST_PAGE) || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        self.check(address, size, None)?;
        let start = self
            .index(address)
            .map_err(|_| NativeMemoryError::Unmapped(address))?;
        let count = (size / NATIVE_GUEST_PAGE) as usize;
        let mut private_removed = 0;
        for relative in 0..count {
            if let Err(error) = self.operation() {
                if relative == 0 {
                    return Err(error);
                }
                self.poisoned = true;
                return Err(NativeMemoryError::Poisoned);
            }
            let page = self.pages[start + relative].address;
            if self.region(page).kind == RegionKind::Private {
                private_removed += NATIVE_GUEST_PAGE;
            }
            if self.region_mut(page).unmap().is_err() {
                self.poisoned = true;
                return Err(NativeMemoryError::Poisoned);
            }
        }
        self.pages.drain(start..start + count);
        self.private_bytes -= private_removed;
        Ok(())
    }

    /// Irreversibly revoke this owner's access after a caller cannot complete
    /// rollback. Keep reservation ownership for Drop; all live APIs refuse.
    pub fn abandon(&mut self) {
        self.invalidate_stack_preflight();
        self.poisoned = true;
    }
    fn check(
        &self,
        address: u64,
        size: u64,
        permission: Option<NativePermissions>,
    ) -> Result<(), NativeMemoryError> {
        self.check_permissions(address, size, permission, 0)
    }

    fn check_permissions(
        &self,
        address: u64,
        size: u64,
        permission: Option<NativePermissions>,
        forbidden: u8,
    ) -> Result<(), NativeMemoryError> {
        self.live()?;
        if size == 0 {
            return Ok(());
        }
        let end = range(address, size)?;
        let first = address / NATIVE_GUEST_PAGE * NATIVE_GUEST_PAGE;
        let index = self
            .index(first)
            .map_err(|_| NativeMemoryError::Unmapped(address))?;
        // Pages are sorted and unique. Locate the first once, then verify every
        // expected address in order; a hole never inherits the next page's rights.
        for (offset, base) in (first..end).step_by(NATIVE_GUEST_PAGE as usize).enumerate() {
            let at = base.max(address);
            let page = self
                .pages
                .get(index + offset)
                .filter(|page| page.address == base)
                .ok_or(NativeMemoryError::Unmapped(at))?;
            if permission.is_some_and(|required| required.0 & page.permissions.0 != required.0)
                || page.permissions.0 & forbidden != 0
            {
                return Err(NativeMemoryError::PermissionDenied(at));
            }
        }
        Ok(())
    }
    /// Exclusive paused access. There is no API to turn this owner into a
    /// running guest or expose pointer-backed borrows beyond this lifetime.
    ///
    /// ```compile_fail
    /// use pentax_platform::native_memory::{NativeAddressSpace,NativePermissions};
    /// let mut owner=NativeAddressSpace::new(Default::default()).unwrap();
    /// let mut paused=owner.pause().unwrap();
    /// owner.protect(0x10000,0x4000,NativePermissions::READ).unwrap();
    /// paused.write(0x10000,b"still borrowed").unwrap();
    /// ```
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, NativeMemoryError> {
        self.live()?;
        Ok(PausedNativeMemory { owner: self })
    }
}

pub struct PausedNativeMemory<'a> {
    owner: &'a mut NativeAddressSpace,
}
impl PausedNativeMemory<'_> {
    /// Identity of the exclusively borrowed owner, not an execution authority.
    pub const fn owner_id(&self) -> NativeOwnerId {
        self.owner.owner_id()
    }
    /// Validate the entire logical executable span against the live owner.
    /// No bytes are copied and the physical pages remain NX. As with
    /// `fetch_source`, an empty span is a no-op after the liveness check.
    pub fn preflight_fetch_source(&self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.owner.check(
            address,
            size,
            Some(NativePermissions::new(false, false, true)),
        )
    }
    /// Check committed same-owner coverage without requiring or granting any
    /// CPU/GPU permission. NOACCESS pages are mapped; reservation holes are not.
    pub fn preflight_mapped(&self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.owner.check(address, size, None)
    }
    /// Require every logical page in a live mapped span to be non-executable.
    /// This grants no data access; callers check required read/write rights too.
    /// Empty spans are no-ops only after checking owner liveness.
    pub fn preflight_non_executable(
        &self,
        address: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.owner.check_permissions(address, size, None, 4)
    }
    /// Require every live logical page to allow CPU read/write and no CPU execute.
    /// GPU rights do not satisfy CPU rights. One exact successful span is
    /// retained only while this owner's mapping/protection metadata is unchanged.
    /// No payload, operand, guard, RSP or TLS proof is retained. Empty spans and
    /// cache hits still require a live owner, as with the individual checks.
    pub fn preflight_read_write_non_executable(
        &self,
        address: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.owner.live()?;
        if size == 0 {
            return Ok(());
        }
        let span = NativeRange { address, size };
        if self.owner.stack_preflight.get() == Some(span) {
            return Ok(());
        }
        // No negative proof or a different/partially covered span is reused.
        self.owner.stack_preflight.set(None);
        #[cfg(test)]
        {
            self.owner
                .stack_preflight_scans
                .set(self.owner.stack_preflight_scans.get().saturating_add(1));
        }
        self.owner
            .check_permissions(address, size, Some(NativePermissions::READ_WRITE), 4)?;
        self.owner.stack_preflight.set(Some(span));
        Ok(())
    }
    /// Require nonempty aligned guard storage wholly covered by this owner's
    /// actual reservation union, with no logical mapped page in that span.
    /// Live reservation holes are OS NOACCESS under the mapping invariant;
    /// poisoned owners are refused before any range or metadata inspection.
    pub fn preflight_reserved_unmapped(
        &self,
        address: u64,
        size: u64,
    ) -> Result<(), NativeMemoryError> {
        self.owner.live()?;
        let end = range(address, size)?;
        if !address.is_multiple_of(NATIVE_GUEST_PAGE) || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::Unaligned);
        }
        let mut cursor = address;
        let mut at = self
            .owner
            .reservations
            .partition_point(|r| r.end() <= address);
        while cursor < end {
            let reservation = self
                .owner
                .reservations
                .get(at)
                .ok_or(NativeMemoryError::Unmapped(cursor))?;
            if reservation.address() > cursor {
                return Err(NativeMemoryError::Unmapped(cursor));
            }
            cursor = reservation.end().min(end);
            at += 1;
        }
        let first = self
            .owner
            .pages
            .partition_point(|page| page.address < address);
        if self
            .owner
            .pages
            .get(first)
            .is_some_and(|page| page.address < end)
        {
            return Err(NativeMemoryError::Overlap);
        }
        Ok(())
    }
    pub fn preflight_read(&self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.owner
            .check(address, size, Some(NativePermissions::READ))
    }
    pub fn preflight_write(&self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.owner.check(
            address,
            size,
            Some(NativePermissions::new(false, true, false)),
        )
    }
    pub fn read(&self, address: u64, output: &mut [u8]) -> Result<(), NativeMemoryError> {
        self.copy_out(address, output, NativePermissions::READ)
    }
    pub fn preflight_gpu_read(&self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.owner.check(
            address,
            size,
            Some(NativePermissions::new(false, false, false).with_gpu(true, false)),
        )
    }
    pub fn preflight_gpu_write(&self, address: u64, size: u64) -> Result<(), NativeMemoryError> {
        self.owner.check(
            address,
            size,
            Some(NativePermissions::new(false, false, false).with_gpu(false, true)),
        )
    }
    pub fn read_gpu(&self, address: u64, output: &mut [u8]) -> Result<(), NativeMemoryError> {
        self.copy_out(
            address,
            output,
            NativePermissions::new(false, false, false).with_gpu(true, false),
        )
    }
    /// Stage a shader buffer using its compiler-derived access requirements.
    /// Write-only staging preserves untouched bytes for complete fenced writeback;
    /// it does not admit shader reads or ordinary guest CPU access to those bytes.
    pub fn snapshot_gpu_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), NativeMemoryError> {
        self.owner.live()?;
        if !read && !written {
            return Err(NativeMemoryError::InvalidRange);
        }
        self.copy_out(
            address,
            output,
            NativePermissions::new(false, false, false).with_gpu(read, written),
        )
    }
    pub fn fetch_source(&self, address: u64, output: &mut [u8]) -> Result<(), NativeMemoryError> {
        self.copy_out(address, output, NativePermissions::new(false, false, true))
    }
    fn copy_out(
        &self,
        address: u64,
        output: &mut [u8],
        permission: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.owner
            .check(address, output.len() as u64, Some(permission))?;
        if output.is_empty() {
            return Ok(());
        }
        // SAFETY: Whole-span preflight proves committed owned readable backing;
        // physical backing is readable and NX. All allocations stay alive under
        // this exclusive owner borrow. The checked pause exposes no native-page
        // reference; unsafe native execution must not overlap this copy or
        // address Rust output storage. Safe output storage cannot inhabit the
        // privately owned guest pages.
        unsafe {
            std::ptr::copy_nonoverlapping(
                address as usize as *const u8,
                output.as_mut_ptr(),
                output.len(),
            )
        };
        Ok(())
    }
    /// Preflight the whole writable committed span before one non-fallible
    /// copy. No allocations/protection changes occur after preflight.
    pub fn write(&mut self, address: u64, input: &[u8]) -> Result<(), NativeMemoryError> {
        self.copy_in(address, input, NativePermissions::new(false, true, false))
    }
    fn copy_in(
        &mut self,
        address: u64,
        input: &[u8],
        permission: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        self.owner
            .check(address, input.len() as u64, Some(permission))?;
        if input.is_empty() {
            return Ok(());
        }
        // SAFETY: Whole-span preflight proves owned PAGE_READWRITE backing;
        // this exclusive paused borrow prevents changes. Safe input slices
        // cannot alias these private, non-exposed allocations.
        unsafe {
            std::ptr::copy_nonoverlapping(input.as_ptr(), address as usize as *mut u8, input.len())
        };
        Ok(())
    }
}

fn mapping_region_mut<'a>(
    owned: &'a mut [Reservation],
    pending: &'a mut [Reservation],
    address: u64,
) -> &'a mut Region {
    let at = owned.partition_point(|r| r.address() <= address);
    if at != 0 && address < owned[at - 1].end() {
        return owned[at - 1].region_mut(address);
    }
    let at = pending.partition_point(|r| r.address() <= address);
    pending[at - 1].region_mut(address)
}

fn physical(p: NativePermissions) -> u32 {
    if p.writable() || p.gpu_writable() {
        PAGE_READWRITE
    } else if p.readable() || p.executable() || p.gpu_readable() {
        PAGE_READONLY
    } else {
        PAGE_NOACCESS
    }
}
fn protect_page(address: u64, permissions: u32) -> Result<(), NativeMemoryError> {
    let mut old = 0;
    // SAFETY: Caller preflights one owned committed guest page. This helper is
    // private and receives only non-executable physical protection constants.
    if unsafe {
        VirtualProtect(
            address as usize as *const c_void,
            NATIVE_GUEST_PAGE as usize,
            permissions,
            &mut old,
        )
    } == 0
    {
        Err(os_error("VirtualProtect(NX)", address))
    } else {
        Ok(())
    }
}
fn range(address: u64, size: u64) -> Result<u64, NativeMemoryError> {
    address
        .checked_add(size)
        .filter(|end| size != 0 && *end <= NATIVE_ADDRESS_LIMIT)
        .ok_or(NativeMemoryError::InvalidRange)
}
fn round(value: u64, unit: u64) -> Result<u64, NativeMemoryError> {
    value
        .checked_add(unit - 1)
        .map(|n| n / unit * unit)
        .ok_or(NativeMemoryError::InvalidRange)
}
fn os_error(operation: &'static str, address: u64) -> NativeMemoryError {
    // SAFETY: GetLastError has no memory preconditions and is captured before
    // another OS call can overwrite the failed operation's error code.
    let code = unsafe { GetLastError() };
    NativeMemoryError::OperatingSystem {
        operation,
        address,
        code,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod ownership_tests;

#[cfg(test)]
mod deferred_tests;

#[cfg(test)]
mod stack_preflight_tests;

#[cfg(test)]
mod constrained_tests;

mod provider_preflight;
mod replacement;
mod transaction;

#[cfg(test)]
std::thread_local! {static LAST_RELEASE:std::cell::Cell<Option<(u64,bool)>>=const{std::cell::Cell::new(None)};}
