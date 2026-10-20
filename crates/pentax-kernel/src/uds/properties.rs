// SPDX-License-Identifier: GPL-2.0-only
// Pinned libTrophy.cpp:565-575,810-1228; exact source attribution in parent.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum PropertyOwner {
    Object(usize),
    Event(usize),
}

pub(super) fn cstring(
    address: u64,
    held: usize,
    guest: &impl GuestAccess,
    budget: &mut Budget,
) -> Result<Vec<u8>, Failure> {
    let mut bytes = Vec::new();
    for index in 0..=MAX_STRING {
        let address = address
            .checked_add(index as u64)
            .ok_or(GuestError::AddressOverflow)?;
        let mut byte = [0];
        budget.read(guest, address, &mut byte)?;
        if byte[0] == 0 {
            return Ok(bytes);
        }
        if index == MAX_STRING {
            break;
        }
        let next = growth(&bytes, 1, MAX_STRING, add(held, capacity(&bytes))?, budget)?;
        insert(&mut bytes, next, byte[0]);
    }
    Err(Failure::Limit("UDS string bytes"))
}

pub(super) fn clone_properties(
    source: &[UdsProperty],
    held: usize,
    budget: &mut Budget,
) -> Result<Vec<UdsProperty>, Failure> {
    budget.work(source.len() as u64 + 1)?;
    let mut copy = Vec::new();
    let prepared = growth(&copy, source.len(), MAX_KEYS, held, budget)?;
    if let Some(prepared) = prepared {
        copy = prepared;
    }
    let mut extra = capacity(&copy);
    for property in source {
        let mut key = Vec::new();
        let next = growth(
            &key,
            property.key.len(),
            MAX_STRING,
            add(held, extra)?,
            budget,
        )?;
        if let Some(next) = next {
            key = next;
        }
        budget.work(property.key.len() as u64 + 1)?;
        key.extend_from_slice(&property.key);
        extra = add(extra, key.capacity())?;
        copy.push(UdsProperty {
            key,
            value: property.value,
        });
    }
    Ok(copy)
}

