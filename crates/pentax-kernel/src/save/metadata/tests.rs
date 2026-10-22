// SPDX-License-Identifier: GPL-2.0-only
//! Original persistent metadata/icon fixtures; no title or private save inputs.
use super::*;
use crate::{
    Kernel, Limits, ProcessIdentity, ReadOnlyFile, ReadOnlyRoot,
    tests::{ARG, TestGuest},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "pentax-save-metadata-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn storage(&self) -> SaveStorage {
        let directory = Dir::open_ambient_dir(&self.0, cap_std::ambient_authority()).unwrap();
        SaveStorage::directory_for_title(directory.into_std_file(), "ORIG00001").unwrap()
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let temporary_root = std::env::temp_dir();
        assert_eq!(self.0.parent(), Some(temporary_root.as_path()));
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pentax-save-metadata-")
        );
        std::fs::remove_dir_all(&self.0).unwrap();
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
    owner.invoke(service, CallArgs(args), guest)
}
fn setup(
    root: &Root,
    roots: Vec<ReadOnlyRoot>,
) -> (
    Kernel,
    TestGuest,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let storage = root.storage();
    let failure = storage.failure_control();
    let mut owner = Kernel::new(
        Limits::default(),
        ProcessIdentity {
            process_id: 71,
            initial_thread_id: 9,
        },
        roots,
    )
    .unwrap();
    owner.configure_save_storage(storage).unwrap();
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, Service::SaveInitialize, &[]),
        Ok(0)
    );
    (owner, guest, failure)
}
fn mount(owner: &mut Kernel, guest: &mut TestGuest, directory: &[u8]) {
    guest.put(0, &[0; 80]);
    guest.put(0, &1_i32.to_le_bytes());
    guest.put(8, &(ARG + 128).to_le_bytes());
    guest.put(16, &48_u64.to_le_bytes());
    guest.put(32, &4_u32.to_le_bytes());
    guest.put(128, &[0; 32]);
    guest.put(128, directory);
    assert_eq!(
        call(owner, guest, Service::SaveMount3, &[ARG, ARG + 256]),
        Ok(0)
    );
    let point = guest.get(ARG + 256, 16);
    guest.put(512, &point);
}
fn set(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    kind: u64,
    payload: &[u8],
) -> Result<u64, Failure> {
    guest.put(4096, payload);
    call(
        owner,
        guest,
        Service::SaveSetParam,
        &[ARG + 512, kind, ARG + 4096, payload.len() as u64],
    )
}
fn get(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    kind: u64,
    capacity: u64,
) -> Result<u64, Failure> {
    call(
        owner,
        guest,
        Service::SaveGetParam,
        &[ARG + 512, kind, ARG + 2048, capacity, ARG + 1800],
    )
}
fn icon(guest: &mut TestGuest, buffer: u64, capacity: u64, length: u64) {
    guest.put(1024, &[0xa5; 56]);
    guest.put(1024, &buffer.to_le_bytes());
    guest.put(1032, &capacity.to_le_bytes());
    guest.put(1040, &length.to_le_bytes());
}
fn search(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    pattern: Option<&[u8]>,
    capacity: u32,
) -> Result<u64, Failure> {
    guest.put(0, &[0; 64]);
    guest.put(0, &1_i32.to_le_bytes());
    if let Some(pattern) = pattern {
        guest.put(16, &(ARG + 96).to_le_bytes());
        guest.put(96, &[0; 32]);
        guest.put(96, pattern);
    }
    guest.put(128, &[0xa5; 56]);
    guest.put(136, &(ARG + 1024).to_le_bytes());
    guest.put(144, &capacity.to_le_bytes());
    guest.put(152, &(ARG + 4096).to_le_bytes());
    guest.put(160, &(ARG + 6144).to_le_bytes());
    call(owner, guest, Service::SaveDirNameSearch, &[ARG, ARG + 128])
}

