// SPDX-License-Identifier: GPL-2.0-or-later

//! A checked, read-only plan for the symbol-independent AMD64 relocations.
//!
//! The [AMD64 psABI](https://gitlab.com/x86-psABIs/x86-64-ABI/-/blob/master/x86-64-ABI/object-files.tex)
//! defines `R_X86_64_NONE` as no operation and `R_X86_64_RELATIVE` as `B + A`
//! in a `word64` for LP64, with arbitrary byte alignment. The
//! [gABI](https://gabi.xinuos.com/elf/06-reloc.html) supplies the signed explicit
//! RELA addend and executable/shared-object virtual-address interpretation of
//! `r_offset`. The stored value is the low 64 bits of `B + A`; its bits are
//! data and are not required to describe a mapped or canonical address.

use std::{error::Error, fmt};

use pentax_loader::{ExecutableView, LoadError, RuntimeLoadPlan};

use crate::{Inspection, Limits, LinkError, Relocation, TableKind, inspect};

const R_X86_64_NONE: u32 = 0;
const R_X86_64_RELATIVE: u32 = 8;
const PATCH_BYTES: u64 = 8;

/// An original record and its position, retained for diagnostics and later work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelocationSource {
    pub table: TableKind,
    /// Zero-based index within `table`.
    pub index: usize,
    pub record: Relocation,
}

impl fmt::Display for RelocationSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} record {}", self.table, self.index)
    }
}

/// One checked eight-byte little-endian write. No write has been performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelativePatch {
    /// Guest target after checked addition of the plan's load bias.
    pub address: u64,
    /// The low 64 bits of `load_bias + signed r_addend`.
    pub value: u64,
    pub source: RelocationSource,
}

/// A structurally inspected record whose semantics remain unsupported.
///
/// No target width, value, symbol resolution or write is invented for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredRelocation {
    pub source: RelocationSource,
}

/// An image-bound plan for relative writes, with all other records explicit.
///
/// Construction is private. Both the runtime mappings and inspected records
/// come from the same immutable executable view. Borrowed source bytes remain
/// owned by the caller. This plan does not map memory or authorize execution;
/// even an empty deferred list says nothing about imports, TLS or startup ABI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelativeRelocationPlan<'a> {
    runtime: RuntimeLoadPlan<'a>,
    load_bias: u64,
    patches: Vec<RelativePatch>,
    deferred: Vec<DeferredRelocation>,
}

impl<'a> RelativeRelocationPlan<'a> {
    #[must_use]
    pub fn runtime_plan(&self) -> &RuntimeLoadPlan<'a> {
        &self.runtime
    }

    #[must_use]
    pub const fn load_bias(&self) -> u64 {
        self.load_bias
    }

    /// Nonoverlapping patches sorted by biased target address.
    #[must_use]
    pub fn patches(&self) -> &[RelativePatch] {
        &self.patches
    }

    /// Unsupported records in regular-table, then PLT-table order.
    #[must_use]
    pub fn deferred(&self) -> &[DeferredRelocation] {
        &self.deferred
    }

    /// Number of planned writes; none have been applied by this crate.
    #[must_use]
    pub fn patch_count(&self) -> usize {
        self.patches.len()
    }

    #[must_use]
    pub fn deferred_count(&self) -> usize {
        self.deferred.len()
    }
}

/// Why no complete relative plan could be returned.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RelocationError {
    /// Executable parsing or bounded linking-table inspection failed.
    Link(LinkError),
    /// A nonempty logical mapping cannot be represented in 64 bits.
    SegmentRangeOverflow {
        segment: usize,
        address: u64,
        size: u64,
    },
    /// Nonempty logical mappings overlap; ownership of their bytes is ambiguous.
    OverlappingSegments {
        first: usize,
        second: usize,
        address: u64,
    },
    /// A RELATIVE record must not name a symbol.
    RelativeSymbolIndex {
        source: RelocationSource,
        symbol: u32,
    },
    /// The original eight-byte field overflows the original address domain.
    TargetRangeOverflow { source: RelocationSource },
    /// Some bytes lie outside the union of logical, unrounded mappings.
    TargetOutsideSegments { source: RelocationSource },
    /// The biased target or its eight-byte range overflows the guest address domain.
    BiasedTargetOverflow {
        source: RelocationSource,
        load_bias: u64,
    },
    /// Planned writes overlap, including exact duplicates across tables.
    OverlappingPatches {
        first: RelocationSource,
        second: RelocationSource,
        address: u64,
    },
    /// Consecutive same-offset records may compose under the gABI. This stage
    /// cannot interpret such a sequence containing a RELATIVE record in isolation.
    UnsupportedComposition {
        first: RelocationSource,
        second: RelocationSource,
    },
}

