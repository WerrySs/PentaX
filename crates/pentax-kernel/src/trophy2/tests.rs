// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX package-backed fixtures. No guest code or external assets.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest};
use crate::{GuestError, Limits, ReadOnlyFile, ReadOnlyRoot};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const TICK: u64 = 62_135_596_800_000_055;

fn original_package() -> Vec<u8> {
    let entries: [(&str, &[u8]); 2] = [
        ("tropconf.json", br#"{"defaultLanguage":"en-US","trophies":[{"id":2,"grade":"B"},{"id":7,"grade":"G","groupId":2,"hidden":true,"hasReward":true,"unlockCondition":{"progressive":true,"targetValue":"100"}},{"id":9,"grade":"P","groupId":2}]}"#),
        ("tropmeta_en-US.json", br#"{"metadata":{"titleMetadata":{"name":"Original Fixture"},"groupMetadata":[{"id":2,"name":"Original Group"}],"trophyMetadata":[{"id":2,"name":"Bronze"},{"id":7,"name":"Gold\u0000suffix","detail":"Original description","reward":"Badge"},{"id":9,"name":"Platinum"}]}}"#),
    ];
    let payload = 0x60 + entries.len() * 0x40;
    let length = payload + entries.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
    let mut output = vec![0; length];
    output[..4].copy_from_slice(&0xb228_c60a_u32.to_be_bytes());
    output[4..8].copy_from_slice(&1_u32.to_be_bytes());
    output[8..16].copy_from_slice(&(length as u64).to_be_bytes());
    output[16..20].copy_from_slice(&(entries.len() as u32).to_be_bytes());
    output[20..24].copy_from_slice(&0x40_u32.to_be_bytes());
    let mut cursor = payload;
    for (index, (name, bytes)) in entries.iter().enumerate() {
        let entry = 0x60 + index * 0x40;
        output[entry..entry + name.len()].copy_from_slice(name.as_bytes());
        output[entry + 32..entry + 40].copy_from_slice(&(cursor as u64).to_be_bytes());
        output[entry + 40..entry + 48].copy_from_slice(&(bytes.len() as u64).to_be_bytes());
        output[cursor..cursor + bytes.len()].copy_from_slice(bytes);
        cursor += bytes.len();
    }
    output
}

struct Fixture {
    path: PathBuf,
    files: Option<FileState>,
    storage: Option<SaveStorage>,
}

impl Fixture {
    fn new(budget: &mut Budget, package: Option<&[u8]>, saved: bool) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pentax-trophy-lifecycle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let mut files = Vec::new();
        if let Some(bytes) = package {
            std::fs::write(path.join("package.ucp"), bytes).unwrap();
            files.push(
                ReadOnlyFile::new(
                    "sce_sys/trophy2/trophy07.ucp",
                    std::fs::File::open(path.join("package.ucp")).unwrap(),
                )
                .unwrap(),
            );
        }
        if saved {
            std::fs::write(
                path.join("trophies-u42-s7.json"),
                br#"{"unlockedTrophies":[7],"unlockedAt":{"7":62135596800000055}}"#,
            )
            .unwrap();
        }
        let root = ReadOnlyRoot::new("/app0", files).unwrap();
        let files = FileState::new(budget.limits, vec![root], budget).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let storage = SaveStorage::directory(directory.into_std_file()).unwrap();
        Self {
            path,
            files: Some(files),
            storage: Some(storage),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.files.take();
        self.storage.take();
        for name in ["package.ucp", "trophies-u42-s7.json", ".pentax-save.lock"] {
            match std::fs::remove_file(self.path.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("original trophy fixture cleanup: {error}"),
            }
        }
        std::fs::remove_dir(&self.path).unwrap();
    }
}

fn policy() -> Limits {
    Limits {
        max_scratch_bytes: MAX_METADATA,
        ..Default::default()
    }
}

fn call(
    state: &mut TrophyState,
    fixture: &Fixture,
    budget: &mut Budget,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
    stack: Option<u64>,
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    state.invoke(
        service,
        CallArgs(args),
        stack,
        fixture.files.as_ref().unwrap(),
        fixture.storage.as_ref(),
        1,
        0,
        guest,
        budget,
    )
}

fn registered(
    state: &mut TrophyState,
    fixture: &Fixture,
    budget: &mut Budget,
    guest: &mut TestGuest,
) -> u64 {
    assert_eq!(
        call(
            state,
            fixture,
            budget,
            guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 7, 0],
            None
        ),
        Ok(0)
    );
    let id = u32::from_le_bytes(guest.get(ARG, 4).try_into().unwrap()) as u64;
    assert_eq!(
        call(
            state,
            fixture,
            budget,
            guest,
            Service::Trophy2RegisterContext,
            &[id, u64::MAX, 0],
            None
        ),
        Ok(0)
    );
    id
}

fn u32_at(guest: &TestGuest, address: u64) -> u32 {
    u32::from_le_bytes(guest.get(address, 4).try_into().unwrap())
}
fn u64_at(guest: &TestGuest, address: u64) -> u64 {
    u64::from_le_bytes(guest.get(address, 8).try_into().unwrap())
}

#[test]
fn contexts_preserve_output_failure_identity_duplicate_precedence_and_monotonic_recreation() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget, None, false);
    let mut state = TrophyState::default();
    let mut guest = TestGuest::new();
    guest.put(0, &[0x72; 4]);
    guest.refuse = true;
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 0, 0],
            None
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(
        (
            state.next_context,
            state.retained_handle_count(),
            budget.usage().handles_live
        ),
        (1, 0, 0)
    );
    assert_eq!(guest.get(ARG, 4), [0x72; 4]);
    guest.refuse = false;
    for label in 0..8 {
        assert_eq!(
            call(
                &mut state,
                &fixture,
                &mut budget,
                &mut guest,
                Service::Trophy2CreateContext,
                &[ARG, 42, label, 0],
                None
            ),
            Ok(0)
        );
        assert_eq!(u32_at(&guest, ARG), label as u32 + 1);
    }
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 0, 0],
            None
        ),
        Ok(DUPLICATE_CONTEXT)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 8, 0],
            None
        ),
        Ok(MAX_CONTEXTS_ERROR)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, u64::MAX, 9, 0],
            None
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2DestroyContext,
            &[1],
            None
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2DestroyContext,
            &[1],
            None
        ),
        Ok(INVALID_CONTEXT)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 0, 0],
            None
        ),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, ARG), 9);
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn real_package_and_persistent_unlocks_fill_exact_optional_game_group_and_trophy_records() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget, Some(&original_package()), true);
    let mut state = TrophyState::default();
    let mut guest = TestGuest::new();
    let id = registered(&mut state, &fixture, &mut budget, &mut guest);
    assert_eq!(state.retained_handle_count(), 3);
    assert_eq!(state.retained_record_counts(), (1, 1, 1, 1));
    assert_eq!(budget.usage().handles_live, 3);
    assert!(budget.usage().save_bytes_read > 0);
    guest.put(512, &[0x8e; 200]);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGameInfo,
            &[id, 99, ARG + 512, ARG + 672],
            None
        ),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG + 512, 24),
        [2_u32, 3, 1, 1, 0, 1]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>()
    );
    assert_eq!(guest.get(ARG + 536, 17), b"Original Fixture\0");
    assert_eq!(
        guest.get(ARG + 672, 24),
        [1_u32, 0, 1, 0, 0, 85]
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>()
    );
    assert_eq!(guest.get(ARG + 664, 8), [0x8e; 8]);
    guest.put(1024, &[0xa1; 152]);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGroupInfo,
            &[id, 0, 2, ARG + 1024, ARG + 1200],
            None
        ),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, ARG + 1028), 2);
    assert_eq!(u32_at(&guest, ARG + 1224), 100);
    assert_eq!(guest.get(ARG + 1228, 4), [0; 4]);
    guest.put(2048, &[0xff; 1312]);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetTrophyInfo,
            &[id, 0, 7, ARG + 2048, ARG + 4096],
            None
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 2060, 4), [1, 1, 0, 0]);
    assert_eq!(u32_at(&guest, ARG + 2064), 1);
    assert_eq!(u64_at(&guest, ARG + 2072), 100);
    assert_eq!(guest.get(ARG + 2080, 6), b"Gold\0\0");
    assert_eq!(guest.get(ARG + 4100, 4), [1, 0, 0, 0]);
    assert_eq!(u32_at(&guest, ARG + 4104), 1);
    assert_eq!(u64_at(&guest, ARG + 4112), 100);
    assert_eq!(u64_at(&guest, ARG + 4120), TICK);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetTrophyInfo,
            &[id, 0, 99, ARG + 2048, 0],
            None
        ),
        Ok(INVALID_TROPHY)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGroupInfo,
            &[id, 0, 99, 0, ARG + 1200],
            None
        ),
        Ok(INVALID_GROUP)
    );
    assert_eq!(
        std::fs::read(fixture.path.join("trophies-u42-s7.json")).unwrap(),
        br#"{"unlockedTrophies":[7],"unlockedAt":{"7":62135596800000055}}"#
    );
    assert_eq!(budget.usage().save_bytes_written, 0);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2DestroyContext,
            &[id],
            None
        ),
        Ok(0)
    );
    // Cached package/user state survives context removal and is counted once.
    assert_eq!(state.retained_record_counts(), (0, 1, 1, 0));
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn array_seventh_stack_argument_ordinal_selection_and_late_count_refusal_are_atomic() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget, Some(&original_package()), true);
    let mut state = TrophyState::default();
    let mut guest = TestGuest::new();
    let id = registered(&mut state, &fixture, &mut budget, &mut guest);
    let stack = ARG + 128;
    guest.put(136, &(ARG + PAGE).to_le_bytes());
    guest.put(4096, &[0x6c; 1312]);
    guest.put(8192, &[0x7d; 32]);
    let values = [id, 0, 1, 1, ARG + 4096, ARG + 8192];
    assert!(matches!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetTrophyInfoArray,
            &values,
            None
        ),
        Err(Failure::Unsupported(_))
    ));
    let before = budget.usage();
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetTrophyInfoArray,
            &values,
            Some(stack)
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + 4096, 1312), [0x6c; 1312]);
    assert_eq!(guest.get(ARG + 8192, 32), [0x7d; 32]);
    assert_eq!(
        budget.usage().bytes_written,
        before.bytes_written + 1312 + 32 + 4
    );
    assert_eq!(budget.usage().bytes_read, before.bytes_read + 8);
    guest.put(136, &(ARG + 256).to_le_bytes());
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetTrophyInfoArray,
            &values,
            Some(stack)
        ),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, ARG + 256), 1);
    assert_eq!(u32_at(&guest, ARG + 4096), 7); // Offset1 is the second ID, not trophy1.
    assert_eq!(u32_at(&guest, ARG + 8192), 7);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGroupInfoArray,
            &[id, 0, 0, 2, ARG + 1024, 0],
            Some(stack)
        ),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, ARG + 256), 2);
    assert_eq!(u32_at(&guest, ARG + 1024), u32::MAX);
    assert_eq!(u32_at(&guest, ARG + 1176), 2);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetTrophyInfoArray,
            &[id, 0, u32::MAX as u64, 0, u64::MAX, 0],
            Some(stack)
        ),
        Ok(0)
    );
    assert_eq!(u32_at(&guest, ARG + 256), 0);
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn actual_missing_invalid_and_unavailable_inputs_do_not_fake_registered_state_and_retry_works() {
    let mut budget = Budget::new(policy()).unwrap();
    let missing = Fixture::new(&mut budget, None, false);
    let present = Fixture::new(&mut budget, Some(b"original invalid package"), false);
    let mut state = TrophyState::default();
    let mut guest = TestGuest::new();
    assert_eq!(
        call(
            &mut state,
            &missing,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 7, 0],
            None
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &missing,
            &mut budget,
            &mut guest,
            Service::Trophy2RegisterContext,
            &[1, 0, 0],
            None
        ),
        Ok(PACKAGE_MISSING)
    );
    assert_eq!(
        call(
            &mut state,
            &present,
            &mut budget,
            &mut guest,
            Service::Trophy2RegisterContext,
            &[1, 0, 0],
            None
        ),
        Ok(PACKAGE_MISSING)
    );
    assert_eq!(
        call(
            &mut state,
            &present,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGameInfo,
            &[1, 0, ARG + 512, 0],
            None
        ),
        Ok(NOT_REGISTERED)
    );
    std::fs::write(present.path.join("package.ucp"), original_package()).unwrap();
    assert!(matches!(
        state.invoke(
            Service::Trophy2RegisterContext,
            CallArgs([1, 0, 0, 0, 0, 0]),
            None,
            present.files.as_ref().unwrap(),
            None,
            1,
            0,
            &mut guest,
            &mut budget
        ),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        (state.retained_handle_count(), budget.usage().handles_live),
        (1, 1)
    );
    let unavailable = FileState::new(budget.limits, Vec::new(), &mut budget).unwrap();
    assert!(matches!(
        state.invoke(
            Service::Trophy2RegisterContext,
            CallArgs([1, 0, 0, 0, 0, 0]),
            None,
            &unavailable,
            present.storage.as_ref(),
            1,
            0,
            &mut guest,
            &mut budget
        ),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        call(
            &mut state,
            &present,
            &mut budget,
            &mut guest,
            Service::Trophy2RegisterContext,
            &[1, 0, 0],
            None
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut state,
            &present,
            &mut budget,
            &mut guest,
            Service::Trophy2RegisterContext,
            &[1, 0, 1],
            None
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        call(
            &mut state,
            &present,
            &mut budget,
            &mut guest,
            Service::Trophy2RegisterContext,
            &[1, 0, 0],
            None
        ),
        Ok(ALREADY_REGISTERED)
    );
}

#[test]
fn package_and_user_caches_consume_shared_quota_and_survive_context_destruction() {
    let mut budget = Budget::new(Limits {
        max_handles: 3,
        ..policy()
    })
    .unwrap();
    let fixture = Fixture::new(&mut budget, Some(&original_package()), true);
    let mut state = TrophyState::default();
    let mut guest = TestGuest::new();
    let id = registered(&mut state, &fixture, &mut budget, &mut guest);
    assert_eq!(budget.usage().handles_live, 3);
    assert!(matches!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG + 4, 43, 7, 0],
            None
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2DestroyContext,
            &[id],
            None
        ),
        Ok(0)
    );
    assert_eq!(budget.usage().handles_live, 2);
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateContext,
            &[ARG, 42, 7, 0],
            None
        ),
        Ok(0)
    );
    let next = u32_at(&guest, ARG) as u64;
    assert!(next > id);
    let saved_reads = budget.usage().save_bytes_read;
    let no_files = FileState::new(budget.limits, Vec::new(), &mut budget).unwrap();
    assert_eq!(
        state.invoke(
            Service::Trophy2RegisterContext,
            CallArgs([next, 0, 0, 0, 0, 0]),
            None,
            &no_files,
            None,
            1,
            0,
            &mut guest,
            &mut budget
        ),
        Ok(0)
    );
    assert_eq!(budget.usage().save_bytes_read, saved_reads);
    assert_eq!(state.retained_handle_count(), 3);
    assert_eq!(budget.usage().handles_live, 3);
}