impl UdsState {
    pub(super) fn property_owner(
        &self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<PropertyOwner, Failure> {
        for (index, object) in self.objects.iter().enumerate() {
            if object.runtime.address == address {
                memory.check_runtime_object(object.runtime, budget)?;
                return Ok(PropertyOwner::Object(index));
            }
        }
        for (index, event) in self.events.iter().enumerate() {
            if event.runtime.address + 8 == address {
                memory.check_runtime_object(event.runtime, budget)?;
                return Ok(PropertyOwner::Event(index));
            }
        }
        Err(Failure::Unsupported("UDS property object owner"))
    }
    pub(super) fn property_table(&self, owner: PropertyOwner) -> &Vec<UdsProperty> {
        match owner {
            PropertyOwner::Object(index) => &self.objects[index].properties,
            PropertyOwner::Event(index) => &self.events[index].properties,
        }
    }
    fn property_table_mut(&mut self, owner: PropertyOwner) -> &mut Vec<UdsProperty> {
        match owner {
            PropertyOwner::Object(index) => &mut self.objects[index].properties,
            PropertyOwner::Event(index) => &mut self.events[index].properties,
        }
    }
    fn array_index(
        &self,
        address: u64,
        memory: &MemoryState,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        let index = self
            .arrays
            .iter()
            .position(|array| array.runtime.address == address)
            .ok_or(Failure::Unsupported("UDS property array owner"))?;
        memory.check_runtime_object(self.arrays[index].runtime, budget)?;
        Ok(index)
    }
    fn property_index(
        &self,
        owner: PropertyOwner,
        key: &[u8],
        budget: &mut Budget,
    ) -> Result<Result<usize, usize>, Failure> {
        let properties = self.property_table(owner);
        let mut insertion = properties.len();
        for (index, property) in properties.iter().enumerate() {
            budget.work(property.key.len() as u64 + key.len() as u64 + 1)?;
            match property.key.as_slice().cmp(key) {
                core::cmp::Ordering::Equal => {
                    budget.work(properties.len() as u64 + 1)?;
                    return Ok(Ok(index));
                }
                core::cmp::Ordering::Greater => {
                    insertion = index;
                    break;
                }
                core::cmp::Ordering::Less => {}
            }
        }
        budget.work(properties.len() as u64 + 1)?; // Subsequent insertion/removal shift.
        Ok(Err(insertion))
    }

    fn create_empty(
        &mut self,
        array: bool,
        destination: u64,
        held_extra: usize,
        resources: &mut UdsResources<'_>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        if self.retained_handle_count() >= MAX_RECORDS {
            return Err(Failure::Limit("UDS record count"));
        }
        // Actual backing is a separate retained resource from its logical owner.
        budget.replacement_handles(0, 2)?;
        let held = add(
            add(
                self.metadata_bytes(budget)?,
                resources.trophies.metadata_bytes(budget)?,
            )?,
            held_extra,
        )?;
        if array {
            let prepared = growth(&self.arrays, 1, MAX_RECORDS, held, budget)?;
            let scratch = add(held, prepared.as_ref().map_or(0, capacity))?;
            let runtime =
                resources
                    .memory
                    .runtime_object(8, scratch, guest, budget, |base, _bytes| {
                        let mut writes = RuntimeDataOutputs::new();
                        writes.push(destination, &base.to_le_bytes())?;
                        Ok(writes)
                    })?;
            insert(&mut self.arrays, prepared, Array { runtime });
        } else {
            let prepared = growth(&self.objects, 1, MAX_RECORDS, held, budget)?;
            let scratch = add(held, prepared.as_ref().map_or(0, capacity))?;
            let runtime =
                resources
                    .memory
                    .runtime_object(8, scratch, guest, budget, |base, _bytes| {
                        let mut writes = RuntimeDataOutputs::new();
                        writes.push(destination, &base.to_le_bytes())?;
                        Ok(writes)
                    })?;
            insert(
                &mut self.objects,
                prepared,
                Object {
                    runtime,
                    properties: Vec::new(),
                },
            );
        }
        budget.added_handle();
        Ok(())
    }

