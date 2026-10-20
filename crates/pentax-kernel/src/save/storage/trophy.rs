// SPDX-License-Identifier: GPL-2.0-only
//! Numeric per-user trophy records within the already selected per-title save root.
use super::*;

impl SaveStorage {
    /// Replace one source-format trophy record within the selected save root.
    /// The caller prepares its RAM replacement before this transaction and
    /// publishes it only after the final successful rename.
    pub(crate) fn trophy_write(
        &self,
        user: i32,
        label: u32,
        bytes: &[u8],
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        if user < 0 || bytes.len() > 1024 * 1024 {
            return Err(Failure::Unsupported("trophy save record bounds"));
        }
        budget.save_io(bytes.len() as u64, true)?;
        let name = format!("trophies-u{user}-s{label}.json");
        let mut created = None;
        for _ in 0..8 {
            let serial = TEMP_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .map_err(|_| Failure::Limit("trophy save temporary identities"))?;
            let temporary = format!(".trophies-{}-{serial}.tmp", std::process::id());
            match self
                .directory
                .open_with(&temporary, OpenOptions::new().write(true).create_new(true))
            {
                Ok(file) => {
                    created = Some((temporary, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(_) => return Err(Failure::Unsupported("trophy save temporary creation")),
            }
        }
        let (temporary, mut file) =
            created.ok_or(Failure::Unsupported("trophy save temporary collisions"))?;
        let written = file.write_all(bytes).and_then(|_| file.sync_all());
        drop(file);
        if written.is_err() {
            let _ = self.directory.remove_file(&temporary);
            return Err(Failure::Unsupported("trophy save write"));
        }
        #[cfg(test)]
        if self.failure.load(Ordering::Relaxed) {
            let _ = self.directory.remove_file(&temporary);
            return Err(Failure::Unsupported("trophy save replacement"));
        }
        if self
            .directory
            .rename(&temporary, &self.directory, name)
            .is_err()
        {
            let _ = self.directory.remove_file(&temporary);
            return Err(Failure::Unsupported("trophy save replacement"));
        }
        // No fallible operation follows replacement of the durable record.
        Ok(())
    }

    pub(crate) fn trophy_read(
        &self,
        user: i32,
        label: u32,
        held: usize,
        budget: &mut Budget,
    ) -> Result<Option<Vec<u8>>, Failure> {
        if user < 0 {
            return Err(Failure::Unsupported("trophy save user"));
        }
        let name = format!("trophies-u{user}-s{label}.json");
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = match self.directory.open_with(name, &options) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Failure::Unsupported("trophy save read")),
        };
        let before = file
            .metadata()
            .map_err(|_| Failure::Unsupported("trophy save metadata"))?;
        if !before.is_file() || before.len() > 1024 * 1024 {
            return Ok(None);
        }
        let length = before.len() as usize;
        budget.save_io(length as u64, false)?;
        let mut bytes = budget.buffer_held(length, held)?;
        file.read_exact(&mut bytes)
            .map_err(|_| Failure::Unsupported("changed trophy save input"))?;
        let after = file
            .metadata()
            .map_err(|_| Failure::Unsupported("trophy save metadata"))?;
        if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
            return Err(Failure::Unsupported("changed trophy save input"));
        }
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    #[test]
    fn uds_trophy_save_replaces_only_its_numeric_record_and_failed_replacement_keeps_old_bytes() {
        let path = std::env::temp_dir().join(format!("pentax-trophy-write-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let name = path.join("trophies-u7-s3.json");
        let other = path.join("trophies-u8-s3.json");
        let old = br#"{"unlockedTrophies":[2]}"#;
        let next = br#"{"unlockedTrophies":[2,4],"unlockedAt":{"4":62135596800000001}}"#;
        std::fs::write(&name, old).unwrap();
        std::fs::write(&other, b"unchanged").unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let storage = SaveStorage::directory(directory.into_std_file()).unwrap();
        let failure = storage.failure_control();
        let mut budget = Budget::new(Limits::default()).unwrap();
        failure.store(true, Ordering::Relaxed);
        assert!(storage.trophy_write(7, 3, next, &mut budget).is_err());
        assert_eq!(std::fs::read(&name).unwrap(), old);
        failure.store(false, Ordering::Relaxed);
        storage.trophy_write(7, 3, next, &mut budget).unwrap();
        assert_eq!(std::fs::read(&name).unwrap(), next);
        assert_eq!(std::fs::read(&other).unwrap(), b"unchanged");
        assert_eq!(budget.usage().save_bytes_written, next.len() as u64 * 2);
        assert_eq!(std::fs::read_dir(&path).unwrap().count(), 3);
        drop(storage);
        std::fs::remove_file(name).unwrap();
        std::fs::remove_file(other).unwrap();
        std::fs::remove_file(path.join(".pentax-save.lock")).unwrap();
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn trophy_save_reads_numeric_keys_from_the_selected_root_without_mutating_other_saves() {
        let path = std::env::temp_dir().join(format!("pentax-trophy-save-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let name = path.join("trophies-u7-s3.json");
        let content = br#"{"unlockedTrophies":[2],"unlockedAt":{"2":62135596800000001}}"#;
        std::fs::write(&name, content).unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority()).unwrap();
        let storage = SaveStorage::directory(directory.into_std_file()).unwrap();
        let mut budget = Budget::new(Limits::default()).unwrap();
        assert_eq!(
            storage.trophy_read(7, 3, 0, &mut budget).unwrap().unwrap(),
            content
        );
        assert_eq!(storage.trophy_read(8, 3, 0, &mut budget).unwrap(), None);
        assert_eq!(budget.usage().save_bytes_read, content.len() as u64);
        assert_eq!(budget.usage().save_bytes_written, 0);
        assert_eq!(budget.usage().attempts, 0);
        drop(storage);
        assert_eq!(std::fs::read(&name).unwrap(), content);
        std::fs::remove_file(name).unwrap();
        std::fs::remove_file(path.join(".pentax-save.lock")).unwrap();
        std::fs::remove_dir(path).unwrap();
    }
}
