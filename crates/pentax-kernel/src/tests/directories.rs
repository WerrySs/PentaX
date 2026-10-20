// SPDX-License-Identifier: GPL-2.0-only

//! Owned original directory capabilities, including real junction escapes.

use super::*;
use std::fs::{OpenOptions, create_dir, remove_dir, remove_file};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

struct Directory {
    path: PathBuf,
    junction: bool,
}
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax-owned-directory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        create_dir(&path).unwrap();
        create_dir(path.join("assets")).unwrap();
        std::fs::write(path.join("assets/ab\u{03b4}"), [37, 42, 99]).unwrap();
        Self {
            path,
            junction: false,
        }
    }
    fn root(&self, mount: &str) -> ReadOnlyRoot {
        let directory = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0200_0000)
            .open(&self.path)
            .unwrap();
        ReadOnlyRoot::directory(mount, directory).unwrap()
    }
    fn junction(&mut self, target: &Directory) {
        let status = Command::new("powershell.exe")
            .args(["-NoProfile","-NonInteractive","-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:PENTAX_TEST_LINK -Target $env:PENTAX_TEST_TARGET | Out-Null"])
            .env("PENTAX_TEST_LINK",self.path.join("escape"))
            .env("PENTAX_TEST_TARGET",&target.path)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .creation_flags(0x0800_0000).status().unwrap();
        assert!(status.success(), "original junction fixture setup failed");
        self.junction = true;
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if self.junction {
            remove_dir(self.path.join("escape")).unwrap(); // Junction only, never recursive.
        }
        assert_eq!(
            std::fs::read(self.path.join("assets/ab\u{03b4}")).unwrap(),
            [37, 42, 99]
        );
        remove_file(self.path.join("assets/ab\u{03b4}")).unwrap();
        remove_dir(self.path.join("assets")).unwrap();
        remove_dir(&self.path).unwrap();
    }
}

fn owner(roots: Vec<ReadOnlyRoot>, limits: Limits) -> Kernel {
    Kernel::new(
        limits,
        ProcessIdentity {
            process_id: 100,
            initial_thread_id: 7,
        },
        roots,
    )
    .unwrap()
}

#[test]
fn directory_descriptors_keep_packed_offsets_base_outputs_and_source_error_widths() {
    let directory = Directory::new();
    let mut kernel = owner(vec![directory.root("/app0")], Limits::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"/app0/assets/\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0x20000, 0]),
        Ok(3)
    );
    assert_eq!(
        (kernel.usage().handles_live, kernel.usage().directory_bytes),
        (1, 512)
    );
    guest.put(256, &37_u64.to_le_bytes());
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdirentries,
            &[3, ARG + 1024, 513, ARG + 256]
        ),
        Ok(512)
    );
    assert_eq!(guest.u64(256), 0);
    assert_eq!(guest.u64(1024), 0x0104_000c_48fc_91ec);
    assert_eq!(guest.u64(1024 + 12), 0x0204_000c_6696_98f4);
    assert_eq!(guest.get(ARG + 1024 + 30, 2), [8, 4]);
    assert_eq!(guest.get(ARG + 1024 + 32, 5), "ab\u{03b4}\0".as_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Getdents, &[3, 1, 512, 1]),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdirentries,
            &[3, 1, 512, ARG + 256]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(256), 512);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 0]),
        Ok(0)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[3, ARG + 1024, 1]),
        Ok(1)
    );
    assert_eq!(guest.get(ARG + 1024, 1), [0xec]);
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdents,
            &[3, ARG + 1024, 512]
        ),
        Err(Failure::Errno(Errno::Invalid))
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 513, 0]),
        Ok(513)
    );
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdents,
            &[3, ARG + 1024, 512]
        ),
        Err(Failure::Errno(Errno::Invalid))
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[3, 1, 512]),
        Ok(0)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 1]),
        Ok(513)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 2]),
        Ok(512)
    );
    assert!(matches!(
        call(
            &mut kernel,
            &mut guest,
            Service::Pread,
            &[3, ARG + 1024, 1, 0]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        kernel.invoke_library(
            Service::Getdents,
            CallArgs([0, ARG + 1024, 512, 0, 0, 0]),
            &mut guest
        ),
        Ok(0x8002_0009)
    );
    assert_eq!(
        kernel.invoke_library(
            Service::Getdirentries,
            CallArgs([3, 0, 512, 0, 0, 0]),
            &mut guest
        ),
        Ok(0x8002_000e)
    );
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
    assert_eq!(
        (kernel.usage().handles_live, kernel.usage().directory_bytes),
        (0, 0)
    );
    guest.put(0, "/app0/assets/ab\u{03b4}\0".as_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0x20000, 0]),
        Err(Failure::Errno(Errno::NotDirectory))
    );
}