#[test]
fn save_directory_metadata_roundtrips_all_fields_and_partial_text_layout_without_padding_damage() {
    let root = Root::new();
    let (mut owner, mut guest, _) = setup(&root, vec![]);
    mount(&mut owner, &mut guest, b"Data");
    let mut original = [0xa5; PARAM_BYTES];
    original[..15].copy_from_slice(b"Original title\0");
    original[1280..1284].copy_from_slice(&42_u32.to_le_bytes());
    assert_eq!(set(&mut owner, &mut guest, 0, &original), Ok(0));
    assert_eq!(set(&mut owner, &mut guest, 1, b"Fresh"), Ok(0));
    assert_eq!(set(&mut owner, &mut guest, 4, &99_u32.to_le_bytes()), Ok(0));
    guest.put(2040, &[0x5a; PARAM_BYTES + 16]);
    assert_eq!(get(&mut owner, &mut guest, 1, 4), Ok(0));
    assert_eq!(guest.get(ARG + 2048, 4), b"Fre\0");
    assert_eq!(guest.u64(1800), 4);
    assert_eq!(guest.get(ARG + 2044, 4), [0x5a; 4]);
    assert_eq!(guest.get(ARG + 2052, 4), [0x5a; 4]);
    assert_eq!(get(&mut owner, &mut guest, 0, PARAM_BYTES as u64), Ok(0));
    let actual = guest.get(ARG + 2048, PARAM_BYTES);
    assert_eq!(&actual[..6], b"Fresh\0");
    assert!(actual[6..128].iter().all(|byte| *byte == 0));
    assert_eq!(&actual[128..1280], &original[128..1280]);
    assert_eq!(&actual[1280..1284], &99_u32.to_le_bytes());
    assert_eq!(&actual[1284..1288], &original[1284..1288]);
    assert!(i64::from_le_bytes(actual[1288..1296].try_into().unwrap()) > 0);
    assert_eq!(&actual[1296..], &original[1296..]);
    assert_eq!(owner.usage().save_memory_bytes, 0);
    guest.put(4096, b"ABI");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveSetParam,
            &[ARG + 512, 0xaabb_ccdd_0000_0001, ARG + 4096, 3]
        ),
        Ok(0)
    );
    assert_eq!(get(&mut owner, &mut guest, 0xaabb_ccdd_0000_0001, 4), Ok(0));
    assert_eq!(guest.get(ARG + 2048, 4), b"ABI\0");
}

#[test]
fn save_directory_metadata_failed_replacements_and_atomic_guest_output_refusals_keep_prior_data() {
    let root = Root::new();
    let (mut owner, mut guest, failure) = setup(&root, vec![]);
    mount(&mut owner, &mut guest, b"Data");
    assert_eq!(set(&mut owner, &mut guest, 1, b"Before"), Ok(0));
    failure.store(true, Ordering::Relaxed);
    assert_eq!(set(&mut owner, &mut guest, 1, b"After"), Ok(INTERNAL));
    failure.store(false, Ordering::Relaxed);
    guest.put(2048, &[0x5a; 32]);
    guest.put(1800, &[0x5a; 8]);
    guest.refuse = true;
    assert!(matches!(
        get(&mut owner, &mut guest, 1, 32),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.get(ARG + 2048, 32), [0x5a; 32]);
    assert_eq!(guest.get(ARG + 1800, 8), [0x5a; 8]);
    assert_eq!(get(&mut owner, &mut guest, 1, 32), Ok(0));
    assert_eq!(guest.get(ARG + 2048, 7), b"Before\0");
    assert_eq!(get(&mut owner, &mut guest, 4, 3), Ok(PARAMETER));
    assert_eq!(get(&mut owner, &mut guest, 6, 8), Ok(PARAMETER));
}

#[test]
fn save_directory_search_returns_real_sorted_matches_full_hit_count_params_and_blocks() {
    let root = Root::new();
    let (mut owner, mut guest, _) = setup(&root, vec![]);
    mount(&mut owner, &mut guest, b"Beta");
    let beta: [u8; 16] = guest.get(ARG + 512, 16).try_into().unwrap();
    for directory in [b"alpha".as_slice(), b"sce_hidden"] {
        mount(&mut owner, &mut guest, directory);
    }
    guest.put(1024, &[0x5a; 64]);
    assert_eq!(search(&mut owner, &mut guest, None, 1), Ok(0));
    assert_eq!(u32_at(&guest.get(ARG + 128, 56), 0), 2);
    assert_eq!(u32_at(&guest.get(ARG + 128, 56), 20), 1);
    assert_eq!(guest.get(ARG + 1024, 5), b"Beta\0");
    assert_eq!(guest.get(ARG + 1029, 27), [0x5a; 27]);
    assert_eq!(guest.u64(136), ARG + 1024);
    assert_eq!(guest.u64(6144), 48);
    assert_eq!(guest.u64(6152), 48);
    assert_eq!(guest.get(ARG + 6160, 32), [0; 32]);
    assert_eq!(search(&mut owner, &mut guest, Some(b"AL%"), 2), Ok(0));
    assert_eq!(u32_at(&guest.get(ARG + 128, 56), 0), 1);
    assert_eq!(guest.get(ARG + 1024, 6), b"alpha\0");
    assert!(i64::from_le_bytes(guest.get(ARG + 4096 + 1288, 8).try_into().unwrap()) > 0);
    {
        use std::io::Write;
        let mounted = owner.save.directories.mounted(&beta).unwrap();
        let mut file = mounted
            .directory
            .open_with(
                "sce_sys/param.bin",
                cap_std::fs::OpenOptions::new().write(true).truncate(true),
            )
            .unwrap();
        file.write_all(b"bad").unwrap();
    }
    guest.put(4096, &[0x5a; PARAM_BYTES]);
    guest.put(6144, &[0x5a; 48]);
    assert_eq!(search(&mut owner, &mut guest, None, 1), Ok(BROKEN));
    // Source publishes counts/name then clears the actual param destination
    // before the corrupt-file refusal; later info output remains untouched.
    assert_eq!(u32_at(&guest.get(ARG + 128, 56), 0), 2);
    assert_eq!(u32_at(&guest.get(ARG + 128, 56), 20), 1);
    assert_eq!(guest.get(ARG + 1024, 5), b"Beta\0");
    assert_eq!(guest.get(ARG + 4096, PARAM_BYTES), [0; PARAM_BYTES]);
    assert_eq!(guest.get(ARG + 6144, 48), [0x5a; 48]);
    guest.put(0, &(-1_i32).to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveDirNameSearch,
            &[ARG, ARG + 128]
        ),
        Ok(INVALID_USER)
    );
}

