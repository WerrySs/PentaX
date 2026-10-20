// SPDX-License-Identifier: GPL-2.0-only
//! Bounded application metadata reads through explicitly selected capabilities.
use super::*;

impl FileState {
    pub(crate) fn application_metadata(
        &self,
        path: &str,
        prefix: Option<usize>,
        maximum: usize,
        budget: &mut Budget,
    ) -> Result<Option<Vec<u8>>, Failure> {
        if !path.starts_with("/app0/") || !safe_components(&path[1..]) {
            return Err(Failure::Unsupported("application metadata virtual path"));
        }
        let Some(file) = self.selected_canonical(path, budget)? else {
            return Ok(None);
        };
        let before = file.metadata().map_err(|_| Errno::Io)?;
        if !before.is_file() {
            return Ok(None);
        }
        let requested = prefix.map_or(before.len(), |limit| before.len().min(limit as u64));
        if requested > maximum as u64 {
            return Err(Failure::Limit("application metadata bytes"));
        }
        let length =
            usize::try_from(requested).map_err(|_| Failure::Limit("application metadata bytes"))?;
        budget.work(requested)?;
        let mut bytes = budget.buffer_held(length, 0)?;
        let mut offset = 0;
        while offset < length {
            let end = length.min(offset.saturating_add(1024 * 1024));
            let count = positional_read(&file, &mut bytes[offset..end], offset as u64)
                .map_err(|_| Errno::Io)?;
            if count == 0 {
                return Err(Failure::Unsupported("changed application metadata"));
            }
            offset += count;
        }
        let after = file.metadata().map_err(|_| Errno::Io)?;
        if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
            return Err(Failure::Unsupported("changed application metadata"));
        }
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kernel, ProcessIdentity};

    #[test]
    fn selected_metadata_prefix_is_bounded_and_whole_file_refusal_retains_original() {
        let mut owner = crate::tests::kernel(Default::default());
        assert_eq!(
            owner.files.application_metadata(
                "/app0/sce_sys/playgo-chunk.dat",
                Some(12),
                12,
                &mut owner.budget,
            ),
            Ok(None)
        );
        let path =
            std::env::temp_dir().join(format!("pentax-playgo-input-{}.bin", std::process::id()));
        let mut original = [0xa5; 32];
        original[..4].copy_from_slice(b"plgo");
        original[10..12].copy_from_slice(&3u16.to_le_bytes());
        std::fs::write(&path, original).unwrap();
        let root = ReadOnlyRoot::new(
            "/app0",
            vec![
                ReadOnlyFile::new("sce_sys/playgo-chunk.dat", File::open(&path).unwrap()).unwrap(),
            ],
        )
        .unwrap();
        owner = Kernel::new(
            Default::default(),
            ProcessIdentity {
                process_id: 1,
                initial_thread_id: 1,
            },
            vec![root],
        )
        .unwrap();
        let bytes = owner
            .files
            .application_metadata(
                "/app0/sce_sys/playgo-chunk.dat",
                Some(12),
                12,
                &mut owner.budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(bytes, original[..12]);
        assert_eq!(
            owner.files.application_metadata(
                "/app0/sce_sys/playgo-chunk.dat",
                None,
                12,
                &mut owner.budget,
            ),
            Err(Failure::Limit("application metadata bytes"))
        );
        assert_eq!(
            owner.files.application_metadata(
                "/app0/playgo-chunkdefs.xml",
                None,
                1024,
                &mut owner.budget,
            ),
            Ok(None)
        );
        assert_eq!(owner.files.retained_handle_count(), 0);
        assert_eq!(owner.usage().attempts, 0);
        drop(owner);
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }
}
