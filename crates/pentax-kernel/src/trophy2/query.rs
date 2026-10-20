// SPDX-License-Identifier: GPL-2.0-only
// Modified 2026 for PentaX: bounded exact-layout Trophy2 query publication.
// Adapted from the pinned libTrophy.cpp cited in the parent module.

use super::package::{Group, Trophy};
use super::*;
use crate::GuestError;

#[derive(Clone, Copy)]
enum Selection {
    Game,
    Group(usize),
    Trophy(usize),
    Groups { first: usize, count: usize },
    Trophies { first: usize, count: usize },
}

impl Selection {
    fn count(&self) -> usize {
        match *self {
            Self::Groups { count, .. } | Self::Trophies { count, .. } => count,
            _ => 1,
        }
    }
    fn strides(&self) -> (usize, usize) {
        match self {
            Self::Game => (152, 24),
            Self::Group(_) | Self::Groups { .. } => (152, 32),
            Self::Trophy(_) | Self::Trophies { .. } => (1312, 32),
        }
    }
}

fn lookup_work(length: usize) -> u64 {
    if length == 0 {
        1
    } else {
        u64::from(length.ilog2()) + 2
    }
}

fn buffer(length: usize, held: usize, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    admit(add_bytes(held, length)?, budget)?;
    let output = budget.buffer_held(length, held)?;
    admit(add_bytes(held, output.capacity())?, budget)?;
    Ok(output)
}

