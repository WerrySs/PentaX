// SPDX-License-Identifier: GPL-2.0-only
//! Original directory-save consumers through the actual kernel owner and file state.
use super::*;
use crate::{GuestError, Kernel, ProcessIdentity};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const BASE: u64 = 0x10000;
const NAME: u64 = BASE + 0x100;
const TITLE: u64 = BASE + 0x200;
const OUT: u64 = BASE + 0x400;
const POINT: u64 = BASE + 0x800;
static SERIAL: AtomicU64 = AtomicU64::new(1);

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pentax-directory-save-{}-{}",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn storage(&self) -> SaveStorage {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x0200_0000);
        }
        SaveStorage::directory_for_title(options.open(&self.0).unwrap(), "PPSA12345").unwrap()
    }
    fn kernel(&self) -> Kernel {
        let mut kernel = Kernel::new(
            Default::default(),
            ProcessIdentity {
                process_id: 7,
                initial_thread_id: 7,
            },
            vec![],
        )
        .unwrap();
        kernel.configure_save_storage(self.storage()).unwrap();
        kernel
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let parent = std::env::temp_dir();
        assert_eq!(self.0.parent(), Some(parent.as_path()));
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pentax-directory-save-")
        );
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

struct Guest {
    bytes: Vec<u8>,
    writes: usize,
    fail_write: Option<usize>,
}
impl Guest {
    fn new() -> Self {
        Self {
            bytes: vec![0; 0x4000],
            writes: 0,
            fail_write: None,
        }
    }
    fn put(&mut self, address: u64, bytes: &[u8]) {
        let offset = (address - BASE) as usize;
        self.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
    }
    fn get(&self, address: u64, size: usize) -> &[u8] {
        &self.bytes[(address - BASE) as usize..(address - BASE) as usize + size]
    }
}
impl GuestAccess for Guest {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        let offset = address.checked_sub(BASE).ok_or(GuestError::ReadDenied)? as usize;
        let end = offset
            .checked_add(output.len())
            .ok_or(GuestError::AddressOverflow)?;
        output.copy_from_slice(self.bytes.get(offset..end).ok_or(GuestError::ReadDenied)?);
        Ok(())
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        if !matches!(effect, MemoryEffect::None) {
            return Err(GuestError::UnsupportedMapping);
        }
        self.writes += 1;
        if self.fail_write == Some(self.writes) {
            return Err(GuestError::WriteDenied);
        }
        for write in writes {
            let offset = write
                .address
                .checked_sub(BASE)
                .ok_or(GuestError::WriteDenied)? as usize;
            let end = offset
                .checked_add(write.bytes.len())
                .ok_or(GuestError::AddressOverflow)?;
            if self.bytes.get(offset..end).is_none() {
                return Err(GuestError::WriteDenied);
            }
        }
        for write in writes {
            self.put(write.address, write.bytes);
        }
        Ok(())
    }
}
fn call(
    kernel: &mut Kernel,
    guest: &mut Guest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    kernel.invoke_library(service, CallArgs(args), guest)
}
fn name(guest: &mut Guest, value: &str) {
    let mut bytes = [0; 32];
    bytes[..value.len()].copy_from_slice(value.as_bytes());
    guest.put(NAME, &bytes);
}
fn mount3(
    kernel: &mut Kernel,
    guest: &mut Guest,
    value: &str,
    mode: u32,
    blocks: u64,
) -> Result<u64, Failure> {
    guest.put(BASE, &[0; 80]);
    guest.put(BASE, &7i32.to_le_bytes());
    guest.put(BASE + 8, &NAME.to_le_bytes());
    guest.put(BASE + 16, &blocks.to_le_bytes());
    guest.put(BASE + 32, &mode.to_le_bytes());
    name(guest, value);
    call(kernel, guest, Service::SaveMount3, &[BASE, OUT])
}
fn transfer(kernel: &mut Kernel, guest: &mut Guest, value: &str) -> Result<u64, Failure> {
    guest.put(BASE, &[0; 32]);
    guest.put(BASE, &7i32.to_le_bytes());
    guest.put(BASE + 8, &TITLE.to_le_bytes());
    guest.put(BASE + 16, &NAME.to_le_bytes());
    guest.put(TITLE, b"PPSA12345\0");
    name(guest, value);
    call(kernel, guest, Service::SaveTransferringMount, &[BASE, OUT])
}
fn remember_point(guest: &mut Guest) -> [u8; 16] {
    let point: [u8; 16] = guest.get(OUT, 16).try_into().unwrap();
    guest.put(POINT, &point);
    point
}
fn unmount(kernel: &mut Kernel, guest: &mut Guest, mode: u32) -> Result<u64, Failure> {
    call(
        kernel,
        guest,
        Service::SaveUmount2,
        &[u64::from(mode), POINT],
    )
}
fn key(value: &str) -> SaveKey {
    let mut name = [0; 32];
    name[..value.len()].copy_from_slice(value.as_bytes());
    SaveKey::new(7, *b"PPSA12345\0", name)
}

