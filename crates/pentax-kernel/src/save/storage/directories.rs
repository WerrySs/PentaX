// SPDX-License-Identifier: GPL-2.0-only
//! Capability-relative directory saves and atomic fixed metadata replacement.
use super::*;
use crate::save::directories::{BROKEN, INTERNAL, NO_PERMISSION, SaveKey};
use std::fmt::Write as _;
use std::io::ErrorKind;

fn code(error: std::io::Error) -> Error {
    Error::Code(if error.kind() == ErrorKind::PermissionDenied {
        NO_PERMISSION
    } else {
        INTERNAL
    })
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

fn child(parent: &Dir, name: &str) -> Result<Option<Dir>, Error> {
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(code(error)),
    };
    if !ordinary(&metadata) || !metadata.is_dir() {
        return Err(Error::Code(NO_PERMISSION));
    }
    parent.open_dir(name).map(Some).map_err(code)
}

fn ensure(parent: &Dir, name: &str) -> Result<Dir, Error> {
    if let Some(directory) = child(parent, name)? {
        return Ok(directory);
    }
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
        Err(error) => return Err(code(error)),
    }
    child(parent, name)?.ok_or(Error::Code(INTERNAL))
}

fn text(bytes: &[u8]) -> Result<&str, Error> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::Code(super::super::PARAMETER))?;
    let value = &bytes[..end];
    if !crate::save::directories::valid_component(value) {
        return Err(Error::Code(super::super::PARAMETER));
    }
    std::str::from_utf8(value).map_err(|_| Error::Code(super::super::PARAMETER))
}

fn encoded(prefix: char, name: &[u8]) -> Result<String, Error> {
    let name = text(name)?;
    let mut result = String::new();
    result
        .try_reserve_exact(1 + name.len() * 2)
        .map_err(|_| Error::Policy(Failure::Allocation))?;
    result.push(prefix);
    for byte in name.bytes() {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        result.push(char::from(HEX[(byte >> 4) as usize]));
        result.push(char::from(HEX[(byte & 15) as usize]));
    }
    Ok(result)
}

impl SaveStorage {
    pub(in crate::save) fn current_title(&self) -> [u8; 10] {
        self.title
    }

    pub(in crate::save) fn decode_directory_component(name: &str) -> Option<[u8; 32]> {
        let source = name.strip_prefix('d')?.as_bytes();
        if source.is_empty() || !source.len().is_multiple_of(2) || source.len() > 62 {
            return None;
        }
        let mut result = [0; 32];
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        for (slot, pair) in result.iter_mut().zip(source.chunks_exact(2)) {
            *slot = digit(pair[0])? * 16 + digit(pair[1])?;
        }
        crate::save::directories::valid_component(&result[..source.len() / 2]).then_some(result)
    }

    pub(in crate::save) fn title_directory(&self, title: &[u8; 10]) -> Result<Option<Dir>, Error> {
        let Some(root) = child(&self.directory, "directories")? else {
            return Ok(None);
        };
        child(&root, &encoded('t', title)?)
    }

    fn save_parent(&self, key: &SaveKey, create: bool) -> Result<Option<(Dir, String)>, Error> {
        let title_name = encoded('t', &key.title)?;
        let name = encoded('d', &key.name)?;
        let title = if create {
            let root = ensure(&self.directory, "directories")?;
            ensure(&root, &title_name)?
        } else {
            let Some(root) = child(&self.directory, "directories")? else {
                return Ok(None);
            };
            let Some(title) = child(&root, &title_name)? else {
                return Ok(None);
            };
            title
        };
        if let Some(user) = key.user {
            let parent = if create {
                ensure(&title, &name)?
            } else {
                let Some(directory) = child(&title, &name)? else {
                    return Ok(None);
                };
                directory
            };
            let mut component = String::new();
            component
                .try_reserve_exact(10)
                .map_err(|_| Error::Policy(Failure::Allocation))?;
            write!(&mut component, "{user}").map_err(|_| Error::Code(INTERNAL))?;
            Ok(Some((parent, component)))
        } else {
            Ok(Some((title, name)))
        }
    }

    pub(in crate::save) fn open_save(&self, key: &SaveKey) -> Result<Option<Dir>, Error> {
        let Some((parent, name)) = self.save_parent(key, false)? else {
            return Ok(None);
        };
        child(&parent, &name)
    }

