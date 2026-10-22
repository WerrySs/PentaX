// SPDX-License-Identifier: GPL-2.0-only
// Original bounded APR ABI consumers; no commercial input or host path imports.
use super::*;

const LIST: u64 = ARG + 0x100;
const IDS: u64 = ARG + 0x200;
const SIZES: u64 = ARG + 0x300;
const RESULTS: u64 = ARG + 0x400;
const PREFIX: u64 = ARG + 0x600;
const STAT: u64 = ARG + 0x1800;

fn owner(file: &OriginalFile, limits: Limits) -> Kernel {
    Kernel::new(
        limits,
        ProcessIdentity {
            process_id: 41,
            initial_thread_id: 73,
        },
        vec![file.root()],
    )
    .unwrap()
}
fn invoke(
    kernel: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    args: &[u64],
) -> Result<u64, Failure> {
    let mut words = [0; 6];
    words[..args.len()].copy_from_slice(args);
    kernel.invoke_library(service, CallArgs(words), guest)
}
fn paths(guest: &mut TestGuest, names: &[&str]) {
    for (index, name) in names.iter().enumerate() {
        let offset = 0x800 + index as u64 * 0x500;
        guest.put(offset, name.as_bytes());
        guest.put(offset + name.len() as u64, &[0]);
        guest.put(0x100 + index as u64 * 8, &(ARG + offset).to_le_bytes());
    }
}
fn u32_at(guest: &TestGuest, address: u64) -> u32 {
    u32::from_le_bytes(guest.get(address, 4).try_into().unwrap())
}
fn errno(kernel: &mut Kernel, guest: &mut TestGuest) -> u64 {
    invoke(kernel, guest, Service::ErrnoLocation, &[]).unwrap()
}

#[test]
fn apr_all_eight_resolver_abis_share_actual_ids_sizes_prefixes_and_low32_count() {
    use Service::*;
    let file = OriginalFile::new();
    let cases = [
        (AprResolveFilepathsToIds, false, false, false),
        (AprResolveFilepathsToIdsAndFileSizes, false, true, false),
        (AprResolveFilepathsWithPrefixToIds, true, false, false),
        (
            AprResolveFilepathsWithPrefixToIdsAndFileSizes,
            true,
            true,
            false,
        ),
        (AprResolveFilepathsToIdsForEach, false, false, true),
        (
            AprResolveFilepathsToIdsAndFileSizesForEach,
            false,
            true,
            true,
        ),
        (AprResolveFilepathsWithPrefixToIdsForEach, true, false, true),
        (
            AprResolveFilepathsWithPrefixToIdsAndFileSizesForEach,
            true,
            true,
            true,
        ),
    ];
    for (service, prefixed, sized, each) in cases {
        let mut kernel = owner(&file, Default::default());
        let mut guest = TestGuest::new();
        paths(
            &mut guest,
            &[if prefixed {
                "data.txt"
            } else {
                "/original/data.txt"
            }],
        );
        guest.put(0x600, b"/original/\0");
        guest.put(0x400, &91_u32.to_le_bytes());
        let mut args = Vec::new();
        if prefixed {
            args.push(PREFIX);
        }
        args.extend([LIST, (19 << 32) | 1, IDS]);
        if sized {
            args.push(SIZES);
        }
        args.push(RESULTS);
        assert_eq!(
            invoke(&mut kernel, &mut guest, service, &args),
            Ok(u64::from(each))
        );
        assert_eq!(u32_at(&guest, IDS), 1);
        assert_eq!(u32_at(&guest, RESULTS), if each { 0 } else { 91 });
        if sized {
            assert_eq!(guest.u64(0x300), 24);
        }
        assert_eq!(kernel.usage().apr_paths, 1);
        assert_eq!(
            invoke(
                &mut kernel,
                &mut guest,
                AprGetFileSize,
                &[(17 << 32) | 1, SIZES]
            ),
            Ok(0)
        );
        assert_eq!(guest.u64(0x300), 24);
    }
}

