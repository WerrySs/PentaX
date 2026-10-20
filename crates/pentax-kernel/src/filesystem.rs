// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: explicit read-only file capabilities, checked guest
// buffers, transactional logical positions and bounded descriptors; see NOTICE.

use crate::budget::{Budget, Limits};
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Service};
use cap_std::fs::{Dir, OpenOptions};
use std::fs::{File, Metadata};
use std::io::ErrorKind;
use std::time::{SystemTime, UNIX_EPOCH};

mod apr;
mod content;
mod directories;
mod trophy;

enum FileData {
    Regular(File),
    Directory(directories::Snapshot),
}

/// An explicitly selected host file capability. Guest input never names a host
/// path. This crate only reads the handle, even if its caller opened it writable.
pub struct ReadOnlyFile {
    relative: String,
    file: File,
}

fn safe_components(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn owned_name(name: &str) -> Result<String, Failure> {
    let mut value = String::new();
    value
        .try_reserve_exact(name.len())
        .map_err(|_| Failure::Allocation)?;
    value.push_str(name);
    Ok(value)
}

impl ReadOnlyFile {
    pub fn new(relative: &str, file: File) -> Result<Self, Failure> {
        if relative.len() > 1024 || !safe_components(relative) {
            return Err(Errno::Invalid.into());
        }
        if !file.metadata().map_err(|_| Errno::Io)?.is_file() {
            return Err(Errno::IsDirectory.into());
        }
        Ok(Self {
            relative: owned_name(relative)?,
            file,
        })
    }
}

/// One explicit read-only guest mount: an inventory or an owned directory.
/// Directory lookup stays inside its handle capability; no ambient host path
/// is formed from a guest name. No recursive discovery occurs at admission.
pub struct ReadOnlyRoot {
    mount: String,
    files: Vec<ReadOnlyFile>,
    directory: Option<Dir>,
}

impl ReadOnlyRoot {
    pub fn new(mount: &str, files: Vec<ReadOnlyFile>) -> Result<Self, Failure> {
        if !mount.starts_with('/') || mount.len() > 1024 || !safe_components(&mount[1..]) {
            return Err(Errno::Invalid.into());
        }
        Ok(Self {
            mount: owned_name(mount)?,
            files,
            directory: None,
        })
    }

    /// Retain a caller-selected directory without enumerating or reading it.
    /// Reopening establishes cap-std's required directory-handle semantics,
    /// including exclusion of directory replacement on Windows.
    pub fn directory(mount: &str, directory: File) -> Result<Self, Failure> {
        let mut root = Self::new(mount, Vec::new())?;
        if !directory.metadata().map_err(|_| Errno::Io)?.is_dir() {
            return Err(Errno::NotDirectory.into());
        }
        root.directory = Some(Dir::reopen_dir(&directory).map_err(directory_error)?);
        Ok(root)
    }
}

fn directory_error(error: std::io::Error) -> Failure {
    match error.kind() {
        ErrorKind::NotFound => Errno::NoEntry.into(),
        ErrorKind::PermissionDenied => Errno::Access.into(),
        ErrorKind::NotADirectory => Errno::NotDirectory.into(),
        ErrorKind::IsADirectory => Errno::IsDirectory.into(),
        ErrorKind::InvalidInput | ErrorKind::InvalidFilename => Errno::Invalid.into(),
        ErrorKind::OutOfMemory => Failure::Allocation,
        ErrorKind::Unsupported => Failure::Unsupported("host directory capability operation"),
        _ => Errno::Io.into(),
    }
}

fn directory_name(path: &str) -> bool {
    // Avoid DOS device/ADS names and Windows-normalized aliases. The generic
    // guest canonical-path check runs first; these checks apply only where a
    // guest component actually reaches a host directory resolver.
    path.split('/').all(|part| {
        if part.ends_with(['.', ' ']) || part.chars().any(|ch| ch < ' ' || "<>\"|?*".contains(ch)) {
            return false;
        }
        let stem = part.split('.').next().unwrap_or("");
        !stem.ends_with(' ')
            && !["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
                .iter()
                .any(|name| stem.eq_ignore_ascii_case(name))
            && !(stem.get(..3).is_some_and(|prefix| {
                prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT")
            }) && stem.get(3..).is_some_and(|number| {
                matches!(
                    number,
                    "0" | "1"
                        | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        | "\u{00b9}"
                        | "\u{00b2}"
                        | "\u{00b3}"
                )
            }))
    })
}

struct OpenFile {
    data: FileData,
    position: u64,
}
pub(crate) struct FileState {
    roots: Vec<ReadOnlyRoot>,
    open: Vec<Option<OpenFile>>,
    limits: Limits,
    apr: apr::State,
}

impl FileState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.open.iter().filter(|entry| entry.is_some()).count()
    }

    pub(crate) fn new(
        limits: Limits,
        roots: Vec<ReadOnlyRoot>,
        budget: &mut Budget,
    ) -> Result<Self, Failure> {
        if roots.len() > limits.max_handles {
            return Err(Failure::Limit("mount roots"));
        }
        let total_count = roots
            .iter()
            .try_fold(0_usize, |count, root| count.checked_add(root.files.len()))
            .ok_or(Failure::Limit("file capabilities"))?;
        if total_count > limits.max_handles {
            return Err(Failure::Limit("file capabilities"));
        }
        budget.work((roots.len() + total_count) as u64)?;
        let mut comparison_work = 0_u64;
        for root in &roots {
            let bytes = root
                .files
                .iter()
                .try_fold(0_u64, |count, file| {
                    count.checked_add(file.relative.len() as u64)
                })
                .ok_or(Failure::Limit("file admission work"))?;
            comparison_work = comparison_work
                .checked_add(
                    bytes
                        .checked_mul(root.files.len() as u64)
                        .ok_or(Failure::Limit("file admission work"))?,
                )
                .ok_or(Failure::Limit("file admission work"))?;
            comparison_work = comparison_work
                .checked_add(
                    (root.mount.len() as u64)
                        .checked_mul(roots.len() as u64)
                        .ok_or(Failure::Limit("file admission work"))?,
                )
                .ok_or(Failure::Limit("file admission work"))?;
        }
        budget.work(comparison_work)?;
        let mut total = 0_usize;
        for (index, root) in roots.iter().enumerate() {
            total = total
                .checked_add(root.files.len())
                .ok_or(Failure::Limit("file capabilities"))?;
            if total > limits.max_handles || root.mount.len() > limits.max_name_bytes {
                return Err(Failure::Limit("file capabilities"));
            }
            if roots[..index].iter().any(|prior| prior.mount == root.mount) {
                return Err(Errno::Exists.into());
            }
            for (file_index, file) in root.files.iter().enumerate() {
                if file.relative.len() > limits.max_name_bytes {
                    return Err(Failure::Limit("file capability names"));
                }
                if root.files[..file_index]
                    .iter()
                    .any(|prior| prior.relative == file.relative)
                {
                    return Err(Errno::Exists.into());
                }
            }
        }
        let mut open = Vec::new();
        open.try_reserve_exact(limits.max_handles)
            .map_err(|_| Failure::Allocation)?;
        Ok(Self {
            roots,
            open,
            limits,
            apr: apr::State::default(),
        })
    }

    fn selected(&self, path: &str, budget: &mut Budget) -> Result<Option<File>, Failure> {
        let trimmed = path.strip_suffix('/').unwrap_or(path);
        let file = self.selected_canonical(trimmed, budget)?;
        if trimmed.len() != path.len()
            && let Some(file) = &file
            && !file.metadata().map_err(|_| Errno::Io)?.is_dir()
        {
            return Err(Errno::NotDirectory.into());
        }
        Ok(file)
    }

    fn selected_canonical(&self, path: &str, budget: &mut Budget) -> Result<Option<File>, Failure> {
        if !path.starts_with('/') || !safe_components(&path[1..]) {
            return Err(Errno::NoEntry.into());
        }
        budget.work(self.roots.len() as u64)?;
        // Longest mount prefix wins, as in source mount resolution.
        let root = self
            .roots
            .iter()
            .filter(|root| {
                (root.directory.is_some() && path == root.mount)
                    || path
                        .strip_prefix(&root.mount)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            })
            .max_by_key(|root| root.mount.len());
        let Some(root) = root else {
            return Ok(None);
        };
        if let Some(directory) = &root.directory {
            // Charge resolver work before an OS operation; bounded names and
            // cap-std's finite symlink walk prevent unbounded path traversal.
            budget.work(path.len() as u64 * 64 + 256)?;
            if path == root.mount {
                return directory
                    .try_clone()
                    .map(|dir| Some(dir.into_std_file()))
                    .map_err(directory_error);
            }
            let relative = &path[root.mount.len() + 1..];
            if !directory_name(relative) {
                return Err(Errno::NoEntry.into());
            }
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(windows)]
            {
                use cap_std::fs::OpenOptionsExt;
                // Retain a real read-only file handle that excludes writers
                // and replacement while this descriptor/query is alive.
                options.share_mode(1);
            }
            return match directory.open_with(relative, &options) {
                Ok(file) => Ok(Some(file.into_std())),
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::IsADirectory | ErrorKind::PermissionDenied
                    ) =>
                {
                    // Windows file opens can reject directories. A separately
                    // scoped directory open admits reachability without
                    // pretending it is a readable regular-file descriptor.
                    match directory.open_dir(relative) {
                        Ok(dir) => Ok(Some(dir.into_std_file())),
                        Err(_) => Err(directory_error(error)),
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
                Err(error) => Err(directory_error(error)),
            };
        }
        budget.work(root.files.len() as u64)?;
        let relative = &path[root.mount.len() + 1..];
        root.files
            .iter()
            .find(|file| file.relative == relative)
            .map(|file| file.file.try_clone().map_err(|_| Errno::Io.into()))
            .transpose()
    }

    fn index(
        &self,
        descriptor: u64,
        low_error: Errno,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        // Library C int arguments consume their low 32 bits.
        let descriptor = descriptor as u32 as i32;
        if descriptor < 3 {
            return Err(low_error.into());
        }
        let index = (descriptor - 3) as usize;
        budget.work(1)?;
        if self.open.get(index).is_none_or(Option::is_none) {
            return Err(Errno::BadFd.into());
        }
        Ok(index)
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let a = args.0;
        match service {
            Service::Open => self.open_file(a, guest, budget),
            Service::Close => {
                let index = self.index(a[0], Errno::Perm, budget)?;
                let file = self.open[index].take().ok_or(Errno::BadFd)?;
                if let FileData::Directory(directory) = file.data {
                    budget.released_directory(directory.bytes.capacity() as u64);
                }
                budget.removed_handle();
                Ok(0)
            }
            Service::Read | Service::Pread => self.read_file(service, a, guest, budget),
            Service::Lseek => self.seek_file(a, budget),
            Service::Getdirentries | Service::Getdents => {
                self.get_directory(service, a, guest, budget)
            }
            Service::Fstat => {
                // Source checks the low descriptor first, then null output,
                // then whether an ordinary descriptor exists.
                if (a[0] as u32 as i32) < 3 {
                    return Err(Errno::Perm.into());
                }
                if a[1] == 0 {
                    return Err(Errno::Fault.into());
                }
                let index = self.index(a[0], Errno::Perm, budget)?;
                let bytes = match &self.open[index].as_ref().ok_or(Errno::BadFd)?.data {
                    FileData::Regular(file) => {
                        stat_bytes(&file.metadata().map_err(|_| Errno::Io)?)?
                    }
                    FileData::Directory(directory) => directory_stat(directory.bytes.len())?,
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            Service::Stat => {
                if a[0] == 0 || a[1] == 0 {
                    return Err(Errno::Invalid.into());
                }
                let path = budget.name(guest, a[0])?;
                let file = self.selected(&path, budget)?.ok_or(Errno::NoEntry)?;
                let metadata = file.metadata().map_err(|_| Errno::Io)?;
                let bytes = if metadata.is_dir() {
                    let snapshot = directories::capture(file, budget)?;
                    directory_stat(snapshot.bytes.len())?
                } else {
                    stat_bytes(&metadata)?
                };
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            Service::CheckReachability => {
                if a[0] == 0 {
                    return Err(Errno::Invalid.into());
                }
                let path = budget.name(guest, a[0])?;
                if path.len() > 255 {
                    return Err(Errno::NameTooLong.into());
                }
                if self.selected(&path, budget)?.is_some() {
                    Ok(0)
                } else {
                    Err(Errno::NoEntry.into())
                }
            }
            _ => Err(Failure::Unsupported("file service")),
        }
    }

    fn open_file(
        &mut self,
        a: [u64; 6],
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if a[0] == 0 {
            return Err(Errno::Invalid.into());
        }
        let path = budget.name(guest, a[0])?;
        let flags = a[1] as u32;
        if flags & 3 == 3 {
            return Err(Failure::Unsupported("source-aborting file access mode"));
        }
        if flags & 3 != 0 || flags & (0x8 | 0x200 | 0x400 | 0x800 | 0x80 | 0x1000) != 0 {
            return Err(Errno::ReadOnly.into());
        }
        if flags & !(0x10000 | 0x100000 | 0x20000) != 0 {
            return Err(Failure::Unsupported("file open flag profile"));
        }
        budget.can_add_handle()?;
        let file = self.selected(&path, budget)?.ok_or(Errno::NoEntry)?;
        let metadata = file.metadata().map_err(|_| Errno::Io)?;
        let data = if metadata.is_dir() {
            let snapshot = directories::capture(file, budget)?;
            budget.can_retain_directory(snapshot.bytes.capacity() as u64)?;
            FileData::Directory(snapshot)
        } else {
            if flags & 0x20000 != 0 {
                return Err(Errno::NotDirectory.into());
            }
            if !metadata.is_file() {
                return Err(Failure::Unsupported("device descriptor"));
            }
            FileData::Regular(file)
        };
        budget.work(self.open.len() as u64)?;
        let index = self
            .open
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.open.len());
        let storage = match &data {
            FileData::Directory(directory) => directory.bytes.capacity() as u64,
            _ => 0,
        };
        let opened = Some(OpenFile { data, position: 0 });
        if index == self.open.len() {
            self.open.push(opened);
        } else {
            self.open[index] = opened;
        }
        budget.added_handle();
        budget.retained_directory(storage);
        Ok((index + 3) as u64)
    }

    fn read_file(
        &mut self,
        service: Service,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if (a[0] as u32 as i32) < 3 {
            return Err(Errno::Perm.into());
        }
        if a[1] == 0 {
            return Err(Errno::Fault.into());
        }
        if service == Service::Pread && (a[3] as i64) < 0 {
            return Err(Errno::Invalid.into());
        }
        let index = self.index(a[0], Errno::Perm, budget)?;
        if service == Service::Read && a[2] > i32::MAX as u64 {
            return Err(Errno::Invalid.into());
        }
        let opened = self.open[index].as_mut().ok_or(Errno::BadFd)?;
        if matches!(opened.data, FileData::Directory(_)) && service == Service::Pread {
            return Err(Failure::Unsupported("directory pread"));
        }
        if a[2] > self.limits.max_io_bytes as u64 {
            return Err(Failure::Limit("file I/O bytes"));
        }
        if let FileData::Directory(directory) = &opened.data {
            let remaining = (directory.bytes.len() as u64).saturating_sub(opened.position);
            let count = a[2].min(remaining) as usize;
            budget.work(count as u64)?;
            if count != 0 {
                let offset = opened.position as usize;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &directory.bytes[offset..offset + count],
                    }],
                )?;
                opened.position += count as u64;
            }
            return Ok(count as u64);
        }
        let offset = if service == Service::Pread {
            a[3]
        } else {
            opened.position
        };
        let mut bytes = budget.buffer(a[2] as usize)?;
        // Positional host reads do not race a shared OS cursor on cloned file
        // handles. The logical guest cursor commits only after guest output.
        budget.work(a[2])?;
        let FileData::Regular(file) = &opened.data else {
            unreachable!()
        };
        let count = positional_read(file, &mut bytes, offset)?;
        let next = offset
            .checked_add(count as u64)
            .filter(|v| *v <= i64::MAX as u64)
            .ok_or(Errno::Overflow)?;
        budget.write(
            guest,
            &[GuestWrite {
                address: a[1],
                bytes: &bytes[..count],
            }],
        )?;
        if service == Service::Read {
            opened.position = next;
        }
        Ok(count as u64)
    }

    fn seek_file(&mut self, a: [u64; 6], budget: &mut Budget) -> Result<u64, Failure> {
        let index = self.index(a[0], Errno::BadFd, budget)?;
        let opened = self.open[index].as_mut().ok_or(Errno::BadFd)?;
        let base = match a[2] as u32 as i32 {
            0 => 0,
            1 => opened.position,
            2 => match &opened.data {
                FileData::Regular(file) => file.metadata().map_err(|_| Errno::Io)?.len(),
                FileData::Directory(directory) => directory.bytes.len() as u64,
            },
            _ => return Err(Errno::Invalid.into()),
        };
        let position = i128::from(base) + i128::from(a[1] as i64);
        if position < 0 {
            return Err(Errno::Invalid.into());
        }
        if position > i128::from(i64::MAX) {
            return Err(Errno::Overflow.into());
        }
        opened.position = position as u64;
        Ok(opened.position)
    }

    fn get_directory(
        &mut self,
        service: Service,
        a: [u64; 6],
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if (a[0] as u32 as i32) < 3 {
            return Err(Errno::BadFd.into());
        }
        if a[1] == 0 {
            return Err(Errno::Fault.into());
        }
        let index = self.index(a[0], Errno::BadFd, budget)?;
        let opened = self.open[index].as_mut().ok_or(Errno::BadFd)?;
        let FileData::Directory(directory) = &opened.data else {
            return Err(Errno::Invalid.into());
        };
        let requested = a[2] as u32 as i32;
        if requested < directories::BLOCK as i32
            || opened.position > directory.bytes.len() as u64
            || !opened.position.is_multiple_of(directories::BLOCK as u64)
        {
            return Err(Errno::Invalid.into());
        }
        if requested as usize > self.limits.max_io_bytes {
            return Err(Failure::Limit("directory I/O bytes"));
        }
        let count = ((requested as usize) & !(directories::BLOCK - 1))
            .min(directory.bytes.len() - opened.position as usize);
        budget.work(count as u64)?;
        let offset = opened.position as usize;
        let base = opened.position.to_le_bytes();
        let writes = [
            GuestWrite {
                address: a[1],
                bytes: &directory.bytes[offset..offset + count],
            },
            GuestWrite {
                address: a[3],
                bytes: &base,
            },
        ];
        let mut begin = 0;
        if count == 0 {
            begin = 1;
        }
        let end = if service == Service::Getdirentries && a[3] != 0 {
            2
        } else {
            1
        };
        if begin < end {
            budget.write(guest, &writes[begin..end])?;
        }
        opened.position += count as u64;
        Ok(count as u64)
    }
}

