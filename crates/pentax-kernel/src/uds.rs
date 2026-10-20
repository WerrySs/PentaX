// SPDX-License-Identifier: GPL-2.0-only
// Adapted from authorized KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/libs/libTrophy.cpp:548-1323, blob216922e65cb8736f0bb663931d38764cf89dde76.
// Opaque pointers own real guest RW/NX backing; no host container pointer is exposed.
// Exact source effects and PentaX limits are documented in NOTICE and provenance.

use crate::budget::Budget;
use crate::filesystem::FileState;
use crate::memory::{MemoryState, RuntimeObject};
use crate::save::SaveStorage;
use crate::trophy2::{TrophyState, UdsPost};
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, RuntimeDataOutputs, Service};

mod events;
mod properties;
#[cfg(test)]
mod tests;

const INVALID_ARGUMENT: u64 = 0x8055_3102;
const INVALID_CONTEXT: u64 = 0x8055_3104;
const MAX_CONTEXTS: u64 = 0x8055_3105;
const INVALID_USER: u64 = 0x8055_310a;
const NOT_REGISTERED: u64 = 0x8055_3120;
const ALREADY_REGISTERED: u64 = 0x8055_3121;
const MAX_METADATA: usize = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const MAX_KEYS: usize = 65_536;
const MAX_STRING: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UdsInteger {
    I32(i32),
    U32(u32),
    U64(u64),
}
pub(crate) struct UdsProperty {
    pub(crate) key: Vec<u8>,
    pub(crate) value: UdsInteger,
}

pub(crate) struct UdsResources<'a> {
    pub(crate) memory: &'a mut MemoryState,
    pub(crate) trophies: &'a mut TrophyState,
    pub(crate) files: &'a FileState,
    pub(crate) storage: Option<&'a SaveStorage>,
    pub(crate) language: u8,
}

struct Context {
    id: i32,
    user: i32,
    label: u32,
    registered: bool,
}
struct Object {
    runtime: RuntimeObject,
    properties: Vec<UdsProperty>,
}
struct Array {
    runtime: RuntimeObject,
}
struct Event {
    runtime: RuntimeObject,
    name: Vec<u8>,
    properties: Vec<UdsProperty>,
}

pub(crate) struct UdsState {
    contexts: Vec<Context>,
    objects: Vec<Object>,
    arrays: Vec<Array>,
    events: Vec<Event>,
    next_context: u64,
}
impl Default for UdsState {
    fn default() -> Self {
        Self {
            contexts: Vec::new(),
            objects: Vec::new(),
            arrays: Vec::new(),
            events: Vec::new(),
            next_context: 1,
        }
    }
}

fn add(left: usize, right: usize) -> Result<usize, Failure> {
    left.checked_add(right)
        .filter(|total| *total <= MAX_METADATA)
        .ok_or(Failure::Limit("UDS retained and staged metadata"))
}
fn capacity<T>(table: &Vec<T>) -> usize {
    table.capacity() * core::mem::size_of::<T>()
}
fn property_bytes(table: &Vec<UdsProperty>) -> usize {
    capacity(table)
        + table
            .iter()
            .map(|property| property.key.capacity())
            .sum::<usize>()
}
fn admit(bytes: usize, budget: &mut Budget) -> Result<(), Failure> {
    if bytes > MAX_METADATA {
        return Err(Failure::Limit("UDS retained and staged metadata"));
    }
    budget.scratch(bytes)?;
    budget.record_scratch(bytes);
    Ok(())
}

