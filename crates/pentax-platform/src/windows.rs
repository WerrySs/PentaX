// SPDX-License-Identifier: GPL-2.0-or-later

use crate::{GUEST_PAGE_SIZE, HostLayout, PlatformError};
use std::ffi::c_void;
use std::ptr::NonNull;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Diagnostics::Debug::{
    FlushInstructionCache, GetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
    SetErrorMode,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_NOACCESS, PAGE_READWRITE,
    VirtualAlloc, VirtualFree, VirtualProtect,
};
use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[path = "windows/traps.rs"]
mod traps;

#[path = "windows/dispatch.rs"]
pub(crate) mod dispatch;
#[path = "windows/entry.rs"]
mod entry;
#[path = "windows/stack.rs"]
mod stack;

pub(super) fn execute_trapped_sum42() -> Result<crate::OriginalFixtureResult, PlatformError> {
    traps::execute()
}

pub(super) fn execute_guest_stack_entry42() -> Result<crate::OriginalFixtureResult, PlatformError> {
    entry::execute()
}

pub(super) fn execute_native_block_dispatch() -> Result<crate::OriginalFixtureResult, PlatformError>
{
    dispatch::execute_native_block_dispatch()
}

#[cfg(test)]
std::thread_local! {
    // Observe this thread's actual release result without querying an address
    // after ownership ends, when another allocator could already have reused it.
    static LAST_RELEASE: std::cell::Cell<Option<(u64, bool)>> = const {
        std::cell::Cell::new(None)
    };
}

pub(super) fn host_layout() -> Result<HostLayout, PlatformError> {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: info is a valid writable SYSTEM_INFO, and GetSystemInfo retains no pointer.
    unsafe { GetSystemInfo(&mut info) };
    let layout = HostLayout {
        page_size: info.dwPageSize as usize,
        allocation_granularity: info.dwAllocationGranularity as usize,
    };
    if !layout.page_size.is_power_of_two()
        || !layout.allocation_granularity.is_power_of_two()
        || !GUEST_PAGE_SIZE.is_multiple_of(layout.page_size)
        || !layout
            .allocation_granularity
            .is_multiple_of(layout.page_size)
    {
        return Err(PlatformError::UnsupportedPageLayout);
    }
    Ok(layout)
}

pub(super) fn configure_worker_fault_policy() -> Result<(), PlatformError> {
    // SAFETY: These APIs only inspect/set error-dialog flags in this worker process.
    // They install no handlers and do not alter the system or another process.
    unsafe { SetErrorMode(GetErrorMode() | SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) };
    Ok(())
}

pub(super) struct Region {
    base: NonNull<c_void>,
    length: usize,
}

impl Region {
    pub(super) fn reserve(length: usize) -> Result<Self, PlatformError> {
        let layout = host_layout()?;
        // SAFETY: NULL asks Windows to choose an unused address. length has been
        // bounded and guest-page validated by GuestReservation::reserve, and the
        // host geometry above proves commitment/protection alignment is possible.
        let raw = unsafe { VirtualAlloc(std::ptr::null(), length, MEM_RESERVE, PAGE_NOACCESS) };
        let base = NonNull::new(raw).ok_or_else(|| last_error("VirtualAlloc(MEM_RESERVE)"))?;
        let region = Self { base, length };
        if !(base.as_ptr() as usize).is_multiple_of(layout.allocation_granularity) {
            return Err(PlatformError::UnsupportedPageLayout);
        }
        Ok(region)
    }

    pub(super) fn len(&self) -> usize {
        self.length
    }
    pub(super) fn address(&self) -> u64 {
        self.base.as_ptr() as usize as u64
    }

    pub(super) fn commit_writable(&mut self) -> Result<(), PlatformError> {
        self.commit_writable_range(0, self.length)
    }

    fn commit_writable_range(&mut self, offset: usize, length: usize) -> Result<(), PlatformError> {
        crate::checked_range(self.length, offset, length)?;
        let layout = host_layout()?;
        if length == 0
            || !offset.is_multiple_of(layout.page_size)
            || !length.is_multiple_of(layout.page_size)
        {
            return Err(PlatformError::OutOfBounds);
        }
        // SAFETY: This object uniquely owns the entire reserved interval. Its
        // base/length satisfy the verified Windows page geometry; only those
        // already-reserved pages are committed, with no executable permission.
        let target = unsafe { self.base.as_ptr().cast::<u8>().add(offset) }.cast();
        // SAFETY: target/length are host-page-aligned and completely inside this
        // uniquely owned reservation. Only those pages become committed RW/NX.
        let committed = unsafe { VirtualAlloc(target, length, MEM_COMMIT, PAGE_READWRITE) };
        if committed.is_null() {
            return Err(last_error("VirtualAlloc(MEM_COMMIT)"));
        }
        if committed != target {
            return Err(PlatformError::OutOfBounds);
        }
        Ok(())
    }

