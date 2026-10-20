// SPDX-License-Identifier: GPL-2.0-only

//! Explicit data files and directory capabilities, without admission scans.

use super::*;
use pentax_kernel::{ReadOnlyFile, ReadOnlyRoot};

fn overlaps(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn file_mount(path: &str) -> &str {
    let mount_end = path[1..]
        .find('/')
        .map(|offset| offset + 1)
        .unwrap_or(path.len());
    &path[..mount_end]
}

#[derive(Default)]
pub(super) struct DataFiles {
    files: Vec<(String, File)>,
    directories: Vec<(String, File)>,
    work: u64,
}
impl DataFiles {
    pub(super) fn bind(
        &mut self,
        binding: &OsString,
        limits: &NativeImageLimits,
    ) -> Result<(), ProbeError> {
        if self.files.len() + self.directories.len() >= limits.startup.kernel.max_handles {
            return Err(ProbeError::Input("data-file-limit"));
        }
        let (guest, host) = binding
            .to_str()
            .and_then(|value| value.split_once('='))
            .filter(|(guest, host)| {
                !host.is_empty()
                    && guest.starts_with('/')
                    && guest[1..].contains('/')
                    && guest.len() < limits.startup.kernel.max_name_bytes
                    && !guest.contains(['\\', ':', '\0'])
                    && guest[1..]
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
            })
            .ok_or(ProbeError::Input("data-file-binding"))?;
        let work = (guest.len() as u64)
            .checked_add(1)
            .and_then(|bytes| {
                bytes.checked_mul(self.files.len() as u64 + self.directories.len() as u64 + 1)
            })
            .and_then(|work| work.checked_mul(4))
            .and_then(|work| work.checked_add(128))
            .ok_or(ProbeError::Input("data-file-work"))?;
        self.work = self
            .work
            .checked_add(work)
            .filter(|value| *value <= limits.max_layout_work_steps)
            .ok_or(ProbeError::Input("data-file-work"))?;
        if self.files.iter().any(|(path, _)| path == guest) {
            return Err(ProbeError::Input("duplicate-data-file"));
        }
        if self
            .directories
            .iter()
            .any(|(path, _)| overlaps(file_mount(guest), path))
        {
            return Err(ProbeError::Input("overlapping-data-mount"));
        }
        self.files
            .try_reserve_exact(1)
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        let mut path = String::new();
        path.try_reserve_exact(guest.len())
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        path.push_str(guest);
        // Retain the selected regular file, read-only, denying writers and
        // replacement through the entire session. No content is copied here.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(host)
            .map_err(|_| ProbeError::Input("data-file-open"))?;
        if !file
            .metadata()
            .map_err(|_| ProbeError::Input("data-file-metadata"))?
            .is_file()
        {
            return Err(ProbeError::Input("data-file-not-regular"));
        }
        self.files.push((path, file));
        Ok(())
    }

    pub(super) fn bind_directory(
        &mut self,
        binding: &OsString,
        limits: &NativeImageLimits,
    ) -> Result<(), ProbeError> {
        if self.files.len() + self.directories.len() >= limits.startup.kernel.max_handles {
            return Err(ProbeError::Input("data-directory-limit"));
        }
        let (guest, host) = binding
            .to_str()
            .and_then(|value| value.split_once('='))
            .filter(|(guest, host)| {
                !host.is_empty()
                    && guest.starts_with('/')
                    && guest.len() < limits.startup.kernel.max_name_bytes
                    && !guest.contains(['\\', ':', '\0'])
                    && guest[1..]
                        .split('/')
                        .all(|part| !part.is_empty() && part != "." && part != "..")
            })
            .ok_or(ProbeError::Input("data-directory-binding"))?;
        let work = (guest.len() as u64 + 1)
            .checked_mul(self.files.len() as u64 + self.directories.len() as u64 + 1)
            .and_then(|value| value.checked_mul(4))
            .and_then(|value| value.checked_add(128))
            .ok_or(ProbeError::Input("data-directory-work"))?;
        self.work = self
            .work
            .checked_add(work)
            .filter(|value| *value <= limits.max_layout_work_steps)
            .ok_or(ProbeError::Input("data-directory-work"))?;
        // Refuse ambiguous overlays regardless of argument order. Per-file
        // inventories group at their first mount component.
        if self
            .directories
            .iter()
            .any(|(path, _)| overlaps(guest, path))
            || self
                .files
                .iter()
                .any(|(path, _)| overlaps(guest, file_mount(path)))
        {
            return Err(ProbeError::Input("overlapping-data-mount"));
        }
        self.directories
            .try_reserve_exact(1)
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        let mut path = String::new();
        path.try_reserve_exact(guest.len())
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        path.push_str(guest);
        let directory = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: open a directory handle.
            .open(host)
            .map_err(|_| ProbeError::Input("data-directory-open"))?;
        if !directory
            .metadata()
            .map_err(|_| ProbeError::Input("data-directory-metadata"))?
            .is_dir()
        {
            return Err(ProbeError::Input("data-directory-not-directory"));
        }
        self.directories.push((path, directory));
        Ok(())
    }

    pub(super) fn into_roots(self) -> Result<Vec<ReadOnlyRoot>, ProbeError> {
        let mut groups: Vec<(String, Vec<ReadOnlyFile>)> = Vec::new();
        groups
            .try_reserve_exact(self.files.len())
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        for (guest, file) in self.files {
            let (mount, relative) = guest[1..]
                .split_once('/')
                .ok_or(ProbeError::Input("data-file-binding"))?;
            let prefix = &guest[..mount.len() + 1];
            let index = if let Some(index) = groups.iter().position(|(path, _)| path == prefix) {
                index
            } else {
                let mut owned = String::new();
                owned
                    .try_reserve_exact(prefix.len())
                    .map_err(|_| ProbeError::Input("input-allocation"))?;
                owned.push_str(prefix);
                groups.push((owned, Vec::new()));
                groups.len() - 1
            };
            let files = &mut groups[index].1;
            files
                .try_reserve_exact(1)
                .map_err(|_| ProbeError::Input("input-allocation"))?;
            files.push(
                ReadOnlyFile::new(relative, file)
                    .map_err(|_| ProbeError::Input("data-file-capability"))?,
            );
        }
        let mut roots = Vec::new();
        roots
            .try_reserve_exact(groups.len() + self.directories.len())
            .map_err(|_| ProbeError::Input("input-allocation"))?;
        for (mount, files) in groups {
            roots.push(
                ReadOnlyRoot::new(&mount, files)
                    .map_err(|_| ProbeError::Input("data-file-capability"))?,
            );
        }
        for (mount, directory) in self.directories {
            roots.push(
                ReadOnlyRoot::directory(&mount, directory)
                    .map_err(|_| ProbeError::Input("data-directory-capability"))?,
            );
        }
        Ok(roots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Input {
        path: PathBuf,
        bytes: Vec<u8>,
    }
    impl Input {
        fn new(bytes: Vec<u8>) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pentax data = \u{03b4} {} {}.bin",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(&bytes).unwrap();
            Self { path, bytes }
        }
        fn binding(&self, guest: &str) -> OsString {
            let mut binding = OsString::from(guest);
            binding.push("=");
            binding.push(&self.path);
            binding
        }
    }
    impl Drop for Input {
        fn drop(&mut self) {
            assert_eq!(std::fs::read(&self.path).unwrap(), self.bytes);
            std::fs::remove_file(&self.path).unwrap();
        }
    }

    #[test]
    fn directory_admission_bounds_work_and_retains_only_an_explicit_root() {
        let path =
            std::env::temp_dir().join(format!("pentax-directory-intake-{}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let mut binding = OsString::from("/app0=");
        binding.push(&path);
        let limits = NativeImageLimits::default();
        let mut data = DataFiles::default();
        let mut small = limits.clone();
        small.startup.kernel.max_handles = 0;
        assert_eq!(
            data.bind_directory(&binding, &small).err().unwrap().code(),
            "data-directory-limit"
        );
        small = limits.clone();
        small.max_layout_work_steps = 0;
        assert_eq!(
            data.bind_directory(&binding, &small).err().unwrap().code(),
            "data-directory-work"
        );
        assert!(data.directories.is_empty() && data.files.is_empty());
        data.bind_directory(&binding, &limits)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(data.directories.len(), 1);
        assert!(data.files.is_empty());
        assert_eq!(
            data.bind_directory(&binding, &limits).err().unwrap().code(),
            "overlapping-data-mount"
        );
        let roots = data
            .into_roots()
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(roots.len(), 1);
        assert!(std::fs::rename(&path, path.with_extension("renamed")).is_err());
        drop(roots);
        std::fs::rename(&path, path.with_extension("renamed")).unwrap();
        std::fs::remove_dir(path.with_extension("renamed")).unwrap();
    }

    #[test]
    fn data_handles_accept_unicode_spaces_and_equals_without_consuming_image_byte_budget() {
        let image = Input::new(b"original image bytes".to_vec());
        let data = Input::new(vec![0xa5; 8192]);
        let limits = NativeImageLimits {
            max_input_bytes: image.bytes.len() as u64,
            ..Default::default()
        };
        let args = vec![
            "--image".into(),
            image.path.clone().into(),
            "--data-file".into(),
            data.binding("/app0/assets/original.bin"),
        ];
        let intake =
            super::super::intake(&args, &limits).unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(intake.images.as_slice(), std::slice::from_ref(&image.bytes));
        assert_eq!(intake.data_roots.len(), 1);
        assert!(OpenOptions::new().write(true).open(&data.path).is_err());
        assert!(OpenOptions::new().append(true).open(&data.path).is_err());
        drop(intake);
        assert!(OpenOptions::new().write(true).open(&data.path).is_ok());
    }

    #[test]
    fn data_binding_refusals_precede_open_and_failed_duplicates_keep_the_inventory() {
        let mut data = DataFiles::default();
        let limits = NativeImageLimits::default();
        for binding in [
            "/app0/file",
            "/app0/file=",
            "/app0/../file=missing",
            "/app0//file=missing",
            "/app0\\file=missing",
            "/app0=missing",
        ] {
            assert_eq!(
                data.bind(&binding.into(), &limits).err().unwrap().code(),
                "data-file-binding"
            );
            assert!(data.files.is_empty());
        }
        let mut small = limits.clone();
        small.max_layout_work_steps = 0;
        assert_eq!(
            data.bind(&"/app0/file=missing".into(), &small)
                .err()
                .unwrap()
                .code(),
            "data-file-work"
        );
        small.startup.kernel.max_handles = 0;
        assert_eq!(
            data.bind(&"/app0/file=missing".into(), &small)
                .err()
                .unwrap()
                .code(),
            "data-file-limit"
        );
        let file = Input::new(vec![37, 42, 99]);
        let binding = file.binding("/app0/original.bin");
        data.bind(&binding, &limits)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(
            data.bind(&binding, &limits).err().unwrap().code(),
            "duplicate-data-file"
        );
        assert_eq!(data.files.len(), 1);
        data.bind(&file.binding("/app0/assets/second.bin"), &limits)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        data.bind(&file.binding("/cache/third.bin"), &limits)
            .unwrap_or_else(|error| panic!("{}", error.code()));
        assert_eq!(
            data.into_roots()
                .unwrap_or_else(|error| panic!("{}", error.code()))
                .len(),
            2
        );
    }
}
