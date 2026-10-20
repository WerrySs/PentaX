// SPDX-License-Identifier: GPL-2.0-only
// Modified 2026 for PentaX: owned, bounded Trophy2 lifecycle and guest outputs.
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/libs/libTrophy.cpp blob216922e65cb8736f0bb663931d38764cf89dde76.
// Package parsing and persistence use explicit read-only/save capabilities.

use crate::budget::Budget;
use crate::filesystem::FileState;
use crate::save::SaveStorage;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service};

mod package;
mod query;
mod uds;
use package::{Package, UnlockData};
pub(crate) use uds::UdsPost;

const INVALID_ARGUMENT: u64 = 0x8055_3904;
const INVALID_CONTEXT: u64 = 0x8055_3909;
const INVALID_TROPHY: u64 = 0x8055_390a;
const INVALID_GROUP: u64 = 0x8055_390b;
const DUPLICATE_CONTEXT: u64 = 0x8055_3910;
const ICON_MISSING: u64 = 0x8055_3911;
const MAX_CONTEXTS_ERROR: u64 = 0x8055_391b;
const PACKAGE_MISSING: u64 = 0x8055_391e;
const NOT_REGISTERED: u64 = 0x8055_3920;
const ALREADY_REGISTERED: u64 = 0x8055_3921;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const MAX_PACKAGE: usize = 1024 * 1024 * 1024;

struct Context {
    id: i32,
    user: i32,
    label: u32,
    registered: Option<usize>,
}

struct CachedPackage {
    label: u32,
    package: Package,
}

struct UserState {
    user: i32,
    label: u32,
    package: usize,
    unlocks: UnlockData,
}

struct PendingUser {
    package: Option<CachedPackage>,
    user: UserState,
    package_growth: Option<Vec<CachedPackage>>,
    user_growth: Option<Vec<UserState>>,
    handles: usize,
}

struct UserPlan {
    index: usize,
    pending: Option<PendingUser>,
    staged_bytes: usize,
}

pub(crate) struct TrophyState {
    contexts: Vec<Context>,
    packages: Vec<CachedPackage>,
    users: Vec<UserState>,
    next_context: u64,
}

impl Default for TrophyState {
    fn default() -> Self {
        Self {
            contexts: Vec::new(),
            packages: Vec::new(),
            users: Vec::new(),
            next_context: 1,
        }
    }
}

fn add_bytes(a: usize, b: usize) -> Result<usize, Failure> {
    a.checked_add(b)
        .filter(|total| *total <= MAX_METADATA)
        .ok_or(Failure::Limit("Trophy2 retained and staged metadata"))
}

fn capacity_bytes<T>(table: &Vec<T>) -> Result<usize, Failure> {
    table
        .capacity()
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("Trophy2 record capacity"))
}

fn admit(bytes: usize, budget: &mut Budget) -> Result<(), Failure> {
    add_bytes(0, bytes)?;
    budget.scratch(bytes)?;
    budget.record_scratch(bytes);
    Ok(())
}

fn package_bytes(package: &Package, budget: &mut Budget) -> Result<usize, Failure> {
    budget.work(
        (package.groups.len() as u64
            + package.trophies.len() as u64
            + package.event_rules.len() as u64
            + 1)
            * 2,
    )?;
    for event in &package.event_rules {
        budget.work(event.rules.len() as u64 * 2 + 1)?;
    }
    let bytes = package.metadata_bytes();
    add_bytes(0, bytes)?;
    Ok(bytes)
}

/// Keep replacement storage private until all fallible work/output succeeds.
fn growth<T>(
    table: &Vec<T>,
    maximum: usize,
    held: usize,
    budget: &mut Budget,
) -> Result<Option<Vec<T>>, Failure> {
    if table.len() >= maximum || table.capacity() > maximum {
        return Err(Failure::Limit("Trophy2 records"));
    }
    admit(held, budget)?;
    if table.len() < table.capacity() {
        return Ok(None);
    }
    let requested = (table.len() + 1)
        .max(table.capacity().saturating_mul(2))
        .min(maximum);
    let expected = requested
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("Trophy2 record capacity"))?;
    admit(add_bytes(held, expected)?, budget)?;
    budget.work(requested as u64 + 1)?;
    let mut prepared = Vec::new();
    prepared
        .try_reserve_exact(requested)
        .map_err(|_| Failure::Allocation)?;
    if prepared.capacity() > maximum {
        return Err(Failure::Limit("Trophy2 record capacity"));
    }
    admit(add_bytes(held, capacity_bytes(&prepared)?)?, budget)?;
    Ok(Some(prepared))
}

