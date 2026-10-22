// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026: checked directory-save mounts within explicit capabilities; see NOTICE.
use super::{EVENT_BYTES, EVENT_COUNT, SaveStorage, memory::Error};
use crate::budget::Budget;
use crate::filesystem::FileState;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, MemoryEffect, Service};
use cap_std::fs::Dir;
use std::collections::VecDeque;

pub(super) const PARAMETER: u64 = 0x809f_0000;
pub(super) const BUSY: u64 = 0x809f_0003;
pub(super) const NOT_MOUNTED: u64 = 0x809f_0004;
pub(super) const NO_PERMISSION: u64 = 0x809f_0005;
pub(super) const EXISTS: u64 = 0x809f_0007;
pub(super) const NOT_FOUND: u64 = 0x809f_0008;
pub(super) const INTERNAL: u64 = 0x809f_000b;
pub(super) const MOUNT_FULL: u64 = 0x809f_000c;
pub(super) const BROKEN: u64 = 0x809f_000f;
pub(super) const INVALID_USER: u64 = 0x809f_0011;
const COUNT: usize = 16;

pub(super) fn valid_component(name: &[u8]) -> bool {
    !name.is_empty()
        && name != b"."
        && name != b".."
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.@".contains(byte))
}

pub(super) fn component<const N: usize>(bytes: [u8; N]) -> Result<[u8; N], Error> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::Code(PARAMETER))?;
    if !valid_component(&bytes[..end]) {
        return Err(Error::Code(PARAMETER));
    }
    let mut result = bytes;
    result[end..].fill(0);
    Ok(result)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SaveKey {
    pub title: [u8; 10],
    pub name: [u8; 32],
    pub user: Option<i32>,
}
impl SaveKey {
    fn new(user: i32, title: [u8; 10], name: [u8; 32]) -> Self {
        let mut key = Self {
            title,
            name,
            user: None,
        };
        if key.memory_slot().is_some() {
            key.user = Some(user);
        }
        key
    }
    pub(super) fn memory_slot(&self) -> Option<u32> {
        let name = &self.name[..self.name.iter().position(|byte| *byte == 0)?];
        match name {
            b"sce_sdmemory" => Some(0),
            b"sce_sdmemory1" => Some(1),
            b"sce_sdmemory2" => Some(2),
            b"sce_sdmemory3" => Some(3),
            _ => None,
        }
    }
    fn memory(title: [u8; 10], user: i32, slot: u32) -> Self {
        let mut name = [0; 32];
        name[..12].copy_from_slice(b"sce_sdmemory");
        if slot != 0 {
            name[12] = b'0' + slot as u8;
        }
        Self {
            title,
            name,
            user: Some(user),
        }
    }
}

