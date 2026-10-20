// SPDX-License-Identifier: GPL-2.0-only

//! Explicit directory capability for bounded, exclusive, delete-on-close files.
//! File-backed shared data pages consume disk backing rather than pagefile
//! commitment; neither their logical length nor mapped bytes measure live RAM.
use super::*;
use cap_std::fs::{Dir, OpenOptions as CapOpenOptions, OpenOptionsExt as CapOpenOptionsExt};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
use std::path::{Component, Path, Prefix};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, HANDLE},
    Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_DELETE_ON_CLOSE,
        GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationByHandleW, GetVolumeInformationW,
        WRITE_DAC,
    },
    System::{
        IO::DeviceIoControl,
        Ioctl::{FILE_ZERO_DATA_INFORMATION, FSCTL_SET_SPARSE, FSCTL_SET_ZERO_DATA},
        SystemServices::FILE_SUPPORTS_SPARSE_FILES,
        WindowsProgramming::DRIVE_FIXED,
    },
};

mod security;

const DISK_HEADROOM: u64 = 2 * 1024 * 1024 * 1024;
const MAX_CAPACITY: u64 = 16 * 1024 * 1024 * 1024;
static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

/// Owned directory authority, not a guest mount or a resumable data file.
/// Only fresh fixed-format names are created; existing files are never opened,
/// truncated or deleted. Unsupported remote/non-sparse volumes refuse.
pub struct NativeFileBacking {
    directory: Dir,
    volume_root: [u16; 4],
    capacity: u64,
    #[cfg(test)]
    fail_stage: Option<usize>,
}
impl fmt::Debug for NativeFileBacking {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeFileBacking")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}
impl NativeFileBacking {
    /// Capture an existing caller-selected local directory. Before any guest
    /// execution, require sparse-file support, matching pinned/root volumes and
    /// enough quota-visible disk space for full capacity plus 2 GiB headroom.
    pub fn open(path: &Path, capacity: u64) -> Result<Self, NativeMemoryError> {
        if capacity == 0 || capacity > MAX_CAPACITY || !capacity.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::InvalidRange);
        }
        let canonical = path
            .canonicalize()
            .map_err(|e| file_error("backing directory path", e))?;
        let Some(Component::Prefix(prefix)) = canonical.components().next() else {
            return Err(NativeMemoryError::InvalidRange);
        };
        let drive = match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => letter,
            _ => return Err(NativeMemoryError::InvalidRange),
        };
        let volume_root = [u16::from(drive), b':' as u16, b'\\' as u16, 0];
        // SAFETY: Fixed NUL-terminated local drive root; this only queries type.
        if unsafe { GetDriveTypeW(volume_root.as_ptr()) } != DRIVE_FIXED {
            return Err(NativeMemoryError::InvalidRange);
        }
        // Keep this directory open without delete sharing, then use capability-
        // relative create_new exclusively. No guest name or ambient fallback.
        let pinned = OpenOptions::new()
            .read(true)
            .share_mode(3)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(canonical)
            .map_err(|e| file_error("backing directory open", e))?;
        if !pinned
            .metadata()
            .map_err(|e| file_error("backing directory metadata", e))?
            .is_dir()
        {
            return Err(NativeMemoryError::InvalidRange);
        }
        let mut pinned_serial = 0;
        let mut root_serial = 0;
        let mut flags = 0;
        // SAFETY: Live owned directory handle and exact scalar output storage;
        // optional names/component outputs are omitted, no pointer is retained.
        if unsafe {
            GetVolumeInformationByHandleW(
                pinned.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                &mut pinned_serial,
                std::ptr::null_mut(),
                &mut flags,
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(os_error("backing volume query", 0));
        }
        // SAFETY: Fixed NUL-terminated drive root and writable scalar output.
        if unsafe {
            GetVolumeInformationW(
                volume_root.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut root_serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        } == 0
        {
            return Err(os_error("backing root volume query", 0));
        }
        if pinned_serial != root_serial || flags & FILE_SUPPORTS_SPARSE_FILES == 0 {
            return Err(NativeMemoryError::UnsupportedPageLayout);
        }
        let result = Self {
            directory: Dir::from_std_file(pinned),
            volume_root,
            capacity,
            #[cfg(test)]
            fail_stage: None,
        };
        result.check_space()?;
        Ok(result)
    }
    pub const fn capacity_bytes(&self) -> u64 {
        self.capacity
    }
    #[cfg(test)]
    pub(super) fn check_stage(&self, stage: usize) -> Result<(), NativeMemoryError> {
        if self.fail_stage == Some(stage) {
            return Err(NativeMemoryError::OperatingSystem {
                operation: "backing injected preparation",
                address: 0,
                code: 8,
            });
        }
        Ok(())
    }

    fn check_space(&self) -> Result<(), NativeMemoryError> {
        let mut available = 0;
        // SAFETY: Fixed admitted local drive root and exact writable output.
        // This volatile snapshot changes no files/quotas/host settings.
        if unsafe {
            GetDiskFreeSpaceExW(
                self.volume_root.as_ptr(),
                &mut available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(os_error("backing disk space query", 0));
        }
        if available < self.capacity + DISK_HEADROOM {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        Ok(())
    }
    pub(super) fn create(&self, size: u64) -> Result<File, NativeMemoryError> {
        if size == 0 || size > self.capacity || !size.is_multiple_of(NATIVE_GUEST_PAGE) {
            return Err(NativeMemoryError::InvalidRange);
        }
        self.check_space()?;
        let mut options = CapOpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(true)
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE | WRITE_DAC)
            .share_mode(0)
            .custom_flags(FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE);
        // A stale/colliding name never grants access to that file. Attempts and
        // process-local IDs are finite; fresh handles are noninheritable.
        for _ in 0..16 {
            let id = NEXT_FILE
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
                .map_err(|_| NativeMemoryError::BudgetExceeded)?;
            let name = format!("pentax-{}-{id:016x}.tmp", std::process::id());
            let file = match self.directory.open_with(name, &options) {
                Ok(file) => file.into_std(),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(file_error("backing file create", error)),
            };
            #[cfg(test)]
            self.check_stage(0)?;
            security::protect(file.as_raw_handle())?;
            #[cfg(test)]
            self.check_stage(1)?;
            control(file.as_raw_handle(), FSCTL_SET_SPARSE, None)?;
            #[cfg(test)]
            self.check_stage(2)?;
            file.set_len(size)
                .map_err(|e| file_error("backing file length", e))?;
            #[cfg(test)]
            self.check_stage(3)?;
            // Explicitly zero the entire new sparse extent. CreateFileMapping
            // alone does not promise zero contents when extending data files.
            control(
                file.as_raw_handle(),
                FSCTL_SET_ZERO_DATA,
                Some(FILE_ZERO_DATA_INFORMATION {
                    FileOffset: 0,
                    BeyondFinalZero: size as i64,
                }),
            )?;
            #[cfg(test)]
            self.check_stage(4)?;
            return Ok(file);
        }
        Err(NativeMemoryError::Allocation)
    }
}

#[cfg(test)]
mod tests;

fn control(
    handle: HANDLE,
    code: u32,
    data: Option<FILE_ZERO_DATA_INFORMATION>,
) -> Result<(), NativeMemoryError> {
    let (pointer, size) = data.as_ref().map_or((std::ptr::null(), 0), |v| {
        (
            std::ptr::from_ref(v).cast::<c_void>(),
            std::mem::size_of_val(v) as u32,
        )
    });
    let mut returned = 0;
    // SAFETY: Live exclusively opened synchronous data-file handle. Inputs
    // are either absent or the exact documented scalar zero-range structure;
    // no output buffer, overlapped operation or retained pointer is requested.
    if unsafe {
        DeviceIoControl(
            handle,
            code,
            pointer,
            size,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(os_error("backing sparse initialization", 0));
    }
    Ok(())
}
fn file_error(operation: &'static str, error: io::Error) -> NativeMemoryError {
    NativeMemoryError::OperatingSystem {
        operation,
        address: 0,
        code: error
            .raw_os_error()
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
    }
}
