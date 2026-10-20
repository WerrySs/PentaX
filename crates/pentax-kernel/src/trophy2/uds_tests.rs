// SPDX-License-Identifier: GPL-2.0-only
// Original package/rule/save fixtures; no guest code or external assets.

use super::*;
use crate::tests::{ARG, TestGuest};
use crate::{Limits, ReadOnlyFile, ReadOnlyRoot};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn ucp(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let start = 0x60 + entries.len() * 0x40;
    let length = start + entries.iter().map(|(_, bytes)| bytes.len()).sum::<usize>();
    let mut bytes = vec![0; length];
    bytes[..4].copy_from_slice(&0xb228_c60a_u32.to_be_bytes());
    bytes[4..8].copy_from_slice(&1_u32.to_be_bytes());
    bytes[8..16].copy_from_slice(&(length as u64).to_be_bytes());
    bytes[16..20].copy_from_slice(&(entries.len() as u32).to_be_bytes());
    bytes[20..24].copy_from_slice(&0x40_u32.to_be_bytes());
    let mut cursor = start;
    for (index, (name, data)) in entries.iter().enumerate() {
        let entry = 0x60 + index * 0x40;
        bytes[entry..entry + name.len()].copy_from_slice(name.as_bytes());
        bytes[entry + 32..entry + 40].copy_from_slice(&(cursor as u64).to_be_bytes());
        bytes[entry + 40..entry + 48].copy_from_slice(&(data.len() as u64).to_be_bytes());
        bytes[cursor..cursor + data.len()].copy_from_slice(data);
        cursor += data.len();
    }
    bytes
}

struct Fixture {
    path: PathBuf,
    files: Option<FileState>,
    storage: Option<SaveStorage>,
}