pub(super) struct Mount {
    pub directory: Dir,
    pub key: SaveKey,
}
pub(super) struct DirectoryState {
    mounts: [Option<Mount>; COUNT],
}
impl DirectoryState {
    pub(super) fn new() -> Self {
        Self {
            mounts: std::array::from_fn(|_| None),
        }
    }
    pub(super) fn live(&self) -> bool {
        self.mounts.iter().any(Option::is_some)
    }
    pub(super) fn memory_busy(&self, title: [u8; 10], user: i32, slot: u32) -> bool {
        let key = SaveKey::memory(title, user, slot);
        self.mounts.iter().flatten().any(|mount| mount.key == key)
    }
    fn available(&self, key: SaveKey) -> Result<usize, Error> {
        if self.mounts.iter().flatten().any(|mount| mount.key == key) {
            return Err(Error::Code(BUSY));
        }
        self.mounts
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Code(MOUNT_FULL))
    }
    fn find(&self, point: &[u8; 16]) -> Result<usize, Error> {
        let length = point
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(Error::Code(PARAMETER))?;
        self.mounts
            .iter()
            .enumerate()
            .find(|(slot, mounted)| {
                let expected = mount_point(*slot);
                mounted.is_some()
                    && expected[..length] == point[..length]
                    && expected.get(length) == Some(&0)
            })
            .map(|(slot, _)| slot)
            .ok_or(Error::Code(NOT_MOUNTED))
    }
    pub(super) fn mounted(&self, point: &[u8; 16]) -> Result<&Mount, Error> {
        self.mounts[self.find(point)?]
            .as_ref()
            .ok_or(Error::Code(NOT_MOUNTED))
    }
    fn equivalent_busy(&self, directory: &Dir, budget: &mut Budget) -> Result<(), Error> {
        // Encoded ASCII components and non-following backend lookup exclude
        // Windows case/DOS/reparse aliases. Unix also has stable inode identity
        // for host-provided equivalent directories (for example a bind mount).
        budget.work(COUNT as u64 * 256)?;
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;
            // The source uses equivalent(..., error_code): an unavailable
            // identity is a failed comparison, not a replacement save error.
            let Ok(target) = directory.dir_metadata() else {
                return Ok(());
            };
            for mount in self.mounts.iter().flatten() {
                let Ok(current) = mount.directory.dir_metadata() else {
                    continue;
                };
                if target.dev() == current.dev() && target.ino() == current.ino() {
                    return Err(Error::Code(BUSY));
                }
            }
        }
        #[cfg(not(unix))]
        let _ = directory;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        storage: Option<&SaveStorage>,
        memory: &super::memory::SaveMemory,
        events: &mut VecDeque<[u8; EVENT_BYTES]>,
        files: &mut FileState,
    ) -> Result<u64, Error> {
        match service {
            Service::SaveTransferringMount | Service::SaveMount3 => {
                self.mount(service, args, guest, budget, storage, files)
            }
            Service::SaveUmount2 => self.unmount(args, guest, budget, events, files),
            Service::SaveGetMountInfo => self.info(args, guest, budget, storage),
            Service::SaveDelete => self.delete(args, guest, budget, storage, memory),
            Service::SaveBackup => self.backup(args, guest, budget, events),
            _ => {
                if args.0[0] == 0 {
                    return Err(Error::Code(PARAMETER));
                }
                self.invoke_metadata(
                    service,
                    args,
                    guest,
                    budget,
                    storage.ok_or(Error::Policy(Failure::Unsupported("bound save storage")))?,
                    files,
                )
            }
        }
    }

    fn mount(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        storage: Option<&SaveStorage>,
        files: &mut FileState,
    ) -> Result<u64, Error> {
        if args.0[0] == 0 || args.0[1] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let fallback = storage
            .map(SaveStorage::current_title)
            .unwrap_or(*b"UNKNOWN\0\0\0");
        let _ = mount_input(service, args.0[0], guest, budget, fallback)?;
        // This visible source store precedes BUSY, FULL, missing-directory and
        // creation validation. A failed later admission leaves exactly zeros.
        budget.write(
            guest,
            &[GuestWrite {
                address: args.0[1],
                bytes: &[0; 64],
            }],
        )?;
        // Re-read fields used after the source memset: aliased guest outputs
        // cannot cause the emulator to retain pre-store path or mode values.
        let request = mount_input(service, args.0[0], guest, budget, fallback)?;
        let slot = match self.available(request.key) {
            Err(Error::Code(MOUNT_FULL)) => {
                // FindAvailable compares physical identity before returning
                // FULL. Probe only an existing capability; never create a
                // save or validate its mode on this path. Source filesystem
                // comparison errors leave the FULL result unchanged.
                if let Some(storage) = storage {
                    budget.work(4096)?;
                    if let Ok(Some(directory)) = storage.open_save(&request.key) {
                        self.equivalent_busy(&directory, budget)?;
                    }
                }
                return Err(Error::Code(MOUNT_FULL));
            }
            result => result?,
        };
        // Physical BUSY also precedes mode validation and CREATE's EXISTS
        // result when a slot is free. Defer backing lookup errors until after
        // mode validation, just as source equivalent(error_code) ignores them.
        let existing = if let Some(storage) = storage {
            budget.work(4096)?;
            Some(storage.open_save(&request.key))
        } else {
            None
        };
        if let Some(Ok(Some(directory))) = &existing {
            self.equivalent_busy(directory, budget)?;
        }
        if !request.valid_mode {
            return Err(Error::Policy(Failure::Unsupported(
                "source-aborting save mount mode",
            )));
        }
        budget.reserve_write(args.0[1], 64)?;
        let storage = storage.ok_or(Error::Policy(Failure::Unsupported("bound save storage")))?;
        let existing =
            existing.ok_or(Error::Policy(Failure::Unsupported("bound save storage")))??;
        let mut created = false;
        let directory = match existing {
            Some(_) if request.create => return Err(Error::Code(EXISTS)),
            Some(directory) => directory,
            None if request.create || request.create2 => {
                if !(48..=16384).contains(&request.blocks) {
                    return Err(Error::Code(PARAMETER));
                }
                created = true;
                storage.create_save(&request.key, request.blocks, budget)?
            }
            None => return Err(Error::Code(NOT_FOUND)),
        };
        let admission = (|| {
            let filesystem = files.prepare_save_mount(
                slot,
                directory.try_clone().map_err(|_| Error::Code(INTERNAL))?,
                budget,
            )?;
            let mut output = [0; 64];
            output[..16].copy_from_slice(&mount_point(slot));
            output[28..32].copy_from_slice(&u32::from(created).to_le_bytes());
            guest
                .apply(
                    MemoryEffect::None,
                    &[GuestWrite {
                        address: args.0[1],
                        bytes: &output,
                    }],
                )
                .map_err(Failure::Guest)?;
            Ok(filesystem)
        })();
        let prepared = match admission {
            Ok(prepared) => prepared,
            Err(error) => {
                drop(directory);
                if created {
                    storage.rollback_created(&request.key)?;
                }
                return Err(error);
            }
        };
        // Every allocation, host operation and guest output has succeeded.
        // These two owner-local publications are infallible and cannot interleave.
        files.commit_save_mount(prepared);
        self.mounts[slot] = Some(Mount {
            directory,
            key: request.key,
        });
        Ok(0)
    }

    fn unmount(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
        events: &mut VecDeque<[u8; EVENT_BYTES]>,
        files: &mut FileState,
    ) -> Result<u64, Error> {
        if args.0[1] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let point = read::<16>(guest, budget, args.0[1])?;
        let slot = match self.find(&point) {
            Err(Error::Code(NOT_MOUNTED)) => return Err(Error::Code(NOT_FOUND)),
            result => result?,
        };
        budget.work(256)?;
        if args.0[0] as u32 & (1 << 16) != 0 {
            budget.scratch(EVENT_BYTES)?;
            budget.work(EVENT_BYTES as u64 + 1)?;
            budget.record_scratch(EVENT_BYTES);
            let mut event = [0; EVENT_BYTES];
            event[..4].copy_from_slice(&1u32.to_le_bytes());
            if events.len() == EVENT_COUNT {
                events.pop_front();
            }
            events.push_back(event);
        }
        files.unmount_save(slot);
        self.mounts[slot] = None;
        Ok(0)
    }

    fn info(
        &self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        storage: Option<&SaveStorage>,
    ) -> Result<u64, Error> {
        if args.0[0] == 0 || args.0[1] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let point = read::<16>(guest, budget, args.0[0])?;
        let mounted = self.mounted(&point)?;
        let blocks = storage
            .ok_or(Error::Policy(Failure::Unsupported("bound save storage")))?
            .read_blocks(&mounted.directory, budget)?;
        let mut output = [0; 48];
        output[..8].copy_from_slice(&blocks.to_le_bytes());
        output[8..16].copy_from_slice(&blocks.to_le_bytes());
        budget.write(
            guest,
            &[GuestWrite {
                address: args.0[1],
                bytes: &output,
            }],
        )?;
        Ok(0)
    }

    fn delete(
        &self,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
        storage: Option<&SaveStorage>,
        memory: &super::memory::SaveMemory,
    ) -> Result<u64, Error> {
        if args.0[0] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let input = read::<24>(guest, budget, args.0[0])?;
        let user = u32_at(&input, 0) as i32;
        let title_pointer = u64_at(&input, 8);
        let directory_pointer = u64_at(&input, 16);
        if directory_pointer == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let name = component(read::<32>(guest, budget, directory_pointer)?)?;
        let title = if title_pointer != 0 {
            component(read::<10>(guest, budget, title_pointer)?)?
        } else {
            storage
                .map(SaveStorage::current_title)
                .unwrap_or(*b"UNKNOWN\0\0\0")
        };
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        let key = SaveKey::new(user, title, name);
        if self.mounts.iter().flatten().any(|mount| mount.key == key)
            || memory.contains_directory(&key, storage.map(SaveStorage::current_title))
        {
            return Err(Error::Code(BUSY));
        }
        let storage = storage.ok_or(Error::Policy(Failure::Unsupported("bound save storage")))?;
        if let Some(directory) = storage.open_save(&key)? {
            self.equivalent_busy(&directory, budget)?;
        }
        storage.delete_save(&key, budget)?;
        Ok(0)
    }

    fn backup(
        &self,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
        events: &mut VecDeque<[u8; EVENT_BYTES]>,
    ) -> Result<u64, Error> {
        if args.0[0] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let input = read::<24>(guest, budget, args.0[0])?;
        if u64_at(&input, 16) == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let mut event = [0; EVENT_BYTES];
        event[..4].copy_from_slice(&2u32.to_le_bytes());
        event[8..12].copy_from_slice(&input[..4]);
        if u64_at(&input, 8) != 0 {
            event[16..32].copy_from_slice(&read::<16>(guest, budget, u64_at(&input, 8))?);
        }
        event[32..64].copy_from_slice(&read::<32>(guest, budget, u64_at(&input, 16))?);
        budget.scratch(EVENT_BYTES)?;
        budget.work(EVENT_BYTES as u64 + 1)?;
        budget.record_scratch(EVENT_BYTES);
        if events.len() == EVENT_COUNT {
            events.pop_front();
        }
        events.push_back(event);
        Ok(0)
    }
}

