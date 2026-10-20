// SPDX-License-Identifier: GPL-2.0-only

//! Explicit directory capability and atomic PentaX save bundles.
//! File names contain only validated numeric user/slot keys, never guest paths.

use super::memory::{Error, PARAM_BYTES, SOURCE_MAX};
use crate::Failure;
use crate::budget::Budget;
use cap_std::fs::{Dir, OpenOptions};
use std::fs::File;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

mod trophy;

const MAGIC: &[u8; 8] = b"PXSAVE1\0";
const HEADER: usize = 24;
static TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub struct SaveStorage {
    directory: Dir,
    _lock: File,
    #[cfg(test)]
    failure: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl SaveStorage {
    /// Select an existing dedicated per-title directory. Exclusive OS locking
    /// prevents concurrent PentaX sessions from overwriting each other's saves.
    pub fn directory(directory: File) -> Result<Self, Failure> {
        let directory = Dir::reopen_dir(&directory)
            .map_err(|_| Failure::Unsupported("save directory capability"))?;
        let lock = directory
            .open_with(
                ".pentax-save.lock",
                OpenOptions::new().read(true).write(true).create(true),
            )
            .map_err(|_| Failure::Unsupported("save storage lock"))?
            .into_std();
        lock.try_lock()
            .map_err(|_| Failure::Unsupported("save storage already in use"))?;
        Ok(Self {
            directory,
            _lock: lock,
            #[cfg(test)]
            failure: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }
    pub(super) fn load(
        &self,
        user: i32,
        slot: u32,
        data: &mut [u8],
        param: &mut [u8; PARAM_BYTES],
        budget: &mut Budget,
    ) -> Result<usize, Error> {
        let name = format!("memory-u{user}-s{slot}.pxsave");
        let mut file = match self.directory.open(&name) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(_) => return Err(Error::Code(0x809f_000b)),
        };
        let metadata = file.metadata().map_err(|_| Error::Code(0x809f_000b))?;
        if !metadata.is_file()
            || metadata.len() < (HEADER + PARAM_BYTES + 1) as u64
            || metadata.len() > (HEADER + PARAM_BYTES) as u64 + SOURCE_MAX
        {
            return Err(Error::Code(0x809f_000f));
        }
        budget.save_io(HEADER as u64, false)?;
        let mut header = [0; HEADER];
        file.read_exact(&mut header)
            .map_err(|_| Error::Code(0x809f_000f))?;
        let size = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let checksum = u64::from_le_bytes(header[16..24].try_into().unwrap());
        if &header[..8] != MAGIC
            || size == 0
            || size > SOURCE_MAX
            || metadata.len() != size + (HEADER + PARAM_BYTES) as u64
        {
            return Err(Error::Code(0x809f_000f));
        }
        budget.save_io(PARAM_BYTES as u64 + size, false)?;
        budget.scratch(8192)?;
        budget.record_scratch(8192);
        file.read_exact(param)
            .map_err(|_| Error::Code(0x809f_000f))?;
        let mut hash = hash_bytes(0xcbf2_9ce4_8422_2325, param);
        let mut chunk = [0; 8192];
        let mut consumed = 0_usize;
        while consumed < size as usize {
            let count = chunk.len().min(size as usize - consumed);
            file.read_exact(&mut chunk[..count])
                .map_err(|_| Error::Code(0x809f_000f))?;
            hash = hash_bytes(hash, &chunk[..count]);
            if consumed < data.len() {
                let copied = count.min(data.len() - consumed);
                data[consumed..consumed + copied].copy_from_slice(&chunk[..copied]);
            }
            consumed += count;
        }
        if hash != checksum {
            return Err(Error::Code(0x809f_000f));
        }
        Ok(size as usize)
    }
    pub(super) fn store(
        &self,
        user: i32,
        slot: u32,
        data: &[u8],
        param: &[u8; PARAM_BYTES],
        budget: &mut Budget,
    ) -> Result<(), Error> {
        budget.save_io((HEADER + PARAM_BYTES + data.len()) as u64, true)?;
        let name = format!("memory-u{user}-s{slot}.pxsave");
        // A previous crashed process may leave a temporary with a reused PID.
        // Skip bounded collisions; never delete or truncate an unowned file.
        let mut created = None;
        for _ in 0..8 {
            let serial = TEMP_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| Error::Policy(Failure::Limit("save temporary IDs")))?;
            let temporary = format!(".memory-{}-{serial}.tmp", std::process::id());
            match self
                .directory
                .open_with(&temporary, OpenOptions::new().write(true).create_new(true))
            {
                Ok(file) => {
                    created = Some((temporary, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(Error::Code(0x809f_000b)),
            }
        }
        let (temporary, mut file) = created.ok_or(Error::Code(0x809f_000b))?;
        let mut header = [0; HEADER];
        header[..8].copy_from_slice(MAGIC);
        header[8..16].copy_from_slice(&(data.len() as u64).to_le_bytes());
        let checksum = hash_bytes(hash_bytes(0xcbf2_9ce4_8422_2325, param), data);
        header[16..].copy_from_slice(&checksum.to_le_bytes());
        let written = file
            .write_all(&header)
            .and_then(|_| file.write_all(param))
            .and_then(|_| file.write_all(data))
            .and_then(|_| file.sync_all());
        drop(file);
        if written.is_err() {
            let _ = self.directory.remove_file(&temporary);
            return Err(Error::Code(0x809f_000b));
        }
        #[cfg(test)]
        if self.failure.load(Ordering::Relaxed) {
            let _ = self.directory.remove_file(&temporary);
            return Err(Error::Code(0x809f_000b));
        }
        if self
            .directory
            .rename(&temporary, &self.directory, &name)
            .is_err()
        {
            let _ = self.directory.remove_file(&temporary);
            return Err(Error::Code(0x809f_000b));
        }
        // No fallible action after replacement: the caller can now publish its
        // already admitted RAM snapshot without diverging from the stored bundle.
        Ok(())
    }
}
#[cfg(test)]
impl SaveStorage {
    pub(crate) fn failure_control(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.failure.clone()
    }
}
fn hash_bytes(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}