#[test]
fn directory_multi_output_failures_never_publish_base_buffer_or_advance_position() {
    let directory = Directory::new();
    let mut kernel = owner(vec![directory.root("/app0")], Limits::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"/app0/assets\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Ok(3)
    );
    guest.put(1024, &[0xa5; 512]);
    guest.put(256, &37_u64.to_le_bytes());
    for (buffer, base) in [(1, ARG + 256), (ARG + 1024, 1)] {
        assert!(matches!(
            call(
                &mut kernel,
                &mut guest,
                Service::Getdirentries,
                &[3, buffer, 512, base]
            ),
            Err(Failure::Guest(_))
        ));
        assert_eq!(guest.u64(256), 37);
        assert_eq!(guest.get(ARG + 1024, 512), [0xa5; 512]);
        assert_eq!(
            call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 1]),
            Ok(0)
        );
    }
    assert!(matches!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdirentries,
            &[3, ARG + 1024, 512, ARG + 1028]
        ),
        Err(Failure::Unsupported(_))
    ));
    guest.refuse = true;
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdirentries,
            &[3, ARG + 1024, 512, ARG + 256]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(guest.u64(256), 37);
    assert_eq!(guest.get(ARG + 1024, 512), [0xa5; 512]);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 1]),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdirentries,
            &[3, ARG + 1024, 512, ARG + 256]
        ),
        Ok(512)
    );
    assert_eq!(guest.u64(256), 0);
}

#[test]
fn directory_snapshots_bound_retained_storage_and_stat_observes_new_entries() {
    let directory = Directory::new();
    let mut kernel = owner(
        vec![directory.root("/app0")],
        Limits {
            max_directory_bytes: 1024,
            ..Default::default()
        },
    );
    let mut guest = TestGuest::new();
    guest.put(0, b"/app0/assets\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Ok(3)
    );
    let names = ["a".repeat(255), "b".repeat(255)];
    for name in &names {
        std::fs::write(directory.path.join("assets").join(name), [42]).unwrap();
    }
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Fstat, &[3, ARG + 512]),
        Ok(0)
    );
    assert_eq!(guest.u64(512 + 72), 512);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Stat, &[ARG, ARG + 512]),
        Ok(0)
    );
    assert_eq!(guest.u64(512 + 72), 1024);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Err(Failure::Limit("directory bytes"))
    );
    assert_eq!(
        (kernel.usage().handles_live, kernel.usage().directory_bytes),
        (1, 512)
    );
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Ok(3)
    );
    assert_eq!(kernel.usage().directory_bytes, 1024);
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
    for name in &names {
        std::fs::remove_file(directory.path.join("assets").join(name)).unwrap();
    }
    let mut small = owner(
        vec![directory.root("/app0")],
        Limits {
            max_scratch_bytes: 511,
            max_name_bytes: 128,
            max_io_bytes: 511,
            ..Default::default()
        },
    );
    assert_eq!(
        call(&mut small, &mut guest, Service::Open, &[ARG, 0, 0]),
        Err(Failure::Limit("scratch"))
    );
    assert_eq!(
        (small.usage().handles_live, small.usage().directory_bytes),
        (0, 0)
    );
}