    /// Create complete metadata in a private staging directory and publish its
    /// final encoded name only after both files have been flushed successfully.
    pub(in crate::save) fn create_save(
        &self,
        key: &SaveKey,
        blocks: u64,
        budget: &mut Budget,
    ) -> Result<Dir, Error> {
        budget.work(2048)?;
        let (parent, name) = self.save_parent(key, true)?.ok_or(Error::Code(INTERNAL))?;
        let (temporary, directory) = self.temporary_directory(&parent)?;
        let created = (|| {
            directory.create_dir("sce_sys").map_err(code)?;
            self.write_atomic(
                &directory,
                "sce_sys/blocks.bin",
                &blocks.to_le_bytes(),
                budget,
            )?;
            self.write_atomic(&directory, "sce_sys/param.bin", &[0; PARAM_BYTES], budget)?;
            Ok(())
        })();
        // Directory capabilities deliberately deny replacement on Windows.
        // Close the staging handle before its atomic namespace rename.
        drop(directory);
        if let Err(error) = created {
            cleanup_created(&parent, &temporary);
            return Err(error);
        }
        if let Err(error) = parent.rename(&temporary, &parent, &name) {
            cleanup_created(&parent, &temporary);
            return Err(code(error));
        }
        child(&parent, &name)?.ok_or(Error::Code(INTERNAL))
    }

    fn temporary_directory(&self, parent: &Dir) -> Result<(String, Dir), Error> {
        for _ in 0..8 {
            let name = temporary_name("create")?;
            match parent.create_dir(&name) {
                Ok(()) => {
                    return parent
                        .open_dir(&name)
                        .map(|directory| (name, directory))
                        .map_err(code);
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(code(error)),
            }
        }
        Err(Error::Code(INTERNAL))
    }

    pub(in crate::save) fn read_file(
        &self,
        directory: &Dir,
        path: &str,
        maximum: usize,
        held: usize,
        budget: &mut Budget,
    ) -> Result<Option<Vec<u8>>, Error> {
        let name = metadata_name(path)?;
        let Some(parent) = child(directory, "sce_sys")? else {
            return Ok(None);
        };
        let metadata = match parent.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(code(error)),
        };
        if !ordinary(&metadata) || !metadata.is_file() {
            return Err(Error::Code(BROKEN));
        }
        let length = usize::try_from(metadata.len()).map_err(|_| Error::Code(BROKEN))?;
        if length > maximum {
            return Err(Error::Code(BROKEN));
        }
        budget.save_io(length as u64, false)?;
        let mut bytes = budget.buffer_held(length, held)?;
        let mut file = parent.open(name).map_err(code)?;
        let before = file.metadata().map_err(code)?;
        if !before.is_file() || before.len() != metadata.len() {
            return Err(Error::Code(BROKEN));
        }
        file.read_exact(&mut bytes)
            .map_err(|_| Error::Code(BROKEN))?;
        let after = file.metadata().map_err(code)?;
        if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
            return Err(Error::Code(BROKEN));
        }
        Ok(Some(bytes))
    }

    pub(in crate::save) fn write_atomic(
        &self,
        directory: &Dir,
        path: &str,
        bytes: &[u8],
        budget: &mut Budget,
    ) -> Result<(), Error> {
        let (parent, name) = metadata_parent(directory, path)?;
        budget.save_io(bytes.len() as u64, true)?;
        let mut owned = None;
        for _ in 0..8 {
            let temporary = temporary_name("metadata")?;
            match parent.open_with(&temporary, OpenOptions::new().write(true).create_new(true)) {
                Ok(file) => {
                    owned = Some((temporary, file));
                    break;
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => return Err(code(error)),
            }
        }
        let (temporary, mut file) = owned.ok_or(Error::Code(INTERNAL))?;
        let result = file.write_all(bytes).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = parent.remove_file(&temporary);
            return Err(code(error));
        }
        #[cfg(test)]
        if self.failure.load(Ordering::Relaxed) {
            let _ = parent.remove_file(&temporary);
            return Err(Error::Code(INTERNAL));
        }
        if let Err(error) = parent.rename(&temporary, &parent, name) {
            let _ = parent.remove_file(&temporary);
            return Err(code(error));
        }
        Ok(())
    }

    pub(in crate::save) fn read_blocks(
        &self,
        directory: &Dir,
        budget: &mut Budget,
    ) -> Result<u64, Error> {
        // The source reports every blocks-file backing failure as BROKEN.
        // Keep independent admission failures typed instead of hiding limits.
        let bytes = self
            .read_file(directory, "sce_sys/blocks.bin", 8, 0, budget)
            .map_err(|error| match error {
                Error::Code(_) => Error::Code(BROKEN),
                Error::Policy(_) => error,
            })?
            .filter(|bytes| bytes.len() == 8)
            .ok_or(Error::Code(BROKEN))?;
        let value = u64::from_le_bytes(bytes.as_slice().try_into().unwrap());
        if !(48..=16384).contains(&value) {
            return Err(Error::Code(BROKEN));
        }
        Ok(value)
    }

    /// Remove only after a bounded, non-following inventory has admitted every
    /// descendant. The capability itself prevents an ambient-path fallback.
    pub(in crate::save) fn delete_save(
        &self,
        key: &SaveKey,
        budget: &mut Budget,
    ) -> Result<(), Error> {
        if let Some((parent, name)) = self.save_parent(key, false)?
            && let Some(directory) = child(&parent, &name)?
        {
            admit_delete(&directory, 0, budget)?;
            drop(directory);
            parent.remove_dir_all(&name).map_err(code)?;
        }
        if let (Some(slot), Some(user)) = (key.memory_slot(), key.user)
            && key.title == self.title
        {
            let mut name = String::new();
            name.try_reserve_exact(64)
                .map_err(|_| Error::Policy(Failure::Allocation))?;
            write!(&mut name, "memory-u{user}-s{slot}.pxsave")
                .map_err(|_| Error::Code(INTERNAL))?;
            budget.work(256)?;
            match self.directory.symlink_metadata(&name) {
                Ok(metadata) => {
                    if !ordinary(&metadata) || !metadata.is_file() {
                        return Err(Error::Code(NO_PERMISSION));
                    }
                    self.directory.remove_file(&name).map_err(code)?;
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(code(error)),
            }
        }
        Ok(())
    }

    pub(in crate::save) fn rollback_created(&self, key: &SaveKey) -> Result<(), Error> {
        let Some((parent, name)) = self.save_parent(key, false)? else {
            return Ok(());
        };
        let Some(directory) = child(&parent, &name)? else {
            return Ok(());
        };
        directory.remove_file("sce_sys/blocks.bin").map_err(code)?;
        directory.remove_file("sce_sys/param.bin").map_err(code)?;
        directory.remove_dir("sce_sys").map_err(code)?;
        drop(directory);
        parent.remove_dir(name).map_err(code)
    }
}