    pub(super) fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), PlatformError> {
        if bytes.is_empty() {
            return Ok(());
        }
        // SAFETY: Only WritableGuestMemory exposes this method after commitment.
        // Its write preflight proves [offset, offset+len) lies inside this unique
        // writable allocation. No safe API exposes an alias into native memory.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.base.as_ptr().cast::<u8>().add(offset),
                bytes.len(),
            )
        };
        Ok(())
    }

    pub(super) fn seal_executable(&mut self) -> Result<(), PlatformError> {
        let mut previous = 0;
        // SAFETY: All pages are committed in one uniquely owned reservation.
        // previous is writable. This transition revokes write access before
        // granting execute access; the mapping is never simultaneously writable.
        if unsafe {
            VirtualProtect(
                self.base.as_ptr(),
                self.length,
                PAGE_EXECUTE_READ,
                &mut previous,
            )
        } == 0
        {
            return Err(last_error("VirtualProtect(PAGE_EXECUTE_READ)"));
        }
        // SAFETY: The pseudo-process handle names this process, and the entire
        // owned interval remains allocated and readable during cache publication.
        if unsafe { FlushInstructionCache(GetCurrentProcess(), self.base.as_ptr(), self.length) }
            == 0
        {
            return Err(last_error("FlushInstructionCache"));
        }
        Ok(())
    }

    /// Caller must have sealed a complete closed fixture at offset 0. For the
    /// prepared sum fixture, its thread-local handler scope must remain armed
    /// throughout the call; the original SYSCALL bytes must never be published.
    pub(super) unsafe fn call_original_fixture(&self) -> u64 {
        // SAFETY: The caller proves code identity and complete sysv64 leaf ABI.
        // The sealed allocation remains alive throughout the call. This conversion
        // is intentionally private; arbitrary user bytes have no execution API.
        let entry: unsafe extern "sysv64" fn(u64, u64) -> u64 =
            unsafe { std::mem::transmute(self.base.as_ptr()) };
        // SAFETY: Same code identity, ABI, mapping lifetime, and RX preconditions.
        unsafe { entry(19, 23) }
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: base is the original reservation address and this object is its
        // sole owner. MEM_RELEASE requires size 0 and releases the whole region.
        // No guest thread or borrowed view survives the owned typestate object.
        let released = unsafe { VirtualFree(self.base.as_ptr(), 0, MEM_RELEASE) };
        #[cfg(test)]
        // Destruction can occur during thread teardown, after test TLS has gone.
        // Failure to observe that late release must never make Drop panic.
        let _ = LAST_RELEASE.try_with(|record| record.set(Some((self.address(), released != 0))));
        let _ = released;
    }
}

fn last_error(operation: &'static str) -> PlatformError {
    // SAFETY: GetLastError reads the calling thread's error slot and retains nothing.
    let code = unsafe { GetLastError() };
    PlatformError::OperatingSystem { operation, code }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GuestReservation, OriginalFixture, execute_original_fixture};
    use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, VirtualQuery};

    pub(super) fn query(address: u64) -> MEMORY_BASIC_INFORMATION {
        let mut information = MEMORY_BASIC_INFORMATION::default();
        // SAFETY: VirtualQuery inspects a numeric address without dereferencing it;
        // information is a valid correctly sized writable output buffer.
        let written = unsafe {
            VirtualQuery(
                address as usize as *const c_void,
                &mut information,
                std::mem::size_of_val(&information),
            )
        };
        assert_eq!(written, std::mem::size_of_val(&information));
        information
    }

    #[test]
    fn owned_pages_stage_rw_seal_rx_release_and_execute_only_the_original_leaf() {
        let layout = host_layout().unwrap();
        let reservation = GuestReservation::reserve(GUEST_PAGE_SIZE * 2).unwrap();
        let address = reservation.address();
        assert!((address as usize).is_multiple_of(layout.allocation_granularity));
        assert_eq!(query(address).State, MEM_RESERVE);
        let mut staging = reservation.commit_writable().unwrap();
        for offset in (0..staging.len()).step_by(layout.page_size) {
            assert_eq!(query(address + offset as u64).Protect, PAGE_READWRITE);
        }
        staging
            .write(0, crate::fixture_bytes(OriginalFixture::Return42))
            .unwrap();
        assert_eq!(
            staging.write(usize::MAX, &[1]),
            Err(PlatformError::OutOfBounds)
        );
        assert_eq!(
            staging.write(staging.len() - 1, &[1, 2]),
            Err(PlatformError::OutOfBounds)
        );
        let executable = staging.seal_executable().unwrap();
        for offset in (0..executable.len()).step_by(layout.page_size) {
            assert_eq!(query(address + offset as u64).Protect, PAGE_EXECUTE_READ);
        }
        // SAFETY: The exact original System V argument-summing leaf was written above and no
        // unchecked writer is exposed after seal_executable revoked write access.
        assert_eq!(unsafe { executable.region.call_original_fixture() }, 42);
        LAST_RELEASE.with(|record| record.set(None));
        drop(executable);
        assert_eq!(
            LAST_RELEASE.with(std::cell::Cell::take),
            Some((address, true))
        );
        assert_eq!(execute_original_fixture(OriginalFixture::Return42), Ok(42));
    }
}
