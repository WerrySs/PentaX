// SPDX-License-Identifier: GPL-2.0-only
//! Original owned-directory persistence and refusal proofs; no commercial inputs.
use super::*;
use crate::tests::{ARG, TestGuest};
use crate::{GuestError, Kernel, ProcessIdentity};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pentax-save-fs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn directory(&self) -> Dir {
        Dir::open_ambient_dir(&self.0, cap_std::ambient_authority()).unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        // The fixed private test root is never derived from a guest name.
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pentax-save-fs-")
        );
        assert_eq!(
            self.0.canonicalize().unwrap().parent(),
            Some(std::env::temp_dir().canonicalize().unwrap().as_path())
        );
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn new_owner(roots: Vec<ReadOnlyRoot>) -> Kernel {
    Kernel::new(
        Limits::default(),
        ProcessIdentity {
            process_id: 100,
            initial_thread_id: 7,
        },
        roots,
    )
    .unwrap()
}
fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    args: &[u64],
) -> Result<u64, Failure> {
    let mut words = [0; 6];
    words[..args.len()].copy_from_slice(args);
    owner.invoke(service, CallArgs(words), guest)
}
fn mount(owner: &mut Kernel, root: &Root, slot: usize) {
    let prepared = owner
        .files
        .prepare_save_mount(slot, root.directory(), &mut owner.budget)
        .unwrap();
    owner.files.commit_save_mount(prepared);
}
fn open(owner: &mut Kernel, guest: &mut TestGuest, path: &str, flags: u32) -> Result<u64, Failure> {
    let mut name = path.as_bytes().to_vec();
    name.push(0);
    guest.put(0, &name);
    call(owner, guest, Service::Open, &[ARG, u64::from(flags), 0o600])
}

#[test]
fn save_mount_publication_real_persistence_positional_io_and_unmounted_open_handles() {
    let root = Root::new();
    let mut owner = new_owner(vec![]);
    let mut guest = TestGuest::new();
    let prepared = owner
        .files
        .prepare_save_mount(15, root.directory(), &mut owner.budget)
        .unwrap();
    assert_eq!(
        open(&mut owner, &mut guest, "/savedata15/state.bin", 0),
        Err(Errno::NoEntry.into())
    );
    owner.files.commit_save_mount(prepared);
    let fd = open(
        &mut owner,
        &mut guest,
        "/savedata15/nested/state.bin",
        0x202,
    )
    .unwrap();
    guest.put(128, b"abcde");
    assert_eq!(
        call(&mut owner, &mut guest, Service::Write, &[fd, ARG + 128, 5]),
        Ok(5)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Pwrite,
            &[fd, ARG + 128, 2, 1]
        ),
        Ok(2)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::Lseek, &[fd, 0, 1]),
        Ok(5)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Pread,
            &[fd, ARG + 256, 5, 0]
        ),
        Ok(5)
    );
    assert_eq!(guest.get(ARG + 256, 5), b"aabde");
    assert_eq!(
        call(&mut owner, &mut guest, Service::Lseek, &[fd, 0, 1]),
        Ok(5)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::Ftruncate, &[fd, 3]),
        Ok(0)
    );
    assert_eq!(call(&mut owner, &mut guest, Service::Fsync, &[fd]), Ok(0));
    owner.files.unmount_save(15);
    assert_eq!(
        open(&mut owner, &mut guest, "/savedata15/nested/state.bin", 0),
        Err(Errno::NoEntry.into())
    );
    guest.put(128, b"Z");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Pwrite,
            &[fd, ARG + 128, 1, 0]
        ),
        Ok(1)
    );
    assert_eq!(call(&mut owner, &mut guest, Service::Close, &[fd]), Ok(0));
    drop(owner);
    assert_eq!(
        std::fs::read(root.0.join("nested/state.bin")).unwrap(),
        b"Zab"
    );
}

