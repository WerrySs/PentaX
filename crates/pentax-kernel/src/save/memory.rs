// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: retained save memory with atomic persistent updates.
// Adapted from pinned libSaveData.cpp; see NOTICE.

use super::{EVENT_BYTES, EVENT_COUNT, SaveStorage};
use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Service};
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const PARAM_BYTES: usize = 1328;
pub(super) const SOURCE_MAX: u64 = 32 * 1024 * 1024;
const PARAMETER: u64 = 0x809f_0000;
const BUSY: u64 = 0x809f_0003;
const INVALID_USER: u64 = 0x809f_0011;
const NOT_READY: u64 = 0x809f_0012;
const LIMITATION: u64 = 0x809f_0017;

pub(super) enum Error {
    Code(u64),
    Policy(Failure),
}
impl From<Failure> for Error {
    fn from(value: Failure) -> Self {
        Self::Policy(value)
    }
}
struct Memory {
    user: i32,
    slot: u32,
    option: u32,
    data: Vec<u8>,
    param: [u8; PARAM_BYTES],
}
pub(super) struct SaveMemory {
    entries: Vec<Memory>,
    storage: Option<SaveStorage>,
}
fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn add(address: u64, offset: u64) -> Result<u64, Error> {
    address
        .checked_add(offset)
        .ok_or(Error::Policy(Failure::Guest(GuestError::AddressOverflow)))
}
fn read<const N: usize>(
    guest: &impl GuestAccess,
    budget: &mut Budget,
    address: u64,
) -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    budget.read(guest, address, &mut bytes)?;
    Ok(bytes)
}
fn allocate(size: usize, budget: &mut Budget) -> Result<Vec<u8>, Error> {
    budget.work(size as u64)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| Error::Policy(Failure::Allocation))?;
    bytes.resize(size, 0);
    Ok(bytes)
}
fn range(descriptor: &[u8], length: usize) -> Result<(u64, usize, usize), Error> {
    let buffer = u64_at(descriptor, 0);
    let size = u64_at(descriptor, 8);
    let offset = u64_at(descriptor, 16) as i64;
    if buffer == 0
        || offset < 0
        || offset as u64 > length as u64
        || size > length as u64 - offset as u64
    {
        return Err(Error::Code(PARAMETER));
    }
    add(buffer, size)?;
    Ok((buffer, offset as usize, size as usize))
}
impl SaveMemory {
    pub(super) fn new() -> Result<Self, Failure> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(4)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            entries,
            storage: None,
        })
    }
    pub(super) fn clear(&mut self, budget: &mut Budget) -> Result<(), Failure> {
        budget.work(self.entries.len() as u64)?;
        self.entries.clear();
        budget.retained_save_memory(0);
        Ok(())
    }
    fn index(&self, user: i32, slot: u32) -> Result<usize, Error> {
        self.entries
            .iter()
            .position(|entry| entry.user == user && entry.slot == slot)
            .ok_or(Error::Code(NOT_READY))
    }
    pub(super) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        events: &mut VecDeque<[u8; EVENT_BYTES]>,
    ) -> Result<u64, Failure> {
        match self.dispatch(service, args, guest, budget, events) {
            Ok(value) | Err(Error::Code(value)) => Ok(value),
            Err(Error::Policy(error)) => Err(error),
        }
    }
    fn dispatch(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        events: &mut VecDeque<[u8; EVENT_BYTES]>,
    ) -> Result<u64, Error> {
        if args.0[0] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        match service {
            Service::SaveMemorySetup => self.setup(args, guest, budget),
            Service::SaveMemoryGet => self.get(args.0[0], guest, budget),
            Service::SaveMemorySet => self.set(args.0[0], guest, budget),
            Service::SaveMemorySync => self.sync(args.0[0], guest, budget, events),
            _ => Err(Failure::Unsupported("save memory service").into()),
        }
    }
    fn setup(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Error> {
        let input = read::<44>(guest, budget, args.0[0])?;
        let option = u32_at(&input, 0);
        let user = u32_at(&input, 4) as i32;
        let size = u64_at(&input, 8);
        let slot = u32_at(&input, 40);
        if slot >= 4 || size == 0 || size > SOURCE_MAX || option & !3 != 0 {
            return Err(Error::Code(PARAMETER));
        }
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        if self
            .entries
            .iter()
            .any(|entry| entry.user == user && entry.slot == slot)
        {
            return Err(Error::Code(BUSY));
        }
        let retained = self
            .entries
            .iter()
            .map(|entry| entry.data.len() as u64 * if entry.option & 2 != 0 { 2 } else { 1 })
            .sum::<u64>();
        let next = retained + size * if option & 2 != 0 { 2 } else { 1 };
        if self.entries.len() >= 4 || next > SOURCE_MAX {
            return Err(Error::Code(LIMITATION));
        }
        budget.admit_save_memory(next)?;
        let storage = self
            .storage
            .as_ref()
            .ok_or(Failure::Unsupported("bound save storage"))?;
        let mut data = allocate(size as usize, budget)?;
        let mut param = [0; PARAM_BYTES];
        param[..11].copy_from_slice(b"Saved Data\0");
        let existed = storage.load(user, slot, &mut data, &mut param, budget)?;
        let initial = u64_at(&input, 24);
        if existed == 0 && initial != 0 && option & 1 != 0 {
            budget.scratch(PARAM_BYTES)?;
            budget.record_scratch(PARAM_BYTES);
            param = read::<PARAM_BYTES>(guest, budget, initial)?;
            param[1288..1296].fill(0);
        }
        if args.0[1] != 0 {
            let mut result = [0; 24];
            result[..8].copy_from_slice(&(existed as u64).to_le_bytes());
            budget.write(
                guest,
                &[GuestWrite {
                    address: args.0[1],
                    bytes: &result,
                }],
            )?;
        }
        self.entries.push(Memory {
            user,
            slot,
            option,
            data,
            param,
        });
        budget.retained_save_memory(next);
        Ok(0)
    }
    fn get(
        &self,
        address: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Error> {
        let input = read::<36>(guest, budget, address)?;
        let user = u32_at(&input, 0) as i32;
        let slot = u32_at(&input, 32);
        if slot >= 4 {
            return Err(Error::Code(PARAMETER));
        }
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        let entry = &self.entries[self.index(user, slot)?];
        let descriptor = u64_at(&input, 8);
        let param = u64_at(&input, 16);
        let icon = u64_at(&input, 24);
        if param != 0 && entry.option & 1 == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let data = if descriptor != 0 {
            Some(range(
                &read::<24>(guest, budget, descriptor)?,
                entry.data.len(),
            )?)
        } else {
            None
        };
        let zero = 0_u64.to_le_bytes();
        let mut writes = [GuestWrite {
            address: 0,
            bytes: &[],
        }; 3];
        let mut count = 0;
        if let Some((buffer, offset, size)) = data {
            writes[count] = GuestWrite {
                address: buffer,
                bytes: &entry.data[offset..offset + size],
            };
            count += 1;
        }
        if param != 0 {
            writes[count] = GuestWrite {
                address: param,
                bytes: &entry.param,
            };
            count += 1;
        }
        if icon != 0 {
            writes[count] = GuestWrite {
                address: add(icon, 16)?,
                bytes: &zero,
            };
            count += 1;
        }
        if count != 0 {
            budget.write(guest, &writes[..count])?;
        }
        Ok(0)
    }
    fn set(
        &mut self,
        address: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Error> {
        let input = read::<40>(guest, budget, address)?;
        let user = u32_at(&input, 0) as i32;
        let slot = u32_at(&input, 36);
        let raw_count = u32_at(&input, 32);
        let descriptors = u64_at(&input, 8);
        let param_address = u64_at(&input, 16);
        if slot >= 4 || raw_count > 5 || descriptors == 0 && raw_count != 0 {
            return Err(Error::Code(PARAMETER));
        }
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        let index = self.index(user, slot)?;
        let entry = &self.entries[index];
        if param_address != 0 && entry.option & 1 == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let count = if descriptors != 0 && raw_count == 0 {
            1
        } else {
            raw_count as usize
        };
        let mut ranges = [(0, 0, 0); 5];
        for (i, target) in ranges[..count].iter_mut().enumerate() {
            *target = range(
                &read::<24>(guest, budget, add(descriptors, i as u64 * 64)?)?,
                entry.data.len(),
            )?;
        }
        if count == 0 && param_address == 0 {
            return Ok(0);
        }
        let mut data = allocate(entry.data.len(), budget)?;
        data.copy_from_slice(&entry.data);
        let mut param = entry.param;
        for &(buffer, offset, size) in &ranges[..count] {
            budget.read(guest, buffer, &mut data[offset..offset + size])?;
        }
        if param_address != 0 {
            budget.scratch(PARAM_BYTES)?;
            budget.record_scratch(PARAM_BYTES);
            param = read::<PARAM_BYTES>(guest, budget, param_address)?;
        }
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Failure::Unsupported("save clock epoch"))?
            .as_secs();
        let seconds = i64::try_from(seconds).map_err(|_| Failure::Limit("save timestamp"))?;
        param[1288..1296].copy_from_slice(&seconds.to_le_bytes());
        self.storage
            .as_ref()
            .ok_or(Failure::Unsupported("bound save storage"))?
            .store(user, slot, &data, &param, budget)?;
        self.entries[index].data = data;
        self.entries[index].param = param;
        Ok(0)
    }
    fn sync(
        &self,
        address: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        events: &mut VecDeque<[u8; EVENT_BYTES]>,
    ) -> Result<u64, Error> {
        let input = read::<12>(guest, budget, address)?;
        let user = u32_at(&input, 0) as i32;
        let slot = u32_at(&input, 4);
        let option = u32_at(&input, 8);
        if slot >= 4 || option > 1 {
            return Err(Error::Code(PARAMETER));
        }
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        let entry = &self.entries[self.index(user, slot)?];
        let mut event = [0; EVENT_BYTES];
        if option == 0 {
            budget.scratch(EVENT_BYTES)?;
            budget.work(EVENT_BYTES as u64 + 1)?;
            budget.record_scratch(EVENT_BYTES);
            event[..4].copy_from_slice(&3_u32.to_le_bytes());
            event[8..12].copy_from_slice(&user.to_le_bytes());
            event[16..24].copy_from_slice(b"UNKNOWN\0");
            let name = if slot == 0 {
                "sce_sdmemory".to_owned()
            } else {
                format!("sce_sdmemory{slot}")
            };
            event[32..32 + name.len()].copy_from_slice(name.as_bytes());
        }
        let status = match self
            .storage
            .as_ref()
            .ok_or(Failure::Unsupported("bound save storage"))?
            .store(user, slot, &entry.data, &entry.param, budget)
        {
            Ok(()) => 0,
            Err(Error::Code(code)) => code,
            Err(error) => return Err(error),
        };
        if option == 1 {
            return Ok(status);
        }
        event[4..8].copy_from_slice(&(status as u32).to_le_bytes());
        if events.len() == EVENT_COUNT {
            events.pop_front();
        }
        events.push_back(event);
        Ok(0)
    }
}
impl crate::Kernel {
    /// Bind an explicitly selected per-title writable directory before save use.
    pub fn configure_save_storage(&mut self, storage: SaveStorage) -> Result<(), Failure> {
        if self.save.initialized
            || self.save.memory.storage.is_some()
            || !self.save.memory.entries.is_empty()
        {
            return Err(Failure::Unsupported("active save storage configuration"));
        }
        self.save.memory.storage = Some(storage);
        Ok(())
    }
}

impl super::SaveState {
    pub(crate) fn trophy_storage(&self) -> Option<&SaveStorage> {
        self.memory.storage.as_ref()
    }
}
