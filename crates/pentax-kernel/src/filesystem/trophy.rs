// SPDX-License-Identifier: GPL-2.0-only
//! Internal bounded trophy package reads through existing selected mount handles.
use super::*;

impl FileState {
    pub(crate) fn trophy_read(
        &self,
        path: &str,
        maximum: usize,
        held: usize,
        budget: &mut Budget,
    ) -> Result<Option<Vec<u8>>, Failure> {
        // These are internal source-derived virtual names, not host paths.
        if !path.starts_with("/app0/") || !safe_components(&path[1..]) {
            return Err(Failure::Unsupported("trophy virtual path"));
        }
        if !self.roots.iter().any(|root| {
            path.strip_prefix(&root.mount)
                .is_some_and(|suffix| suffix.starts_with('/'))
        }) {
            return Err(Failure::Unsupported("bound trophy package mount"));
        }
        let Some(file) = self.selected_canonical(path, budget)? else {
            return Ok(None);
        };
        let before = file.metadata().map_err(|_| Errno::Io)?;
        if !before.is_file() || before.len() > maximum as u64 {
            return Ok(None);
        }
        let length =
            usize::try_from(before.len()).map_err(|_| Failure::Limit("trophy input bytes"))?;
        // Work bounds cumulative host input too, including repeated invalid
        // package attempts. No guest file descriptor or position is changed.
        budget.work(length as u64)?;
        let mut bytes = budget.buffer_held(length, held)?;
        let mut offset = 0;
        while offset < length {
            let end = length.min(offset.saturating_add(1024 * 1024));
            let count = positional_read(&file, &mut bytes[offset..end], offset as u64)
                .map_err(|_| Failure::Errno(Errno::Io))?;
            if count == 0 {
                return Err(Failure::Unsupported("changed trophy package input"));
            }
            offset += count;
        }
        let after = file.metadata().map_err(|_| Errno::Io)?;
        if after.len() != before.len() || after.modified().ok() != before.modified().ok() {
            return Err(Failure::Unsupported("changed trophy package input"));
        }
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProcessIdentity;
    use crate::tests::kernel;
    use std::io::Write;

    #[test]
    fn trophy_input_requires_a_bound_mount_and_reads_only_its_selected_capability() {
        let mut absent = kernel(Default::default());
        assert_eq!(
            absent.files.trophy_read(
                "/app0/sce_sys/trophy2/trophy00.ucp",
                1024,
                0,
                &mut absent.budget
            ),
            Err(Failure::Unsupported("bound trophy package mount"))
        );
        let path =
            std::env::temp_dir().join(format!("pentax-trophy-input-{}.bin", std::process::id()));
        let original = b"original trophy package capability";
        std::fs::File::create(&path)
            .unwrap()
            .write_all(original)
            .unwrap();
        let root = ReadOnlyRoot::new(
            "/app0",
            vec![
                ReadOnlyFile::new("sce_sys/trophy2/trophy00.ucp", File::open(&path).unwrap())
                    .unwrap(),
            ],
        )
        .unwrap();
        let mut owner = crate::Kernel::new(
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
            .trophy_read(
                "/app0/sce_sys/trophy2/trophy00.ucp",
                1024,
                0,
                &mut owner.budget,
            )
            .unwrap()
            .unwrap();
        assert_eq!(bytes, original);
        assert_eq!(
            owner.files.trophy_read(
                "/app0/sce_sys/trophy2/trophy01.ucp",
                1024,
                0,
                &mut owner.budget
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