#[cfg(windows)]
fn positional_read(file: &File, bytes: &mut [u8], offset: u64) -> Result<usize, Failure> {
    use std::os::windows::fs::FileExt;
    file.seek_read(bytes, offset).map_err(|_| Errno::Io.into())
}
#[cfg(unix)]
fn positional_read(file: &File, bytes: &mut [u8], offset: u64) -> Result<usize, Failure> {
    use std::os::unix::fs::FileExt;
    file.read_at(bytes, offset).map_err(|_| Errno::Io.into())
}
#[cfg(not(any(windows, unix)))]
fn positional_read(_file: &File, _bytes: &mut [u8], _offset: u64) -> Result<usize, Failure> {
    Err(Failure::Unsupported("host positional file reads"))
}

#[repr(C)]
struct Timespec {
    sec: i64,
    nsec: i64,
}
#[repr(C)]
struct FileStatLayout {
    dev: u32,
    ino: u32,
    mode: u16,
    nlink: u16,
    uid: u32,
    gid: u32,
    rdev: u32,
    atim: Timespec,
    mtim: Timespec,
    ctim: Timespec,
    size: i64,
    blocks: i64,
    blksize: u32,
    flags: u32,
    generation: u32,
    lspare: i32,
    birthtim: Timespec,
}
const _: () = assert!(std::mem::size_of::<FileStatLayout>() == 120);
const _: () = assert!(std::mem::offset_of!(FileStatLayout, atim) == 24);
const _: () = assert!(std::mem::offset_of!(FileStatLayout, size) == 72);
const _: () = assert!(std::mem::offset_of!(FileStatLayout, birthtim) == 104);