#[test]
fn apr_foreach_continues_semantic_errors_returns_success_count_and_preserves_errno() {
    let file = OriginalFile::new();
    let mut kernel = owner(&file, Default::default());
    let mut guest = TestGuest::new();
    let errno = errno(&mut kernel, &mut guest);
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address: errno,
                bytes: &77_u32.to_le_bytes(),
            }],
        )
        .unwrap();
    paths(
        &mut guest,
        &[
            "/original/data.txt",
            "/original/missing",
            "/original/data.txt",
        ],
    );
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIdsAndFileSizesForEach,
            &[LIST, 3, IDS, SIZES, RESULTS]
        ),
        Ok(2)
    );
    assert_eq!(
        [
            u32_at(&guest, IDS),
            u32_at(&guest, IDS + 4),
            u32_at(&guest, IDS + 8)
        ],
        [1, u32::MAX, 1]
    );
    assert_eq!(
        [guest.u64(0x300), guest.u64(0x308), guest.u64(0x310)],
        [24, 0, 24]
    );
    assert_eq!(
        [
            u32_at(&guest, RESULTS),
            u32_at(&guest, RESULTS + 4),
            u32_at(&guest, RESULTS + 8)
        ],
        [0, 0x8002_0002, 0]
    );
    assert_eq!(u32_at(&guest, errno), 77);
    assert_eq!(kernel.usage().apr_paths, 2);
    // A null results pointer selects the source first-error behavior even for
    // the ForEach export; outputs beyond the failing item remain untouched.
    guest.put(0x208, &85_u32.to_le_bytes());
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIdsForEach,
            &[LIST, 3, IDS, 0]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(u32_at(&guest, IDS + 8), 85);
    assert_eq!(u32_at(&guest, errno), 2);
    // Results alone is a valid request: it resolves/cache-checks real paths.
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIdsForEach,
            &[LIST, 3, 0, RESULTS]
        ),
        Ok(2)
    );
}

#[test]
fn apr_foreach_virtual_aliases_follow_success_and_failure_store_order_and_live_inputs() {
    let file = OriginalFile::new();
    let mut kernel = owner(&file, Default::default());
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/original/data.txt", "/original/missing"]);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIdsForEach,
            &[LIST, 2, IDS, IDS]
        ),
        Ok(1)
    );
    assert_eq!(u32_at(&guest, IDS), 0); // success: ID then result
    assert_eq!(u32_at(&guest, IDS + 4), u32::MAX); // failure: result then ID
    paths(
        &mut guest,
        &[
            "/original/data.txt",
            "/original/data.txt",
            "/original/data.txt",
        ],
    );
    // First result overwrites the low half of the next live pointer. The
    // resulting semantic null fault is recorded and the third item still runs.
    guest.put(0x110, &(ARG + 0x1200).to_le_bytes());
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIdsForEach,
            &[LIST, 3, IDS, LIST + 8]
        ),
        Ok(2)
    );
    assert_eq!(u32_at(&guest, LIST + 12), 0x8002_000e);
    assert_eq!(u32_at(&guest, IDS + 8), 1);
}

