// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded directory metadata/search and real icons.
// Source ABI: pinned libSaveData.cpp; see NOTICE. Icon persistence is original.

use super::{
    SaveStorage,
    directories::{DirectoryState, SaveKey},
    memory::{Error, PARAM_BYTES, SOURCE_MAX},
};
use crate::{
    CallArgs, Errno, Failure, GuestAccess, GuestWrite, Service, budget::Budget,
    filesystem::FileState,
};
use cap_std::fs::Dir;
use std::{mem::size_of, time::UNIX_EPOCH};

const PARAMETER: u64 = 0x809f_0000;
const NOT_FOUND: u64 = 0x809f_0008;
const INTERNAL: u64 = 0x809f_000b;
const BROKEN: u64 = 0x809f_000f;
const INVALID_USER: u64 = 0x809f_0011;
const MAX_ENTRIES: usize = 1024;
const MAX_DEPTH: usize = 16;
const SEARCH_FIXED: usize = 168;

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn read<const N: usize>(
    guest: &impl GuestAccess,
    address: u64,
    budget: &mut Budget,
) -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    budget.read(guest, address, &mut bytes)?;
    Ok(bytes)
}
fn add(address: u64, offset: u64) -> Result<u64, Error> {
    address
        .checked_add(offset)
        .ok_or(Error::Policy(Failure::Guest(
            crate::GuestError::AddressOverflow,
        )))
}
fn name<const N: usize>(bytes: &[u8; N]) -> Result<&[u8], Error> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::Code(PARAMETER))?;
    Ok(&bytes[..end])
}
fn valid_component(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes != b"."
        && bytes != b".."
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-.@".contains(byte))
}
fn ordinary(metadata: &cap_std::fs::Metadata) -> bool {
    if metadata.is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    true
}

// The source wildcard language is '%' (zero or more bytes), '_' (one byte).
// Two fixed rows replace the source's exponentially recursive matcher.
fn matches(candidate: &[u8], pattern: &[u8], budget: &mut Budget) -> Result<bool, Error> {
    budget.work(((candidate.len() + 1) * (pattern.len() + 1)) as u64)?;
    let mut previous = [false; 33];
    previous[0] = true;
    for symbol in pattern {
        let mut next = [false; 33];
        // Source only enters its '%' branch while input remains. A trailing
        // '%' reached exactly at the terminator is therefore not discarded.
        if *symbol == b'%' && !candidate.is_empty() {
            next[0] = previous[0];
        }
        for index in 1..=candidate.len() {
            next[index] = match *symbol {
                b'%' => (previous[index] && index < candidate.len()) || next[index - 1],
                b'_' => previous[index - 1],
                byte => previous[index - 1] && candidate[index - 1].eq_ignore_ascii_case(&byte),
            };
        }
        previous = next;
    }
    Ok(previous[candidate.len()])
}

fn newest_time(
    directory: &Dir,
    depth: usize,
    visited: &mut usize,
    budget: &mut Budget,
) -> Result<i64, Error> {
    if depth >= MAX_DEPTH {
        return Err(Failure::Limit("save metadata depth").into());
    }
    budget.work(1)?;
    let mut newest = 0;
    for entry in directory.entries().map_err(|_| Error::Code(INTERNAL))? {
        budget.work(1)?;
        *visited = visited
            .checked_add(1)
            .ok_or(Failure::Limit("save metadata entries"))?;
        if *visited > MAX_ENTRIES {
            return Err(Failure::Limit("save metadata entries").into());
        }
        let entry = entry.map_err(|_| Error::Code(INTERNAL))?;
        let metadata = directory
            .symlink_metadata(entry.file_name())
            .map_err(|_| Error::Code(INTERNAL))?;
        if !ordinary(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            let child = directory
                .open_dir(entry.file_name())
                .map_err(|_| Error::Code(INTERNAL))?;
            newest = newest.max(newest_time(&child, depth + 1, visited, budget)?);
        } else if metadata.is_file()
            && let Ok(seconds) = metadata
                .modified()
                .map(|time| time.into_std())
                .and_then(|time| {
                    time.duration_since(UNIX_EPOCH)
                        .map_err(std::io::Error::other)
                })
        {
            newest = newest.max(i64::try_from(seconds.as_secs()).map_err(|_| Error::Code(BROKEN))?);
        }
    }
    Ok(newest)
}

