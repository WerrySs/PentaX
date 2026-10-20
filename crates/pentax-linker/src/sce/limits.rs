// SPDX-License-Identifier: GPL-2.0-or-later

//! Budgets specific to SCE descriptor inspection.

use std::fmt;

const MIB: u64 = 1024 * 1024;

/// Inclusive upper bounds for one [`inspect_sce`](crate::inspect_sce) call.
///
/// These are PentaX hostile-input policy, not ABI requirements. They apply in
/// addition to the table, symbol, name and relocation bounds of
/// [`Limits`](crate::Limits), which keep their existing meaning. Zero and very
/// small values are honoured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceLimits {
    /// Declared 16-byte `PT_DYNAMIC` records, including `DT_NULL` and any
    /// records after it. Checked from program-header metadata before the
    /// dynamic section is parsed or any entry vector is allocated.
    pub max_dynamic_entries: u64,
    /// Retained records combined: `DT_NEEDED`, descriptors of either encoding
    /// (including both raw sides of a role conflict), opaque attributes and
    /// opaque original-filename markers. Checked before any descriptor output
    /// is allocated.
    pub max_descriptor_records: u64,
    /// Total owned metadata name bytes, excluding terminators. Every retained
    /// name is charged, including repeated references to one offset.
    pub max_metadata_name_bytes: u64,
    /// Bytes of any single metadata name, excluding its terminator.
    pub max_single_metadata_name_bytes: u64,
}

impl SceLimits {
    /// 65,536 dynamic records (1 MiB), 4,096 retained records, 1 MiB of names
    /// and 4,096 bytes per name.
    pub const DEFAULT: Self = Self {
        max_dynamic_entries: 65_536,
        max_descriptor_records: 4_096,
        max_metadata_name_bytes: MIB,
        max_single_metadata_name_bytes: 4_096,
    };
}

impl Default for SceLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A count checked by [`SceLimits`] before allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceLimitKind {
    DynamicEntries,
    DescriptorRecords,
}

impl SceLimitKind {
    /// The [`SceLimits`] field controlling this count.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::DynamicEntries => "max_dynamic_entries",
            Self::DescriptorRecords => "max_descriptor_records",
        }
    }
}

impl fmt::Display for SceLimitKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DynamicEntries => "declared dynamic record count",
            Self::DescriptorRecords => "retained SCE metadata record count",
        })
    }
}
