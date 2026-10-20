// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: capability-relative metadata and bounded owner IDs.
// Adapted from pinned libAmpr.cpp; see NOTICE.

use super::{FileState, owned_name, positional_read};
use crate::budget::Budget;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite};
use std::collections::HashMap;
use std::hash::BuildHasher;

#[derive(Clone, Copy)]
struct Resolved {
    id: u32,
    file_size: u64,
    directory: bool,
    error: Option<Errno>,
}
impl Resolved {
    fn failed(error: Errno) -> Self {
        Self {
            id: u32::MAX,
            file_size: 0,
            directory: false,
            error: Some(error),
        }
    }
    fn resolve_size(self) -> u64 {
        if self.directory {
            0x1_0000
        } else {
            self.file_size
        }
    }
}

#[derive(Default)]
pub(super) struct State {
    // Exact guest spellings are keys. IDs are unique sequence values, not hashes
    // or ordinary file descriptors. No OS handle is retained for each identity.
    // The randomized hash is only an index: collision chains always compare the
    // complete spelling. Entries own each name once; both indices retain scalars.
    paths: HashMap<u64, usize>,
    entries: Vec<Entry>,
    ids: Vec<usize>,
    last_id: u32,
}

struct Entry {
    path: String,
    info: Resolved,
    // Publication prepends a new entry to an existing chain. Indices strictly
    // decrease, so a collision walk cannot cycle or outgrow the entry count.
    next: Option<usize>,
}

impl State {
    fn cached(
        &self,
        path: &str,
        mut head: Option<usize>,
        budget: &mut Budget,
    ) -> Result<Option<Resolved>, Failure> {
        while let Some(index) = head {
            budget.work(path.len() as u64 + 1)?;
            let entry = self
                .entries
                .get(index)
                .ok_or(Failure::Unsupported("APR path index"))?;
            if entry.path == path {
                return Ok(Some(entry.info));
            }
            head = entry.next;
        }
        Ok(None)
    }

    fn identified(&self, id: u32) -> Option<&Entry> {
        let index = *self.ids.get(id.checked_sub(1)? as usize)?;
        self.entries
            .get(index)
            .filter(|entry| entry.info.id == id && entry.info.error.is_none())
    }
}

fn reserve<T>(entries: &mut Vec<T>, budget: &mut Budget) -> Result<(), Failure> {
    if entries.len() == entries.capacity() {
        budget.work(entries.len() as u64 + 1)?;
    }
    entries.try_reserve(1).map_err(|_| Failure::Allocation)
}

fn range(address: u64, length: u64) -> Result<(), Failure> {
    if address == 0 || address.checked_add(length).is_none() {
        return Err(Errno::Fault.into());
    }
    Ok(())
}

fn path(guest: &impl GuestAccess, address: u64, budget: &mut Budget) -> Result<String, Failure> {
    if address == 0 {
        return Err(Errno::Fault.into());
    }
    let maximum = budget.limits.max_name_bytes.min(1024);
    let mut bytes = budget.buffer(maximum)?;
    for index in 0..maximum {
        let at = address.checked_add(index as u64).ok_or(Errno::Fault)?;
        budget.read(guest, at, &mut bytes[index..index + 1])?;
        if bytes[index] == 0 {
            bytes.truncate(index);
            return String::from_utf8(bytes).map_err(|_| Failure::Unsupported("non-UTF8 APR path"));
        }
    }
    if maximum == 1024 {
        Err(Errno::NameTooLong.into())
    } else {
        Err(Failure::Limit("APR path bytes"))
    }
}

