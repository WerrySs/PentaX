// SPDX-License-Identifier: GPL-2.0-only
use super::*;

const LIST: u64 = ARG + 0x100;
const IDS: u64 = ARG + 0x200;
const SIZES: u64 = ARG + 0x300;
const ERROR: u64 = ARG + 0x400;

fn owner(roots: Vec<ReadOnlyRoot>, limits: Limits) -> Kernel {
    Kernel::new(
        limits,
        ProcessIdentity {
            process_id: 41,
            initial_thread_id: 73,
        },
        roots,
    )
    .unwrap()
}
fn directory_root(file: &OriginalFile) -> ReadOnlyRoot {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0200_0000);
    }
    ReadOnlyRoot::directory("/directory", options.open(&file.directory).unwrap()).unwrap()
}
fn paths(guest: &mut TestGuest, names: &[&str]) {
    for (index, name) in names.iter().enumerate() {
        let at = 0x800 + index as u64 * 0x500;
        guest.put(at, name.as_bytes());
        guest.put(at + name.len() as u64, &[0]);
        guest.put(0x100 + index as u64 * 8, &(ARG + at).to_le_bytes());
    }
}
fn resolve(owner: &mut Kernel, guest: &mut TestGuest, args: &[u64]) -> Result<u64, Failure> {
    let mut values = [0; 6];
    values[..args.len()].copy_from_slice(args);
    owner.invoke_library(
        Service::AprResolveFilepathsToIdsAndFileSizes,
        CallArgs(values),
        guest,
    )
}
fn u32_at(guest: &TestGuest, address: u64) -> u32 {
    u32::from_le_bytes(guest.get(address, 4).try_into().unwrap())
}
fn errno(owner: &mut Kernel, guest: &mut TestGuest) -> u64 {
    owner
        .invoke_library(Service::ErrnoLocation, CallArgs([0; 6]), guest)
        .unwrap()
}

fn apr_read(
    owner: &mut Kernel,
    guest: &mut impl GuestAccess,
    id: u32,
    destination: u64,
    size: u64,
    offset: u64,
) -> Result<(), Failure> {
    owner
        .files
        .read_apr_file(id, destination, size, offset, guest, &mut owner.budget)
}

#[test]
fn apr_resolution_retains_unique_ids_exact_sizes_directories_and_cached_misses() {
    let file = OriginalFile::new();
    let mut owner = owner(vec![file.root(), directory_root(&file)], Default::default());
    let mut guest = TestGuest::new();
    let errno = errno(&mut owner, &mut guest);
    guest.put(0x400, &37_u32.to_le_bytes());
    paths(
        &mut guest,
        &["/original/data.txt", "/directory", "/original/data.txt"],
    );
    assert_eq!(
        resolve(
            &mut owner,
            &mut guest,
            &[LIST, (9 << 32) | 3, IDS, SIZES, ERROR]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.get(IDS, 12),
        [1_u32, 2, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(guest.u64(0x300), 24);
    assert_eq!(guest.u64(0x308), 0x1_0000);
    assert_eq!(guest.u64(0x310), 24);
    assert_eq!(u32_at(&guest, ERROR), 37);
    assert_eq!(u32_at(&guest, errno), 0);
    assert_eq!(owner.usage().apr_paths, 2);

    paths(&mut guest, &["/directory/new.bin"]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, ERROR]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(u32_at(&guest, errno), 2);
    std::fs::write(file.directory.join("new.bin"), [1, 2, 3]).unwrap();
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, ERROR]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(owner.usage().apr_paths, 3);
    std::fs::remove_file(file.directory.join("new.bin")).unwrap();

    paths(&mut guest, &["/directory/original.txt"]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, 0, ERROR]),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, IDS), 3); // A cached miss never consumes a successful ID.
    assert_eq!(u32_at(&guest, errno), 2); // Success preserves errno.
    let mut other = owner_for_isolation(&file);
    assert_eq!(
        resolve(&mut other, &mut guest, &[LIST, 1, IDS, 0, 0]),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, IDS), 1);
}
fn owner_for_isolation(file: &OriginalFile) -> Kernel {
    owner(vec![directory_root(file)], Default::default())
}