#[test]
fn save_directory_icons_persist_exact_payload_and_refuse_short_loads_without_descriptor_damage() {
    let root = Root::new();
    let (mut owner, mut guest, failure) = setup(&root, vec![]);
    mount(&mut owner, &mut guest, b"Data");
    let original = b"original-icon-bytes";
    guest.put(4096, original);
    icon(&mut guest, ARG + 4096, 64, original.len() as u64);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveSaveIcon,
            &[ARG + 512, ARG + 1024]
        ),
        Ok(0)
    );
    guest.put(4096, b"replacement");
    icon(&mut guest, ARG + 4096, 64, 11);
    failure.store(true, Ordering::Relaxed);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveSaveIcon,
            &[ARG + 512, ARG + 1024]
        ),
        Ok(INTERNAL)
    );
    failure.store(false, Ordering::Relaxed);
    guest.put(8192, &[0x5a; 64]);
    icon(&mut guest, ARG + 8192, 2, 0x1234);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveLoadIcon,
            &[ARG + 512, ARG + 1024]
        ),
        Ok(PARAMETER)
    );
    assert_eq!(guest.get(ARG + 8192, 64), [0x5a; 64]);
    assert_eq!(guest.u64(1040), 0x1234);
    icon(&mut guest, ARG + 8192, 64, 0x1234);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveLoadIcon,
            &[ARG + 512, ARG + 1024]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(1040), original.len() as u64);
    assert_eq!(guest.get(ARG + 8192, original.len()), original);
    assert_eq!(guest.get(ARG + 1048, 32), [0xa5; 32]);
}

#[test]
fn save_directory_icon_path_reads_only_the_selected_guest_capability_and_survives_remount() {
    let root = Root::new();
    let input = root.0.join("original-icon.bin");
    std::fs::write(&input, b"capability-selected-icon").unwrap();
    let file = ReadOnlyFile::new("icon.bin", std::fs::File::open(&input).unwrap()).unwrap();
    let files = vec![ReadOnlyRoot::new("/app0", vec![file]).unwrap()];
    let (mut owner, mut guest, _) = setup(&root, files);
    mount(&mut owner, &mut guest, b"Data");
    guest.put(4096, b"/app0/icon.bin\0");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveSaveIconByPath,
            &[ARG + 512, ARG + 4096]
        ),
        Ok(0)
    );
    guest.put(4096, b"/app0/../icon.bin\0");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveSaveIconByPath,
            &[ARG + 512, ARG + 4096]
        ),
        Ok(PARAMETER)
    );
    guest.put(4096, b"/unselected/icon.bin\0");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveSaveIconByPath,
            &[ARG + 512, ARG + 4096]
        ),
        Ok(NOT_FOUND)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveUmount2,
            &[0, ARG + 512]
        ),
        Ok(0)
    );
    drop(owner);
    let (mut owner, mut guest, _) = setup(&root, vec![]);
    // Existing directory open, rather than CREATE, proves persistent bytes.
    guest.put(0, &[0; 80]);
    guest.put(0, &1_i32.to_le_bytes());
    guest.put(8, &(ARG + 128).to_le_bytes());
    guest.put(32, &1_u32.to_le_bytes());
    guest.put(128, b"Data\0");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveMount3,
            &[ARG, ARG + 256]
        ),
        Ok(0)
    );
    let point = guest.get(ARG + 256, 16);
    guest.put(512, &point);
    icon(&mut guest, ARG + 8192, 64, 0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveLoadIcon,
            &[ARG + 512, ARG + 1024]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 8192, 24), b"capability-selected-icon");
}