impl fmt::Display for RelocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link(error) => write!(f, "relative relocation plan: {error}"),
            Self::SegmentRangeOverflow {
                segment,
                address,
                size,
            } => write!(
                f,
                "logical runtime segment {segment} range {address:#x} + {size:#x} overflows 64 bits"
            ),
            Self::OverlappingSegments {
                first,
                second,
                address,
            } => write!(
                f,
                "logical runtime segments {first} and {second} overlap at {address:#x}"
            ),
            Self::RelativeSymbolIndex { source, symbol } => write!(
                f,
                "{source} is R_X86_64_RELATIVE but references nonzero symbol {symbol}"
            ),
            Self::TargetRangeOverflow { source } => write!(
                f,
                "{source} eight-byte target at {:#x} overflows the original guest address domain",
                source.record.offset
            ),
            Self::TargetOutsideSegments { source } => write!(
                f,
                "{source} eight-byte target at {:#x} is not fully covered by logical runtime mappings",
                source.record.offset
            ),
            Self::BiasedTargetOverflow { source, load_bias } => write!(
                f,
                "{source} eight-byte target at {:#x} with load bias {load_bias:#x} overflows the guest address domain",
                source.record.offset
            ),
            Self::OverlappingPatches {
                first,
                second,
                address,
            } => write!(
                f,
                "relative patches from {first} and {second} overlap at guest address {address:#x}"
            ),
            Self::UnsupportedComposition { first, second } => write!(
                f,
                "{first} and {second} share an offset in a consecutive relocation sequence containing R_X86_64_RELATIVE; composition is unsupported"
            ),
        }
    }
}

impl Error for RelocationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Link(error) => Some(error),
            _ => None,
        }
    }
}

impl From<LinkError> for RelocationError {
    fn from(error: LinkError) -> Self {
        Self::Link(error)
    }
}

impl From<LoadError> for RelocationError {
    fn from(error: LoadError) -> Self {
        Self::Link(LinkError::Load(error))
    }
}

/// Plan symbol-independent AMD64 RELA writes without modifying or executing bytes.
///
/// The original target's complete eight-byte range must lie in the union of
/// logical `PT_LOAD` and standalone `PT_SCE_RELRO` ranges, including BSS.
/// Adjacent ranges may jointly cover a field; rounded page padding does not.
/// Unaligned and initially read-only/executable targets are allowed for a later
/// staging writer. Data-value arithmetic wraps at 64 bits, whereas target-range
/// and load-bias additions are checked. `R_X86_64_NONE` creates no write; all
/// types except `R_X86_64_RELATIVE` remain explicit deferred records.
///
/// Inspection checks the combined regular/PLT record count against
/// `limits.max_relocations` before decoding, so the combined output record
/// count cannot exceed that same bound. No partial plan escapes an error.
///
/// # Errors
///
/// Returns [`RelocationError`] for inspection failures, overlapping logical
/// mappings, invalid relative symbols or targets, overlapping writes, or a
/// consecutive same-offset sequence containing a RELATIVE record.
pub fn plan_relative<'a>(
    view: &ExecutableView<'a>,
    limits: &Limits,
    load_bias: u64,
) -> Result<RelativeRelocationPlan<'a>, RelocationError> {
    let runtime = view.runtime_plan()?;
    let inspection = inspect(view, limits)?;
    let ranges = logical_ranges(&runtime)?;
    let mut patches = Vec::new();
    let mut deferred = Vec::new();
    if let Inspection::Dynamic(metadata) = inspection {
        for (table, records) in [
            (TableKind::Rela, metadata.rela()),
            (TableKind::PltRela, metadata.plt_rela()),
        ] {
            reject_composition(table, records)?;
            for (index, &record) in records.iter().enumerate() {
                let source = RelocationSource {
                    table,
                    index,
                    record,
                };
                match record.kind() {
                    R_X86_64_NONE => {}
                    R_X86_64_RELATIVE => patches.push(relative_patch(source, &ranges, load_bias)?),
                    _ => deferred.push(DeferredRelocation { source }),
                }
            }
        }
    }
    // Sorting in place keeps overlap checking linear after the O(n log n)
    // sort and needs no second allocation proportional to the record count.
    patches.sort_unstable_by_key(|patch| patch.address);
    for pair in patches.windows(2) {
        // relative_patch already proved that every target + PATCH_BYTES fits.
        if pair[1].address < pair[0].address + PATCH_BYTES {
            return Err(RelocationError::OverlappingPatches {
                first: pair[0].source,
                second: pair[1].source,
                address: pair[1].address,
            });
        }
    }
    Ok(RelativeRelocationPlan {
        runtime,
        load_bias,
        patches,
        deferred,
    })
}