#[test]
fn save_descriptor_access_guest_input_and_all_io_caps_refuse_before_persistent_effects() {
    let root = Root::new();
    std::fs::write(root.0.join("state.bin"), b"original").unwrap();
    let mut owner = new_owner(vec![]);
    let mut guest = TestGuest::new();
    mount(&mut owner, &root, 0);
    let read_only = open(&mut owner, &mut guest, "/savedata0/state.bin", 0).unwrap();
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Write,
            &[read_only, ARG + 128, 3]
        ),
        Err(Errno::BadFd.into())
    );
    let write_only = open(&mut owner, &mut guest, "/savedata0/state.bin", 1).unwrap();
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Read,
            &[write_only, ARG + 256, 1]
        ),
        Err(Errno::BadFd.into())
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Write,
            &[write_only, u64::MAX - 1, 3]
        ),
        Err(Failure::Guest(GuestError::AddressOverflow))
    );
    guest.put(128, b"new");
    owner.files.limits.max_io_bytes = 2;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::Write,
            &[write_only, ARG + 128, 3]
        ),
        Err(Failure::Limit(_))
    ));
    owner.files.limits.max_io_bytes = 1024;
    owner.budget.limits.max_save_io_bytes = 2;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::Write,
            &[write_only, ARG + 128, 3]
        ),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_save_io_bytes = 1024;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::Pwrite,
            &[write_only, ARG + 128, 1, u64::MAX]
        ),
        Err(Failure::Errno(Errno::Invalid))
    ));
    assert_eq!(
        call(&mut owner, &mut guest, Service::Write, &[write_only, 0, 0]),
        Err(Errno::Fault.into())
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::Lseek, &[write_only, 0, 1]),
        Ok(0)
    );
    assert_eq!(
        open(&mut owner, &mut guest, "/savedata0/state.bin", 0x400),
        Err(Errno::Access.into())
    );
    assert_eq!(
        open(&mut owner, &mut guest, "/savedata0/state.bin", 0xa01),
        Err(Errno::Exists.into())
    );
    drop(owner);
    assert_eq!(
        std::fs::read(root.0.join("state.bin")).unwrap(),
        b"original"
    );
}

#[test]
fn writable_save_mounts_do_not_expand_readonly_roots_or_unsafe_guest_names() {
    assert_eq!(namespace("/savedata10/data"), Some((10, "data")));
    assert_eq!(namespace("/savedata+10/data"), None);
    assert_eq!(namespace("/savedata01/data"), None);
    let root = Root::new();
    std::fs::write(root.0.join("original"), b"readonly").unwrap();
    let ro = ReadOnlyRoot::new(
        "/app0",
        vec![
            ReadOnlyFile::new(
                "original",
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(root.0.join("original"))
                    .unwrap(),
            )
            .unwrap(),
        ],
    )
    .unwrap();
    let mut owner = new_owner(vec![ro]);
    let mut guest = TestGuest::new();
    mount(&mut owner, &root, 0);
    let fd = open(&mut owner, &mut guest, "/app0/original", 0).unwrap();
    guest.put(128, b"X");
    assert_eq!(
        call(&mut owner, &mut guest, Service::Write, &[fd, ARG + 128, 1]),
        Err(Errno::BadFd.into())
    );
    assert_eq!(
        open(&mut owner, &mut guest, "/app0/original", 1),
        Err(Errno::ReadOnly.into())
    );
    for path in [
        "/savedata0/../escape",
        "/savedata0/C:escape",
        "/savedata0/CON",
        "/savedata0/trailing.",
        "/savedata+0/escape",
        "/savedata00/escape",
        "/savedata16/escape",
    ] {
        assert!(open(&mut owner, &mut guest, path, 0x201).is_err(), "{path}");
    }
    assert_eq!(std::fs::read(root.0.join("original")).unwrap(), b"readonly");
    drop(owner);
    let ro = ReadOnlyRoot::new(
        "/savedata0/guarded",
        vec![ReadOnlyFile::new("data", File::open(root.0.join("original")).unwrap()).unwrap()],
    )
    .unwrap();
    let mut blocked = new_owner(vec![ro]);
    // An inactive save slot must not shadow an owner's pre-existing RO mount.
    let fd = open(&mut blocked, &mut guest, "/savedata0/guarded/data", 0).unwrap();
    assert_eq!(
        call(&mut blocked, &mut guest, Service::Read, &[fd, ARG + 256, 8]),
        Ok(8)
    );
    assert_eq!(guest.get(ARG + 256, 8), b"readonly");
    assert_eq!(
        open(&mut blocked, &mut guest, "/savedata0/guarded/data", 1),
        Err(Errno::ReadOnly.into())
    );
    assert_eq!(
        call(
            &mut blocked,
            &mut guest,
            Service::Write,
            &[fd, ARG + 128, 1]
        ),
        Err(Errno::BadFd.into())
    );
    assert!(matches!(
        blocked
            .files
            .prepare_save_mount(0, root.directory(), &mut blocked.budget),
        Err(Failure::Errno(Errno::ReadOnly))
    ));
    assert!(matches!(
        blocked
            .files
            .prepare_save_mount(16, root.directory(), &mut blocked.budget),
        Err(Failure::Errno(Errno::Invalid))
    ));
}