/// Prepare a replacement table without changing its source capacity or values.
/// The caller must move elements only after every fallible guest operation.
fn growth<T>(
    table: &Vec<T>,
    extra: usize,
    maximum: usize,
    held: usize,
    budget: &mut Budget,
) -> Result<Option<Vec<T>>, Failure> {
    let needed = table
        .len()
        .checked_add(extra)
        .filter(|value| *value <= maximum)
        .ok_or(Failure::Limit("UDS record count"))?;
    if needed <= table.capacity() {
        return Ok(None);
    }
    let requested = needed.max(table.capacity().saturating_mul(2)).min(maximum);
    let expected = requested
        .checked_mul(core::mem::size_of::<T>())
        .ok_or(Failure::Limit("UDS metadata"))?;
    admit(add(held, expected)?, budget)?;
    budget.work(expected as u64 + table.len() as u64 + 1)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(requested)
        .map_err(|_| Failure::Allocation)?;
    if output.capacity() > maximum {
        return Err(Failure::Limit("UDS record capacity"));
    }
    admit(add(held, capacity(&output))?, budget)?;
    Ok(Some(output))
}
fn insert<T>(table: &mut Vec<T>, prepared: Option<Vec<T>>, item: T) {
    if let Some(mut next) = prepared {
        next.append(table);
        next.push(item);
        *table = next;
    } else {
        table.push(item);
    }
}
fn output(
    address: u64,
    value: u64,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<(), Failure> {
    budget.write(
        guest,
        &[GuestWrite {
            address,
            bytes: &value.to_le_bytes(),
        }],
    )
}

impl UdsState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.contexts.len() + self.objects.len() + self.arrays.len() + self.events.len()
    }
    fn key_count(&self) -> usize {
        self.objects
            .iter()
            .map(|object| object.properties.len())
            .sum::<usize>()
            + self
                .events
                .iter()
                .map(|event| event.properties.len())
                .sum::<usize>()
    }
    pub(crate) fn metadata_bytes(&self, budget: &mut Budget) -> Result<usize, Failure> {
        budget.work(self.retained_handle_count() as u64 + 1)?;
        budget.work(self.key_count() as u64 + 1)?;
        let mut total = add(capacity(&self.contexts), capacity(&self.objects))?;
        total = add(total, capacity(&self.arrays))?;
        total = add(total, capacity(&self.events))?;
        for object in &self.objects {
            total = add(total, property_bytes(&object.properties))?;
        }
        for event in &self.events {
            total = add(
                total,
                add(event.name.capacity(), property_bytes(&event.properties))?,
            )?;
        }
        Ok(total)
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        resources: UdsResources<'_>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        let a = args.0;
        budget.work((self.retained_handle_count() as u64 + 1) * 4)?;
        match service {
            UdsInitialize => {
                if a[0] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let mut parameters = [0; 16];
                budget.read(guest, a[0], &mut parameters)?;
                Ok(0)
            }
            UdsCreateContext => {
                if a[0] == 0 || a[3] != 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let user = a[1] as u32 as i32;
                if user < 0 {
                    return Ok(INVALID_USER);
                }
                if self.contexts.len() == 8 {
                    return Ok(MAX_CONTEXTS);
                }
                if self.next_context > i32::MAX as u64 {
                    return Err(Failure::Limit("UDS context identifiers"));
                }
                if self.retained_handle_count() >= MAX_RECORDS {
                    return Err(Failure::Limit("UDS record count"));
                }
                budget.can_add_handle()?;
                let held = add(
                    self.metadata_bytes(budget)?,
                    resources.trophies.metadata_bytes(budget)?,
                )?;
                let prepared = growth(&self.contexts, 1, 8, held, budget)?;
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
                        label: a[2] as u32,
                        registered: false,
                    },
                );
                self.next_context += 1;
                budget.added_handle();
                Ok(0)
            }
            UdsCreateHandle => {
                if a[0] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[0],
                        bytes: &1i32.to_le_bytes(),
                    }],
                )?;
                Ok(0)
            }
            UdsRegisterContext => {
                let Some(context) = self
                    .contexts
                    .iter_mut()
                    .find(|context| context.id == a[0] as u32 as i32)
                else {
                    return Ok(INVALID_CONTEXT);
                };
                if a[2] != 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if context.registered {
                    return Ok(ALREADY_REGISTERED);
                }
                context.registered = true;
                Ok(0)
            }
            UdsDestroyContext => {
                let Some(index) = self
                    .contexts
                    .iter()
                    .position(|context| context.id == a[0] as u32 as i32)
                else {
                    return Ok(INVALID_CONTEXT);
                };
                self.contexts.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            UdsDestroyHandle | UdsAbortHandle | UdsTerminate => Ok(0),
            UdsGetMemoryStat | UdsGetStorageStat => {
                let address = if service == UdsGetMemoryStat {
                    a[0]
                } else {
                    a[1]
                };
                if address == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let bytes = [0; 56];
                let length = if service == UdsGetMemoryStat { 24 } else { 56 };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &bytes[..length],
                    }],
                )?;
                Ok(0)
            }
            UdsCreateEvent | UdsPostEvent | UdsEventEstimateSize | UdsEventToString
            | UdsDestroyEvent => self.events(service, a, resources, guest, budget),
            _ => self.properties(service, a, resources, guest, budget),
        }
    }
}
