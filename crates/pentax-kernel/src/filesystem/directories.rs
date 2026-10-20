// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned bounded directory snapshots and explicit
// little-endian packing, without unchecked pointer casts; see NOTICE.
// Adapted from pinned fileSystem.cpp PackDirents and common/hash.h.

use super::{Budget, Failure, File, directory_error};
use cap_std::fs::Dir;

pub(super) const BLOCK: usize = 512;

pub(super) struct Snapshot {
    // Retain the actual selected directory identity throughout the descriptor.
    _owner: File,
    pub(super) bytes: Vec<u8>,
}

/// Pinned hash.h processes complete groups forwards but its final 1..3
/// bytes backwards. Preserve that source behavior, not standard Jenkins order.
fn name_hash(bytes: &[u8]) -> u32 {
    let end = bytes.len() / 4 * 4;
    let mut value = 0_u32;
    for byte in bytes[..end].iter().chain(bytes[end..].iter().rev()) {
        value = value.wrapping_add(u32::from(*byte));
        value = value.wrapping_add(value << 10);
        value ^= value >> 6;
    }
    value = value.wrapping_add(value << 3);
    value ^= value >> 11;
    value.wrapping_add(value << 15)
}

struct Packer {
    bytes: Vec<u8>,
    offset: usize,
    last_length: Option<usize>,
}
impl Packer {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            offset: 0,
            last_length: None,
        }
    }
    fn pad_last(&mut self) {
        if let Some(index) = self.last_length {
            let old = u16::from_le_bytes(self.bytes[index..index + 2].try_into().unwrap());
            let length = old + (self.bytes.len() - self.offset) as u16;
            self.bytes[index..index + 2].copy_from_slice(&length.to_le_bytes());
        }
    }
    fn push(&mut self, name: &str, directory: bool, budget: &mut Budget) -> Result<(), Failure> {
        if name.is_empty() || name.len() > 255 || name.contains(['/', '\0']) {
            return Err(Failure::Unsupported("directory entry name profile"));
        }
        budget.work(name.len() as u64 * 4 + 64)?;
        let length = (8 + name.len() + 1 + 3) & !3;
        if self.offset + length > self.bytes.len() {
            let size = self
                .bytes
                .len()
                .checked_add(BLOCK)
                .ok_or(Failure::Limit("directory snapshot"))?;
            budget.scratch(size)?;
            budget.work((2 * BLOCK) as u64)?;
            if size > self.bytes.capacity() {
                let capacity = self
                    .bytes
                    .capacity()
                    .saturating_mul(2)
                    .max(size)
                    .min(budget.limits.max_scratch_bytes);
                budget.scratch(capacity)?;
                self.bytes
                    .try_reserve_exact(capacity - self.bytes.len())
                    .map_err(|_| Failure::Allocation)?;
            }
            self.pad_last();
            self.offset = self.bytes.len();
            self.bytes.resize(size, 0);
            budget.record_scratch(self.bytes.capacity());
        }
        let offset = self.offset;
        self.bytes[offset..offset + 4].copy_from_slice(&name_hash(name.as_bytes()).to_le_bytes());
        self.bytes[offset + 4..offset + 6].copy_from_slice(&(length as u16).to_le_bytes());
        self.bytes[offset + 6] = if directory { 4 } else { 8 };
        self.bytes[offset + 7] = name.len() as u8;
        self.bytes[offset + 8..offset + 8 + name.len()].copy_from_slice(name.as_bytes());
        // The allocated block's zero tail supplies the name terminator/padding.
        self.last_length = Some(offset + 4);
        self.offset += length;
        Ok(())
    }
    fn finish(mut self) -> Vec<u8> {
        self.pad_last();
        self.bytes
    }
}