fn param(
    storage: &SaveStorage,
    directory: &Dir,
    held: usize,
    timestamp: bool,
    budget: &mut Budget,
) -> Result<[u8; PARAM_BYTES], Error> {
    let scratch = held
        .checked_add(PARAM_BYTES * 2)
        .ok_or(Failure::Limit("save param scratch"))?;
    budget.scratch(scratch)?;
    budget.record_scratch(scratch);
    let bytes = storage
        .read_file(
            directory,
            "sce_sys/param.bin",
            PARAM_BYTES,
            held + PARAM_BYTES,
            budget,
        )?
        .ok_or(Error::Code(NOT_FOUND))?;
    let mut output: [u8; PARAM_BYTES] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::Code(BROKEN))?;
    if timestamp {
        let time = newest_time(directory, 0, &mut 0, budget)?;
        output[1288..1296].copy_from_slice(&time.to_le_bytes());
    }
    Ok(output)
}
fn text_field(kind: u32) -> (usize, usize) {
    match kind {
        1 => (0, 128),
        2 => (128, 128),
        3 => (256, 1024),
        _ => unreachable!("validated parameter type"),
    }
}
fn icon_error(error: Failure) -> Error {
    match error {
        Failure::Errno(Errno::NoEntry) => Error::Code(NOT_FOUND),
        Failure::Errno(Errno::Access | Errno::Perm | Errno::ReadOnly) => {
            Error::Code(super::directories::NO_PERMISSION)
        }
        Failure::Errno(Errno::Invalid | Errno::IsDirectory | Errno::NotDirectory) => {
            Error::Code(PARAMETER)
        }
        Failure::Errno(_) => Error::Code(INTERNAL),
        other => Error::Policy(other),
    }
}
type SearchInput = ([u8; 64], [u8; 56], [u8; 10], [u8; 32]);

fn search_input(
    guest: &impl GuestAccess,
    args: CallArgs,
    budget: &mut Budget,
    storage: &SaveStorage,
) -> Result<SearchInput, Error> {
    let condition = read::<64>(guest, args.0[0], budget)?;
    let result = read::<56>(guest, args.0[1], budget)?;
    if u32_at(&condition, 24) > 5
        || u32_at(&condition, 28) > 1
        || (u32_at(&result, 16) != 0 && u64_at(&result, 8) == 0)
    {
        return Err(Error::Code(PARAMETER));
    }
    let title = if u64_at(&condition, 8) == 0 {
        storage.current_title()
    } else {
        let raw = read::<16>(guest, u64_at(&condition, 8), budget)?;
        raw[..10].try_into().unwrap()
    };
    if !valid_component(name(&title)?) {
        return Err(Error::Code(PARAMETER));
    }
    let pattern = if u64_at(&condition, 16) == 0 {
        [0; 32]
    } else {
        read::<32>(guest, u64_at(&condition, 16), budget)?
    };
    name(&pattern)?;
    // The pinned function reports malformed fields before invalid login user.
    if (u32_at(&condition, 0) as i32) < 0 {
        return Err(Error::Code(INVALID_USER));
    }
    Ok((condition, result, title, pattern))
}

fn metadata_arguments(service: Service, args: CallArgs) -> Result<(), Error> {
    let kind = args.0[1] as u32;
    let invalid = match service {
        Service::SaveGetParam => args.0[2] == 0 || kind > 5,
        Service::SaveSetParam => args.0[2] == 0 || kind > 4,
        Service::SaveLoadIcon | Service::SaveSaveIcon | Service::SaveSaveIconByPath => {
            args.0[1] == 0
        }
        _ => false,
    };
    if invalid {
        Err(Error::Code(PARAMETER))
    } else {
        Ok(())
    }
}
fn metadata_sizes(service: Service, args: CallArgs) -> Result<(), Error> {
    let kind = args.0[1] as u32;
    if service == Service::SaveSetParam
        && ((kind == 0 && args.0[3] < PARAM_BYTES as u64)
            || (kind == 4 && args.0[3] < 4)
            || ((1..=3).contains(&kind) && args.0[3] == 0))
    {
        Err(Error::Code(PARAMETER))
    } else {
        Ok(())
    }
}