#[test]
fn directory_capability_reads_on_demand_and_retains_actual_descriptor_identity() {
    let directory = Directory::new();
    let mut kernel = owner(vec![directory.root("/app0")], Limits::default());
    let mut guest = TestGuest::new();
    guest.put(0, "/app0/assets/ab\u{03b4}\0".as_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Ok(3)
    );
    assert!(
        OpenOptions::new()
            .write(true)
            .open(directory.path.join("assets/ab\u{03b4}"))
            .is_err()
    );
    assert!(std::fs::rename(&directory.path, directory.path.with_extension("renamed")).is_err());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[3, ARG + 256, 1]),
        Ok(1)
    );
    assert_eq!(guest.get(ARG + 256, 1), [37]);
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Pread,
            &[3, ARG + 256, 1, 1]
        ),
        Ok(1)
    );
    assert_eq!(guest.get(ARG + 256, 1), [42]);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 1]),
        Ok(1)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Fstat, &[3, ARG + 512]),
        Ok(0)
    );
    assert_eq!(guest.u64(512 + 72), 3);
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Stat, &[ARG, ARG + 512]),
        Ok(0)
    );
    assert_eq!(guest.u64(512 + 72), 3);
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
    assert!(
        OpenOptions::new()
            .write(true)
            .open(directory.path.join("assets/ab\u{03b4}"))
            .is_ok()
    );
    // This file did not exist when the root was admitted. Opening it must
    // use the retained directory capability, not a captured file inventory.
    std::fs::write(directory.path.join("late.bin"), [42]).unwrap();
    guest.put(0, b"/app0/late.bin\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Ok(3)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[3, ARG + 256, 1]),
        Ok(1)
    );
    assert_eq!(guest.get(ARG + 256, 1), [42]);
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
    std::fs::remove_file(directory.path.join("late.bin")).unwrap();
    guest.put(0, b"/app0\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::CheckReachability, &[ARG]),
        Ok(0)
    );
    guest.put(0, b"/app0/assets\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::CheckReachability, &[ARG]),
        Ok(0)
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Stat, &[ARG, ARG + 512]),
        Ok(0)
    );
    assert_eq!(guest.u64(512 + 72), 512);
    assert_eq!(
        u16::from_le_bytes(guest.get(ARG + 512 + 8, 2).try_into().unwrap()),
        0o040777
    );
    drop(kernel);
    assert!(std::fs::rename(&directory.path, directory.path.with_extension("renamed")).is_ok());
    std::fs::rename(directory.path.with_extension("renamed"), &directory.path).unwrap();
}

#[test]
fn directory_paths_cannot_escape_through_junctions_devices_or_normalized_aliases() {
    let mut inside = Directory::new();
    let outside = Directory::new();
    inside.junction(&outside);
    let mut kernel = owner(vec![inside.root("/app0")], Limits::default());
    let mut guest = TestGuest::new();
    for path in [
        "/app0/../assets/ab\u{03b4}",
        "/app0/assets/../ab\u{03b4}",
        "/app0/assets//ab\u{03b4}",
        "/app0/NUL",
        "/app0/CONIN$",
        "/app0/COM\u{00b9}.txt",
        "/app0/NUL .txt",
        "/app0/COM1.txt",
        "/app0/Lpt9",
        "/app0/assets/ab\u{03b4}.",
        "/app0/assets/ab\u{03b4} ",
        "/app0/assets/file:stream",
        "/app0/assets/*",
        "/app0/assets/ab\u{03b4}/extra",
    ] {
        guest.put(0, format!("{path}\0").as_bytes());
        assert!(
            matches!(
                call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
                Err(Failure::Errno(_))
            ),
            "invalid path unexpectedly opened"
        );
        assert_eq!(kernel.usage().handles_live, 0);
    }
    guest.put(0, "/app0/escape/assets/ab\u{03b4}\0".as_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Err(Failure::Errno(Errno::Access))
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::CheckReachability, &[ARG]),
        Err(Failure::Errno(Errno::Access))
    );
    assert_eq!(kernel.usage().handles_live, 0);
    guest.put(0, "/app0/assets/ab\u{03b4}\0".as_bytes());
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 1, 0]),
        Err(Failure::Errno(Errno::ReadOnly))
    );
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0, 0]),
        Ok(3)
    );
    guest.refuse = true;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Read, &[3, ARG + 256, 1]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Lseek, &[3, 0, 1]),
        Ok(0)
    );
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
    guest.put(0, b"/app0\0");
    assert_eq!(
        call(&mut kernel, &mut guest, Service::Open, &[ARG, 0x20000, 0]),
        Ok(3)
    );
    assert_eq!(
        call(
            &mut kernel,
            &mut guest,
            Service::Getdents,
            &[3, ARG + 1024, 512]
        ),
        Ok(512)
    );
    let records = guest.get(ARG + 1024, 512);
    let mut offset = 0;
    let mut link_type = None;
    while offset < records.len() {
        let length =
            u16::from_le_bytes(records[offset + 4..offset + 6].try_into().unwrap()) as usize;
        assert!(length >= 12 && offset + length <= records.len());
        let name = &records[offset + 8..offset + 8 + records[offset + 7] as usize];
        if name == b"escape" {
            link_type = Some(records[offset + 6]);
        }
        offset += length;
    }
    assert_eq!(link_type, Some(4)); // List the junction itself without opening its target.
    assert_eq!(call(&mut kernel, &mut guest, Service::Close, &[3]), Ok(0));
}