#[test]
fn missing_transfer_and_real_created_existing_mounts_preserve_outputs_info_events_and_lifecycle() {
    let root = Root::new();
    let mut kernel = root.kernel();
    let mut guest = Guest::new();
    guest.put(OUT - 8, &[0xa5; 80]);
    assert_eq!(transfer(&mut kernel, &mut guest, "primary"), Ok(NOT_FOUND));
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    assert_eq!(guest.get(OUT - 8, 8), [0xa5; 8]);
    assert_eq!(guest.get(OUT + 64, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveInitialize, &[0]),
        Ok(0)
    );
    assert_eq!(mount3(&mut kernel, &mut guest, "primary", 4, 48), Ok(0));
    assert_eq!(guest.get(OUT + 28, 4), 1u32.to_le_bytes());
    assert_eq!(remember_point(&mut guest), mount_point(0));
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveTerminate, &[]),
        Ok(BUSY)
    );
    assert!(kernel.save_data_initialized());
    assert_eq!(transfer(&mut kernel, &mut guest, "primary"), Ok(BUSY));
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::SaveGetMountInfo,
            &[POINT, BASE + 0x500]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.get(BASE + 0x500, 16),
        [48u64.to_le_bytes(), 48u64.to_le_bytes()].concat()
    );
    assert_eq!(guest.get(BASE + 0x510, 32), [0; 32]);
    assert_eq!(unmount(&mut kernel, &mut guest, 1 << 16), Ok(0));
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::SaveGetEvent,
            &[0, BASE + 0x600]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(BASE + 0x600, 4), 1u32.to_le_bytes());
    assert_eq!(guest.get(BASE + 0x604, 100), [0; 100]);
    assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(NOT_FOUND));
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveTerminate, &[]),
        Ok(0)
    );
    drop(kernel);
    // A genuinely fresh owner reopens the persisted directory rather than an
    // in-memory fixture record. The root lock is also released with the owner.
    let mut next = root.kernel();
    assert_eq!(transfer(&mut next, &mut guest, "primary"), Ok(0));
    assert_eq!(guest.get(OUT + 28, 4), 0u32.to_le_bytes());
    assert_eq!(remember_point(&mut guest), mount_point(0));
    assert_eq!(
        call(
            &mut next,
            &mut guest,
            Service::SaveGetMountInfo,
            &[POINT, BASE + 0x500]
        ),
        Ok(0)
    );
    {
        let point: [u8; 16] = guest.get(POINT, 16).try_into().unwrap();
        let directory = &next.save.directories.mounted(&point).unwrap().directory;
        let limits = crate::Limits {
            max_save_io_bytes: 7,
            ..Default::default()
        };
        let mut limited = Budget::new(limits).unwrap();
        assert!(matches!(
            next.save
                .memory
                .storage()
                .unwrap()
                .read_blocks(directory, &mut limited),
            Err(Error::Policy(Failure::Limit(_)))
        ));
    }
    {
        // An original persisted save becomes unreadable as a blocks backing
        // store. This must report BROKEN, not the generic backend permission
        // code, and must leave the caller's output unchanged.
        let point: [u8; 16] = guest.get(POINT, 16).try_into().unwrap();
        let directory = &next.save.directories.mounted(&point).unwrap().directory;
        directory.remove_file("sce_sys/blocks.bin").unwrap();
        directory.remove_file("sce_sys/param.bin").unwrap();
        directory.remove_dir("sce_sys").unwrap();
        directory
            .write("sce_sys", b"original-nondirectory")
            .unwrap();
    }
    guest.put(BASE + 0x500, &[0xa5; 48]);
    assert_eq!(
        call(
            &mut next,
            &mut guest,
            Service::SaveGetMountInfo,
            &[POINT, BASE + 0x500]
        ),
        Ok(BROKEN)
    );
    assert_eq!(guest.get(BASE + 0x500, 48), [0xa5; 48]);
    assert_eq!(unmount(&mut next, &mut guest, 0), Ok(0));
}

