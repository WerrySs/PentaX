// SPDX-License-Identifier: GPL-2.0-or-later

//! Copy, allocation and name budgets.

use crate::{LimitKind, LinkError};

const MIB: u64 = 1024 * 1024;

/// Inclusive upper bounds for one inspection.
///
/// Table, symbol and relocation bounds are checked before full table reads or
/// allocations. SysV hash inference first checks the eight-byte header budget,
/// then derives and checks all complete extents. Zero budgets only admit images
/// whose inspected tables are empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Combined declared bytes of the inspected string, symbol, RELA and PLT
    /// RELA tables and any SysV hash used to establish the symbol count, whether
    /// they are borrowed from dynlib data or copied from guest virtual addresses.
    pub max_table_bytes: u64,
    /// Total bytes of owned symbol names, excluding terminators. Every retained
    /// name is charged, even when symbols share one string-table offset.
    pub max_name_bytes: u64,
    pub max_symbols: u64,
    /// Regular and PLT RELA records combined.
    pub max_relocations: u64,
}

impl Limits {
    /// Generous defaults for real images: 64 MiB of tables, 64 MiB of names,
    /// one million symbols and one million combined relocations.
    pub const DEFAULT: Self = Self {
        max_table_bytes: 64 * MIB,
        max_name_bytes: 64 * MIB,
        max_symbols: 1_000_000,
        max_relocations: 1_000_000,
    };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub(crate) fn ensure_within(
    limit: LimitKind,
    requested: u64,
    maximum: u64,
) -> Result<(), LinkError> {
    if requested > maximum {
        Err(LinkError::LimitExceeded {
            limit,
            requested,
            maximum,
        })
    } else {
        Ok(())
    }
}

/// Running total of owned name bytes. Invariant: `used <= maximum`.
#[derive(Debug)]
pub(crate) struct NameBudget {
    used: u64,
    maximum: u64,
}

impl NameBudget {
    pub(crate) const fn new(maximum: u64) -> Self {
        Self { used: 0, maximum }
    }

    pub(crate) const fn used(&self) -> u64 {
        self.used
    }

    pub(crate) const fn remaining(&self) -> u64 {
        self.maximum.saturating_sub(self.used)
    }

    pub(crate) const fn exceeded(&self, symbol: usize, name_offset: u32) -> LinkError {
        LinkError::NameBudgetExceeded {
            symbol,
            name_offset,
            used: self.used,
            maximum: self.maximum,
        }
    }

    pub(crate) fn charge(
        &mut self,
        symbol: usize,
        name_offset: u32,
        length: usize,
    ) -> Result<(), LinkError> {
        let used = u64::try_from(length)
            .ok()
            .and_then(|length| self.used.checked_add(length))
            .filter(|&used| used <= self.maximum)
            .ok_or_else(|| self.exceeded(symbol, name_offset))?;
        self.used = used;
        Ok(())
    }
}