impl Fixture {
    fn new(budget: &mut Budget) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pentax-uds-unlocks-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, AtomicOrdering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let package = ucp(&[
            ("tropconf.json", br#"{"defaultLanguage":"en-US","trophies":[{"id":2,"grade":"G","platinumTrophyId":9,"unlockCondition":{"udsStatId":"6","targetValue":"1","comparator":"gt"}},{"id":4,"grade":"B","platinumTrophyId":9,"unlockCondition":{"progressive":true,"udsStatId":"5","targetValue":"10","comparator":"ge"}},{"id":9,"grade":"P"}]}"#),
            ("tropmeta_en-US.json", br#"{"metadata":{"titleMetadata":{"name":"Original UDS Fixture"},"trophyMetadata":[{"id":2,"name":"Gold"},{"id":4,"name":"Progress"},{"id":9,"name":"Platinum"}]}}"#),
        ]);
        let rules = ucp(&[
            ("stats_definition.json", br#"{"statDefinitionArray":[{"statId":"5","aggregation":"latest","dataType":"uint32","minValue":"0","maxValue":"4294967295"},{"statId":"6","aggregation":"latest","dataType":"uint64","minValue":"0","maxValue":"18446744073709551615"}]}"#),
            ("stats_extraction.json", br#"{"statsExtractionRuleArray":[{"condition":{"eventName":"Score"},"action":{"input":"$.score","output":{"statId":"5"}}},{"condition":{"eventName":"Score64"},"action":{"input":"$.score","output":{"statId":"6"}}}]}"#),
        ]);
        std::fs::write(path.join("package.ucp"), package).unwrap();
        std::fs::write(path.join("rules.ucp"), rules).unwrap();
        let root = ReadOnlyRoot::new(
            "/app0",
            vec![
                ReadOnlyFile::new(
                    "sce_sys/trophy2/trophy00.ucp",
                    std::fs::File::open(path.join("package.ucp")).unwrap(),
                )
                .unwrap(),
                ReadOnlyFile::new(
                    "sce_sys/uds/uds00.ucp",
                    std::fs::File::open(path.join("rules.ucp")).unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap();
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

    fn saved(&self) -> PathBuf {
        self.path.join("trophies-u7-s0.json")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.files.take();
        self.storage.take();
        for name in [
            "package.ucp",
            "rules.ucp",
            "trophies-u7-s0.json",
            ".pentax-save.lock",
        ] {
            match std::fs::remove_file(self.path.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("original UDS fixture cleanup: {error}"),
            }
        }
        std::fs::remove_dir(&self.path).unwrap();
    }
}

fn policy() -> crate::Limits {
    Limits {
        max_scratch_bytes: MAX_METADATA,
        ..Default::default()
    }
}

fn properties(key: &[u8], value: UdsInteger) -> Vec<UdsProperty> {
    vec![UdsProperty {
        key: key.to_vec(),
        value,
    }]
}

fn held(properties: &Vec<UdsProperty>) -> usize {
    properties.capacity() * core::mem::size_of::<UdsProperty>()
        + properties
            .iter()
            .map(|property| property.key.capacity())
            .sum::<usize>()
}

fn post(
    state: &mut TrophyState,
    fixture: &Fixture,
    event: &[u8],
    properties: &Vec<UdsProperty>,
    budget: &mut Budget,
) -> Result<UdsPost, Failure> {
    state.post_uds(
        7,
        0,
        event,
        properties,
        fixture.files.as_ref().unwrap(),
        fixture.storage.as_ref(),
        1,
        held(properties),
        budget,
    )
}

fn tick(state: &TrophyState, id: i32) -> u64 {
    let unlocks = &state.users[0].unlocks;
    unlocks.timestamps[unlocks
        .timestamps
        .binary_search_by_key(&id, |entry| entry.0)
        .unwrap()]
    .1
}

#[test]
fn direct_i32_validation_unknown_progressive_and_platinum_preserve_source_no_change_paths() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget);
    let mut state = TrophyState::default();
    for value in [UdsInteger::U32(2), UdsInteger::U64(2), UdsInteger::I32(-1)] {
        assert_eq!(
            post(
                &mut state,
                &fixture,
                b"_UnlockTrophy",
                &properties(b"_trophy_id", value),
                &mut budget
            ),
            Ok(UdsPost::InvalidArgument)
        );
        assert_eq!(state.retained_handle_count(), 0);
    }
    assert_eq!(
        post(&mut state, &fixture, b"ignored", &Vec::new(), &mut budget),
        Ok(UdsPost::Posted)
    );
    assert_eq!(state.retained_handle_count(), 0);
    for id in [99, 4, 9] {
        assert_eq!(
            post(
                &mut state,
                &fixture,
                b"_UnlockTrophy",
                &properties(b"_trophy_id", UdsInteger::I32(id)),
                &mut budget
            ),
            Ok(UdsPost::Posted)
        );
        assert_eq!(state.retained_handle_count(), 2);
        assert!(state.users[0].unlocks.unlocked.is_empty());
        assert!(!fixture.saved().exists());
    }
    let direct = properties(b"_trophy_id", UdsInteger::I32(2));
    assert_eq!(
        post(&mut state, &fixture, b"_UnlockTrophy", &direct, &mut budget),
        Ok(UdsPost::Posted)
    );
    assert_eq!(state.users[0].unlocks.unlocked, [2]);
    assert!(tick(&state, 2) > UNIX_EPOCH_TICK);
    let before = std::fs::read(fixture.saved()).unwrap();
    let bytes_written = budget.usage().save_bytes_written;
    let original_tick = tick(&state, 2);
    assert_eq!(
        post(&mut state, &fixture, b"_UnlockTrophy", &direct, &mut budget),
        Ok(UdsPost::Posted)
    );
    assert_eq!(std::fs::read(fixture.saved()).unwrap(), before);
    assert_eq!(budget.usage().save_bytes_written, bytes_written);
    assert_eq!(tick(&state, 2), original_tick);
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn unsigned_latest_rules_unlock_actual_progress_and_associated_platinum_visible_to_trophy_queries()
{
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget);
    let mut state = TrophyState::default();
    for value in [UdsInteger::I32(10), UdsInteger::U64(10), UdsInteger::U32(9)] {
        assert_eq!(
            post(
                &mut state,
                &fixture,
                b"Score",
                &properties(b"score", value),
                &mut budget
            ),
            Ok(UdsPost::Posted)
        );
        assert!(state.users[0].unlocks.unlocked.is_empty());
    }
    assert_eq!(
        post(
            &mut state,
            &fixture,
            b"Score",
            &properties(b"score", UdsInteger::U32(10)),
            &mut budget
        ),
        Ok(UdsPost::Posted)
    );
    assert_eq!(state.users[0].unlocks.unlocked, [4]);
    let progressive_tick = tick(&state, 4);
    for value in [UdsInteger::U32(2), UdsInteger::U64(1)] {
        assert_eq!(
            post(
                &mut state,
                &fixture,
                b"Score64",
                &properties(b"score", value),
                &mut budget
            ),
            Ok(UdsPost::Posted)
        );
        assert_eq!(state.users[0].unlocks.unlocked, [4]);
    }
    assert_eq!(
        post(
            &mut state,
            &fixture,
            b"Score64",
            &properties(b"score", UdsInteger::U64(2)),
            &mut budget
        ),
        Ok(UdsPost::Posted)
    );
    assert_eq!(state.users[0].unlocks.unlocked, [2, 4, 9]);
    assert_eq!(tick(&state, 2), tick(&state, 9));
    assert_eq!(tick(&state, 4), progressive_tick);
    let saved = std::fs::read(fixture.saved()).unwrap();
    let loaded = package::parse_unlocks(&saved, saved.capacity(), &mut budget).unwrap();
    assert_eq!(loaded.unlocked, [2, 4, 9]);
    assert_eq!(loaded.timestamps, state.users[0].unlocks.timestamps);

    let files = fixture.files.as_ref().unwrap();
    let storage = fixture.storage.as_ref();
    let mut guest = TestGuest::new();
    assert_eq!(
        state.invoke(
            Service::Trophy2CreateContext,
            CallArgs([ARG, 7, 0, 0, 0, 0]),
            None,
            files,
            storage,
            1,
            0,
            &mut guest,
            &mut budget
        ),
        Ok(0)
    );
    assert_eq!(
        state.invoke(
            Service::Trophy2RegisterContext,
            CallArgs([1, 0, 0, 0, 0, 0]),
            None,
            files,
            storage,
            1,
            0,
            &mut guest,
            &mut budget
        ),
        Ok(0)
    );
    assert_eq!(
        state.invoke(
            Service::Trophy2GetTrophyInfo,
            CallArgs([1, 0, 4, 0, ARG + 256, 0]),
            None,
            files,
            storage,
            1,
            0,
            &mut guest,
            &mut budget
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 260, 4), [1, 0, 0, 0]);
    assert_eq!(
        u64::from_le_bytes(guest.get(ARG + 272, 8).try_into().unwrap()),
        10
    );
    assert_eq!(
        u64::from_le_bytes(guest.get(ARG + 280, 8).try_into().unwrap()),
        progressive_tick
    );
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn persistence_refusal_rolls_back_new_caches_and_existing_unlocks_then_retry_commits() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget);
    let failure = fixture.storage.as_ref().unwrap().failure_control();
    let mut state = TrophyState::default();
    let direct = properties(b"_trophy_id", UdsInteger::I32(2));
    failure.store(true, AtomicOrdering::Relaxed);
    assert!(matches!(
        post(&mut state, &fixture, b"_UnlockTrophy", &direct, &mut budget),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(state.retained_handle_count(), 0);
    assert_eq!(budget.usage().handles_live, 0);
    assert!(!fixture.saved().exists());
    assert_eq!(state.next_context, 1);
    assert!(budget.usage().save_bytes_written > 0);
    failure.store(false, AtomicOrdering::Relaxed);
    assert_eq!(
        post(&mut state, &fixture, b"_UnlockTrophy", &direct, &mut budget),
        Ok(UdsPost::Posted)
    );
    let original_tick = tick(&state, 2);
    let original_file = std::fs::read(fixture.saved()).unwrap();
    let score = properties(b"score", UdsInteger::U32(10));
    failure.store(true, AtomicOrdering::Relaxed);
    assert!(matches!(
        post(&mut state, &fixture, b"Score", &score, &mut budget),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(state.users[0].unlocks.unlocked, [2]);
    assert_eq!(tick(&state, 2), original_tick);
    assert_eq!(std::fs::read(fixture.saved()).unwrap(), original_file);
    assert_eq!(state.retained_handle_count(), 2);
    failure.store(false, AtomicOrdering::Relaxed);
    assert_eq!(
        post(&mut state, &fixture, b"Score", &score, &mut budget),
        Ok(UdsPost::Posted)
    );
    assert_eq!(state.users[0].unlocks.unlocked, [2, 4, 9]);
    assert_eq!(tick(&state, 2), original_tick);
    assert_eq!(state.retained_handle_count(), budget.usage().handles_live);
}

#[test]
fn real_missing_package_is_ignored_but_capabilities_quotas_and_combined_staging_remain_typed() {
    let mut budget = Budget::new(policy()).unwrap();
    let fixture = Fixture::new(&mut budget);
    let mut state = TrophyState::default();
    let direct = properties(b"_trophy_id", UdsInteger::I32(2));
    let absent_mount = FileState::new(budget.limits, Vec::new(), &mut budget).unwrap();
    assert!(matches!(
        state.post_uds(
            7,
            0,
            b"_UnlockTrophy",
            &direct,
            &absent_mount,
            fixture.storage.as_ref(),
            1,
            held(&direct),
            &mut budget
        ),
        Err(Failure::Unsupported(_))
    ));
    let missing_file = FileState::new(
        budget.limits,
        vec![ReadOnlyRoot::new("/app0", Vec::new()).unwrap()],
        &mut budget,
    )
    .unwrap();
    assert_eq!(
        state.post_uds(
            7,
            0,
            b"_UnlockTrophy",
            &direct,
            &missing_file,
            fixture.storage.as_ref(),
            1,
            held(&direct),
            &mut budget
        ),
        Ok(UdsPost::Posted)
    );
    assert!(matches!(
        state.post_uds(
            7,
            0,
            b"_UnlockTrophy",
            &direct,
            fixture.files.as_ref().unwrap(),
            None,
            1,
            held(&direct),
            &mut budget
        ),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        state.post_uds(
            7,
            0,
            b"_UnlockTrophy",
            &direct,
            fixture.files.as_ref().unwrap(),
            fixture.storage.as_ref(),
            1,
            MAX_METADATA,
            &mut budget
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(
        (state.retained_handle_count(), budget.usage().handles_live),
        (0, 0)
    );
    assert!(!fixture.saved().exists());

    let mut tight = Budget::new(Limits {
        max_handles: 2,
        ..policy()
    })
    .unwrap();
    let mut guest = TestGuest::new();
    assert_eq!(
        state.invoke(
            Service::Trophy2CreateContext,
            CallArgs([ARG, 7, 0, 0, 0, 0]),
            None,
            fixture.files.as_ref().unwrap(),
            fixture.storage.as_ref(),
            1,
            0,
            &mut guest,
            &mut tight
        ),
        Ok(0)
    );
    assert!(matches!(
        post(&mut state, &fixture, b"_UnlockTrophy", &direct, &mut tight),
        Err(Failure::Limit(_))
    ));
    assert_eq!(
        (state.retained_handle_count(), tight.usage().handles_live),
        (1, 1)
    );
    assert!(!fixture.saved().exists());
    let mut fresh = TrophyState::default();
    guest.put(0, &[0xac; 4]);
    assert!(matches!(
        fresh.invoke(
            Service::Trophy2CreateContext,
            CallArgs([ARG, 7, 0, 0, 0, 0]),
            None,
            fixture.files.as_ref().unwrap(),
            fixture.storage.as_ref(),
            1,
            MAX_METADATA,
            &mut guest,
            &mut budget
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG, 4), [0xac; 4]);
    assert_eq!(fresh.retained_handle_count(), 0);
}
