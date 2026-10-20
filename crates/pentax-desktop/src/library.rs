// SPDX-License-Identifier: GPL-2.0-or-later

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_HEADER_SIZE: u64 = 8 * 1024 * 1024;
const MAX_LIBRARY_SIZE: u64 = 1024 * 1024;
static NEXT_TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryEntry {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_id: Option<String>,
}

impl LibraryEntry {
    pub fn name(&self) -> String {
        let path = if self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("eboot.bin"))
        {
            self.path.parent().unwrap_or(&self.path)
        } else {
            &self.path
        };
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    schema_version: u32,
    entries: Vec<LibraryEntry>,
}

#[derive(Clone, Debug)]
pub struct Inspection {
    pub entry: u64,
    pub format: String,
    pub os_abi: u8,
    pub abi_version: u8,
    pub size: u64,
    pub mapped_bytes: u64,
    pub headers: usize,
    pub segments: Vec<Segment>,
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub address: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub permissions: String,
}

/// Opens the source read-only and bounds the read even if the file grows.
pub fn inspect(path: &Path) -> Result<Inspection, String> {
    let mut file = File::open(path).map_err(|error| format!("Cannot open image: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect file: {error}"))?;
    if !metadata.is_file() {
        return Err("Select a regular ELF or eboot.bin file.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_HEADER_SIZE)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read image headers: {error}"))?;
    let current_size = file
        .metadata()
        .map_err(|error| format!("Cannot verify file size: {error}"))?
        .len();
    if current_size != metadata.len() {
        return Err("The image changed during inspection. Try again once it is stable.".into());
    }
    let image = pentax_loader::inspect_executable(&bytes, metadata.len())
        .map_err(|error| format!("Image header inspection (8 MiB header limit): {error}"))?;
    let loads: Vec<_> = image
        .program_headers
        .iter()
        .filter(|header| header.kind == pentax_loader::PT_LOAD)
        .collect();
    let mapped_bytes = loads
        .iter()
        .try_fold(0_u64, |total, segment| total.checked_add(segment.mem_size))
        .ok_or("Combined segment size overflow")?;
    let segments = loads
        .iter()
        .map(|segment| Segment {
            address: segment.vaddr,
            file_size: segment.file_size,
            memory_size: segment.mem_size,
            permissions: format!(
                "{}{}{}",
                if segment.flags.readable() { 'R' } else { '-' },
                if segment.flags.writable() { 'W' } else { '-' },
                if segment.flags.executable() { 'X' } else { '-' }
            ),
        })
        .collect();
    Ok(Inspection {
        entry: image.entry,
        format: format!("{:?} / {:?}", image.format, image.elf_type),
        os_abi: image.os_abi,
        abi_version: image.abi_version,
        size: metadata.len(),
        mapped_bytes,
        headers: image.program_headers.len(),
        segments,
    })
}

pub fn load(directory: &Path) -> Result<Vec<LibraryEntry>, String> {
    let file = match File::open(directory.join("library.toml")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Cannot read local library: {error}")),
    };
    let mut source = String::new();
    file.take(MAX_LIBRARY_SIZE + 1)
        .read_to_string(&mut source)
        .map_err(|error| format!("Cannot read local library: {error}"))?;
    if source.len() as u64 > MAX_LIBRARY_SIZE {
        return Err("Local library exceeds the 1 MiB limit.".into());
    }
    let document: Document =
        toml::from_str(&source).map_err(|error| format!("Invalid local library: {error}"))?;
    if document.schema_version != 1 {
        return Err(format!(
            "Unsupported library schema {}. Existing file preserved.",
            document.schema_version
        ));
    }
    Ok(document.entries)
}

pub fn save(directory: &Path, entries: &[LibraryEntry]) -> Result<(), String> {
    save_with_sequence(directory, entries, &NEXT_TEMPORARY_ID)
}

