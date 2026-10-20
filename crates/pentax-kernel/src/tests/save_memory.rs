// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use Service::*;

struct SaveRoot(PathBuf);
impl SaveRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pentax-save-memory-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
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
        SaveStorage::directory(options.open(&self.0).unwrap()).unwrap()
    }
    fn bundle(&self) -> PathBuf {
        self.0.join("memory-u1-s0.pxsave")
    }
}
impl Drop for SaveRoot {
    fn drop(&mut self) {
        for item in std::fs::read_dir(&self.0).unwrap() {
            let path = item.unwrap().path();
            assert_eq!(path.parent(), Some(self.0.as_path()));
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}
fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn setup(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    size: u64,
    option: u32,
    slot: u32,
) -> Result<u64, Failure> {
    guest.put(0, &[0; 64]);
    guest.put(0, &option.to_le_bytes());
    guest.put(4, &1_i32.to_le_bytes());
    guest.put(8, &size.to_le_bytes());
    guest.put(40, &slot.to_le_bytes());
    call(owner, guest, SaveMemorySetup, &[ARG, ARG + 64])
}
fn descriptor(guest: &mut TestGuest, buffer: u64, size: u64, offset: i64) {
    guest.put(128, &buffer.to_le_bytes());
    guest.put(136, &size.to_le_bytes());
    guest.put(144, &offset.to_le_bytes());
}
fn get(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    size: u64,
    offset: i64,
    param: u64,
) -> Result<u64, Failure> {
    descriptor(guest, ARG + 1024, size, offset);
    guest.put(256, &[0; 64]);
    guest.put(256, &1_i32.to_le_bytes());
    guest.put(264, &(ARG + 128).to_le_bytes());
    guest.put(272, &param.to_le_bytes());
    call(owner, guest, SaveMemoryGet, &[ARG + 256])
}
fn set(owner: &mut Kernel, guest: &mut TestGuest, bytes: &[u8]) -> Result<u64, Failure> {
    guest.put(1024, bytes);
    descriptor(guest, ARG + 1024, bytes.len() as u64, 0);
    guest.put(256, &[0; 64]);
    guest.put(256, &1_i32.to_le_bytes());
    guest.put(264, &(ARG + 128).to_le_bytes());
    // Source treats a nonnull descriptor with count0 as one input.
    call(owner, guest, SaveMemorySet, &[ARG + 256])
}

#[test]
fn save_memory_persists_replacements_and_reloads_real_data_and_metadata_after_process_drop() {
    let root = SaveRoot::new();
    let mut guest = TestGuest::new();
    let mut owner = kernel(Default::default());
    owner.configure_save_storage(root.storage()).unwrap();
    assert_eq!(setup(&mut owner, &mut guest, 2 * 1024 * 1024, 1, 0), Ok(0));
    assert_eq!(guest.u64(64), 0);
    assert_eq!(owner.usage().save_memory_bytes, 2 * 1024 * 1024);
    assert!(!root.bundle().exists());
    assert_eq!(get(&mut owner, &mut guest, 8, 0, ARG + 2048), Ok(0));
    assert_eq!(guest.get(ARG + 1024, 8), [0; 8]);
    assert_eq!(guest.get(ARG + 2048, 11), b"Saved Data\0");
    assert_eq!(set(&mut owner, &mut guest, b"old-data"), Ok(0));
    assert_eq!(set(&mut owner, &mut guest, b"new-data"), Ok(0));
    assert!(root.bundle().is_file());
    drop(owner);
    let mut owner = kernel(Default::default());
    owner.configure_save_storage(root.storage()).unwrap();
    assert_eq!(setup(&mut owner, &mut guest, 96, 1, 0), Ok(0));
    assert_eq!(guest.u64(64), 2 * 1024 * 1024);
    assert_eq!(get(&mut owner, &mut guest, 8, 0, ARG + 2048), Ok(0));
    assert_eq!(guest.get(ARG + 1024, 8), b"new-data");
    assert_eq!(guest.get(ARG + 2048, 11), b"Saved Data\0");
    assert!(i64::from_le_bytes(guest.get(ARG + 2048 + 1288, 8).try_into().unwrap()) > 0);
    assert_eq!(call(&mut owner, &mut guest, SaveTerminate, &[]), Ok(0));
    assert_eq!(owner.usage().save_memory_bytes, 0);
    drop(owner);
}

#[test]
fn save_memory_faults_preserve_ram_disk_and_pending_sync_error_events() {
    let root = SaveRoot::new();
    let mut guest = TestGuest::new();
    let storage = root.storage();
    let failure = storage.failure_control();
    let mut owner = kernel(Default::default());
    owner.configure_save_storage(storage).unwrap();
    assert_eq!(setup(&mut owner, &mut guest, 64, 1, 0), Ok(0));
    assert_eq!(set(&mut owner, &mut guest, b"old-data"), Ok(0));
    let original = std::fs::read(root.bundle()).unwrap();
    failure.store(true, Ordering::Relaxed);
    assert_eq!(set(&mut owner, &mut guest, b"new-data"), Ok(0x809f_000b));
    assert_eq!(std::fs::read(root.bundle()).unwrap(), original);
    assert_eq!(get(&mut owner, &mut guest, 8, 0, 0), Ok(0));
    assert_eq!(guest.get(ARG + 1024, 8), b"old-data");
    guest.put(512, &[0; 40]);
    guest.put(512, &1_i32.to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, SaveMemorySync, &[ARG + 512]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 2048]),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG + 2048, 12),
        [3_u32, 0x809f_000b, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(std::fs::read(root.bundle()).unwrap(), original);
    guest.put(520, &1_u32.to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, SaveMemorySync, &[ARG + 512]),
        Ok(0x809f_000b)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 2048]),
        Ok(0x809f_0008)
    );
    failure.store(false, Ordering::Relaxed);
    assert_eq!(set(&mut owner, &mut guest, b"new-data"), Ok(0));
    guest.put(1024, &[0xa5; 8]);
    assert_eq!(
        get(&mut owner, &mut guest, 8, 0, ARG + PAGE - 4),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + 1024, 8), [0xa5; 8]);
    assert_eq!(get(&mut owner, &mut guest, 8, 0, 0), Ok(0));
    assert_eq!(guest.get(ARG + 1024, 8), b"new-data");
    drop(owner);
}