#[test]
fn apr_resolution_preserves_partial_lists_and_ordered_output_aliases() {
    let file = OriginalFile::new();
    let mut owner = owner(vec![file.root()], Default::default());
    let mut guest = TestGuest::new();
    let errno = errno(&mut owner, &mut guest);
    paths(
        &mut guest,
        &[
            "/original/data.txt",
            "/original/missing",
            "/original/data.txt",
        ],
    );
    guest.put(0x200, &[0xa5; 16]);
    guest.put(0x300, &[0x5a; 24]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 3, IDS, SIZES, ERROR]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(
        guest.get(IDS, 12),
        [1_u32, u32::MAX, 0xa5a5_a5a5]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>()
    );
    assert_eq!(guest.u64(0x300), 24);
    assert_eq!(guest.u64(0x308), 0);
    assert_eq!(guest.u64(0x310), 0x5a5a_5a5a_5a5a_5a5a);
    assert_eq!(u32_at(&guest, ERROR), 1);
    assert_eq!(u32_at(&guest, errno), 2);

    guest.put(0x1fe, &[0xa5; 32]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 2, IDS, IDS - 2, IDS + 5]),
        Ok(u64::from(u32::MAX))
    );
    let mut expected = [0xa5; 32];
    expected[2..6].copy_from_slice(&1_u32.to_le_bytes());
    expected[0..8].copy_from_slice(&24_u64.to_le_bytes());
    expected[6..10].copy_from_slice(&u32::MAX.to_le_bytes());
    expected[8..16].copy_from_slice(&0_u64.to_le_bytes());
    expected[7..11].copy_from_slice(&1_u32.to_le_bytes());
    assert_eq!(guest.get(IDS - 2, 32), expected);
}

#[test]
fn apr_resolution_reads_later_inputs_after_prior_output_and_bounds_strings() {
    let file = OriginalFile::new();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file.path)
        .unwrap()
        .set_len(0)
        .unwrap();
    let mut owner = owner(vec![file.root()], Default::default());
    let mut guest = TestGuest::new();
    let errno = errno(&mut owner, &mut guest);
    paths(&mut guest, &["/original/data.txt", "/original/data.txt"]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 2, IDS, LIST + 8, ERROR]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(u32_at(&guest, IDS), 1);
    assert_eq!(u32_at(&guest, IDS + 4), u32::MAX);
    assert_eq!(u32_at(&guest, ERROR), 1);
    assert_eq!(u32_at(&guest, errno), 14); // First size zeroed the next live pointer.

    guest.put(0x800, &[b'x'; 1024]);
    guest.put(0xc00, &[0]);
    guest.put(0x100, &(ARG + 0x800).to_le_bytes());
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, 0, ERROR]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(u32_at(&guest, errno), 63);
    assert_eq!(owner.usage().apr_paths, 1);
}

#[test]
fn apr_resolution_refuses_invalid_arguments_faults_and_independent_cache_limits() {
    let file = OriginalFile::new();
    for case in 0..6 {
        let limits = match case {
            0 => Limits {
                max_apr_paths: 0,
                ..Default::default()
            },
            1 => Limits {
                max_apr_path_bytes: 1,
                ..Default::default()
            },
            4 => Limits {
                max_bytes: 0,
                ..Default::default()
            },
            5 => Limits {
                max_scratch_bytes: 1024,
                max_io_bytes: 1024,
                ..Default::default()
            },
            _ => Default::default(),
        };
        let mut owner = owner(vec![file.root()], limits);
        let mut guest = TestGuest::new();
        paths(&mut guest, &["/original/data.txt"]);
        guest.put(0x200, &[0xa5; 16]);
        if case == 2 {
            guest.refuse = true;
        }
        let output = if case == 3 { ARG + PAGE - 3 } else { IDS };
        assert!(matches!(
            resolve(&mut owner, &mut guest, &[LIST, 1, output, 0, 0]),
            Err(Failure::Limit(_) | Failure::Guest(_))
        ));
        assert_eq!(owner.usage().apr_paths, 0);
        assert_eq!(guest.get(IDS, 16), [0xa5; 16]);
    }
    let mut owner = owner(
        vec![file.root()],
        Limits {
            max_handles: 1,
            max_apr_paths: 1,
            ..Default::default()
        },
    );
    let mut guest = TestGuest::new();
    let errno = errno(&mut owner, &mut guest);
    for args in [
        [0, 1, IDS, 0, 0],
        [LIST, 0, IDS, 0, 0],
        [LIST, 1025, IDS, 0, 0],
        [LIST, 1, 0, 0, ERROR],
    ] {
        assert_eq!(
            resolve(&mut owner, &mut guest, &args),
            Ok(u64::from(u32::MAX))
        );
        assert_eq!(u32_at(&guest, errno), 22);
    }
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, u64::MAX - 1, 0, 0]),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(u32_at(&guest, errno), 14);
    paths(&mut guest, &["/original/data.txt"]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, ERROR]),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, 1); // Only errno; APR is independent of fd limits.
    paths(&mut guest, &["/original/new"]);
    assert!(matches!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, ERROR]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(owner.usage().apr_paths, 1);
    assert_eq!(u32_at(&guest, IDS), 1);
    assert_eq!(
        super::resolve("libkernel", (1, 1), "libkernel", 1, "gEpBkcwxUjw"),
        Some(Service::AprResolveFilepathsToIdsAndFileSizes)
    );
}