fn insert<T>(table: &mut Vec<T>, prepared: Option<Vec<T>>, value: T) {
    if let Some(mut next) = prepared {
        next.append(table);
        next.push(value);
        *table = next;
    } else {
        table.push(value);
    }
}

struct PackagePath {
    bytes: [u8; 80],
    len: usize,
}

impl PackagePath {
    fn new(label: u32, uds: bool) -> Self {
        let prefix: &[u8] = if uds {
            b"/app0/sce_sys/uds/uds"
        } else {
            b"/app0/sce_sys/trophy2/trophy"
        };
        let mut path = Self {
            bytes: [0; 80],
            len: prefix.len(),
        };
        path.bytes[..prefix.len()].copy_from_slice(prefix);
        let mut digits = [0; 10];
        let mut first = digits.len();
        let mut number = label;
        loop {
            first -= 1;
            digits[first] = b'0' + (number % 10) as u8;
            number /= 10;
            if number == 0 {
                break;
            }
        }
        if digits.len() - first == 1 {
            path.bytes[path.len] = b'0';
            path.len += 1;
        }
        let count = digits.len() - first;
        path.bytes[path.len..path.len + count].copy_from_slice(&digits[first..]);
        path.len += count;
        path.bytes[path.len..path.len + 4].copy_from_slice(b".ucp");
        path.len += 4;
        path
    }

    fn as_str(&self) -> Result<&str, Failure> {
        core::str::from_utf8(&self.bytes[..self.len])
            .map_err(|_| Failure::Unsupported("Trophy2 internal package path"))
    }
}

enum Registered<'a> {
    Error(u64),
    Ready(&'a Package, &'a UnlockData),
}

