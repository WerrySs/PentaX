// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: writable owned save mounts, exact descriptor access
// and bounded checked input. Source fileSystem.cpp and saveDataMountSlots.h.
// PentaX persistence extensions: owned writable-file Fsync flushes actual data
// (the pinned diagnostic source was a no-op), and write-only descriptors reject
// reads with BadFd. Existing read-only roots and standard streams keep their
// original behavior. Every path remains relative to a selected directory cap.
use super::*;

/// An unpublished owned mount; dropping it cannot expose a guest namespace.
/// Only prepare constructs this value. Commit consumes it without allocation.
pub(crate) struct PreparedSaveMount {
    slot: usize,
    directory: Dir,
}

pub(super) fn namespace(path: &str) -> Option<(usize, &str)> {
    let tail = path.strip_prefix("/savedata")?;
    let (number, relative) = tail.split_once('/').unwrap_or((tail, ""));
    let slot: usize = number.parse().ok()?;
    if slot >= 16
        || number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
        || number.len() > 1 && number.starts_with('0')
    {
        return None;
    }
    Some((slot, relative))
}

impl FileState {
    pub(crate) fn prepare_save_mount(
        &self,
        slot: usize,
        directory: Dir,
        budget: &mut Budget,
    ) -> Result<super::PreparedSaveMount, Failure> {
        if slot >= self.save_mounts.len() {
            return Err(Errno::Invalid.into());
        }
        if self.save_mounts[slot].is_some() {
            return Err(Errno::Busy.into());
        }
        budget.work(272 + self.roots.len() as u64 * 32)?;
        // A new writable capability must never supersede an explicit read-only
        // root, or a more specific read-only root below this mount point.
        let mut point = [0_u8; 16];
        let prefix = b"/savedata";
        point[..prefix.len()].copy_from_slice(prefix);
        let length = if slot < 10 {
            point[9] = b'0' + slot as u8;
            10
        } else {
            point[9] = b'1';
            point[10] = b'0' + (slot - 10) as u8;
            11
        };
        let mount = std::str::from_utf8(&point[..length]).map_err(|_| Errno::Invalid)?;
        if self.roots.iter().any(|root| {
            root.mount == mount
                || root
                    .mount
                    .strip_prefix(mount)
                    .is_some_and(|suffix| suffix.starts_with('/'))
                || mount
                    .strip_prefix(&root.mount)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        }) {
            return Err(Errno::ReadOnly.into());
        }
        if !directory.dir_metadata().map_err(directory_error)?.is_dir() {
            return Err(Errno::NotDirectory.into());
        }
        Ok(PreparedSaveMount { slot, directory })
    }
    pub(crate) fn commit_save_mount(&mut self, mount: PreparedSaveMount) {
        debug_assert!(self.save_mounts[mount.slot].is_none());
        self.save_mounts[mount.slot] = Some(mount.directory);
    }
    pub(crate) fn unmount_save(&mut self, slot: usize) {
        if let Some(mount) = self.save_mounts.get_mut(slot) {
            *mount = None;
        }
        // Open descriptors own independent File handles, as in the source's
        // separate descriptor/mount tables. Unmount changes only future lookup.
    }
    fn save_path<'a>(&self, path: &'a str) -> Option<(&Dir, &'a str)> {
        if !path.starts_with('/') || !safe_components(&path[1..]) {
            return None;
        }
        let (slot, relative) = namespace(path)?;
        if !relative.is_empty() && !directory_name(relative) {
            return None;
        }
        self.save_mounts[slot]
            .as_ref()
            .map(|directory| (directory, relative))
    }
    pub(super) fn selected_save(
        &self,
        path: &str,
        budget: &mut Budget,
    ) -> Result<Option<File>, Failure> {
        let Some((directory, relative)) = self.save_path(path) else {
            return Ok(None);
        };
        budget.work(path.len() as u64 * 64 + 256)?;
        if relative.is_empty() {
            return directory
                .try_clone()
                .map(|dir| Some(dir.into_std_file()))
                .map_err(directory_error);
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.share_mode(7);
        }
        match directory.open_with(relative, &options) {
            Ok(file) => Ok(Some(file.into_std())),
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::IsADirectory | ErrorKind::PermissionDenied
                ) =>
            {
                match directory.open_dir(relative) {
                    Ok(dir) => Ok(Some(dir.into_std_file())),
                    Err(_) => Err(directory_error(error)),
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(directory_error(error)),
        }
    }
    pub(super) fn open_save_file(
        &mut self,
        path: &str,
        flags: u32,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if flags & 3 == 3 {
            return Err(Failure::Unsupported("source-aborting file access mode"));
        }
        const ALLOWED: u32 =
            3 | 0x8 | 0x80 | 0x200 | 0x400 | 0x800 | 0x1000 | 0x10000 | 0x20000 | 0x100000;
        if flags & !ALLOWED != 0 {
            return Err(Failure::Unsupported("save file open flag profile"));
        }
        let original_path = path;
        let path = path.strip_suffix('/').unwrap_or(path);
        let directory_requested = flags & 0x20000 != 0 || path.len() != original_path.len();
        let readable = flags & 3 != 1;
        let writable = flags & 3 != 0;
        let create = flags & 0x200 != 0;
        let truncate = flags & 0x400 != 0;
        let append = flags & 8 != 0;
        let sync = flags & (0x80 | 0x1000) != 0;
        if truncate && !writable {
            return Err(Errno::Access.into());
        }
        if flags & 0x20000 != 0 && (writable || create || truncate) {
            return Err(Failure::Unsupported("writable directory descriptor"));
        }
        budget.can_add_handle()?;
        budget.work(self.open.len() as u64 + path.len() as u64 * 64 + 256)?;
        let index = self
            .open
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.open.len());
        let (directory, relative) = self.save_path(path).ok_or(Errno::NoEntry)?;
        if relative.is_empty() || directory_requested {
            if writable || create || truncate {
                return Err(Errno::IsDirectory.into());
            }
            let file = if relative.is_empty() {
                directory.try_clone().map_err(directory_error)?
            } else {
                directory.open_dir(relative).map_err(directory_error)?
            };
            let snapshot = directories::capture(file.into_std_file(), budget)?;
            budget.can_retain_directory(snapshot.bytes.capacity() as u64)?;
            let storage = snapshot.bytes.capacity() as u64;
            self.publish_save_descriptor(
                index,
                FileData::Directory(snapshot),
                true,
                false,
                false,
                false,
                0,
            );
            budget.added_handle();
            budget.retained_directory(storage);
            return Ok((index + 3) as u64);
        }
        // Inspect an existing capability-relative entry before any truncation.
        // cap-std confines final opens too; no ambient or joined host path exists.
        let existing = match directory.metadata(relative) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(directory_error(error)),
        };
        if existing.as_ref().is_some_and(|metadata| metadata.is_dir()) {
            if writable || create || truncate {
                return Err(Errno::IsDirectory.into());
            }
            let snapshot = directories::capture(
                directory
                    .open_dir(relative)
                    .map_err(directory_error)?
                    .into_std_file(),
                budget,
            )?;
            budget.can_retain_directory(snapshot.bytes.capacity() as u64)?;
            let storage = snapshot.bytes.capacity() as u64;
            self.publish_save_descriptor(
                index,
                FileData::Directory(snapshot),
                true,
                false,
                false,
                false,
                0,
            );
            budget.added_handle();
            budget.retained_directory(storage);
            return Ok((index + 3) as u64);
        }
        if existing
            .as_ref()
            .is_some_and(|metadata| !metadata.is_file())
        {
            return Err(Failure::Unsupported("save special file"));
        }
        if existing.is_none() && !create {
            return Err(Errno::NoEntry.into());
        }
        if existing.is_some() && create && flags & 0x800 != 0 {
            return Err(Errno::Exists.into());
        }
        let old_size = existing.as_ref().map_or(0, |metadata| metadata.len());
        if truncate {
            budget.save_io(old_size, true)?;
        }
        let mut options = OpenOptions::new();
        options
            .read(readable)
            .write(writable || create && existing.is_none())
            .create_new(create && existing.is_none());
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.share_mode(7);
        }
        if create
            && existing.is_none()
            && let Some((parent, _)) = relative.rsplit_once('/')
        {
            directory.create_dir_all(parent).map_err(directory_error)?;
        }
        let file = directory
            .open_with(relative, &options)
            .map_err(directory_error)?
            .into_std();
        let metadata = file.metadata().map_err(|_| Errno::Io)?;
        if !metadata.is_file() {
            return Err(Failure::Unsupported("save special file"));
        }
        if truncate {
            file.set_len(0).map_err(|_| Errno::Io)?;
        }
        let position = if append && !truncate {
            metadata.len()
        } else {
            0
        };
        // All guest/limit checks and descriptor storage admission precede the
        // persistent change. Publishing the owned handle allocates nothing.
        self.publish_save_descriptor(
            index,
            FileData::Regular(file),
            readable,
            writable,
            append,
            sync,
            position,
        );
        budget.added_handle();
        Ok((index + 3) as u64)
    }
    #[allow(clippy::too_many_arguments)]
    fn publish_save_descriptor(
        &mut self,
        index: usize,
        data: FileData,
        readable: bool,
        writable: bool,
        append: bool,
        sync: bool,
        position: u64,
    ) {
        let opened = Some(OpenFile {
            data,
            position,
            readable,
            writable,
            append,
            sync,
            save: true,
        });
        if index == self.open.len() {
            self.open.push(opened);
        } else {
            self.open[index] = opened;
        }
    }
    pub(crate) fn write_save_file(
        &mut self,
        service: Service,
        a: [u64; 6],
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        // KernelWrite checks NULL first; Pwrite checks the signed descriptor,
        // then NULL and signed64 offset, before its ordinary descriptor lookup.
        let descriptor = a[0] as u32 as i32;
        if service == Service::Pwrite && descriptor < 3 {
            return Err(Errno::Perm.into());
        }
        if a[1] == 0 {
            return Err(Errno::Fault.into());
        }
        if service == Service::Pwrite && (a[3] as i64) < 0 {
            return Err(Errno::Invalid.into());
        }
        let index = self.index(a[0], Errno::BadFd, budget)?;
        let opened = self.open[index].as_mut().ok_or(Errno::BadFd)?;
        if !opened.save || !opened.writable {
            return Err(Errno::BadFd.into());
        }
        let FileData::Regular(file) = &opened.data else {
            return Err(Errno::Invalid.into());
        };
        let length = usize::try_from(a[2]).map_err(|_| Failure::Limit("save I/O bytes"))?;
        if length > self.limits.max_io_bytes || a[2] > u32::MAX as u64 {
            return Err(Failure::Limit("save I/O bytes"));
        }
        let offset = if opened.append {
            file.metadata().map_err(|_| Errno::Io)?.len()
        } else if service == Service::Pwrite {
            a[3]
        } else {
            opened.position
        };
        offset
            .checked_add(a[2])
            .filter(|end| *end <= i64::MAX as u64 && *end <= self.limits.max_save_io_bytes)
            .ok_or(Failure::Limit("save file extent"))?;
        let mut bytes = budget.buffer(length)?;
        budget.read(guest, a[1], &mut bytes)?;
        budget.save_io(a[2], true)?;
        let written = positional_write(file, &bytes, offset)?;
        if service == Service::Write {
            opened.position = offset + written as u64;
        }
        if opened.sync {
            file.sync_all().map_err(|_| Errno::Io)?;
        }
        Ok(written as u64)
    }
    pub(super) fn save_descriptor_control(
        &mut self,
        service: Service,
        a: [u64; 6],
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if service == Service::Ftruncate && (a[0] as u32 as i32) < 3 {
            return Err(Errno::BadFd.into());
        }
        if service == Service::Ftruncate && (a[1] as i64) < 0 {
            return Err(Errno::Invalid.into());
        }
        let index = self.index(a[0], Errno::BadFd, budget)?;
        let opened = self.open[index].as_ref().ok_or(Errno::BadFd)?;
        if !opened.save || !opened.writable {
            return Err(Errno::BadFd.into());
        }
        let FileData::Regular(file) = &opened.data else {
            return Err(Errno::Invalid.into());
        };
        budget.work(256)?;
        if service == Service::Fsync {
            file.sync_all().map_err(|_| Errno::Io)?;
            return Ok(0);
        }
        if a[1] > self.limits.max_save_io_bytes {
            return Err(Failure::Limit("save file extent"));
        }
        let old = file.metadata().map_err(|_| Errno::Io)?.len();
        budget.save_io(old.abs_diff(a[1]), true)?;
        file.set_len(a[1]).map_err(|_| Errno::Io)?;
        Ok(0)
    }
    pub(super) fn save_path_operation(
        &mut self,
        service: Service,
        a: [u64; 6],
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if a[0] == 0 || service == Service::Rename && a[1] == 0 {
            return Err(Errno::Invalid.into());
        }
        let path = budget.name(guest, a[0])?;
        if service == Service::Mkdir && path.len() > 255 {
            return Err(Errno::NameTooLong.into());
        }
        let (directory, relative) = self.save_path(&path).ok_or_else(|| {
            if namespace(&path).is_some() {
                Failure::Errno(Errno::NoEntry)
            } else {
                Failure::Errno(Errno::ReadOnly)
            }
        })?;
        if relative.is_empty() {
            return Err(Errno::Perm.into());
        }
        budget.work(path.len() as u64 * 64 + 256)?;
        match service {
            Service::Mkdir => directory.create_dir(relative).map_err(directory_error)?,
            Service::Rmdir => directory.remove_dir(relative).map_err(directory_error)?,
            Service::Unlink => {
                let metadata = directory.metadata(relative).map_err(directory_error)?;
                if metadata.is_dir() {
                    return Err(Errno::Perm.into());
                }
                if !metadata.is_file() {
                    return Err(Failure::Unsupported("save special file"));
                }
                directory.remove_file(relative).map_err(directory_error)?;
            }
            Service::Rename => {
                let target = budget.name(guest, a[1])?;
                let (destination, to) = self.save_path(&target).ok_or(Errno::ReadOnly)?;
                if to.is_empty() {
                    return Err(Errno::Perm.into());
                }
                if namespace(&path).map(|(slot, _)| slot)
                    != namespace(&target).map(|(slot, _)| slot)
                {
                    return Err(Failure::Unsupported("cross-save-mount rename"));
                }
                budget.work(target.len() as u64 * 64 + 256)?;
                if !directory
                    .metadata(relative)
                    .map_err(directory_error)?
                    .is_file()
                {
                    return Err(Errno::NoEntry.into());
                }
                if let Some((parent, _)) = to.rsplit_once('/') {
                    destination
                        .create_dir_all(parent)
                        .map_err(directory_error)?;
                }
                directory
                    .rename(relative, destination, to)
                    .map_err(directory_error)?;
            }
            _ => return Err(Failure::Unsupported("save path operation")),
        }
        Ok(0)
    }
    pub(crate) fn save_icon_bytes(
        &self,
        path: &str,
        maximum: usize,
        held: usize,
        budget: &mut Budget,
    ) -> Result<Vec<u8>, Failure> {
        let file = self.selected(path, budget)?.ok_or(Errno::NoEntry)?;
        let before = file.metadata().map_err(|_| Errno::Io)?;
        if !before.is_file() {
            return Err(Errno::IsDirectory.into());
        }
        let length =
            usize::try_from(before.len()).map_err(|_| Failure::Limit("save icon input"))?;
        if length > maximum || length > self.limits.max_io_bytes {
            return Err(Failure::Limit("save icon input"));
        }
        let mut bytes = budget.buffer_held(length, held)?;
        budget.save_io(length as u64, false)?;
        let mut at = 0;
        while at < length {
            let count = positional_read(&file, &mut bytes[at..], at as u64)?;
            if count == 0 {
                return Err(Errno::Io.into());
            }
            at += count;
        }
        let after = file.metadata().map_err(|_| Errno::Io)?;
        if before.len() != after.len()
            || before.modified().map_err(|_| Errno::Io)?
                != after.modified().map_err(|_| Errno::Io)?
        {
            return Err(Errno::Io.into());
        }
        Ok(bytes)
    }
}

#[cfg(windows)]
fn positional_write(file: &File, bytes: &[u8], offset: u64) -> Result<usize, Failure> {
    use std::os::windows::fs::FileExt;
    file.seek_write(bytes, offset).map_err(|_| Errno::Io.into())
}
#[cfg(unix)]
fn positional_write(file: &File, bytes: &[u8], offset: u64) -> Result<usize, Failure> {
    use std::os::unix::fs::FileExt;
    file.write_at(bytes, offset).map_err(|_| Errno::Io.into())
}
#[cfg(not(any(windows, unix)))]
fn positional_write(_file: &File, _bytes: &[u8], _offset: u64) -> Result<usize, Failure> {
    Err(Failure::Unsupported("host positional save writes"))
}

#[cfg(test)]
mod tests;