fn reject_composition(table: TableKind, records: &[Relocation]) -> Result<(), RelocationError> {
    for (index, pair) in records.windows(2).enumerate() {
        if pair[0].offset == pair[1].offset
            && (pair[0].kind() == R_X86_64_RELATIVE || pair[1].kind() == R_X86_64_RELATIVE)
        {
            return Err(RelocationError::UnsupportedComposition {
                first: RelocationSource {
                    table,
                    index,
                    record: pair[0],
                },
                second: RelocationSource {
                    table,
                    index: index + 1,
                    record: pair[1],
                },
            });
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct LogicalRange {
    pub(crate) segment: usize,
    pub(crate) start: u64,
    pub(crate) end: u64,
}

pub(crate) fn logical_ranges(
    runtime: &RuntimeLoadPlan<'_>,
) -> Result<Vec<LogicalRange>, RelocationError> {
    let mut ranges = Vec::with_capacity(runtime.segments.len());
    for (segment, mapping) in runtime.segments.iter().enumerate() {
        if mapping.mem_size == 0 {
            continue;
        }
        let end = mapping.vaddr.checked_add(mapping.mem_size).ok_or(
            RelocationError::SegmentRangeOverflow {
                segment,
                address: mapping.vaddr,
                size: mapping.mem_size,
            },
        )?;
        ranges.push(LogicalRange {
            segment,
            start: mapping.vaddr,
            end,
        });
    }
    ranges.sort_unstable_by_key(|range| range.start);
    for pair in ranges.windows(2) {
        if pair[1].start < pair[0].end {
            return Err(RelocationError::OverlappingSegments {
                first: pair[0].segment,
                second: pair[1].segment,
                address: pair[1].start,
            });
        }
    }
    Ok(ranges)
}

pub(crate) fn relative_patch(
    source: RelocationSource,
    ranges: &[LogicalRange],
    load_bias: u64,
) -> Result<RelativePatch, RelocationError> {
    if source.record.symbol_index() != 0 {
        return Err(RelocationError::RelativeSymbolIndex {
            source,
            symbol: source.record.symbol_index(),
        });
    }
    let address = target_address(source, ranges, load_bias)?;
    Ok(RelativePatch {
        address,
        value: load_bias.wrapping_add_signed(source.record.addend),
        source,
    })
}

pub(crate) fn target_address(
    source: RelocationSource,
    ranges: &[LogicalRange],
    load_bias: u64,
) -> Result<u64, RelocationError> {
    let start = source.record.offset;
    let end = start
        .checked_add(PATCH_BYTES)
        .ok_or(RelocationError::TargetRangeOverflow { source })?;
    // Start at the first mapping whose end passes the target. Because logical
    // ranges are disjoint, any following ranges can contribute only by exact
    // adjacency. No more than eight nonempty ranges can cover an eight-byte field.
    let first = ranges.partition_point(|range| range.end <= start);
    let mut cursor = start;
    for range in &ranges[first..] {
        if range.start > cursor {
            break;
        }
        cursor = range.end.min(end);
        if cursor == end {
            break;
        }
    }
    if cursor != end {
        return Err(RelocationError::TargetOutsideSegments { source });
    }
    let address = load_bias
        .checked_add(start)
        .filter(|address| address.checked_add(PATCH_BYTES).is_some())
        .ok_or(RelocationError::BiasedTargetOverflow { source, load_bias })?;
    Ok(address)
}

#[cfg(test)]
mod tests;
