// SPDX-License-Identifier: GPL-2.0-only
// Original package/save/rule bytes; no game files or host filesystem access.

use super::*;
use crate::Limits;

fn budget() -> Budget {
    Budget::new(Limits::default()).unwrap()
}

fn ucp(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let header = 96 + entries.len() * 64;
    let length = header + entries.iter().map(|(_, data)| data.len()).sum::<usize>();
    let mut bytes = vec![0; length];
    bytes[..4].copy_from_slice(&0xb228c60au32.to_be_bytes());
    bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
    bytes[8..16].copy_from_slice(&(length as u64).to_be_bytes());
    bytes[16..20].copy_from_slice(&(entries.len() as u32).to_be_bytes());
    bytes[20..24].copy_from_slice(&64u32.to_be_bytes());
    let mut offset = header;
    for (index, (name, data)) in entries.iter().enumerate() {
        let at = 96 + index * 64;
        bytes[at..at + name.len()].copy_from_slice(name);
        bytes[at + 32..at + 40].copy_from_slice(&(offset as u64).to_be_bytes());
        bytes[at + 40..at + 48].copy_from_slice(&(data.len() as u64).to_be_bytes());
        bytes[offset..offset + data.len()].copy_from_slice(data);
        offset += data.len();
    }
    bytes
}

const CONF: &[u8] = br#"{"defaultLanguage":"en-US","trophies":[{"id":"2","grade":"S","groupId":"3","platinumTrophyId":"9","hidden":true,"hasReward":true,"unlockCondition":{"progressive":true,"udsStatId":"0","targetValue":"100","comparator":"ge"}},{"id":9,"grade":"P"}]}"#;
const META: &[u8] = br#"{"metadata":{"titleMetadata":{"name":"old","name":"Actual title"},"groupMetadata":[{"id":3,"name":"Group three"}],"trophyMetadata":[{"id":"2","name":"\u00e9\ud83d\ude00","detail":"A detail","reward":"A reward"}]}}"#;

#[test]
fn localized_package_uses_null_fallback_real_metadata_icons_progress_and_source_defaults() {
    let bytes = ucp(&[
        (b"tropconf.json", CONF),
        (b"tropmeta_fr-FR.json", b"null"),
        (b"tropmeta_en-US.json", META),
        (b"trop0002.png", b"first icon"),
        (b"trop0002.png", b"second icon"),
    ]);
    let mut budget = budget();
    let package = parse_package(&bytes, 2, bytes.capacity(), &mut budget)
        .unwrap()
        .unwrap();
    assert_eq!(package.title, b"Actual title");
    assert_eq!(
        package
            .groups
            .iter()
            .map(|group| group.id)
            .collect::<Vec<_>>(),
        [-1, 3]
    );
    assert_eq!(package.groups[0].title, b"Actual title");
    assert_eq!(package.groups[1].title, b"Group three");
    assert_eq!(
        package
            .trophies
            .iter()
            .map(|trophy| trophy.id)
            .collect::<Vec<_>>(),
        [2, 9]
    );
    let trophy = &package.trophies[0];
    assert_eq!(
        (
            trophy.group_id,
            trophy.platinum_id,
            trophy.grade,
            trophy.target,
            trophy.uds_stat_id,
            trophy.comparison,
            trophy.progressive,
            trophy.hidden,
            trophy.has_reward
        ),
        (
            3,
            9,
            3,
            Some(100),
            Some(0),
            Comparison::GreaterEqual,
            true,
            true,
            true
        )
    );
    assert_eq!(trophy.name, b"\xc3\xa9\xf0\x9f\x98\x80");
    assert_eq!(trophy.description, b"A detail");
    assert_eq!(trophy.reward, b"A reward");
    assert_eq!(trophy.icon_png, b"first icon");
    assert_eq!(
        (
            package.trophies[1].group_id,
            package.trophies[1].platinum_id,
            package.trophies[1].comparison
        ),
        (-1, -1, Comparison::None)
    );
    assert!(package.event_rules.is_empty());
    assert!(budget.usage().scratch_peak >= bytes.capacity() + package.metadata_bytes());
    assert_eq!(
        (budget.usage().attempts, budget.usage().handles_live),
        (0, 0)
    );
}