impl TrophyState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.contexts.len() + self.packages.len() + self.users.len()
    }

    /// Disjoint subsets of the aggregate, independent of guest identifiers.
    pub(crate) fn retained_record_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.contexts.len(),
            self.packages.len(),
            self.users.len(),
            self.contexts
                .iter()
                .filter(|context| context.registered.is_some())
                .count(),
        )
    }

    pub(crate) fn metadata_bytes(&self, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.packages.len() as u64 + self.users.len() as u64 + 4)?;
        let mut bytes = add_bytes(
            capacity_bytes(&self.contexts)?,
            capacity_bytes(&self.packages)?,
        )?;
        bytes = add_bytes(bytes, capacity_bytes(&self.users)?)?;
        for record in &self.packages {
            bytes = add_bytes(bytes, package_bytes(&record.package, budget)?)?;
        }
        for record in &self.users {
            bytes = add_bytes(bytes, record.unlocks.metadata_bytes())?;
        }
        Ok(bytes)
    }

    fn registered(&self, context: i32) -> Result<Registered<'_>, Failure> {
        let Some(context) = self.contexts.iter().find(|record| record.id == context) else {
            return Ok(Registered::Error(INVALID_CONTEXT));
        };
        let Some(index) = context.registered else {
            return Ok(Registered::Error(NOT_REGISTERED));
        };
        let user = self
            .users
            .get(index)
            .ok_or(Failure::Unsupported("Trophy2 user identity"))?;
        let package = self
            .packages
            .get(user.package)
            .ok_or(Failure::Unsupported("Trophy2 package identity"))?;
        Ok(Registered::Ready(&package.package, &user.unlocks))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        stack_pointer: Option<u64>,
        files: &FileState,
        storage: Option<&SaveStorage>,
        language: u8,
        external_held: usize,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // Bound table scans, removal shifts and ownership/capacity inspection.
        budget.work((self.retained_handle_count() as u64 + 1) * 4)?;
        let a = args.0;
        use Service::*;
        match service {
            Trophy2CreateContext => {
                let user = a[1] as u32 as i32;
                let label = a[2] as u32;
                if a[0] == 0 || a[3] != 0 || user < 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if self
                    .contexts
                    .iter()
                    .any(|record| record.user == user && record.label == label)
                {
                    return Ok(DUPLICATE_CONTEXT);
                }
                if self.contexts.len() >= 8 {
                    return Ok(MAX_CONTEXTS_ERROR);
                }
                if self.next_context == 0 || self.next_context > i32::MAX as u64 {
                    return Err(Failure::Limit("Trophy2 context identifiers"));
                }
                let total_handles = budget.replacement_handles(0, 1)?;
                let retained = add_bytes(self.metadata_bytes(budget)?, external_held)?;
                let prepared = growth(&self.contexts, 8, retained, budget)?;
                let id = self.next_context as i32;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &id.to_le_bytes(),
                    }],
                )?;
                insert(
                    &mut self.contexts,
                    prepared,
                    Context {
                        id,
                        user,
                        label,
                        registered: None,
                    },
                );
                self.next_context += 1;
                budget.replaced_handles(total_handles);
                Ok(0)
            }
            Trophy2CreateHandle => {
                if a[0] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &1_i32.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            Trophy2DestroyContext => {
                let Some(index) = self
                    .contexts
                    .iter()
                    .position(|record| record.id == a[0] as u32 as i32)
                else {
                    return Ok(INVALID_CONTEXT);
                };
                self.contexts.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            Trophy2RegisterContext => {
                self.register(a, files, storage, language, external_held, budget)
            }
            Trophy2RegisterUnlockCallback | Trophy2AbortHandle | Trophy2DestroyHandle => {
                // Exact source no-ops: no callback retention, execution or
                // asynchronous handle lifecycle is claimed.
                Ok(0)
            }
            Trophy2GetGameIcon | Trophy2GetGroupIcon | Trophy2GetTrophyIcon => {
                let size = if service == Trophy2GetGameIcon {
                    a[3]
                } else {
                    a[4]
                };
                if size != 0 {
                    budget.write(
                        guest,
                        &[GuestWrite {
                            address: size,
                            bytes: &0_u64.to_le_bytes(),
                        }],
                    )?;
                }
                Ok(ICON_MISSING)
            }
            Trophy2GetGameInfo
            | Trophy2GetGroupInfo
            | Trophy2GetGroupInfoArray
            | Trophy2GetTrophyInfo
            | Trophy2GetTrophyInfoArray => query::invoke(
                self,
                service,
                args,
                stack_pointer,
                external_held,
                guest,
                budget,
            ),
            _ => Err(Failure::Unsupported("Trophy2 service")),
        }
    }

    fn register(
        &mut self,
        a: [u64; 6],
        files: &FileState,
        storage: Option<&SaveStorage>,
        language: u8,
        external_held: usize,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let Some(index) = self
            .contexts
            .iter()
            .position(|record| record.id == a[0] as u32 as i32)
        else {
            return Ok(INVALID_CONTEXT);
        };
        if a[2] != 0 {
            return Ok(INVALID_ARGUMENT);
        }
        if self.contexts[index].registered.is_some() {
            return Ok(ALREADY_REGISTERED);
        }
        let user = self.contexts[index].user;
        let label = self.contexts[index].label;
        let Some(plan) =
            self.prepare_user(user, label, files, storage, language, external_held, budget)?
        else {
            return Ok(PACKAGE_MISSING);
        };
        let user_index = self.commit_user(plan, None, budget);
        self.contexts[index].registered = Some(user_index);
        Ok(0)
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_user(
        &self,
        user: i32,
        label: u32,
        files: &FileState,
        storage: Option<&SaveStorage>,
        language: u8,
        external_held: usize,
        budget: &mut Budget,
    ) -> Result<Option<UserPlan>, Failure> {
        let retained = add_bytes(self.metadata_bytes(budget)?, external_held)?;
        admit(retained, budget)?;
        budget.work(self.users.len() as u64 + self.packages.len() as u64 + 2)?;
        if let Some(found) = self
            .users
            .iter()
            .position(|record| record.user == user && record.label == label)
        {
            return Ok(Some(UserPlan {
                index: found,
                pending: None,
                staged_bytes: 0,
            }));
        }
        let existing_package = self
            .packages
            .iter()
            .position(|record| record.label == label);
        let package_index = existing_package.unwrap_or(self.packages.len());
        let mut prepared_package = None;
        if existing_package.is_none() {
            let path = PackagePath::new(label, false);
            let parsed = {
                let Some(input) =
                    files.trophy_read(path.as_str()?, MAX_PACKAGE, retained, budget)?
                else {
                    return Ok(None);
                };
                package::parse_package(
                    &input,
                    language,
                    add_bytes(retained, input.capacity())?,
                    budget,
                )?
            };
            let Some(mut parsed) = parsed else {
                return Ok(None);
            };
            if parsed.trophies.is_empty() {
                return Ok(None);
            }
            let held = add_bytes(retained, package_bytes(&parsed, budget)?)?;
            admit(held, budget)?;
            let path = PackagePath::new(label, true);
            if let Some(input) = files.trophy_read(path.as_str()?, MAX_PACKAGE, held, budget)? {
                let rules =
                    package::parse_uds_rules(&input, add_bytes(held, input.capacity())?, budget)?;
                budget.work(rules.len() as u64 * 2 + 1)?;
                for event in &rules {
                    budget.work(event.rules.len() as u64 * 2 + 1)?;
                }
                admit(
                    add_bytes(held, package::rules_metadata_bytes(&rules))?,
                    budget,
                )?;
                parsed.event_rules = rules;
            }
            prepared_package = Some(CachedPackage {
                label,
                package: parsed,
            });
        }
        let staged_package_bytes = prepared_package
            .as_ref()
            .map_or(Ok(0), |record| package_bytes(&record.package, budget))?;
        let held = add_bytes(retained, staged_package_bytes)?;
        let storage = storage.ok_or(Failure::Unsupported("Trophy2 save storage capability"))?;
        let unlocks = if let Some(input) = storage.trophy_read(user, label, held, budget)? {
            package::parse_unlocks(&input, add_bytes(held, input.capacity())?, budget)?
        } else {
            UnlockData::default()
        };
        let held = add_bytes(held, unlocks.metadata_bytes())?;
        admit(held, budget)?;
        let count = 1 + usize::from(prepared_package.is_some());
        let handles = budget.replacement_handles(0, count)?;
        let package_growth = if prepared_package.is_some() {
            growth(&self.packages, budget.limits.max_handles, held, budget)?
        } else {
            None
        };
        let held = add_bytes(held, package_growth.as_ref().map_or(Ok(0), capacity_bytes)?)?;
        let user_growth = growth(&self.users, budget.limits.max_handles, held, budget)?;
        let held = add_bytes(held, user_growth.as_ref().map_or(Ok(0), capacity_bytes)?)?;
        let staged_bytes = held
            .checked_sub(retained)
            .ok_or(Failure::Limit("Trophy2 prepared metadata"))?;
        Ok(Some(UserPlan {
            index: self.users.len(),
            pending: Some(PendingUser {
                package: prepared_package,
                user: UserState {
                    user,
                    label,
                    package: package_index,
                    unlocks,
                },
                package_growth,
                user_growth,
                handles,
            }),
            staged_bytes,
        }))
    }

    fn planned_data<'a>(
        &'a self,
        plan: &'a UserPlan,
    ) -> Result<(&'a Package, &'a UnlockData), Failure> {
        if let Some(pending) = &plan.pending {
            let package = if let Some(record) = &pending.package {
                &record.package
            } else {
                &self
                    .packages
                    .get(pending.user.package)
                    .ok_or(Failure::Unsupported("Trophy2 package identity"))?
                    .package
            };
            Ok((package, &pending.user.unlocks))
        } else {
            let user = self
                .users
                .get(plan.index)
                .ok_or(Failure::Unsupported("Trophy2 user identity"))?;
            let package = self
                .packages
                .get(user.package)
                .ok_or(Failure::Unsupported("Trophy2 package identity"))?;
            Ok((&package.package, &user.unlocks))
        }
    }

    fn commit_user(
        &mut self,
        plan: UserPlan,
        replacement: Option<UnlockData>,
        budget: &mut Budget,
    ) -> usize {
        if let Some(mut pending) = plan.pending {
            if let Some(unlocks) = replacement {
                pending.user.unlocks = unlocks;
            }
            if let Some(record) = pending.package {
                insert(&mut self.packages, pending.package_growth, record);
            }
            insert(&mut self.users, pending.user_growth, pending.user);
            budget.replaced_handles(pending.handles);
        } else if let Some(unlocks) = replacement {
            self.users[plan.index].unlocks = unlocks;
        }
        plan.index
    }
}

#[cfg(test)]
mod tests;