fn save_with_sequence(
    directory: &Path,
    entries: &[LibraryEntry],
    sequence: &AtomicU64,
) -> Result<(), String> {
    let directory = if directory.as_os_str().is_empty() {
        Path::new(".")
    } else {
        directory
    };
    let source = toml::to_string_pretty(&Document {
        schema_version: 1,
        entries: entries.to_vec(),
    })
    .map_err(|error| error.to_string())?;
    if source.len() as u64 > MAX_LIBRARY_SIZE {
        return Err("Local library would exceed the 1 MiB limit.".into());
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("Cannot create local library directory: {error}"))?;
    let (temporary, mut file) = create_temporary(directory, sequence)?;
    let result = (|| {
        file.write_all(source.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        // The destination may have changed since startup. Refuse unsupported or
        // malformed documents immediately before replacing it.
        load(directory)?;
        fs::rename(&temporary, directory.join("library.toml"))
            .map_err(|error| format!("Cannot save local library: {error}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_temporary(directory: &Path, sequence: &AtomicU64) -> Result<(PathBuf, File), String> {
    loop {
        let id = sequence.fetch_add(1, Ordering::Relaxed);
        let temporary = directory.join(format!(".library-{}-{id}.tmp", std::process::id()));
        match File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Cannot create temporary library: {error}")),
        }
    }
}

pub fn config_directory() -> Result<PathBuf, String> {
    if cfg!(target_os = "windows") {
        std::env::var_os("LOCALAPPDATA")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join("PentaX"))
            .ok_or_else(|| "LOCALAPPDATA is unavailable; preferences cannot be persisted.".into())
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join("Library/Application Support/PentaX"))
            .ok_or_else(|| "HOME is unavailable; preferences cannot be persisted.".into())
    } else if let Some(path) = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        Ok(path.join("PentaX"))
    } else {
        std::env::var_os("HOME")
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(path).join(".config/PentaX"))
            .ok_or_else(|| "HOME is unavailable; preferences cannot be persisted.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pentax-desktop-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 121];
        bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
        bytes[16..18].copy_from_slice(&0xfe10_u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x1078_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
        bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
        bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
        bytes[68..72].copy_from_slice(&5_u32.to_le_bytes());
        bytes[72..80].copy_from_slice(&120_u64.to_le_bytes());
        bytes[80..88].copy_from_slice(&0x1078_u64.to_le_bytes());
        bytes[96..104].copy_from_slice(&1_u64.to_le_bytes());
        bytes[104..112].copy_from_slice(&1_u64.to_le_bytes());
        bytes[112..120].copy_from_slice(&1_u64.to_le_bytes());
        bytes[120] = 0xcc;
        bytes
    }

    #[test]
    fn inspection_reads_real_metadata_and_preserves_source() {
        let directory = Temporary::new();
        let path = directory.0.join("eboot.bin");
        let bytes = fixture();
        fs::write(&path, &bytes).unwrap();
        let inspection = inspect(&path).unwrap();
        assert_eq!(
            (inspection.entry, inspection.os_abi, inspection.abi_version),
            (0x1078, 9, 2)
        );
        assert_eq!(inspection.segments[0].permissions, "R-X");
        assert_eq!(fs::read(path).unwrap(), bytes);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    #[test]
    fn large_sparse_images_are_inspected_without_reading_the_payload() {
        let directory = Temporary::new();
        let path = directory.0.join("oversized.elf");
        fs::write(&path, fixture()).unwrap();
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(512 * 1024 * 1024)
            .unwrap();
        let inspection = inspect(&path).unwrap();
        assert_eq!(inspection.size, 512 * 1024 * 1024);
        assert_eq!(inspection.segments.len(), 1);
    }

    #[test]
    fn local_library_roundtrips_unicode_paths_and_title_ids() {
        let directory = Temporary::new();
        let entries = vec![LibraryEntry {
            path: PathBuf::from("local/λ/eboot.bin"),
            title_id: Some("PPSA12345".into()),
        }];
        save(&directory.0, &entries).unwrap();
        save(&directory.0, &entries).unwrap();
        let loaded = load(&directory.0).unwrap();
        assert_eq!(loaded[0].path, entries[0].path);
        assert_eq!(loaded[0].title_id, entries[0].title_id);
        assert!(!directory.0.join("local").exists());
    }

    #[test]
    fn rejects_a_library_replaced_with_malformed_or_future_content_after_loading() {
        for changed in ["schema_version = [", "schema_version = 2\nentries = []\n"] {
            let directory = Temporary::new();
            save(&directory.0, &[]).unwrap();
            let loaded = load(&directory.0).unwrap();
            fs::write(directory.0.join("library.toml"), changed).unwrap();
            assert!(save(&directory.0, &loaded).is_err());
            assert_eq!(
                fs::read_to_string(directory.0.join("library.toml")).unwrap(),
                changed
            );
            assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
        }
    }

    #[test]
    fn rejects_an_unsupported_library_that_appears_after_an_initial_missing_load() {
        let directory = Temporary::new();
        let loaded = load(&directory.0).unwrap();
        let changed = "schema_version = 3\nentries = []\n";
        fs::write(directory.0.join("library.toml"), changed).unwrap();
        assert!(
            save(&directory.0, &loaded)
                .unwrap_err()
                .contains("Unsupported library schema 3")
        );
        assert_eq!(
            fs::read_to_string(directory.0.join("library.toml")).unwrap(),
            changed
        );
    }

    #[test]
    fn temporary_collisions_retry_and_failed_saves_preserve_unowned_leftovers() {
        let directory = Temporary::new();
        let leftover = directory
            .0
            .join(format!(".library-{}-0.tmp", std::process::id()));
        fs::write(&leftover, "another operation owns this file").unwrap();
        fs::write(
            directory.0.join("library.toml"),
            "schema_version = 2\nentries = []\n",
        )
        .unwrap();
        let sequence = AtomicU64::new(0);
        assert!(save_with_sequence(&directory.0, &[], &sequence).is_err());
        assert_eq!(sequence.load(Ordering::Relaxed), 2);
        assert_eq!(
            fs::read_to_string(&leftover).unwrap(),
            "another operation owns this file"
        );
        assert!(
            !directory
                .0
                .join(format!(".library-{}-1.tmp", std::process::id()))
                .exists()
        );
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }

    #[test]
    fn a_temporary_creation_error_never_removes_the_failed_destination() {
        let directory = Temporary::new();
        let blocked = directory.0.join("blocked-parent");
        fs::write(&blocked, "existing content").unwrap();
        let sequence = AtomicU64::new(0);
        assert!(create_temporary(&blocked, &sequence).is_err());
        assert_eq!(fs::read_to_string(blocked).unwrap(), "existing content");
    }
}
