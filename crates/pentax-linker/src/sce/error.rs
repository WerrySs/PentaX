// SPDX-License-Identifier: GPL-2.0-or-later

//! Structured SCE metadata inspection failures.

use std::{error::Error, fmt};

use crate::LinkError;
use crate::sce::descriptors::{DescriptorEncoding, DescriptorRole, DynamicSource};
use crate::sce::limits::SceLimitKind;

/// Why SCE metadata cannot be inspected. Nothing is partially returned.
///
/// Name failures identify the dynamic entry whose name was being read; they
/// are never reported as symbol-name errors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SceError {
    /// The executable or its generic linking tables were rejected.
    Link(LinkError),
    LimitExceeded {
        limit: SceLimitKind,
        requested: u64,
        maximum: u64,
    },
    /// The same ID appears twice in one decoded role, which has the single
    /// `encoding`. This is a PentaX ambiguity policy, applied even when the
    /// two values are identical; neither is selected.
    DuplicateDescriptorId {
        role: DescriptorRole,
        encoding: DescriptorEncoding,
        id: u16,
        first: DynamicSource,
        second: DynamicSource,
    },
    MissingStringTable {
        source: DynamicSource,
        name_offset: u64,
    },
    NameOffsetOutOfRange {
        source: DynamicSource,
        name_offset: u64,
        table_size: u64,
    },
    UnterminatedName {
        source: DynamicSource,
        name_offset: u64,
    },
    /// The name is longer than `SceLimits::max_single_metadata_name_bytes`.
    NameTooLong {
        source: DynamicSource,
        name_offset: u64,
        maximum: u64,
    },
    /// Retaining this name would exceed `SceLimits::max_metadata_name_bytes`.
    NameBudgetExceeded {
        source: DynamicSource,
        name_offset: u64,
        used: u64,
        maximum: u64,
    },
}

impl fmt::Display for SceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link(error) => write!(f, "linking metadata: {error}"),
            Self::LimitExceeded {
                limit,
                requested,
                maximum,
            } => write!(
                f,
                "{limit} {requested} exceeds SceLimits::{} = {maximum}; raise it only for trusted input",
                limit.field()
            ),
            Self::DuplicateDescriptorId {
                role,
                encoding,
                id,
                first,
                second,
            } => write!(
                f,
                "{} ID {id} is declared by both {} and {}",
                role.tag_name(*encoding),
                Entry(*first),
                Entry(*second)
            ),
            Self::MissingStringTable {
                source,
                name_offset,
            } => write!(
                f,
                "{} has name offset {name_offset:#x}, but the dynamic section has no string table",
                Entry(*source)
            ),
            Self::NameOffsetOutOfRange {
                source,
                name_offset,
                table_size,
            } => write!(
                f,
                "{} name offset {name_offset:#x} is outside the {table_size:#x}-byte string table",
                Entry(*source)
            ),
            Self::UnterminatedName {
                source,
                name_offset,
            } => write!(
                f,
                "{} name at offset {name_offset:#x} has no NUL terminator inside the string table",
                Entry(*source)
            ),
            Self::NameTooLong {
                source,
                name_offset,
                maximum,
            } => write!(
                f,
                "{} name at offset {name_offset:#x} exceeds SceLimits::max_single_metadata_name_bytes = {maximum}; raise it only for trusted input",
                Entry(*source)
            ),
            Self::NameBudgetExceeded {
                source,
                name_offset,
                used,
                maximum,
            } => write!(
                f,
                "{} name at offset {name_offset:#x} exceeds SceLimits::max_metadata_name_bytes = {maximum} after {used} retained bytes; raise it only for trusted input",
                Entry(*source)
            ),
        }
    }
}

impl Error for SceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Self::Link(error) = self {
            Some(error)
        } else {
            None
        }
    }
}

impl From<LinkError> for SceError {
    fn from(error: LinkError) -> Self {
        Self::Link(error)
    }
}

/// Human-readable dynamic entry context.
struct Entry(DynamicSource);

impl fmt::Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "dynamic entry {} (tag {:#x}, value {:#x})",
            self.0.index, self.0.tag, self.0.value
        )
    }
}