#[test]
fn save_path_mutations_append_sync_and_icon_reads_use_only_the_owned_directory() {
    let root = Root::new();
    let mut owner = new_owner(vec![]);
    let mut guest = TestGuest::new();
    mount(&mut owner, &root, 1);
    guest.put(0, b"/savedata1/folder\0");
    assert_eq!(
        call(&mut owner, &mut guest, Service::Mkdir, &[ARG, 0o700]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::Mkdir, &[ARG, 0o700]),
        Err(Errno::Exists.into())
    );
    let fd = open(&mut owner, &mut guest, "/savedata1/folder/icon.bin", 0x20a).unwrap();
    guest.put(128, b"abc");
    assert_eq!(
        call(&mut owner, &mut guest, Service::Write, &[fd, ARG + 128, 3]),
        Ok(3)
    );
    guest.put(128, b"Z");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Pwrite,
            &[fd, ARG + 128, 1, 0]
        ),
        Ok(1)
    ); // Append overrides Pwrite offset.
    assert_eq!(
        call(&mut owner, &mut guest, Service::Lseek, &[fd, 0, 1]),
        Ok(3)
    );
    assert_eq!(call(&mut owner, &mut guest, Service::Fsync, &[fd]), Ok(0));
    assert_eq!(call(&mut owner, &mut guest, Service::Close, &[fd]), Ok(0));
    assert_eq!(
        owner
            .files
            .save_icon_bytes("/savedata1/folder/icon.bin", 4, 16, &mut owner.budget)
            .unwrap(),
        b"abcZ"
    );
    assert!(matches!(
        owner
            .files
            .save_icon_bytes("/savedata1/folder/icon.bin", 3, 0, &mut owner.budget),
        Err(Failure::Limit(_))
    ));
    let before = owner.budget.limits.max_scratch_bytes;
    owner.budget.limits.max_scratch_bytes = 19;
    assert!(matches!(
        owner
            .files
            .save_icon_bytes("/savedata1/folder/icon.bin", 4, 16, &mut owner.budget),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_scratch_bytes = before;
    guest.put(0, b"/savedata1/folder/icon.bin\0");
    guest.put(128, b"/savedata1/moved/icon.bin\0");
    assert_eq!(
        call(&mut owner, &mut guest, Service::Rename, &[ARG, ARG + 128]),
        Ok(0)
    );
    guest.put(0, b"/savedata1/moved/icon.bin\0");
    assert_eq!(call(&mut owner, &mut guest, Service::Unlink, &[ARG]), Ok(0));
    guest.put(0, b"/savedata1/folder\0");
    assert_eq!(call(&mut owner, &mut guest, Service::Rmdir, &[ARG]), Ok(0));
    drop(owner);
    assert!(!root.0.join("moved/icon.bin").exists());
}

#[test]
fn save_directory_catalog_is_append_only_exactly_scoped_and_has_no_cross_product_aliases() {
    use crate::catalog::resolve;
    assert!(
        (Service::SaveTransferringMount as usize)
            > Service::ConvertKeycodeGetImeKeyboardType as usize
    );
    assert_eq!(
        resolve("SaveData", (1, 1), "SaveData", 1, "WAzWTZm1H+I"),
        Some(Service::SaveTransferringMount)
    );
    assert_eq!(
        resolve(
            "SaveData_native",
            (1, 1),
            "SaveData_native",
            1,
            "RjMlsR8EXrw"
        ),
        Some(Service::SaveTransferringMount)
    );
    assert_eq!(
        resolve(
            "libSceSaveData_native",
            (1, 1),
            "libSceSaveData_native",
            1,
            "WAzWTZm1H+I"
        ),
        Some(Service::SaveTransferringMount)
    );
    for (module, library, nid, version) in [
        (
            "libSceSaveData_native",
            "libSceSaveData_native",
            "RjMlsR8EXrw",
            (1, 1),
        ),
        (
            "libSceSaveData_native",
            "SaveData_native",
            "WAzWTZm1H+I",
            (1, 1),
        ),
        (
            "SaveData_native",
            "libSceSaveData_native",
            "WAzWTZm1H+I",
            (1, 1),
        ),
        ("SaveData", "SaveData", "WAzWTZm1H+I", (1, 0)),
        (
            "libSceSaveData_native",
            "libSceSaveData_native",
            "ZP4e7rlzOUk",
            (1, 1),
        ),
    ] {
        assert_eq!(resolve(module, version, library, 1, nid), None);
    }
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "nKWi-N2HBV4"),
        Some(Service::Pwrite)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "ih4CD9-gghM"),
        Some(Service::PosixFtruncate)
    );
}