    pub(super) fn properties(
        &mut self,
        service: Service,
        a: [u64; 6],
        mut resources: UdsResources<'_>,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        use Service::*;
        match service {
            UdsCreateEventPropertyObject | UdsCreateEventPropertyArray => {
                if a[0] == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                self.create_empty(
                    service == UdsCreateEventPropertyArray,
                    a[0],
                    0,
                    &mut resources,
                    guest,
                    budget,
                )?;
                Ok(0)
            }
            UdsDestroyEventPropertyObject => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let owner = self.property_owner(a[0], resources.memory, budget)?;
                let PropertyOwner::Object(index) = owner else {
                    return Err(Failure::Unsupported("UDS embedded property destruction"));
                };
                budget.work(
                    self.objects[index].properties.len() as u64 + self.objects.len() as u64 + 1,
                )?;
                resources.memory.release_runtime_object(
                    self.objects[index].runtime,
                    guest,
                    budget,
                )?;
                self.objects.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            UdsDestroyEventPropertyArray => {
                if a[0] == 0 {
                    return Ok(0);
                }
                let index = self.array_index(a[0], resources.memory, budget)?;
                resources.memory.release_runtime_object(
                    self.arrays[index].runtime,
                    guest,
                    budget,
                )?;
                self.arrays.remove(index);
                budget.removed_handle();
                Ok(0)
            }
            UdsEventPropertyObjectSetString
            | UdsEventPropertyObjectSetInt32
            | UdsEventPropertyObjectSetUInt32
            | UdsEventPropertyObjectSetInt64
            | UdsEventPropertyObjectSetUInt64
            | UdsEventPropertyObjectSetFloat32
            | UdsEventPropertyObjectSetFloat64
            | UdsEventPropertyObjectSetBool
            | UdsEventPropertyObjectSetBinary
            | UdsEventPropertyObjectSetObject
            | UdsEventPropertyObjectSetArray => {
                if a[0] == 0
                    || a[1] == 0
                    || (service == UdsEventPropertyObjectSetString && a[2] == 0)
                    || (service == UdsEventPropertyObjectSetBinary && a[2] == 0 && a[3] != 0)
                {
                    return Ok(INVALID_ARGUMENT);
                }
                let owner = self.property_owner(a[0], resources.memory, budget)?;
                let held = add(
                    self.metadata_bytes(budget)?,
                    resources.trophies.metadata_bytes(budget)?,
                )?;
                let key = cstring(a[1], held, guest, budget)?;
                let at = self.property_index(owner, &key, budget)?;
                let value = match service {
                    UdsEventPropertyObjectSetInt32 => Some(UdsInteger::I32(a[2] as u32 as i32)),
                    UdsEventPropertyObjectSetUInt32 => Some(UdsInteger::U32(a[2] as u32)),
                    UdsEventPropertyObjectSetUInt64 => Some(UdsInteger::U64(a[2])),
                    _ => None,
                };
                if let Some(value) = value {
                    match at {
                        Ok(index) => self.property_table_mut(owner)[index].value = value,
                        Err(index) => {
                            if self.key_count() == MAX_KEYS {
                                return Err(Failure::Limit("UDS integer keys"));
                            }
                            let staged = add(held, key.capacity())?;
                            let prepared =
                                growth(self.property_table(owner), 1, MAX_KEYS, staged, budget)?;
                            let table = self.property_table_mut(owner);
                            if let Some(mut next) = prepared {
                                next.append(table);
                                *table = next;
                            }
                            table.insert(index, UdsProperty { key, value });
                        }
                    }
                    return Ok(0);
                }
                if matches!(
                    service,
                    UdsEventPropertyObjectSetObject | UdsEventPropertyObjectSetArray
                ) && a[3] != 0
                {
                    if a[2] != 0 {
                        output(a[3], a[2], guest, budget)?;
                    } else {
                        self.create_empty(
                            service == UdsEventPropertyObjectSetArray,
                            a[3],
                            key.capacity(),
                            &mut resources,
                            guest,
                            budget,
                        )?;
                    }
                }
                // Source removes integer state for all non-integer setters;
                // ignored values never become nested ownership or guest reads.
                if let Ok(index) = at {
                    self.property_table_mut(owner).remove(index);
                }
                Ok(0)
            }
            UdsEventPropertyArraySetString
            | UdsEventPropertyArraySetInt32
            | UdsEventPropertyArraySetUInt32
            | UdsEventPropertyArraySetInt64
            | UdsEventPropertyArraySetUInt64
            | UdsEventPropertyArraySetFloat32
            | UdsEventPropertyArraySetFloat64
            | UdsEventPropertyArraySetBool
            | UdsEventPropertyArraySetBinary
            | UdsEventPropertyArraySetObject
            | UdsEventPropertyArraySetArray => {
                if a[0] == 0
                    || (service == UdsEventPropertyArraySetString && a[1] == 0)
                    || (service == UdsEventPropertyArraySetBinary && a[1] == 0 && a[2] != 0)
                {
                    return Ok(INVALID_ARGUMENT);
                }
                self.array_index(a[0], resources.memory, budget)?;
                if matches!(
                    service,
                    UdsEventPropertyArraySetObject | UdsEventPropertyArraySetArray
                ) && a[2] != 0
                {
                    if a[1] != 0 {
                        output(a[2], a[1], guest, budget)?;
                    } else {
                        self.create_empty(
                            service == UdsEventPropertyArraySetArray,
                            a[2],
                            0,
                            &mut resources,
                            guest,
                            budget,
                        )?;
                    }
                }
                // Empty source array: no appended elements or value ownership.
                Ok(0)
            }
            _ => Err(Failure::Unsupported("UDS service")),
        }
    }
}
