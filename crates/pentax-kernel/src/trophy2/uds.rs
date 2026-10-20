// SPDX-License-Identifier: GPL-2.0-only
// Modified 2026 for PentaX: transactional shared Trophy2/UDS unlock state.
// Adapted from libTrophy.cpp:127-282,681-718 and common/trophies.cpp:317-459
// at KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de.
// Persistence must commit before the infallible RAM/cache publication below.

use super::package::{Comparison, EventRule, Trophy};
use super::*;
use crate::uds::{UdsInteger, UdsProperty};
use std::cmp::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

const UNIX_EPOCH_TICK: u64 = 62_135_596_800_000_000;
const MAX_TICK: u64 = 315_537_897_599_999_999;
const MAX_SAVE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UdsPost {
    Posted,
    InvalidArgument,
}

fn reserve<T>(length: usize, held: usize, budget: &mut Budget) -> Result<Vec<T>, Failure> {
    let requested = length
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("UDS trophy staging"))?;
    admit(add_bytes(held, requested)?, budget)?;
    budget.work(requested as u64 + 1)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| Failure::Allocation)?;
    admit(add_bytes(held, capacity_bytes(&output)?)?, budget)?;
    Ok(output)
}

fn search_work(length: usize) -> u64 {
    if length == 0 {
        1
    } else {
        u64::from(length.ilog2()) + 2
    }
}

fn validate(event: &[u8], properties: &[UdsProperty], budget: &mut Budget) -> Result<(), Failure> {
    budget.work(event.len() as u64 + 1)?;
    if event.contains(&0) {
        return Err(Failure::Unsupported("UDS event name contains NUL"));
    }
    for (index, property) in properties.iter().enumerate() {
        budget.work(property.key.len() as u64 * 2 + 1)?;
        if property.key.contains(&0) || (index > 0 && properties[index - 1].key >= property.key) {
            return Err(Failure::Unsupported("UDS property key ordering"));
        }
    }
    Ok(())
}

fn property<'a>(
    properties: &'a [UdsProperty],
    key: &[u8],
    budget: &mut Budget,
) -> Result<Option<&'a UdsProperty>, Failure> {
    let mut low = 0;
    let mut high = properties.len();
    while low < high {
        budget.work(key.len() as u64 + 1)?;
        let middle = low + (high - low) / 2;
        match properties[middle].key.as_slice().cmp(key) {
            Ordering::Less => low = middle + 1,
            Ordering::Greater => high = middle,
            Ordering::Equal => return Ok(Some(&properties[middle])),
        }
    }
    Ok(None)
}

fn event_rules<'a>(
    package: &'a Package,
    name: &[u8],
    budget: &mut Budget,
) -> Result<Option<&'a EventRule>, Failure> {
    let mut low = 0;
    let mut high = package.event_rules.len();
    while low < high {
        budget.work(name.len() as u64 + 1)?;
        let middle = low + (high - low) / 2;
        match package.event_rules[middle].event.as_slice().cmp(name) {
            Ordering::Less => low = middle + 1,
            Ordering::Greater => high = middle,
            Ordering::Equal => return Ok(Some(&package.event_rules[middle])),
        }
    }
    Ok(None)
}

fn trophy<'a>(
    package: &'a Package,
    id: i32,
    budget: &mut Budget,
) -> Result<Option<&'a Trophy>, Failure> {
    budget.work(search_work(package.trophies.len()))?;
    Ok(package
        .trophies
        .binary_search_by_key(&id, |record| record.id)
        .ok()
        .map(|index| &package.trophies[index]))
}

fn eligible(
    record: &Trophy,
    from_stat: bool,
    unlocks: &UnlockData,
    budget: &mut Budget,
) -> Result<bool, Failure> {
    if record.grade == 1 || (!from_stat && record.progressive) {
        return Ok(false);
    }
    budget.work(search_work(unlocks.unlocked.len()))?;
    Ok(unlocks.unlocked.binary_search(&record.id).is_err())
}

fn candidates(
    package: &Package,
    name: &[u8],
    properties: &[UdsProperty],
    direct: Option<i32>,
    held: usize,
    budget: &mut Budget,
) -> Result<Vec<i32>, Failure> {
    if let Some(id) = direct {
        let mut selected = reserve(1, held, budget)?;
        selected.push(id);
        return Ok(selected);
    }
    let Some(event) = event_rules(package, name, budget)? else {
        return Ok(Vec::new());
    };
    let mut selected = reserve(package.trophies.len(), held, budget)?;
    for record in &package.trophies {
        budget.work(1)?;
        if record.grade == 1 {
            continue;
        }
        let (Some(stat), Some(target)) = (record.uds_stat_id, record.target) else {
            continue;
        };
        for rule in &event.rules {
            budget.work(1)?;
            if rule.stat_id != stat {
                continue;
            }
            let Some(value) = property(properties, &rule.input, budget)? else {
                continue;
            };
            let number = match (&value.value, rule.is_uint32) {
                (UdsInteger::U32(value), true) => u64::from(*value),
                (UdsInteger::U64(value), false) => *value,
                _ => continue,
            };
            let matches = match record.comparison {
                Comparison::GreaterEqual => number >= target,
                Comparison::Greater => number > target,
                Comparison::LessEqual => number <= target,
                Comparison::Less => number < target,
                Comparison::None => false,
            };
            if matches {
                selected.push(record.id);
                break;
            }
        }
    }
    Ok(selected)
}