#[test]
fn malformed_ucp_json_and_duplicate_definitions_refuse_without_fallback_or_synthetic_package() {
    let valid = ucp(&[(b"tropconf.json", CONF), (b"tropmeta_en-US.json", META)]);
    let mut malformed = valid.clone();
    malformed[96 + 32..96 + 40].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(
        parse_package(&malformed, 1, malformed.capacity(), &mut budget())
            .unwrap()
            .is_none()
    );
    malformed = valid.clone();
    malformed[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(
        parse_package(&malformed, 1, malformed.capacity(), &mut budget())
            .unwrap()
            .is_none()
    );
    let duplicate_file = ucp(&[
        (b"tropconf.json", b"invalid"),
        (b"tropconf.json", CONF),
        (b"tropmeta_en-US.json", META),
    ]);
    assert!(
        parse_package(&duplicate_file, 1, duplicate_file.capacity(), &mut budget())
            .unwrap()
            .is_none()
    );
    let bad_locale = ucp(&[
        (b"tropconf.json", CONF),
        (b"tropmeta_fr-FR.json", b"invalid"),
        (b"tropmeta_en-US.json", META),
    ]);
    assert!(
        parse_package(&bad_locale, 2, bad_locale.capacity(), &mut budget())
            .unwrap()
            .is_none()
    );
    let duplicate =
        br#"{"defaultLanguage":"en-US","trophies":[{"id":2,"grade":"B"},{"id":"02","grade":"G"}]}"#;
    let bytes = ucp(&[
        (b"tropconf.json", duplicate),
        (b"tropmeta_en-US.json", META),
    ]);
    assert!(
        parse_package(&bytes, 99, bytes.capacity(), &mut budget())
            .unwrap()
            .is_none()
    );
    let bad_utf16 = ucp(&[
        (b"tropconf.json", CONF),
        (
            b"tropmeta_en-US.json",
            br#"{"metadata":{"titleMetadata":{"name":"\ud800"},"trophyMetadata":[]}}"#,
        ),
    ]);
    assert!(
        parse_package(&bad_utf16, 1, bad_utf16.capacity(), &mut budget())
            .unwrap()
            .is_none()
    );
}

#[test]
fn unlocks_filter_types_sort_unique_ids_and_preserve_last_json_key_then_lexical_timestamp_alias() {
    let bytes = br#"{"unlockedTrophies":[2,"1","01","-0",true,-1,2147483648,1.0,"+3","2x"],"unlockedAt":{"01":62135596800000001,"1":62135596800000002,"2":62135596799999999,"0":"62135596800000000","01":62135596800000003}}"#;
    let mut budget = budget();
    let unlocks = parse_unlocks(bytes, bytes.len(), &mut budget).unwrap();
    assert_eq!(unlocks.unlocked, [0, 1, 2]);
    assert_eq!(
        unlocks.timestamps,
        [(0, 62_135_596_800_000_000), (1, 62_135_596_800_000_003)]
    );
    assert!(budget.usage().scratch_peak >= bytes.len() + unlocks.metadata_bytes());
    let duplicate = br#"{"unlockedTrophies":[1],"unlockedAt":{"1":62135596800000000,"1":false}}"#;
    assert!(
        parse_unlocks(duplicate, duplicate.len(), &mut budget)
            .unwrap()
            .timestamps
            .is_empty()
    );
    assert!(
        parse_unlocks(b"malformed", 9, &mut budget)
            .unwrap()
            .unlocked
            .is_empty()
    );
}

#[test]
fn uds_rules_only_keep_direct_unsigned_latest_values_and_first_stat_definition() {
    let definitions = br#"{"statDefinitionArray":[{"statId":"5","aggregation":"latest","dataType":"uint32","minValue":0,"maxValue":"4294967295"},{"statId":5,"aggregation":"latest","dataType":"uint64"},{"statId":6,"aggregation":"sum","dataType":"uint32"},{"statId":7,"aggregation":"latest","dataType":"uint64","minValue":1},{"statId":8,"aggregation":"latest","dataType":"uint64"}]}"#;
    let extraction = br#"{"statsExtractionRuleArray":[{"condition":{"eventName":"B"},"action":{"input":"$.count","output":{"statId":8}}},{"condition":{"eventName":"A"},"action":{"input":"$.score","output":{"statId":5}}},{"condition":{"eventName":"A"},"action":{"input":"$.score","output":{"statId":5}}},{"condition":{"eventName":"A","filter":true},"action":{"input":"$.score","output":{"statId":5}}},{"condition":{"eventName":"A"},"action":{"input":"$.nested.score","output":{"statId":5}}},{"condition":{"eventName":"A"},"action":{"input":"$.","output":{"statId":5}}},{"condition":{"eventName":"A"},"action":{"input":"$.score","output":{"statId":6}}}]}"#;
    let bytes = ucp(&[
        (b"stats_definition.json", definitions),
        (b"stats_extraction.json", extraction),
    ]);
    let rules = parse_uds_rules(&bytes, bytes.capacity(), &mut budget()).unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].event, b"A");
    assert_eq!(rules[1].event, b"B");
    assert_eq!(rules[0].rules.len(), 2);
    assert_eq!(
        (rules[0].rules[0].stat_id, rules[0].rules[0].is_uint32),
        (5, true)
    );
    assert_eq!(rules[0].rules[0].input, b"score");
    assert_eq!(
        (rules[1].rules[0].stat_id, rules[1].rules[0].is_uint32),
        (8, false)
    );
    assert!(rules_metadata_bytes(&rules) >= capacity(&rules));
}

#[test]
fn explicit_held_capacity_depth_and_work_limits_refuse_before_unbounded_allocation() {
    let bytes = ucp(&[(b"tropconf.json", CONF), (b"tropmeta_en-US.json", META)]);
    assert!(matches!(
        parse_package(&bytes, 1, MAX_METADATA, &mut budget()),
        Err(Failure::Limit(_))
    ));
    let mut small = Budget::new(Limits {
        max_scratch_bytes: 128,
        max_name_bytes: 32,
        max_io_bytes: 128,
        ..Default::default()
    })
    .unwrap();
    assert!(matches!(
        parse_package(&bytes, 1, bytes.capacity(), &mut small),
        Err(Failure::Limit(_))
    ));
    let mut none = Budget::new(Limits {
        max_work: 0,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        parse_unlocks(b"{}", 2, &mut none).err(),
        Some(Failure::Limit("work"))
    );
    let mut nested = vec![b'['; 66];
    nested.extend_from_slice(b"0");
    nested.extend(std::iter::repeat_n(b']', 66));
    assert_eq!(
        parse_unlocks(&nested, nested.capacity(), &mut budget()).err(),
        Some(Failure::Limit("Trophy JSON depth"))
    );
    assert_eq!((none.usage().attempts, none.usage().handles_live), (0, 0));
}