fn time_bytes(time: SystemTime) -> Result<[u8; 16], Failure> {
    let nanos = match time.duration_since(UNIX_EPOCH) {
        Ok(v) => v.as_nanos() as i128,
        Err(v) => -(v.duration().as_nanos() as i128),
    };
    let sec = i64::try_from(nanos.div_euclid(1_000_000_000)).map_err(|_| Errno::Overflow)?;
    let nsec = nanos.rem_euclid(1_000_000_000) as i64;
    let mut output = [0; 16];
    output[..8].copy_from_slice(&sec.to_le_bytes());
    output[8..].copy_from_slice(&nsec.to_le_bytes());
    Ok(output)
}

fn stat_bytes(metadata: &Metadata) -> Result<[u8; 120], Failure> {
    if !metadata.is_file() {
        // Directory status uses its packed snapshot in the separate branch.
        // No host device type is fabricated as a regular-file status.
        return Err(Failure::Unsupported("directory or device status"));
    }
    let size = i64::try_from(metadata.len()).map_err(|_| Errno::Overflow)?;
    let mut output = [0; 120];
    // Source exposes regular-file type plus 0777, independent of host ACLs.
    output[8..10].copy_from_slice(&0o100777_u16.to_le_bytes());
    let accessed = metadata.accessed().map_err(|_| Errno::Io)?;
    let modified = metadata.modified().map_err(|_| Errno::Io)?;
    let atime = time_bytes(accessed)?;
    let mtime = time_bytes(modified)?;
    output[24..40].copy_from_slice(&atime);
    output[40..56].copy_from_slice(&mtime);
    output[56..72].copy_from_slice(&atime);
    output[104..120].copy_from_slice(&mtime);
    output[72..80].copy_from_slice(&size.to_le_bytes());
    output[80..88].copy_from_slice(&(size / 512 + i64::from(size % 512 != 0)).to_le_bytes());
    output[88..92].copy_from_slice(&512_u32.to_le_bytes());
    Ok(output)
}

fn directory_stat(size: usize) -> Result<[u8; 120], Failure> {
    let length = i64::try_from(size).map_err(|_| Errno::Overflow)?;
    let mut bytes = [0; 120];
    bytes[8..10].copy_from_slice(&0o040777_u16.to_le_bytes());
    let time = time_bytes(SystemTime::now())?;
    for offset in [24, 40, 56, 104] {
        bytes[offset..offset + 16].copy_from_slice(&time);
    }
    bytes[72..80].copy_from_slice(&length.to_le_bytes());
    bytes[80..88].copy_from_slice(&(length / 512).to_le_bytes());
    bytes[88..92].copy_from_slice(&512_u32.to_le_bytes());
    Ok(bytes)
}