#[test]
fn apr_reads_use_current_size_leave_short_tails_and_preserve_metadata_and_fd_position() {
    let file = OriginalFile::new();
    let mut owner = owner(
        vec![file.root()],
        Limits {
            max_io_bytes: 4,
            ..Default::default()
        },
    );
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/original/data.txt"]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, 0]),
        Ok(0)
    );
    let id = u32_at(&guest, IDS);
    let fd = call(&mut owner, &mut guest, Service::Open, &[ARG + 0x800, 0, 0]).unwrap();
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Read,
            &[fd, ARG + 0x3000, 4]
        ),
        Ok(4)
    );
    guest.put(0x3000, &[0xa5; 32]);
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, ARG + 0x3000, 32, 20),
        Ok(())
    );
    assert_eq!(guest.get(ARG + 0x3000, 4), b"6789");
    assert_eq!(guest.get(ARG + 0x3004, 28), [0xa5; 28]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::Lseek, &[fd, 0, 1]),
        Ok(4)
    );
    // Resolve retains its original metadata, while submission rechecks the
    // selected capability's current length and does not synthesize zero tail.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file.path)
        .unwrap()
        .set_len(6)
        .unwrap();
    guest.put(0x3000, &[0x5a; 24]);
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, ARG + 0x3000, 24, 0),
        Ok(())
    );
    assert_eq!(guest.get(ARG + 0x3000, 6), b"origin");
    assert_eq!(guest.get(ARG + 0x3006, 18), [0x5a; 18]);
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, 0]),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, IDS), id);
    assert_eq!(guest.u64(0x300), 24);
    assert_eq!(owner.usage().apr_paths, 1);
    assert_eq!(
        apr_read(&mut owner, &mut guest, u32::MAX, 0, 0, 0),
        Err(Errno::NoEntry.into())
    );
    assert_eq!(apr_read(&mut owner, &mut guest, id, 0, 0, u64::MAX), Ok(()));
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, 0, 1, u64::MAX),
        Err(Errno::Fault.into())
    );
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, u64::MAX - 1, 4, u64::MAX),
        Err(Errno::Fault.into())
    );
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, ARG + PAGE, 1, u64::MAX),
        Ok(())
    );
}

#[test]
fn apr_chunk_failure_keeps_prior_bytes_and_zero_io_limit_cannot_spin() {
    struct LaterFailure {
        guest: TestGuest,
        writes: usize,
    }
    impl GuestAccess for LaterFailure {
        fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
            self.guest.read(address, output)
        }
        fn apply(
            &mut self,
            effect: MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.writes += 1;
            if self.writes == 2 {
                return Err(GuestError::WriteDenied);
            }
            self.guest.apply(effect, writes)
        }
    }
    let file = OriginalFile::new();
    let mut owner = owner(
        vec![file.root()],
        Limits {
            max_io_bytes: 4,
            ..Default::default()
        },
    );
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/original/data.txt"]);
    resolve(&mut owner, &mut guest, &[LIST, 1, IDS, 0, 0]).unwrap();
    let id = u32_at(&guest, IDS);
    guest.put(0x3000, &[0xa5; 12]);
    let mut guest = LaterFailure { guest, writes: 0 };
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, ARG + 0x3000, 12, 0),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.guest.get(ARG + 0x3000, 4), b"orig");
    assert_eq!(guest.guest.get(ARG + 0x3004, 8), [0xa5; 8]);
    assert_eq!(owner.usage().apr_paths, 1);
    let mut zero = self::owner(
        vec![file.root()],
        Limits {
            max_io_bytes: 0,
            ..Default::default()
        },
    );
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/original/data.txt"]);
    resolve(&mut zero, &mut guest, &[LIST, 1, IDS, 0, 0]).unwrap();
    let id = u32_at(&guest, IDS);
    assert_eq!(
        apr_read(&mut zero, &mut guest, id, ARG + 0x3000, 1, 0),
        Err(Failure::Limit("APR read chunk bytes"))
    );
    assert_eq!(apr_read(&mut zero, &mut guest, id, 0, 0, 0), Ok(()));
}

#[test]
fn apr_known_id_numeric_fault_precedes_missing_open_and_directories_are_not_file_bytes() {
    let file = OriginalFile::new();
    let mut owner = owner(vec![directory_root(&file)], Default::default());
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/directory/original.txt", "/directory"]);
    resolve(&mut owner, &mut guest, &[LIST, 2, IDS, SIZES, 0]).unwrap();
    let id = u32_at(&guest, IDS);
    let directory_id = u32_at(&guest, IDS + 4);
    std::fs::remove_file(&file.path).unwrap();
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, 0, 1, u64::MAX),
        Err(Errno::Fault.into())
    );
    assert_eq!(
        apr_read(&mut owner, &mut guest, id, ARG + 0x3000, 1, 0),
        Err(Errno::NoEntry.into())
    );
    assert_eq!(
        apr_read(&mut owner, &mut guest, directory_id, ARG + 0x3000, 1, 0),
        Err(Errno::NoEntry.into())
    );
    assert_eq!(
        resolve(&mut owner, &mut guest, &[LIST, 1, IDS, SIZES, 0]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x300), 24);
    assert_eq!(owner.usage().apr_paths, 2);
}
