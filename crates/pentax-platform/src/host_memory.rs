// SPDX-License-Identifier: GPL-2.0-only

//! A volatile host snapshot, not a reservation or a promise of allocation success.

use crate::PlatformError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostMemoryAvailability {
    pub physical_bytes: u64,
    /// Additional commitment available to this process, not pagefile free space.
    pub commit_bytes: u64,
}

/// Read current availability without changing host memory or process settings.
/// See Microsoft's GlobalMemoryStatusEx / MEMORYSTATUSEX contracts.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub fn available_memory() -> Result<HostMemoryAvailability, PlatformError> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
    };
    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: status is correctly sized writable storage. The call retains no
    // pointer and only reads host counters. Capture GetLastError on failure.
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return Err(PlatformError::OperatingSystem {
            operation: "GlobalMemoryStatusEx",
            // SAFETY: Read this thread's last error immediately after the call.
            code: unsafe { GetLastError() },
        });
    }
    Ok(HostMemoryAvailability {
        physical_bytes: status.ullAvailPhys,
        commit_bytes: status.ullAvailPageFile,
    })
}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub fn available_memory() -> Result<HostMemoryAvailability, PlatformError> {
    Err(PlatformError::UnsupportedHost)
}