#[test]
fn mount_validation_zero_order_create_modes_and_guest_output_failure_do_not_leak_mounts_or_new_saves()
 {
    let root = Root::new();
    let mut kernel = root.kernel();
    let mut guest = Guest::new();
    guest.put(OUT, &[0xa5; 64]);
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::SaveTransferringMount,
            &[0, OUT]
        ),
        Ok(PARAMETER)
    );
    assert_eq!(guest.get(OUT, 64), [0xa5; 64]);
    assert_eq!(
        mount3(&mut kernel, &mut guest, "bad/name", 4, 48),
        Ok(PARAMETER)
    );
    assert_eq!(guest.get(OUT, 64), [0xa5; 64]);
    assert_eq!(
        mount3(&mut kernel, &mut guest, "minimum", 4, 47),
        Ok(PARAMETER)
    );
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    assert!(matches!(
        mount3(&mut kernel, &mut guest, "mode", 0, 48),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    guest.fail_write = Some(guest.writes + 2); // visible zero succeeds; final publication is refused.
    assert!(matches!(
        mount3(&mut kernel, &mut guest, "rollback", 4, 48),
        Err(Failure::Guest(_))
    ));
    guest.fail_write = None;
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    assert!(!kernel.save.directories.live());
    assert!(
        kernel
            .save
            .memory
            .storage()
            .unwrap()
            .open_save(&key("rollback"))
            .unwrap()
            .is_none()
    );
    assert_eq!(mount3(&mut kernel, &mut guest, "minimum", 4, 48), Ok(0));
    remember_point(&mut guest);
    assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(0));
    assert_eq!(
        mount3(&mut kernel, &mut guest, "minimum", 4, 48),
        Ok(EXISTS)
    );
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    assert_eq!(mount3(&mut kernel, &mut guest, "minimum", 32, 0), Ok(0));
    assert_eq!(guest.get(OUT + 28, 4), 0u32.to_le_bytes());
    remember_point(&mut guest);
    assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(0));
}

#[test]
fn all_sixteen_slots_busy_precedence_and_lowest_slot_reuse_follow_the_verified_source_header() {
    let root = Root::new();
    let mut kernel = root.kernel();
    let mut guest = Guest::new();
    for slot in 0..COUNT {
        assert_eq!(
            mount3(&mut kernel, &mut guest, &format!("slot-{slot}"), 4, 48),
            Ok(0)
        );
        assert_eq!(guest.get(OUT, 16), mount_point(slot));
    }
    assert_eq!(transfer(&mut kernel, &mut guest, "slot-0"), Ok(BUSY));
    assert_eq!(transfer(&mut kernel, &mut guest, "missing"), Ok(MOUNT_FULL));
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    #[cfg(unix)]
    {
        // A clone of an actual occupied capability exercises physical BUSY
        // without requiring host bind-mount privileges or fabricated entries.
        let directory = kernel.save.directories.mounts[0]
            .as_ref()
            .unwrap()
            .directory
            .try_clone()
            .unwrap();
        let mut budget = Budget::new(Default::default()).unwrap();
        assert!(matches!(
            kernel
                .save
                .directories
                .equivalent_busy(&directory, &mut budget),
            Err(Error::Code(BUSY))
        ));
    }
    guest.put(POINT, &mount_point(7));
    assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(0));
    assert_eq!(mount3(&mut kernel, &mut guest, "replacement", 4, 48), Ok(0));
    assert_eq!(guest.get(OUT, 16), mount_point(7));
}