#[test]
fn save_posix_wrappers_preserve_errno_widths_low_descriptor_words_and_standard_streams() {
    let root = Root::new();
    let mut owner = new_owner(vec![]);
    let mut guest = TestGuest::new();
    mount(&mut owner, &root, 0);
    let errno = call(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    guest
        .apply(
            crate::MemoryEffect::None,
            &[GuestWrite {
                address: errno,
                bytes: &37_u32.to_le_bytes(),
            }],
        )
        .unwrap();
    let fd = open(&mut owner, &mut guest, "/savedata0/state", 0x202).unwrap();
    guest.put(128, b"persist");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixPwrite,
            &[fd | 0xaabb_ccdd_0000_0000, ARG + 128, 7, 0]
        ),
        Ok(7)
    );
    assert_eq!(
        u32::from_le_bytes(guest.get(errno, 4).try_into().unwrap()),
        37
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixPwrite,
            &[2, ARG + 128, 1, 0]
        ),
        Ok(u64::MAX)
    );
    assert_eq!(
        u32::from_le_bytes(guest.get(errno, 4).try_into().unwrap()),
        Errno::Perm as u32
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixFtruncate, &[2, 0]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(
        u32::from_le_bytes(guest.get(errno, 4).try_into().unwrap()),
        Errno::BadFd as u32
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PosixMkdir, &[0, 0]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(
        u32::from_le_bytes(guest.get(errno, 4).try_into().unwrap()),
        Errno::Invalid as u32
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::Write, &[1, ARG + 128, 7]),
        Ok(7)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PosixWrite,
            &[2, ARG + 128, 7]
        ),
        Ok(7)
    );
    assert_eq!(owner.take_output(crate::OutputStream::Stdout), b"persist");
    assert_eq!(owner.take_output(crate::OutputStream::Stderr), b"persist");
    drop(owner);
    assert_eq!(std::fs::read(root.0.join("state")).unwrap(), b"persist");
}