impl DirectoryState {
    pub(super) fn invoke_metadata(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        storage: &SaveStorage,
        files: &FileState,
    ) -> Result<u64, Error> {
        if args.0[0] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        if service == Service::SaveDirNameSearch {
            return self.search(args, guest, budget, storage);
        }
        metadata_arguments(service, args)?;
        let point = read::<16>(guest, args.0[0], budget)?;
        name(&point)?;
        // Source SetParam memchr precedes its size minima, while both precede
        // mount lookup. An unreadable point is not hidden by a short buffer.
        metadata_sizes(service, args)?;
        let mount = self.mounted(&point)?;
        match service {
            Service::SaveGetParam => {
                let kind = args.0[1] as u32;
                let capacity = usize::try_from(args.0[3]).map_err(|_| Error::Code(PARAMETER))?;
                if kind > 5 || args.0[2] == 0 {
                    return Err(Error::Code(PARAMETER));
                }
                let data = param(storage, &mount.directory, 16, true, budget)?;
                let (start, count) = match kind {
                    0 => (0, PARAM_BYTES),
                    4 => (1280, 4),
                    5 => (1288, 8),
                    _ => {
                        if capacity == 0 {
                            return Err(Error::Code(PARAMETER));
                        }
                        let (start, length) = text_field(kind);
                        let actual = data[start..start + length]
                            .iter()
                            .position(|byte| *byte == 0)
                            .unwrap_or(length);
                        (start, actual.min(capacity - 1))
                    }
                };
                let text = matches!(kind, 1..=3);
                let length = count + usize::from(text);
                if capacity < length {
                    return Err(Error::Code(PARAMETER));
                }
                let mut output = budget.buffer_held(length, PARAM_BYTES + 24)?;
                output[..count].copy_from_slice(&data[start..start + count]);
                let got = (length as u64).to_le_bytes();
                let writes = [
                    GuestWrite {
                        address: args.0[2],
                        bytes: &output,
                    },
                    GuestWrite {
                        address: args.0[4],
                        bytes: &got,
                    },
                ];
                budget.write(guest, &writes[..if args.0[4] == 0 { 1 } else { 2 }])?;
                Ok(0)
            }
            Service::SaveSetParam => {
                let kind = args.0[1] as u32;
                let capacity = usize::try_from(args.0[3]).map_err(|_| Error::Code(PARAMETER))?;
                if kind > 4 || args.0[2] == 0 {
                    return Err(Error::Code(PARAMETER));
                }
                budget.scratch(PARAM_BYTES + 16)?;
                budget.record_scratch(PARAM_BYTES + 16);
                let mut data = if kind == 0 {
                    if capacity < PARAM_BYTES {
                        return Err(Error::Code(PARAMETER));
                    }
                    read::<PARAM_BYTES>(guest, args.0[2], budget)?
                } else {
                    param(storage, &mount.directory, 16, false, budget)?
                };
                if kind == 4 {
                    if capacity < 4 {
                        return Err(Error::Code(PARAMETER));
                    }
                    budget.read(guest, args.0[2], &mut data[1280..1284])?;
                } else if kind != 0 {
                    if capacity == 0 {
                        return Err(Error::Code(PARAMETER));
                    }
                    let (start, length) = text_field(kind);
                    let count = capacity.min(length - 1);
                    let mut input = budget.buffer_held(count, PARAM_BYTES + 16)?;
                    budget.read(guest, args.0[2], &mut input)?;
                    let count = input.iter().position(|byte| *byte == 0).unwrap_or(count);
                    data[start..start + length].fill(0);
                    data[start..start + count].copy_from_slice(&input[..count]);
                }
                storage.write_atomic(&mount.directory, "sce_sys/param.bin", &data, budget)?;
                Ok(0)
            }
            Service::SaveSaveIcon => {
                if args.0[1] == 0 {
                    return Err(Error::Code(PARAMETER));
                }
                let input = read::<56>(guest, args.0[1], budget)?;
                let length =
                    usize::try_from(u64_at(&input, 16)).map_err(|_| Error::Code(PARAMETER))?;
                if length as u64 > u64_at(&input, 8)
                    || length > budget.limits.max_io_bytes
                    || length as u64 > SOURCE_MAX
                    || (length != 0 && u64_at(&input, 0) == 0)
                {
                    return Err(Error::Code(PARAMETER));
                }
                let mut bytes = budget.buffer_held(length, 72)?;
                if length != 0 {
                    budget.read(guest, u64_at(&input, 0), &mut bytes)?;
                }
                storage.write_atomic(&mount.directory, "sce_sys/icon0.png", &bytes, budget)?;
                Ok(0)
            }
            Service::SaveLoadIcon => {
                if args.0[1] == 0 {
                    return Err(Error::Code(PARAMETER));
                }
                let input = read::<56>(guest, args.0[1], budget)?;
                let bytes = storage
                    .read_file(
                        &mount.directory,
                        "sce_sys/icon0.png",
                        budget.limits.max_io_bytes.min(SOURCE_MAX as usize),
                        80,
                        budget,
                    )?
                    .ok_or(Error::Code(NOT_FOUND))?;
                if u64_at(&input, 8) < bytes.len() as u64
                    || (!bytes.is_empty() && u64_at(&input, 0) == 0)
                {
                    return Err(Error::Code(PARAMETER));
                }
                let size = (bytes.len() as u64).to_le_bytes();
                let writes = [
                    GuestWrite {
                        address: add(args.0[1], 16)?,
                        bytes: &size,
                    },
                    GuestWrite {
                        address: u64_at(&input, 0),
                        bytes: &bytes,
                    },
                ];
                budget.write(guest, &writes[..if bytes.is_empty() { 1 } else { 2 }])?;
                Ok(0)
            }
            Service::SaveSaveIconByPath => {
                if args.0[1] == 0 {
                    return Err(Error::Code(PARAMETER));
                }
                let held = budget
                    .limits
                    .max_name_bytes
                    .checked_add(16)
                    .ok_or(Failure::Limit("save icon path scratch"))?;
                budget.scratch(held)?;
                budget.record_scratch(held);
                let path = budget.name(guest, args.0[1])?;
                if !path.starts_with('/')
                    || path.contains(['\\', ':'])
                    || path[1..]
                        .split('/')
                        .any(|part| part.is_empty() || part == "." || part == "..")
                {
                    return Err(Error::Code(PARAMETER));
                }
                let bytes = files
                    .save_icon_bytes(
                        &path,
                        budget.limits.max_io_bytes.min(SOURCE_MAX as usize),
                        path.capacity() + 16,
                        budget,
                    )
                    .map_err(icon_error)?;
                storage.write_atomic(&mount.directory, "sce_sys/icon0.png", &bytes, budget)?;
                Ok(0)
            }
            _ => Err(Failure::Unsupported("save directory metadata service").into()),
        }
    }

