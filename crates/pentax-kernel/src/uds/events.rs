// SPDX-License-Identifier: GPL-2.0-only
// Pinned libTrophy.cpp:665-790: copied integer properties and real shared posts.

use super::*;

impl UdsState {
    pub(super) fn events(
        &mut self,
        service: Service,
        a: [u64; 6],
        resources: UdsResources<'_>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        match service {
            UdsCreateEvent => {
                if a[0] == 0 || a[2] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if self.retained_handle_count() >= MAX_RECORDS {
                    return Err(Failure::Limit("UDS record count"));
                }
                budget.replacement_handles(0, 2)?;
                let held = add(
                    self.metadata_bytes(budget)?,
                    resources.trophies.metadata_bytes(budget)?,
                )?;
                let name = properties::cstring(a[0], held, guest, budget)?;
                let held = add(held, name.capacity())?;
                let properties = if a[1] != 0 {
                    let owner = self.property_owner(a[1], resources.memory, budget)?;
                    let source = self.property_table(owner);
                    if self
                        .key_count()
                        .checked_add(source.len())
                        .is_none_or(|count| count > MAX_KEYS)
                    {
                        return Err(Failure::Limit("UDS integer keys"));
                    }
                    properties::clone_properties(source, held, budget)?
                } else {
                    Vec::new()
                };
                let held = add(held, property_bytes(&properties))?;
                let prepared = growth(&self.events, 1, MAX_RECORDS, held, budget)?;
                let scratch = add(held, prepared.as_ref().map_or(0, capacity))?;
                let runtime = resources.memory.runtime_object(
                    16,
                    scratch,
                    guest,
                    budget,
                    |base, _bytes| {
                        let mut outputs = RuntimeDataOutputs::new();
                        outputs.push(a[2], &base.to_le_bytes())?;
                        if a[3] != 0 {
                            let first_end =
                                a[2].checked_add(8).ok_or(GuestError::AddressOverflow)?;
                            let second_end =
                                a[3].checked_add(8).ok_or(GuestError::AddressOverflow)?;
                            if a[2] < second_end && a[3] < first_end {
                                return Err(GuestError::UnsupportedMapping.into());
                            }
                            let view = base.checked_add(8).ok_or(GuestError::AddressOverflow)?;
                            outputs.push(a[3], &view.to_le_bytes())?;
                        }
                        Ok(outputs)
                    },
                )?;
                insert(
                    &mut self.events,
                    prepared,
                    Event {
                        runtime,
                        name,
                        properties,
                    },
                );
                budget.added_handle();
                Ok(0)
            }
            UdsPostEvent => {
                if a[2] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let Some(context) = self
                    .contexts
                    .iter()
                    .find(|context| context.id == a[0] as u32 as i32)
                else {
                    return Ok(INVALID_CONTEXT);
                };
                if !context.registered {
                    return Ok(NOT_REGISTERED);
                }
                let (user, label) = (context.user, context.label);
                let Some(index) = self
                    .events
                    .iter()
                    .position(|event| event.runtime.address == a[2])
                else {
                    return Err(Failure::Unsupported("UDS event owner"));
                };
                resources
                    .memory
                    .check_runtime_object(self.events[index].runtime, budget)?;
                let held = self.metadata_bytes(budget)?;
                let event = &self.events[index];
                // Handle/options are source-unused; the event remains owned
                // after posting. Only TrophyState performs unlock/persistence.
                match resources.trophies.post_uds(
                    user,
                    label,
                    &event.name,
                    &event.properties,
                    resources.files,
                    resources.storage,
                    resources.language,
                    held,
                    budget,
                )? {
                    UdsPost::Posted => Ok(0),
                    UdsPost::InvalidArgument => Ok(INVALID_ARGUMENT),
                }
            }
            UdsDestroyEvent => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let Some(index) = self
                    .events
                    .iter()
                    .position(|event| event.runtime.address == a[0])
                else {
                    return Err(Failure::Unsupported("UDS event owner"));
                };
                budget.work(
                    self.events[index].properties.len() as u64 + self.events.len() as u64 + 1,
                )?;
                resources.memory.release_runtime_object(
                    self.events[index].runtime,
                    guest,
                    budget,
                )?;
                // Exact backing release succeeded; invalidation and drops are
                // infallible and simultaneously retire the embedded view.
                self.events.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            UdsEventEstimateSize => {
                if a[0] == 0 || a[1] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                output(a[1], 3, guest, budget)?;
                Ok(0)
            }
            UdsEventToString => {
                if a[0] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let mut bytes = *b"{}\0";
                let count = if a[1] == 0 { 0 } else { a[2].min(3) as usize };
                if count != 0 {
                    bytes[count - 1] = 0;
                }
                let size = 3u64.to_le_bytes();
                let mut writes = [GuestWrite {
                    address: 0,
                    bytes: &[],
                }; 2];
                let mut used = 0;
                if a[3] != 0 {
                    writes[used] = GuestWrite {
                        address: a[3],
                        bytes: &size,
                    };
                    used += 1;
                }
                if count != 0 {
                    let end = a[1]
                        .checked_add(count as u64)
                        .ok_or(GuestError::AddressOverflow)?;
                    if a[3] != 0 {
                        let size_end = a[3].checked_add(8).ok_or(GuestError::AddressOverflow)?;
                        if a[1] < size_end && a[3] < end {
                            return Err(GuestError::UnsupportedMapping.into());
                        }
                    }
                    writes[used] = GuestWrite {
                        address: a[1],
                        bytes: &bytes[..count],
                    };
                    used += 1;
                }
                if used != 0 {
                    budget.write(guest, &writes[..used])?;
                }
                Ok(0)
            }
            _ => Err(Failure::Unsupported("UDS event service")),
        }
    }
}
