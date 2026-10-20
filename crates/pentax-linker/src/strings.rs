// SPDX-License-Identifier: GPL-2.0-or-later

//! Bounded lookup of NUL-terminated names inside one declared string table.
//!
//! Names are arbitrary bytes; UTF-8 is never required. A lookup only scans the
//! declared table, never the bytes that follow it in the image, and scans at
//! most one byte more than the remaining name budget. Each successful scan is
//! then charged (without its terminator) to that budget, and the first failed
//! scan ends inspection. Total scanning is therefore at most
//! `2 * max_name_bytes + symbol_count + 1` bytes, even when many symbols share
//! one long string or point into successive positions of its tail, so no
//! offset cache is needed to avoid quadratic work.

use crate::LinkError;
use crate::limits::NameBudget;

#[derive(Debug, Clone, Copy)]
pub(crate) struct StringTable<'a> {
    bytes: &'a [u8],
}

impl<'a> StringTable<'a> {
    pub(crate) const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Name bytes at `name_offset`, without the terminator. The budget is not
    /// charged here; the caller charges exactly what it retains.
    pub(crate) fn name(
        &self,
        symbol: usize,
        name_offset: u32,
        budget: &NameBudget,
    ) -> Result<&'a [u8], LinkError> {
        let tail = usize::try_from(name_offset)
            .ok()
            .and_then(|start| self.bytes.get(start..))
            .filter(|tail| !tail.is_empty())
            .ok_or(LinkError::NameOffsetOutOfRange {
                symbol,
                name_offset,
                table_size: self.bytes.len() as u64,
            })?;
        let window = usize::try_from(budget.remaining())
            .unwrap_or(usize::MAX)
            .saturating_add(1)
            .min(tail.len());
        let (scanned, _) = tail.split_at(window);
        match scanned.iter().position(|&byte| byte == 0) {
            Some(length) => Ok(&scanned[..length]),
            // The whole remaining table was scanned without a terminator.
            None if window == tail.len() => Err(LinkError::UnterminatedName {
                symbol,
                name_offset,
            }),
            None => Err(budget.exceeded(symbol, name_offset)),
        }
    }
}