fn word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn wide(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn cstr(output: &mut [u8], input: &[u8]) {
    let maximum = input.len().min(output.len() - 1);
    let count = input[..maximum]
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(maximum);
    output[..count].copy_from_slice(&input[..count]);
    // The complete record was initialized to zero, including terminator/tail.
}

struct Progress {
    total: [u32; 5],
    earned: [u32; 5],
    total_count: u32,
    earned_count: u32,
}

impl Progress {
    fn percentage(&self) -> u32 {
        let points =
            u64::from(self.total[2]) * 6 + u64::from(self.total[3]) * 2 + u64::from(self.total[4]);
        let earned = u64::from(self.earned[2]) * 6
            + u64::from(self.earned[3]) * 2
            + u64::from(self.earned[4]);
        (earned * 100).checked_div(points).unwrap_or(0) as u32
    }
}

fn progress(
    package: &Package,
    unlocks: &UnlockData,
    group: Option<i32>,
    budget: &mut Budget,
) -> Result<Progress, Failure> {
    let visits = (package.trophies.len() as u64)
        .checked_mul(lookup_work(unlocks.unlocked.len()) + 4)
        .ok_or(Failure::Limit("Trophy2 query work"))?;
    budget.work(visits)?;
    let mut result = Progress {
        total: [0; 5],
        earned: [0; 5],
        total_count: 0,
        earned_count: 0,
    };
    for trophy in &package.trophies {
        if group.is_some_and(|id| trophy.group_id != id) {
            continue;
        }
        let grade = usize::try_from(trophy.grade)
            .ok()
            .filter(|grade| (1..=4).contains(grade))
            .ok_or(Failure::Unsupported("Trophy2 retained grade"))?;
        result.total_count = result
            .total_count
            .checked_add(1)
            .ok_or(Failure::Limit("Trophy2 counts"))?;
        result.total[grade] = result.total[grade]
            .checked_add(1)
            .ok_or(Failure::Limit("Trophy2 counts"))?;
        if unlocks.unlocked.binary_search(&trophy.id).is_ok() {
            result.earned_count = result
                .earned_count
                .checked_add(1)
                .ok_or(Failure::Limit("Trophy2 counts"))?;
            result.earned[grade] = result.earned[grade]
                .checked_add(1)
                .ok_or(Failure::Limit("Trophy2 counts"))?;
        }
    }
    Ok(result)
}

fn fill_counts(output: &mut [u8], start: usize, counts: &Progress, earned: bool) {
    if output.is_empty() {
        return;
    }
    let grades = if earned {
        &counts.earned
    } else {
        &counts.total
    };
    word(
        output,
        start,
        if earned {
            counts.earned_count
        } else {
            counts.total_count
        },
    );
    for (index, count) in grades[1..].iter().enumerate() {
        word(output, start + 4 + index * 4, *count);
    }
    if earned {
        word(output, start + 20, counts.percentage());
    }
}

fn game(
    package: &Package,
    unlocks: &UnlockData,
    details: &mut [u8],
    data: &mut [u8],
    budget: &mut Budget,
) -> Result<(), Failure> {
    let counts = progress(package, unlocks, None, budget)?;
    fill_counts(details, 4, &counts, false);
    fill_counts(data, 0, &counts, true);
    if !details.is_empty() {
        word(details, 0, package.groups.len() as u32);
        cstr(&mut details[24..152], &package.title);
    }
    Ok(())
}

fn group(
    group: &Group,
    package: &Package,
    unlocks: &UnlockData,
    details: &mut [u8],
    data: &mut [u8],
    budget: &mut Budget,
) -> Result<(), Failure> {
    let counts = progress(package, unlocks, Some(group.id), budget)?;
    fill_counts(details, 4, &counts, false);
    fill_counts(data, 4, &counts, true);
    if !details.is_empty() {
        word(details, 0, group.id as u32);
        cstr(&mut details[24..152], &group.title);
    }
    if !data.is_empty() {
        word(data, 0, group.id as u32);
    }
    Ok(())
}

fn trophy(
    trophy: &Trophy,
    unlocks: &UnlockData,
    details: &mut [u8],
    data: &mut [u8],
    budget: &mut Budget,
) -> Result<(), Failure> {
    budget
        .work(lookup_work(unlocks.unlocked.len()) + lookup_work(unlocks.timestamps.len()) + 16)?;
    if !details.is_empty() {
        word(details, 0, trophy.id as u32);
        word(details, 4, trophy.grade as u32);
        word(details, 8, trophy.group_id as u32);
        details[12] = u8::from(trophy.hidden);
        details[13] = u8::from(trophy.has_reward);
        if let Some(target) = trophy.target.filter(|_| trophy.progressive) {
            word(details, 16, 1);
            wide(details, 24, target);
        }
        cstr(&mut details[32..160], &trophy.name);
        cstr(&mut details[160..1184], &trophy.description);
        cstr(&mut details[1184..1312], &trophy.reward);
    }
    if !data.is_empty() {
        let unlocked = unlocks.unlocked.binary_search(&trophy.id).is_ok();
        word(data, 0, trophy.id as u32);
        data[4] = u8::from(unlocked);
        if let Some(target) = trophy.target.filter(|_| trophy.progressive) {
            word(data, 8, 1);
            wide(data, 16, if unlocked { target } else { 0 });
        }
        if let Ok(index) = unlocks
            .timestamps
            .binary_search_by_key(&trophy.id, |entry| entry.0)
        {
            wide(data, 24, unlocks.timestamps[index].1);
        }
    }
    Ok(())
}

pub(super) fn invoke(
    owner: &TrophyState,
    service: Service,
    args: CallArgs,
    stack_pointer: Option<u64>,
    external_held: usize,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    let a = args.0;
    let array = matches!(
        service,
        Service::Trophy2GetGroupInfoArray | Service::Trophy2GetTrophyInfoArray
    );
    let count_address = if array {
        let stack = stack_pointer.ok_or(Failure::Unsupported(
            "missing Trophy2 array stack arguments",
        ))?;
        let at = stack.checked_add(8).ok_or(GuestError::AddressOverflow)?;
        Some(budget.u64(guest, at)?)
    } else {
        None
    };
    let (details_address, data_address) = if array {
        (a[4], a[5])
    } else if service == Service::Trophy2GetGameInfo {
        (a[2], a[3])
    } else {
        (a[3], a[4])
    };
    if (details_address == 0 && data_address == 0) || count_address == Some(0) {
        return Ok(INVALID_ARGUMENT);
    }
    let (package, unlocks) = match owner.registered(a[0] as u32 as i32)? {
        Registered::Error(error) => return Ok(error),
        Registered::Ready(package, unlocks) => (package, unlocks),
    };
    budget.work(lookup_work(package.groups.len()) + lookup_work(package.trophies.len()) + 8)?;
    let selected = match service {
        Service::Trophy2GetGameInfo => Selection::Game,
        Service::Trophy2GetGroupInfo => {
            let Ok(index) = package
                .groups
                .binary_search_by_key(&(a[2] as u32 as i32), |record| record.id)
            else {
                return Ok(INVALID_GROUP);
            };
            Selection::Group(index)
        }
        Service::Trophy2GetTrophyInfo => {
            let Ok(index) = package
                .trophies
                .binary_search_by_key(&(a[2] as u32 as i32), |record| record.id)
            else {
                return Ok(INVALID_TROPHY);
            };
            Selection::Trophy(index)
        }
        Service::Trophy2GetGroupInfoArray => {
            let first = (a[2] as u32 as usize).min(package.groups.len());
            Selection::Groups {
                first,
                count: (package.groups.len() - first).min(a[3] as u32 as usize),
            }
        }
        Service::Trophy2GetTrophyInfoArray => {
            let first = (a[2] as u32 as usize).min(package.trophies.len());
            Selection::Trophies {
                first,
                count: (package.trophies.len() - first).min(a[3] as u32 as usize),
            }
        }
        _ => return Err(Failure::Unsupported("Trophy2 info query")),
    };
    let count = selected.count();
    let (details_stride, data_stride) = selected.strides();
    let retained = add_bytes(owner.metadata_bytes(budget)?, external_held)?;
    let details_length = if details_address == 0 {
        0
    } else {
        count
            .checked_mul(details_stride)
            .ok_or(Failure::Limit("Trophy2 output extent"))?
    };
    let data_length = if data_address == 0 {
        0
    } else {
        count
            .checked_mul(data_stride)
            .ok_or(Failure::Limit("Trophy2 output extent"))?
    };
    let mut details = buffer(details_length, retained, budget)?;
    let mut data = buffer(
        data_length,
        add_bytes(retained, details.capacity())?,
        budget,
    )?;
    budget.work(count as u64 + 1)?;
    for index in 0..count {
        let details_record = if details.is_empty() {
            &mut [][..]
        } else {
            &mut details[index * details_stride..(index + 1) * details_stride]
        };
        let data_record = if data.is_empty() {
            &mut [][..]
        } else {
            &mut data[index * data_stride..(index + 1) * data_stride]
        };
        match selected {
            Selection::Game => game(package, unlocks, details_record, data_record, budget)?,
            Selection::Group(at) => group(
                &package.groups[at],
                package,
                unlocks,
                details_record,
                data_record,
                budget,
            )?,
            Selection::Trophy(at) => trophy(
                &package.trophies[at],
                unlocks,
                details_record,
                data_record,
                budget,
            )?,
            Selection::Groups { first, .. } => group(
                &package.groups[first + index],
                package,
                unlocks,
                details_record,
                data_record,
                budget,
            )?,
            Selection::Trophies { first, .. } => trophy(
                &package.trophies[first + index],
                unlocks,
                details_record,
                data_record,
                budget,
            )?,
        }
    }
    let count_bytes = (count as u32).to_le_bytes();
    let mut writes = [GuestWrite {
        address: 0,
        bytes: &[],
    }; 3];
    let mut used = 0;
    if !details.is_empty() {
        writes[used] = GuestWrite {
            address: details_address,
            bytes: &details,
        };
        used += 1;
    }
    if !data.is_empty() {
        writes[used] = GuestWrite {
            address: data_address,
            bytes: &data,
        };
        used += 1;
    }
    if let Some(address) = count_address {
        writes[used] = GuestWrite {
            address,
            bytes: &count_bytes,
        };
        used += 1;
    }
    budget.write(guest, &writes[..used])?;
    Ok(0)
}