#[test]
fn source_noop_handles_callbacks_icons_and_query_error_order_do_not_create_resources() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget, None, false);
    let mut state = TrophyState::default();
    let mut guest = TestGuest::new();
    for service in [
        Service::Trophy2RegisterUnlockCallback,
        Service::Trophy2AbortHandle,
        Service::Trophy2DestroyHandle,
    ] {
        assert_eq!(
            call(
                &mut state,
                &fixture,
                &mut budget,
                &mut guest,
                service,
                &[u64::MAX, u64::MAX],
                None
            ),
            Ok(0)
        );
    }
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2CreateHandle,
            &[0],
            None
        ),
        Ok(INVALID_ARGUMENT)
    );
    for _ in 0..2 {
        assert_eq!(
            call(
                &mut state,
                &fixture,
                &mut budget,
                &mut guest,
                Service::Trophy2CreateHandle,
                &[ARG],
                None
            ),
            Ok(0)
        );
        assert_eq!(u32_at(&guest, ARG), 1);
    }
    guest.put(32, &[0xa7; 24]);
    for service in [
        Service::Trophy2GetGameIcon,
        Service::Trophy2GetGroupIcon,
        Service::Trophy2GetTrophyIcon,
    ] {
        let values = if service == Service::Trophy2GetGameIcon {
            [u64::MAX, 0, ARG + 32, ARG + 40, 0]
        } else {
            [u64::MAX, 0, u64::MAX, ARG + 32, ARG + 40]
        };
        assert_eq!(
            call(
                &mut state,
                &fixture,
                &mut budget,
                &mut guest,
                service,
                &values,
                None
            ),
            Ok(ICON_MISSING)
        );
        assert_eq!(guest.get(ARG + 32, 8), [0xa7; 8]);
        assert_eq!(guest.get(ARG + 40, 8), [0; 8]);
        assert_eq!(guest.get(ARG + 48, 8), [0xa7; 8]);
    }
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGameInfo,
            &[99, 0, 0, 0],
            None
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2GetGameInfo,
            &[99, 0, ARG + 512, 0],
            None
        ),
        Ok(INVALID_CONTEXT)
    );
    assert_eq!(
        call(
            &mut state,
            &fixture,
            &mut budget,
            &mut guest,
            Service::Trophy2RegisterContext,
            &[99, 0, 1],
            None
        ),
        Ok(INVALID_CONTEXT)
    );
    assert_eq!(
        (state.retained_handle_count(), budget.usage().handles_live),
        (0, 0)
    );
}