pub(super) fn capture(file: File, budget: &mut Budget) -> Result<Snapshot, Failure> {
    let directory = Dir::reopen_dir(&file).map_err(directory_error)?;
    let mut packer = Packer::new();
    // std/cap-std iterators omit these. Pinned host enumeration includes them;
    // encode guest names without ever looking up the host's parent directory.
    packer.push(".", true, budget)?;
    packer.push("..", true, budget)?;
    budget.work(128)?;
    let entries = directory.entries().map_err(directory_error)?;
    for entry in entries {
        budget.work(64)?;
        let entry = entry.map_err(directory_error)?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or(Failure::Unsupported("non-UTF8 directory entry"))?;
        if name == "." || name == ".." {
            continue;
        }
        // This is the entry's own type, without following links or opening a
        // destination. Windows source classifies its DIRECTORY attribute.
        #[cfg(windows)]
        let is_directory = {
            use cap_std::fs::MetadataExt;
            entry.metadata().map_err(directory_error)?.file_attributes() & 0x10 != 0
        };
        #[cfg(not(windows))]
        let is_directory = entry.file_type().map_err(directory_error)?.is_dir();
        packer.push(name, is_directory, budget)?;
    }
    Ok(Snapshot {
        _owner: file,
        bytes: packer.finish(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    #[test]
    fn pinned_hash_tail_order_and_complete_directory_record_bytes() {
        for (name, expected) in [
            ("", 0),
            (".", 0x48fc91ec),
            ("..", 0x669698f4),
            ("a", 0xca2e9442),
            ("ab", 0x7d6b8c42),
            ("abc", 0x85afae58),
            ("abcd", 0xcd8b6206),
            ("abcde", 0xb98559fc),
            ("abcdef", 0xb7833fcc),
            ("abcdefg", 0xfbbe06f5),
        ] {
            assert_eq!(name_hash(name.as_bytes()), expected, "{name}");
        }
        let mut budget = Budget::new(Limits::default()).unwrap();
        let mut packer = Packer::new();
        for (name, dir) in [(".", true), ("..", true), ("abc", false)] {
            packer.push(name, dir, &mut budget).unwrap();
        }
        let bytes = packer.finish();
        let mut expected = vec![0; 512];
        expected[..8].copy_from_slice(&0x0104_000c_48fc_91ec_u64.to_le_bytes());
        expected[8] = b'.';
        expected[12..20].copy_from_slice(&0x0204_000c_6696_98f4_u64.to_le_bytes());
        expected[20..22].copy_from_slice(b"..");
        expected[24..32].copy_from_slice(&0x0308_01e8_85af_ae58_u64.to_le_bytes());
        expected[32..35].copy_from_slice(b"abc");
        assert_eq!(bytes, expected);
    }

    #[test]
    fn long_names_cross_blocks_only_at_record_boundaries_and_respect_storage_limits() {
        let mut budget = Budget::new(Limits {
            max_scratch_bytes: 2048,
            max_name_bytes: 256,
            max_io_bytes: 2048,
            ..Default::default()
        })
        .unwrap();
        let mut packer = Packer::new();
        for value in ['a', 'b', 'c'] {
            packer
                .push(&value.to_string().repeat(255), false, &mut budget)
                .unwrap();
        }
        let bytes = packer.finish();
        assert_eq!(bytes.len(), 1536);
        assert_eq!(budget.usage().scratch_peak, 2048);
        for (index, value) in b"abc".iter().copied().enumerate() {
            let base = index * 512;
            assert_eq!(
                u16::from_le_bytes(bytes[base + 4..base + 6].try_into().unwrap()),
                512
            );
            assert_eq!(&bytes[base + 6..base + 8], &[8, 255]);
            assert!(
                bytes[base + 8..base + 263]
                    .iter()
                    .all(|byte| *byte == value)
            );
            assert!(bytes[base + 263..base + 512].iter().all(|byte| *byte == 0));
        }
        let mut limited = Budget::new(Limits {
            max_scratch_bytes: 512,
            max_name_bytes: 256,
            max_io_bytes: 512,
            ..Default::default()
        })
        .unwrap();
        let mut packer = Packer::new();
        packer.push(&"a".repeat(255), false, &mut limited).unwrap();
        assert_eq!(
            packer.push(&"b".repeat(255), false, &mut limited),
            Err(Failure::Limit("scratch"))
        );
        assert!(matches!(
            packer.push(&"c".repeat(256), false, &mut limited),
            Err(Failure::Unsupported(_))
        ));
    }
}