/// Normalize at most three small ordered stores, preserving virtual alias order
/// without including disjoint gaps. The adapter still checks physical aliases.
fn publish(
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
    fields: &[GuestWrite<'_>],
) -> Result<(), Failure> {
    budget.work(256)?;
    let mut cells = [(0_u64, 0_u8); 16];
    let mut length = 0;
    for field in fields {
        for (offset, byte) in field.bytes.iter().copied().enumerate() {
            let address = field.address + offset as u64;
            if let Some(cell) = cells[..length].iter_mut().find(|cell| cell.0 == address) {
                cell.1 = byte;
            } else {
                cells[length] = (address, byte);
                length += 1;
            }
        }
    }
    cells[..length].sort_unstable_by_key(|cell| cell.0);
    let mut bytes = [0; 16];
    for (target, (_, value)) in bytes.iter_mut().zip(&cells[..length]) {
        *target = *value;
    }
    let mut writes = [GuestWrite {
        address: 0,
        bytes: &[],
    }; 3];
    let mut count = 0;
    let mut start = 0;
    while start < length {
        let mut end = start + 1;
        while end < length && cells[end].0 == cells[end - 1].0 + 1 {
            end += 1;
        }
        writes[count] = GuestWrite {
            address: cells[start].0,
            bytes: &bytes[start..end],
        };
        count += 1;
        start = end;
    }
    budget.write(guest, &writes[..count])
}

impl FileState {
    pub(crate) fn resolve_apr_paths(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [list, count, ids, sizes, error_index, _] = args.0;
        let count = count as u32;
        if list == 0 || count == 0 || count > 1024 || (ids == 0 && sizes == 0) {
            return Err(Errno::Invalid.into());
        }
        // Match source numeric preflight ordering. Live backing permission and
        // ownership checks remain in each checked read/output transaction.
        if error_index != 0 {
            range(error_index, 4)?;
        }
        if ids != 0 {
            range(ids, u64::from(count) * 4)?;
        }
        if sizes != 0 {
            range(sizes, u64::from(count) * 8)?;
        }
        range(list, u64::from(count) * 8)?;

        for index in 0..u64::from(count) {
            let mut pending = None;
            // Read each pointer and string after the preceding item's stores.
            // Output aliases can deliberately change later pointer/string data.
            let result = (|| {
                let pointer = budget.u64(guest, list + index * 8)?;
                let path = path(guest, pointer, budget)?;
                if path.is_empty() {
                    return Err(Errno::Invalid.into());
                }
                budget.work(path.len() as u64 + 64)?;
                let hash = self.apr.paths.hasher().hash_one(path.as_str());
                let head = self.apr.paths.get(&hash).copied();
                if let Some(info) = self.apr.cached(&path, head, budget)? {
                    return Ok(info);
                }
                let retained = owned_name(&path)?;
                budget.scratch(path.capacity() + retained.capacity())?;
                budget.record_scratch(path.capacity() + retained.capacity());
                budget.can_retain_apr_path(retained.capacity())?;
                reserve(&mut self.apr.entries, budget)?;
                if head.is_none() {
                    if self.apr.paths.len() == self.apr.paths.capacity() {
                        budget.work(self.apr.paths.len() as u64 + 1)?;
                    }
                    self.apr
                        .paths
                        .try_reserve(1)
                        .map_err(|_| Failure::Allocation)?;
                }
                let info = match self.selected(&path, budget) {
                    Ok(Some(file)) => {
                        let metadata = file.metadata().map_err(|_| Errno::Io)?;
                        if !metadata.is_file() && !metadata.is_dir() {
                            return Err(Failure::Unsupported("APR device metadata"));
                        }
                        let id = self
                            .apr
                            .last_id
                            .checked_add(1)
                            .filter(|id| *id != u32::MAX)
                            .ok_or(Errno::FileTableFull)?;
                        Resolved {
                            id,
                            file_size: if metadata.is_dir() { 0 } else { metadata.len() },
                            directory: metadata.is_dir(),
                            error: None,
                        }
                    }
                    Ok(None) | Err(Failure::Errno(Errno::NoEntry)) => {
                        Resolved::failed(Errno::NoEntry)
                    }
                    Err(error) => return Err(error),
                };
                if info.error.is_none() {
                    reserve(&mut self.apr.ids, budget)?;
                }
                pending = Some((retained, info, hash, head));
                Ok(info)
            })();
            let info = match result {
                Ok(info) => info,
                Err(Failure::Errno(error)) => Resolved::failed(error),
                Err(error) => return Err(error),
            };
            let id = info.id.to_le_bytes();
            let size = info.resolve_size().to_le_bytes();
            let failed_index = (index as u32).to_le_bytes();
            let mut fields = [GuestWrite {
                address: 0,
                bytes: &[],
            }; 3];
            let mut length = 0;
            if ids != 0 {
                fields[length] = GuestWrite {
                    address: ids + index * 4,
                    bytes: &id,
                };
                length += 1;
            }
            if sizes != 0 {
                fields[length] = GuestWrite {
                    address: sizes + index * 8,
                    bytes: &size,
                };
                length += 1;
            }
            if info.error.is_some() && error_index != 0 {
                fields[length] = GuestWrite {
                    address: error_index,
                    bytes: &failed_index,
                };
                length += 1;
            }
            publish(guest, budget, &fields[..length])?;
            if let Some((path, info, hash, next)) = pending {
                budget.retained_apr_path(path.capacity());
                let index = self.apr.entries.len();
                if info.error.is_none() {
                    self.apr.last_id = info.id;
                    self.apr.ids.push(index);
                }
                self.apr.entries.push(Entry { path, info, next });
                self.apr.paths.insert(hash, index);
            }
            if let Some(error) = info.error {
                return Err(error.into());
            }
        }
        Ok(0)
    }

    /// Read a previously resolved identity through its selected capability.
    /// The cached Resolve size is deliberately untouched: APR submit uses the
    /// current host size and positional I/O, never an ordinary fd's cursor.
    /// Actual chunks commit independently; later failures preserve the prefix.
    pub(crate) fn read_apr_file(
        &self,
        id: u32,
        destination: u64,
        size: u64,
        offset: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        // Source submission rejects an unknown identity before even a zero-size
        // request. Successful IDs are dense; this reverse lookup never scans
        // the retained path table or opens a per-ID descriptor in advance.
        budget.work(1)?;
        let entry = self.apr.identified(id).ok_or(Errno::NoEntry)?;
        if size == 0 {
            return Ok(());
        }
        range(destination, size)?;
        budget.work(entry.path.len() as u64 + 64)?;
        let file = match self.selected(&entry.path, budget) {
            Ok(Some(file)) => file,
            Ok(None) | Err(Failure::Errno(_)) => return Err(Errno::NoEntry.into()),
            Err(error) => return Err(error),
        };
        budget.work(1)?;
        let metadata = file.metadata().map_err(|_| Errno::Io)?;
        if !metadata.is_file() {
            // A metadata-resolvable directory is not a readable source file.
            return Err(Errno::NoEntry.into());
        }
        let file_size = metadata.len();
        if offset >= file_size {
            return Ok(());
        }
        let readable = size.min(file_size - offset);
        let chunk = self
            .limits
            .max_io_bytes
            .min(budget.limits.max_io_bytes)
            .min(budget.limits.max_scratch_bytes)
            .min(4 * 1024 * 1024);
        if chunk == 0 {
            return Err(Failure::Limit("APR read chunk bytes"));
        }
        let mut bytes = budget.buffer((readable.min(chunk as u64)) as usize)?;
        budget.scratch(bytes.capacity())?;
        budget.record_scratch(bytes.capacity());
        let mut copied = 0_u64;
        while copied < readable {
            let request = (readable - copied).min(bytes.len() as u64) as usize;
            budget.work(request as u64)?;
            let at = offset.checked_add(copied).ok_or(Errno::Invalid)?;
            let count = positional_read(&file, &mut bytes[..request], at)?;
            if count == 0 {
                break;
            }
            budget.write(
                guest,
                &[GuestWrite {
                    address: destination + copied,
                    bytes: &bytes[..count],
                }],
            )?;
            copied += count as u64;
        }
        Ok(())
    }
}

#[cfg(test)]
mod index_tests {
    use super::*;

    #[test]
    fn exact_collision_keys_and_dense_id_index_include_cached_miss_entries_safely() {
        let mut state = State {
            entries: vec![
                Entry {
                    path: "missing".into(),
                    info: Resolved::failed(Errno::NoEntry),
                    next: None,
                },
                Entry {
                    path: "first".into(),
                    info: Resolved {
                        id: 1,
                        file_size: 7,
                        directory: false,
                        error: None,
                    },
                    next: Some(0),
                },
                Entry {
                    path: "second".into(),
                    info: Resolved {
                        id: 2,
                        file_size: 9,
                        directory: false,
                        error: None,
                    },
                    next: Some(1),
                },
            ],
            ids: vec![1, 2],
            last_id: 2,
            ..Default::default()
        };
        // Deliberately force one collision chain: random hashes select a
        // bucket, and complete spelling equality remains the authority.
        state.paths.insert(73, 2);
        let mut budget = Budget::new(Default::default()).unwrap();
        assert_eq!(
            state
                .cached("first", state.paths.get(&73).copied(), &mut budget)
                .unwrap()
                .unwrap()
                .id,
            1
        );
        assert_eq!(
            state
                .cached("missing", state.paths.get(&73).copied(), &mut budget)
                .unwrap()
                .unwrap()
                .error,
            Some(Errno::NoEntry)
        );
        assert!(
            state
                .cached("absent", state.paths.get(&73).copied(), &mut budget)
                .unwrap()
                .is_none()
        );
        assert_eq!(state.identified(1).unwrap().path, "first");
        assert_eq!(state.identified(2).unwrap().path, "second");
        assert!(state.identified(0).is_none());
        assert!(state.identified(u32::MAX).is_none());
        assert_eq!(state.last_id, 2);
    }
}