fn copy_unlocks(
    source: &UnlockData,
    extra: usize,
    held: usize,
    budget: &mut Budget,
) -> Result<UnlockData, Failure> {
    let ids = source
        .unlocked
        .len()
        .checked_add(extra)
        .ok_or(Failure::Limit("UDS unlock capacity"))?;
    let times = source
        .timestamps
        .len()
        .checked_add(extra)
        .ok_or(Failure::Limit("UDS timestamp capacity"))?;
    let mut unlocked = reserve(ids, held, budget)?;
    let held = add_bytes(held, capacity_bytes(&unlocked)?)?;
    let mut timestamps = reserve(times, held, budget)?;
    unlocked.extend_from_slice(&source.unlocked);
    timestamps.extend_from_slice(&source.timestamps);
    Ok(UnlockData {
        unlocked,
        timestamps,
    })
}

fn insert_id(unlocks: &mut UnlockData, id: i32, budget: &mut Budget) -> Result<bool, Failure> {
    budget.work(unlocks.unlocked.len() as u64 + 1)?;
    let Err(index) = unlocks.unlocked.binary_search(&id) else {
        return Ok(false);
    };
    if unlocks.unlocked.len() == unlocks.unlocked.capacity() {
        return Err(Failure::Limit("UDS prepared unlock slots"));
    }
    unlocks.unlocked.insert(index, id);
    Ok(true)
}

fn insert_time(
    unlocks: &mut UnlockData,
    id: i32,
    tick: u64,
    budget: &mut Budget,
) -> Result<(), Failure> {
    budget.work(unlocks.timestamps.len() as u64 + 1)?;
    if let Err(index) = unlocks
        .timestamps
        .binary_search_by_key(&id, |entry| entry.0)
    {
        if unlocks.timestamps.len() == unlocks.timestamps.capacity() {
            return Err(Failure::Limit("UDS prepared timestamp slots"));
        }
        unlocks.timestamps.insert(index, (id, tick));
    }
    Ok(())
}

fn source_tick(budget: &mut Budget) -> Result<u64, Failure> {
    budget.work(64)?;
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Failure::Unsupported("UDS realtime before Unix epoch"))?;
    let micros =
        u64::try_from(elapsed.as_micros()).map_err(|_| Failure::Limit("UDS realtime tick"))?;
    UNIX_EPOCH_TICK
        .checked_add(micros)
        .filter(|tick| *tick <= MAX_TICK)
        .ok_or(Failure::Limit("UDS realtime tick"))
}

fn record_unlock(
    package: &Package,
    unlocks: &mut UnlockData,
    id: i32,
    from_stat: bool,
    budget: &mut Budget,
) -> Result<bool, Failure> {
    let Some(record) = trophy(package, id, budget)? else {
        return Ok(false);
    };
    if !eligible(record, from_stat, unlocks, budget)? {
        return Ok(false);
    }
    // Source samples time once for each genuinely new helper invocation; its
    // associated platinum uses that same tick, not a guessed event timestamp.
    let tick = source_tick(budget)?;
    insert_id(unlocks, id, budget)?;
    insert_time(unlocks, id, tick, budget)?;
    let platinum = record.platinum_id;
    if platinum >= 0 && trophy(package, platinum, budget)?.is_some() {
        let mut complete = true;
        for other in &package.trophies {
            budget.work(1)?;
            if other.platinum_id == platinum {
                budget.work(search_work(unlocks.unlocked.len()))?;
                if unlocks.unlocked.binary_search(&other.id).is_err() {
                    complete = false;
                    break;
                }
            }
        }
        if complete && insert_id(unlocks, platinum, budget)? {
            insert_time(unlocks, platinum, tick, budget)?;
        }
    }
    Ok(true)
}

struct Json<'a, 'b> {
    budget: &'a mut Budget,
    output: Option<&'b mut Vec<u8>>,
    length: usize,
}