fn mount_point(slot: usize) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..9].copy_from_slice(b"/savedata");
    if slot < 10 {
        bytes[9] = b'0' + slot as u8;
    } else {
        bytes[9] = b'1';
        bytes[10] = b'0' + (slot - 10) as u8;
    }
    bytes
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
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
struct Request {
    key: SaveKey,
    blocks: u64,
    create: bool,
    create2: bool,
    valid_mode: bool,
}
fn mount_input(
    service: Service,
    address: u64,
    guest: &impl GuestAccess,
    budget: &mut Budget,
    fallback: [u8; 10],
) -> Result<Request, Error> {
    if service == Service::SaveTransferringMount {
        let input = read::<24>(guest, budget, address)?;
        if u64_at(&input, 8) == 0 || u64_at(&input, 16) == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let title = component(read::<10>(guest, budget, u64_at(&input, 8))?)?;
        let name = component(read::<32>(guest, budget, u64_at(&input, 16))?)?;
        let user = u32_at(&input, 0) as i32;
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        Ok(Request {
            key: SaveKey::new(user, title, name),
            blocks: 0,
            create: false,
            create2: false,
            valid_mode: true,
        })
    } else {
        let input = read::<36>(guest, budget, address)?;
        if u64_at(&input, 8) == 0 {
            return Err(Error::Code(PARAMETER));
        }
        let name = component(read::<32>(guest, budget, u64_at(&input, 8))?)?;
        let user = u32_at(&input, 0) as i32;
        if user < 0 {
            return Err(Error::Code(INVALID_USER));
        }
        let mode = u32_at(&input, 32);
        let create = mode & 4 != 0;
        let create2 = mode & 32 != 0;
        Ok(Request {
            key: SaveKey::new(user, fallback, name),
            blocks: u64_at(&input, 16),
            create,
            create2,
            valid_mode: create || create2 || mode & 3 != 0,
        })
    }
}

#[cfg(test)]
mod tests;