    fn search(
        &self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        storage: &SaveStorage,
    ) -> Result<u64, Error> {
        if args.0[1] == 0 {
            return Err(Error::Code(PARAMETER));
        }
        search_input(guest, args, budget, storage)?;
        // Preserve the source's visible partial stores before filesystem work.
        // Pointers and capacity remain caller-owned. Re-read after these stores
        // so aliases cannot retain stale condition/result pointer values.
        budget.write(
            guest,
            &[
                GuestWrite {
                    address: args.0[1],
                    bytes: &[0; 8],
                },
                GuestWrite {
                    address: add(args.0[1], 20)?,
                    bytes: &[0; 4],
                },
                GuestWrite {
                    address: add(args.0[1], 40)?,
                    bytes: &[0; 16],
                },
            ],
        )?;
        let (condition, result, title, pattern) = search_input(guest, args, budget, storage)?;
        let pattern = name(&pattern)?;
        let mut names = Vec::<[u8; 32]>::new();
        if let Some(directory) = storage.title_directory(&title)? {
            for (visited, entry) in directory
                .entries()
                .map_err(|_| Error::Code(INTERNAL))?
                .enumerate()
            {
                budget.work(1)?;
                if visited >= MAX_ENTRIES {
                    return Err(Failure::Limit("save directory search entries").into());
                }
                let entry = entry.map_err(|_| Error::Code(INTERNAL))?;
                if !entry
                    .file_type()
                    .map_err(|_| Error::Code(INTERNAL))?
                    .is_dir()
                {
                    continue;
                }
                let host_name = entry.file_name();
                let Some(host_name) = host_name.to_str() else {
                    continue;
                };
                let Some(decoded) = SaveStorage::decode_directory_component(host_name) else {
                    continue;
                };
                let value = name(&decoded)?;
                if value.starts_with(b"sce_") || !valid_component(value) {
                    continue;
                }
                if !pattern.is_empty() && !matches(value, pattern, budget)? {
                    continue;
                }
                let child = storage
                    .open_save(&SaveKey {
                        title,
                        name: decoded,
                        user: None,
                    })?
                    .ok_or(Error::Code(NOT_FOUND))?;
                let system = match child.symlink_metadata("sce_sys") {
                    Ok(metadata) if ordinary(&metadata) && metadata.is_dir() => child
                        .open_dir("sce_sys")
                        .map_err(|_| Error::Code(INTERNAL))?,
                    Ok(_) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(Error::Code(INTERNAL)),
                };
                match system.symlink_metadata("param.bin") {
                    Ok(metadata) if ordinary(&metadata) && metadata.is_file() => {}
                    Ok(_) => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => return Err(Error::Code(INTERNAL)),
                }
                if names.len() == names.capacity() {
                    let capacity = (names.capacity() * 2).clamp(4, MAX_ENTRIES);
                    let old = names.capacity();
                    budget.scratch((old + capacity) * size_of::<[u8; 32]>() + SEARCH_FIXED)?;
                    budget.work((capacity * size_of::<[u8; 32]>()) as u64)?;
                    names
                        .try_reserve_exact(capacity - names.len())
                        .map_err(|_| Failure::Allocation)?;
                    let actual = (old + names.capacity()) * size_of::<[u8; 32]>() + SEARCH_FIXED;
                    budget.scratch(actual)?;
                    budget.record_scratch(actual);
                }
                names.push(decoded);
            }
        }
        budget.work((names.len() * (names.len().max(1).ilog2() as usize + 1) * 32) as u64)?;
        names.sort_unstable();
        if u32_at(&condition, 28) == 1 {
            names.reverse();
        }
        let count = names.len().min(u32_at(&result, 16) as usize);
        let held = names.capacity() * 32 + SEARCH_FIXED;
        let hits = (names.len() as u32).to_le_bytes();
        let set = (count as u32).to_le_bytes();
        budget.write(
            guest,
            &[
                GuestWrite {
                    address: args.0[1],
                    bytes: &hits,
                },
                GuestWrite {
                    address: add(args.0[1], 20)?,
                    bytes: &set,
                },
            ],
        )?;
        for (index, saved_name) in names.iter().take(count).enumerate() {
            let active = read::<56>(guest, args.0[1], budget)?;
            let length = name(saved_name)?.len() + 1;
            budget.write(
                guest,
                &[GuestWrite {
                    address: add(u64_at(&active, 8), (index * 32) as u64)?,
                    bytes: &saved_name[..length],
                }],
            )?;
            let active = read::<56>(guest, args.0[1], budget)?;
            if u64_at(&active, 24) != 0 {
                let address = add(u64_at(&active, 24), (index * PARAM_BYTES) as u64)?;
                budget.scratch(held + PARAM_BYTES * 2)?;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &[0; PARAM_BYTES],
                    }],
                )?;
                let directory = storage
                    .open_save(&SaveKey {
                        title,
                        name: *saved_name,
                        user: None,
                    })?
                    .ok_or(Error::Code(NOT_FOUND))?;
                let data = param(storage, &directory, held, true, budget)?;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &data,
                    }],
                )?;
            }
            let active = read::<56>(guest, args.0[1], budget)?;
            if u64_at(&active, 32) != 0 {
                let address = add(u64_at(&active, 32), (index * 48) as u64)?;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &[0; 48],
                    }],
                )?;
                let directory = storage
                    .open_save(&SaveKey {
                        title,
                        name: *saved_name,
                        user: None,
                    })?
                    .ok_or(Error::Code(NOT_FOUND))?;
                let blocks = storage.read_blocks(&directory, budget)?;
                let mut info = [0; 16];
                info[..8].copy_from_slice(&blocks.to_le_bytes());
                info[8..].copy_from_slice(&blocks.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &info,
                    }],
                )?;
            }
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests;
