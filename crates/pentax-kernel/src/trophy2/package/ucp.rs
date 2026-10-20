// SPDX-License-Identifier: GPL-2.0-only
// Adapted from pinned common/trophies.cpp; provenance is in the parent module.

use super::*;

pub(super) struct Entry<'a> {
    pub name: &'a [u8],
    pub bytes: &'a [u8],
}
pub(super) struct Files<'a> {
    entries: Vec<Entry<'a>>,
    pub size: usize,
}

fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |n, byte| (n << 8) | u64::from(*byte))
}

impl<'a> Files<'a> {
    pub fn parse(bytes: &'a [u8], ctx: &mut Context<'_>) -> Result<Option<Self>, Failure> {
        ctx.work(64)?;
        if bytes.len() < 64
            || bytes.len() > 1 << 30
            || be(&bytes[..4]) != 0xb228_c60a
            || be(&bytes[4..8]) != 1
        {
            return Ok(None);
        }
        let Ok(size) = usize::try_from(be(&bytes[8..16])) else {
            return Ok(None);
        };
        let count = be(&bytes[16..20]) as usize;
        let toc = be(&bytes[20..24]) as usize;
        if size < 64
            || size > bytes.len()
            || count > 4096
            || toc > size
            || 32 + count * 64 > size - toc
        {
            return Ok(None);
        }
        let mut entries = Vec::new();
        ctx.reserve(&mut entries, count)?;
        for index in 0..count {
            ctx.work(96)?;
            let entry = &bytes[toc + 32 + index * 64..toc + 32 + (index + 1) * 64];
            let name = &entry[..entry[..32].iter().position(|byte| *byte == 0).unwrap_or(32)];
            let offset = be(&entry[32..40]);
            let length = be(&entry[40..48]);
            if offset > size as u64 || length > size as u64 - offset {
                ctx.release(entries.capacity() * core::mem::size_of::<Entry<'_>>());
                return Ok(None);
            }
            entries.push(Entry {
                name,
                bytes: &bytes[offset as usize..(offset + length) as usize],
            });
        }
        Ok(Some(Self { entries, size }))
    }
    pub fn get(&self, name: &[u8], ctx: &mut Context<'_>) -> Result<Option<&'a [u8]>, Failure> {
        for entry in &self.entries {
            ctx.work(33)?;
            if entry.name == name {
                return Ok(Some(entry.bytes));
            }
        }
        Ok(None)
    }
    pub fn localized(
        &self,
        locale: &[u8],
        ctx: &mut Context<'_>,
    ) -> Result<Option<&'a [u8]>, Failure> {
        for entry in &self.entries {
            ctx.work(33)?;
            if entry
                .name
                .strip_prefix(b"tropmeta_")
                .and_then(|name| name.strip_suffix(b".json"))
                == Some(locale)
            {
                return Ok(Some(entry.bytes));
            }
        }
        Ok(None)
    }
}