fn metadata_name(path: &str) -> Result<&str, Error> {
    let name = path
        .strip_prefix("sce_sys/")
        .ok_or(Error::Code(super::super::PARAMETER))?;
    if !matches!(name, "blocks.bin" | "param.bin" | "icon0.png") {
        return Err(Error::Code(super::super::PARAMETER));
    }
    Ok(name)
}
fn metadata_parent<'a>(directory: &Dir, path: &'a str) -> Result<(Dir, &'a str), Error> {
    let name = metadata_name(path)?;
    child(directory, "sce_sys")?
        .map(|parent| (parent, name))
        .ok_or(Error::Code(BROKEN))
}
fn cleanup_created(parent: &Dir, name: &str) {
    if let Ok(Some(directory)) = child(parent, name) {
        let _ = directory.remove_file("sce_sys/blocks.bin");
        let _ = directory.remove_file("sce_sys/param.bin");
        let _ = directory.remove_dir("sce_sys");
    }
    let _ = parent.remove_dir(name);
}

fn temporary_name(role: &str) -> Result<String, Error> {
    let serial = TEMP_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| Error::Policy(Failure::Limit("save directory temporary IDs")))?;
    let mut name = String::new();
    name.try_reserve_exact(64)
        .map_err(|_| Error::Policy(Failure::Allocation))?;
    write!(&mut name, ".{role}-{}-{serial}.tmp", std::process::id())
        .map_err(|_| Error::Code(INTERNAL))?;
    Ok(name)
}

fn admit_delete(directory: &Dir, depth: usize, budget: &mut Budget) -> Result<(), Error> {
    if depth >= 32 {
        return Err(Error::Policy(Failure::Limit("save directory depth")));
    }
    budget.work(256)?;
    for entry in directory.entries().map_err(code)? {
        budget.work(1024)?;
        let entry = entry.map_err(code)?;
        let name = entry.file_name();
        let metadata = directory.symlink_metadata(&name).map_err(code)?;
        if !ordinary(&metadata) {
            return Err(Error::Code(NO_PERMISSION));
        }
        if metadata.is_dir() {
            admit_delete(&directory.open_dir(&name).map_err(code)?, depth + 1, budget)?;
        } else if !metadata.is_file() {
            return Err(Error::Code(NO_PERMISSION));
        }
    }
    Ok(())
}