#[test]
fn apr_identity_queries_use_cached_size_current_stat_layout_and_source_error_order() {
    let file = OriginalFile::new();
    let mut kernel = owner(&file, Default::default());
    let mut guest = TestGuest::new();
    let errno = errno(&mut kernel, &mut guest);
    paths(&mut guest, &["/original/data.txt"]);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIds,
            &[LIST, 1, IDS, 0]
        ),
        Ok(0)
    );
    std::fs::OpenOptions::new()
        .write(true)
        .open(&file.path)
        .unwrap()
        .set_len(521)
        .unwrap();
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprGetFileSize,
            &[1, SIZES]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x300), 24);
    assert_eq!(
        invoke(&mut kernel, &mut guest, Service::AprGetFileStat, &[1, STAT]),
        Ok(0)
    );
    assert_eq!(guest.get(STAT + 8, 2), 0o100777_u16.to_le_bytes());
    assert_eq!(guest.u64(0x1800 + 72), 521);
    assert_eq!(guest.u64(0x1800 + 80), 2);
    assert_eq!(u32_at(&guest, STAT + 88), 512);
    for (service, output, expected) in [
        (Service::AprGetFileSize, 0, 22),
        (Service::AprGetFileStat, 0, 22),
        (Service::AprGetFileSize, u64::MAX, 14),
        (Service::AprGetFileStat, u64::MAX, 2),
    ] {
        assert_eq!(
            invoke(&mut kernel, &mut guest, service, &[99, output]),
            Ok(u64::from(u32::MAX))
        );
        assert_eq!(u32_at(&guest, errno), expected);
    }
    let mut other = owner(&file, Default::default());
    // Each owner has its own guest address space, including its errno mapping.
    let mut other_guest = TestGuest::new();
    other_guest.put(0x300, &85_u64.to_le_bytes());
    assert_eq!(
        invoke(
            &mut other,
            &mut other_guest,
            Service::AprGetFileSize,
            &[1, SIZES]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(other_guest.u64(0x300), 85);
    let other_errno = errno(&mut other, &mut other_guest);
    assert_eq!(u32_at(&other_guest, other_errno), 2);
    guest.regions[0].protection = 1;
    assert_eq!(
        invoke(&mut kernel, &mut guest, Service::AprGetFileStat, &[1, STAT]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
}

#[test]
fn apr_prefix_is_literal_snapshot_bounded_and_refused_items_do_not_publish_ids() {
    let file = OriginalFile::new();
    let mut kernel = owner(&file, Default::default());
    let mut guest = TestGuest::new();
    let errno = errno(&mut kernel, &mut guest);
    paths(&mut guest, &["data.txt", "data.txt"]);
    guest.put(0x600, b"/original\0");
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsWithPrefixToIds,
            &[PREFIX, LIST, 1, IDS, RESULTS]
        ),
        Ok(u64::from(u32::MAX))
    );
    assert_eq!(u32_at(&guest, errno), 2); // no inserted slash
    guest.put(0x600, b"/original/\0");
    // Outputs overwrite the source prefix, but the already-owned prefix
    // snapshot must resolve the next path with its original bytes.
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsWithPrefixToIdsForEach,
            &[PREFIX, LIST, 2, PREFIX, RESULTS]
        ),
        Ok(2)
    );
    assert_eq!(u32_at(&guest, PREFIX), 1);
    assert_eq!(u32_at(&guest, PREFIX + 4), 1);
    paths(&mut guest, &["a"]);
    guest.put(0x2000, &[b'x'; 1023]);
    guest.put(0x2000 + 1023, &[0]);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsWithPrefixToIdsForEach,
            &[ARG + 0x2000, LIST, 1, IDS, RESULTS]
        ),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, RESULTS), 0x8002_003f);
    let mut limited = owner(
        &file,
        Limits {
            max_scratch_bytes: 1024,
            max_io_bytes: 1024,
            ..Default::default()
        },
    );
    guest.put(0x600, b"/original/\0");
    paths(&mut guest, &["data.txt"]);
    assert_eq!(
        invoke(
            &mut limited,
            &mut guest,
            Service::AprResolveFilepathsWithPrefixToIdsForEach,
            &[PREFIX, LIST, 1, IDS, RESULTS]
        ),
        Err(Failure::Limit("scratch"))
    );
    assert_eq!(limited.usage().apr_paths, 0);
    guest.refuse = true;
    assert!(matches!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsWithPrefixToIdsForEach,
            &[PREFIX, LIST, 1, IDS, RESULTS]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(kernel.usage().apr_paths, 2); // one cached miss + one success
}

#[test]
fn apr_directory_and_null_prefix_queries_retain_owner_lifetime_without_file_handles() {
    let file = OriginalFile::new();
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0200_0000);
    }
    let root =
        ReadOnlyRoot::directory("/directory", options.open(&file.directory).unwrap()).unwrap();
    let mut kernel = Kernel::new(
        Default::default(),
        ProcessIdentity {
            process_id: 41,
            initial_thread_id: 73,
        },
        vec![root],
    )
    .unwrap();
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/directory"]);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsWithPrefixToIdsAndFileSizes,
            &[0, LIST, 1, IDS, SIZES, 0]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x300), 0x1_0000);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprGetFileSize,
            &[1, SIZES]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x300), 0);
    assert_eq!(
        invoke(&mut kernel, &mut guest, Service::AprGetFileStat, &[1, STAT]),
        Ok(0)
    );
    assert_eq!(guest.get(STAT + 8, 2), 0o040777_u16.to_le_bytes());
    assert_eq!(guest.u64(0x1800 + 72), 0);
    assert_eq!(guest.u64(0x1800 + 80), 0);
    assert_eq!(u32_at(&guest, STAT + 88), 512);
    assert_eq!(kernel.usage().handles_live, 0);
    assert_eq!(kernel.usage().apr_paths, 1);
}

#[test]
fn apr_late_foreach_output_refusal_keeps_actual_prefix_without_caching_failed_item() {
    let file = OriginalFile::new();
    let mut kernel = owner(&file, Default::default());
    let mut guest = TestGuest::new();
    paths(&mut guest, &["/original/data.txt", "/original/missing"]);
    guest.put(0x200, &[0x55; 8]);
    let results = ARG + PAGE - 4;
    // This original Guest adapter reports refused output transactions as
    // WriteDenied, including an unmapped later store; no current-item effects.
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprResolveFilepathsToIdsForEach,
            &[LIST, 2, IDS, results]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(u32_at(&guest, IDS), 1);
    assert_eq!(u32_at(&guest, IDS + 4), 0x5555_5555);
    assert_eq!(u32_at(&guest, results), 0);
    assert_eq!(kernel.usage().apr_paths, 1);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::AprGetFileSize,
            &[1, SIZES]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x300), 24);
}