#[test]
fn encoded_names_remain_distinct_and_failed_host_or_filesystem_admission_cannot_publish_a_save_mount()
 {
    let root = Root::new();
    let mut kernel = root.kernel();
    let mut guest = Guest::new();
    for value in ["A", "a", "A.", "CON"] {
        assert_eq!(mount3(&mut kernel, &mut guest, value, 4, 48), Ok(0));
        remember_point(&mut guest);
        assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(0));
    }
    let failure = kernel.save.memory.storage().unwrap().failure_control();
    failure.store(true, Ordering::Relaxed);
    assert_eq!(
        mount3(&mut kernel, &mut guest, "host-failed", 4, 48),
        Ok(INTERNAL)
    );
    failure.store(false, Ordering::Relaxed);
    assert!(
        kernel
            .save
            .memory
            .storage()
            .unwrap()
            .open_save(&key("host-failed"))
            .unwrap()
            .is_none()
    );
    assert!(!kernel.save.directories.live());
    drop(kernel);
    let readonly = crate::ReadOnlyRoot::new("/savedata0", vec![]).unwrap();
    let mut conflict = Kernel::new(
        Default::default(),
        ProcessIdentity {
            process_id: 9,
            initial_thread_id: 9,
        },
        vec![readonly],
    )
    .unwrap();
    conflict.configure_save_storage(root.storage()).unwrap();
    assert!(matches!(
        mount3(&mut conflict, &mut guest, "fs-failed", 4, 48),
        Err(Failure::Errno(crate::Errno::ReadOnly))
    ));
    assert_eq!(guest.get(OUT, 64), [0; 64]);
    assert!(
        conflict
            .save
            .memory
            .storage()
            .unwrap()
            .open_save(&key("fs-failed"))
            .unwrap()
            .is_none()
    );
    assert!(!conflict.save.directories.live());
}

#[test]
fn memory_mount_busy_sync_terminate_and_delete_share_real_directory_and_persistent_bundle_lifetimes()
 {
    let root = Root::new();
    let mut kernel = root.kernel();
    let mut guest = Guest::new();
    assert_eq!(
        mount3(&mut kernel, &mut guest, "sce_sdmemory", 4, 48),
        Ok(0)
    );
    remember_point(&mut guest);
    guest.put(BASE, &[0; 64]);
    guest.put(BASE + 4, &7i32.to_le_bytes());
    guest.put(BASE + 8, &8u64.to_le_bytes());
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::SaveMemorySetup,
            &[BASE, BASE + 0x300]
        ),
        Ok(BUSY)
    );
    assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(0));
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::SaveMemorySetup,
            &[BASE, BASE + 0x300]
        ),
        Ok(0)
    );
    guest.put(BASE, &7i32.to_le_bytes());
    guest.put(BASE + 4, &0u32.to_le_bytes());
    guest.put(BASE + 8, &1u32.to_le_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveMemorySync, &[BASE]),
        Ok(0)
    );
    assert!(root.0.join("memory-u7-s0.pxsave").is_file());
    assert_eq!(
        mount3(&mut kernel, &mut guest, "sce_sdmemory", 32, 0),
        Ok(0)
    );
    remember_point(&mut guest);
    guest.put(BASE, &7i32.to_le_bytes());
    guest.put(BASE + 4, &0u32.to_le_bytes());
    guest.put(BASE + 8, &1u32.to_le_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveMemorySync, &[BASE]),
        Ok(BUSY)
    );
    assert_eq!(unmount(&mut kernel, &mut guest, 0), Ok(0));
    guest.put(BASE, &[0; 64]);
    guest.put(BASE, &7i32.to_le_bytes());
    guest.put(BASE + 16, &NAME.to_le_bytes());
    name(&mut guest, "sce_sdmemory");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveDelete, &[BASE]),
        Ok(BUSY)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveTerminate, &[]),
        Ok(0)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::SaveDelete, &[BASE]),
        Ok(0)
    );
    assert!(!root.0.join("memory-u7-s0.pxsave").exists());
    assert!(
        kernel
            .save
            .memory
            .storage()
            .unwrap()
            .open_save(&key("sce_sdmemory"))
            .unwrap()
            .is_none()
    );
}