#[test]
fn save_directory_wildcard_matching_is_bounded_and_uses_source_percent_underscore_case_rules() {
    let mut budget = Budget::new(Limits::default()).unwrap();
    for (candidate, pattern, expected) in [
        (b"Alpha".as_slice(), b"a_%".as_slice(), true),
        (b"ab", b"a_", true),
        (b"a", b"a_", false),
        (b"Alpha", b"A*", false),
        (b"Alpha", b"%PHA", true),
        (b"", b"%", false),
        (b"abc", b"%_%_%_%", false),
        (b"ab", b"ab%", false),
        (b"abc", b"ab%", true),
        (b"a", b"_%", false),
        (b"abc", b"%", true),
    ] {
        assert_eq!(matches(candidate, pattern, &mut budget).unwrap(), expected);
    }
    let limits = Limits {
        max_work: 1,
        ..Default::default()
    };
    let mut budget = Budget::new(limits).unwrap();
    assert!(matches!(
        matches(b"abc", b"%%%", &mut budget),
        Err(Error::Policy(Failure::Limit(_)))
    ));
}

#[test]
fn save_directory_metadata_low32_types_and_invalid_arguments_precede_mount_and_user_errors() {
    let root = Root::new();
    let (mut owner, mut guest, _) = setup(&root, vec![]);
    mount(&mut owner, &mut guest, b"Data");
    let point = guest.get(ARG + 512, 16);
    for (service, args) in [
        (
            Service::SaveGetParam,
            [u64::MAX, 0xaabb_ccdd_0000_0006, ARG + 2048, 4, 0],
        ),
        (
            Service::SaveGetParam,
            [u64::MAX, 0, 0, PARAM_BYTES as u64, 0],
        ),
        (Service::SaveSetParam, [u64::MAX, 5, ARG + 4096, 4, 0]),
        (
            Service::SaveSetParam,
            [u64::MAX, 0, 0, PARAM_BYTES as u64, 0],
        ),
        (Service::SaveLoadIcon, [u64::MAX, 0, 0, 0, 0]),
    ] {
        assert_eq!(call(&mut owner, &mut guest, service, &args), Ok(PARAMETER));
    }
    for (kind, capacity) in [(0, (PARAM_BYTES - 1) as u64), (4, 3), (1, 0)] {
        assert!(matches!(
            call(
                &mut owner,
                &mut guest,
                Service::SaveSetParam,
                &[u64::MAX, kind, ARG + 4096, capacity]
            ),
            Err(Failure::Guest(_))
        ));
    }
    guest.put(512, &[0; 16]);
    guest.put(512, b"/unmounted\0");
    assert_eq!(get(&mut owner, &mut guest, 6, 8), Ok(PARAMETER));
    assert_eq!(set(&mut owner, &mut guest, 4, &[1, 2, 3]), Ok(PARAMETER));
    guest.put(512, &point);
    guest.put(0, &[0; 64]);
    guest.put(0, &(-1_i32).to_le_bytes());
    guest.put(24, &6_u32.to_le_bytes());
    guest.put(128, &[0; 56]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveDirNameSearch,
            &[ARG, ARG + 128]
        ),
        Ok(PARAMETER)
    );
    guest.put(24, &0_u32.to_le_bytes());
    guest.put(16, &(ARG + 96).to_le_bytes());
    guest.put(96, &[b'a'; 32]); // Unterminated pattern also precedes INVALID_USER.
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SaveDirNameSearch,
            &[ARG, ARG + 128]
        ),
        Ok(PARAMETER)
    );
}