#[test]
fn save_memory_output_refusal_limits_and_corrupt_storage_do_not_publish_ready_entries() {
    let root = SaveRoot::new();
    let mut guest = TestGuest::new();
    let mut owner = kernel(Default::default());
    owner.configure_save_storage(root.storage()).unwrap();
    guest.put(0, &[0; 64]);
    guest.put(4, &1_i32.to_le_bytes());
    guest.put(8, &64_u64.to_le_bytes());
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, SaveMemorySetup, &[ARG, ARG + 64]),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(owner.usage().save_memory_bytes, 0);
    assert_eq!(
        setup(&mut owner, &mut guest, 17 * 1024 * 1024, 2, 0),
        Ok(0x809f_0017)
    );
    assert_eq!(setup(&mut owner, &mut guest, 64, 0, 0), Ok(0));
    assert_eq!(setup(&mut owner, &mut guest, 64, 0, 0), Ok(0x809f_0003));
    assert_eq!(set(&mut owner, &mut guest, b"old-data"), Ok(0));
    drop(owner);
    let mut corrupted = std::fs::read(root.bundle()).unwrap();
    let end = corrupted.len() - 1;
    corrupted[end] ^= 1;
    std::fs::write(root.bundle(), &corrupted).unwrap();
    let mut owner = kernel(Default::default());
    owner.configure_save_storage(root.storage()).unwrap();
    guest.put(64, &[0xa5; 24]);
    assert_eq!(setup(&mut owner, &mut guest, 64, 0, 0), Ok(0x809f_000f));
    assert_eq!(owner.usage().save_memory_bytes, 0);
    assert_eq!(guest.get(ARG + 64, 24), [0xa5; 24]);
    assert_eq!(std::fs::read(root.bundle()).unwrap(), corrupted);
    drop(owner);
}

#[test]
fn save_memory_exclusive_storage_and_io_policy_precede_durable_changes() {
    let root = SaveRoot::new();
    let mut guest = TestGuest::new();
    let mut owner = kernel(Limits {
        max_save_io_bytes: 0,
        ..Default::default()
    });
    owner.configure_save_storage(root.storage()).unwrap();
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0200_0000);
    }
    assert!(matches!(
        SaveStorage::directory(options.open(&root.0).unwrap()),
        Err(Failure::Unsupported("save storage already in use"))
    ));
    assert_eq!(setup(&mut owner, &mut guest, 64, 1, 0), Ok(0));
    assert_eq!(
        set(&mut owner, &mut guest, b"new-data"),
        Err(Failure::Limit("save I/O"))
    );
    assert!(!root.bundle().exists());
    assert_eq!(owner.usage().save_bytes_written, 0);
    assert_eq!(get(&mut owner, &mut guest, 8, 0, 0), Ok(0));
    assert_eq!(guest.get(ARG + 1024, 8), [0; 8]);
    // A valid first descriptor followed by an invalid range cannot publish.
    guest.put(256 + 32, &2_u32.to_le_bytes());
    descriptor(&mut guest, ARG + 1024, 8, 0);
    guest.put(264, &(ARG + 128).to_le_bytes());
    guest.put(192, &(ARG + 1024).to_le_bytes());
    guest.put(200, &8_u64.to_le_bytes());
    guest.put(208, &(-1_i64).to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, SaveMemorySet, &[ARG + 256]),
        Ok(0x809f_0000)
    );
    assert!(!root.bundle().exists());
    drop(owner);
    // Releasing the owner releases the OS lock without stale PID cleanup.
    drop(root.storage());
}