impl Json<'_, '_> {
    fn bytes(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let length = self
            .length
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_SAVE)
            .ok_or(Failure::Limit("UDS trophy save bytes"))?;
        self.budget.work(bytes.len() as u64 + 1)?;
        if let Some(output) = &mut self.output {
            if output.capacity() < length {
                return Err(Failure::Limit("UDS prepared save capacity"));
            }
            output.extend_from_slice(bytes);
        }
        self.length = length;
        Ok(())
    }

    fn number(&mut self, mut value: u64) -> Result<(), Failure> {
        self.budget.work(20)?;
        let mut digits = [0; 20];
        let mut at = digits.len();
        loop {
            at -= 1;
            digits[at] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.bytes(&digits[at..])
    }
}

fn emit_json(unlocks: &UnlockData, json: &mut Json<'_, '_>) -> Result<(), Failure> {
    json.bytes(b"{\"unlockedTrophies\":[")?;
    for (index, id) in unlocks.unlocked.iter().enumerate() {
        let id = u64::try_from(*id).map_err(|_| Failure::Unsupported("UDS retained unlock ID"))?;
        if index != 0 {
            json.bytes(b",")?;
        }
        json.number(id)?;
    }
    json.bytes(b"],\"unlockedAt\":{")?;
    let mut comma = false;
    for (id, tick) in &unlocks.timestamps {
        json.budget.work(search_work(unlocks.unlocked.len()))?;
        if unlocks.unlocked.binary_search(id).is_err() {
            continue;
        }
        let id =
            u64::try_from(*id).map_err(|_| Failure::Unsupported("UDS retained timestamp ID"))?;
        if comma {
            json.bytes(b",")?;
        }
        json.bytes(b"\"")?;
        json.number(id)?;
        json.bytes(b"\":")?;
        json.number(*tick)?;
        comma = true;
    }
    json.bytes(b"}}")
}

fn encode(unlocks: &UnlockData, held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    let length = {
        let mut count = Json {
            budget,
            output: None,
            length: 0,
        };
        emit_json(unlocks, &mut count)?;
        count.length
    };
    let mut output = reserve(length, held, budget)?;
    let mut writer = Json {
        budget,
        output: Some(&mut output),
        length: 0,
    };
    emit_json(unlocks, &mut writer)?;
    Ok(output)
}

impl TrophyState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn post_uds(
        &mut self,
        user: i32,
        label: u32,
        event: &[u8],
        properties: &[UdsProperty],
        files: &FileState,
        storage: Option<&SaveStorage>,
        language: u8,
        held: usize,
        budget: &mut Budget,
    ) -> Result<UdsPost, Failure> {
        validate(event, properties, budget)?;
        let direct = if event == b"_UnlockTrophy" {
            match property(properties, b"_trophy_id", budget)?.map(|property| &property.value) {
                Some(UdsInteger::I32(id)) if *id >= 0 => Some(*id),
                _ => return Ok(UdsPost::InvalidArgument),
            }
        } else {
            if properties.is_empty() {
                return Ok(UdsPost::Posted);
            }
            None
        };
        let Some(plan) = self.prepare_user(user, label, files, storage, language, held, budget)?
        else {
            // Actual supported package absence is source-ignored. Missing
            // capabilities/limits remain typed failures from prepare_user.
            return Ok(UdsPost::Posted);
        };
        let held = add_bytes(
            add_bytes(held, self.metadata_bytes(budget)?)?,
            plan.staged_bytes,
        )?;
        let (package, original) = self.planned_data(&plan)?;
        let selected = candidates(package, event, properties, direct, held, budget)?;
        let held = add_bytes(held, capacity_bytes(&selected)?)?;
        let mut changes = false;
        for id in &selected {
            if let Some(record) = trophy(package, *id, budget)?
                && eligible(record, direct.is_none(), original, budget)?
            {
                changes = true;
                break;
            }
        }
        if !changes {
            self.commit_user(plan, None, budget);
            return Ok(UdsPost::Posted);
        }
        // Every possible newly inserted ID/timestamp names an existing package
        // trophy, so this prepares all insertion capacity before any mutation.
        let mut replacement = copy_unlocks(original, package.trophies.len(), held, budget)?;
        let held = add_bytes(held, replacement.metadata_bytes())?;
        for id in &selected {
            record_unlock(package, &mut replacement, *id, direct.is_none(), budget)?;
        }
        let bytes = encode(&replacement, held, budget)?;
        storage
            .ok_or(Failure::Unsupported("UDS trophy save storage capability"))?
            .trophy_write(user, label, &bytes, budget)?;
        // The file is replaced. No allocation, quota/work check, callback or
        // fallible lookup may follow this infallible ownership publication.
        self.commit_user(plan, Some(replacement), budget);
        Ok(UdsPost::Posted)
    }
}

#[cfg(test)]
#[path = "uds_tests.rs"]
mod tests;
