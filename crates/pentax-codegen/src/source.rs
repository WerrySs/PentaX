// SPDX-License-Identifier: GPL-2.0-only

use crate::materialized::MaterializedSource;
use crate::{Accounting, Budget, CachePlacement, CodegenError};
use pentax_linker::{ModuleId, SymbolicLinkPlan};
use pentax_loader::RUNTIME_PAGE_SIZE;

pub(crate) const ADDRESS_LIMIT: u64 = 1 << 47;
pub(crate) struct SourceWindow {
    pub module: ModuleId,
    pub original: Vec<u8>,
    pub relocated: Vec<u8>,
}

pub(crate) fn source_window(
    link: &SymbolicLinkPlan<'_>,
    module_index: usize,
    guest_pc: u64,
    cache: CachePlacement,
    account: &mut Accounting,
) -> Result<SourceWindow, CodegenError> {
    window(
        link,
        module_index,
        guest_pc,
        cache,
        SourceKind::Ordinary,
        account,
    )
}

pub(crate) fn materialized_window(
    declaration: &MaterializedSource<'_, '_>,
    module_index: usize,
    guest_pc: u64,
    cache: CachePlacement,
    staged: &[u8],
    account: &mut Accounting,
) -> Result<SourceWindow, CodegenError> {
    window(
        declaration.source_plan(),
        module_index,
        guest_pc,
        cache,
        SourceKind::Materialized {
            declaration,
            staged,
        },
        account,
    )
}

enum SourceKind<'view, 'plan, 'image, 'staged> {
    Ordinary,
    Materialized {
        declaration: &'view MaterializedSource<'plan, 'image>,
        staged: &'staged [u8],
    },
}

/// Bound aggregate source metadata and charge segment visits before byte copies.
/// Patch traversals are charged by the declaration or bounded window lookup.
/// `Some` is used only by the separately validated, caller-declared path.
pub(crate) fn preflight_metadata(
    link: &SymbolicLinkPlan<'_>,
    declared: Option<usize>,
    account: &mut Accounting,
) -> Result<u64, CodegenError> {
    let limits = account.limits;
    // Aggregate metadata counts and work are bounded before payload copies.
    let modules = link.modules();
    if modules.len() as u64 > limits.max_modules {
        return Err(CodegenError::Budget {
            kind: Budget::Modules,
            limit: limits.max_modules,
        });
    }
    account.charge(modules.len() as u64)?;
    if declared.is_none() && link.deferred_count() != 0 {
        return Err(CodegenError::DeferredRelocations {
            count: link.deferred_count(),
        });
    }
    let patches = (link.patch_count() as u64)
        .checked_add(declared.unwrap_or(0) as u64)
        .ok_or(CodegenError::InvalidAddress)?;
    if patches > limits.max_patches {
        return Err(CodegenError::Budget {
            kind: Budget::Patches,
            limit: limits.max_patches,
        });
    }
    let mut count = 0_u64;
    for module in modules {
        count = count
            .checked_add(module.runtime_plan().segments.len() as u64)
            .ok_or(CodegenError::InvalidAddress)?;
    }
    if count > limits.max_segments {
        return Err(CodegenError::Budget {
            kind: Budget::Segments,
            limit: limits.max_segments,
        });
    }
    account.charge(count.checked_mul(2).ok_or(CodegenError::InvalidAddress)?)?;
    Ok(count)
}

fn window(
    link: &SymbolicLinkPlan<'_>,
    module_index: usize,
    guest_pc: u64,
    cache: CachePlacement,
    kind: SourceKind<'_, '_, '_, '_>,
    account: &mut Accounting,
) -> Result<SourceWindow, CodegenError> {
    let declared = match &kind {
        SourceKind::Ordinary => None,
        SourceKind::Materialized { declaration, .. } => Some(declaration.declared_writes().len()),
    };
    preflight_metadata(link, declared, account)?;
    let limits = account.limits;
    let modules = link.modules();
    let selected = modules
        .get(module_index)
        .ok_or(CodegenError::MissingModule {
            index: module_index,
        })?;
    if guest_pc >= ADDRESS_LIMIT || cache.address == 0 || cache.capacity < 2 {
        return Err(CodegenError::InvalidAddress);
    }
    let cache_end = cache
        .address
        .checked_add(cache.capacity)
        .filter(|end| *end <= ADDRESS_LIMIT)
        .ok_or(CodegenError::InvalidAddress)?;
    let cache_start = cache.address / RUNTIME_PAGE_SIZE * RUNTIME_PAGE_SIZE;
    let cache_end = round_page(cache_end)?;
    for module in modules {
        for segment in &module.runtime_plan().segments {
            if segment.mem_size == 0 {
                continue;
            }
            let start = segment
                .vaddr
                .checked_add(module.load_bias())
                .ok_or(CodegenError::InvalidAddress)?;
            let end = start
                .checked_add(segment.mem_size)
                .filter(|end| *end <= ADDRESS_LIMIT)
                .ok_or(CodegenError::InvalidAddress)?;
            let start = start / RUNTIME_PAGE_SIZE * RUNTIME_PAGE_SIZE;
            let end = round_page(end)?;
            if start < cache_end && cache_start < end {
                return Err(CodegenError::CacheOverlap {
                    module: module.id(),
                });
            }
        }
    }
    let mut found = None;
    for segment in &selected.runtime_plan().segments {
        if !segment.flags.executable() {
            continue;
        }
        let start = segment
            .vaddr
            .checked_add(selected.load_bias())
            .ok_or(CodegenError::InvalidAddress)?;
        let end = start
            .checked_add(segment.file_bytes.len() as u64)
            .ok_or(CodegenError::InvalidAddress)?;
        if guest_pc >= start && guest_pc < end {
            if found.is_some() {
                return Err(CodegenError::AmbiguousSource { guest_pc });
            }
            let offset =
                usize::try_from(guest_pc - start).map_err(|_| CodegenError::InvalidAddress)?;
            found = Some(&segment.file_bytes[offset..]);
        }
    }
    let bytes = found.ok_or(CodegenError::MissingInitializedExecutableSource { guest_pc })?;
    let len = match &kind {
        SourceKind::Ordinary => usize::try_from((bytes.len() as u64).min(limits.max_source_bytes))
            .map_err(|_| CodegenError::InvalidAddress)?,
        SourceKind::Materialized { staged, .. } => {
            if staged.is_empty() || staged.len() > bytes.len() {
                return Err(CodegenError::MaterializedWindowRange {
                    guest_pc,
                    size: staged.len(),
                });
            }
            if staged.len() as u64 > limits.max_source_bytes {
                return Err(CodegenError::Budget {
                    kind: Budget::SourceBytes,
                    limit: limits.max_source_bytes,
                });
            }
            staged.len()
        }
    };
    if len == 0 {
        return Err(CodegenError::Budget {
            kind: Budget::SourceBytes,
            limit: limits.max_source_bytes,
        });
    }
    let end = guest_pc
        .checked_add(len as u64)
        .ok_or(CodegenError::InvalidAddress)?;
    account.charge(
        (len as u64)
            .checked_mul(if declared.is_some() { 3 } else { 2 })
            .ok_or(CodegenError::InvalidAddress)?,
    )?;
    let mut original = account.buffer(len)?;
    original.extend_from_slice(&bytes[..len]);
    let mut relocated = account.buffer(len)?;
    relocated.extend_from_slice(&original);
    for patch in overlapping_patches(link.patches(), guest_pc, end, |p| p.address(), account)? {
        let address = patch.address();
        let patch_end = address
            .checked_add(8)
            .ok_or(CodegenError::SourcePatchRange { address })?;
        if patch.consumer() != selected.id() {
            return Err(CodegenError::SourcePatchRange { address });
        }
        let start = address.max(guest_pc);
        let stop = patch_end.min(end);
        account.charge(stop - start)?;
        let value = patch.value().to_le_bytes();
        relocated[(start - guest_pc) as usize..(stop - guest_pc) as usize]
            .copy_from_slice(&value[(start - address) as usize..(stop - address) as usize]);
    }
    if let SourceKind::Materialized {
        declaration,
        staged,
    } = kind
    {
        for patch in overlapping_patches(
            declaration.declared_writes(),
            guest_pc,
            end,
            |p| p.address,
            account,
        )? {
            let address = patch.address;
            let patch_end = address
                .checked_add(8)
                .ok_or(CodegenError::SourcePatchRange { address })?;
            if patch.consumer != selected.id() {
                return Err(CodegenError::SourcePatchRange { address });
            }
            let start = address.max(guest_pc);
            let stop = patch_end.min(end);
            account.charge(stop - start)?;
            let value = patch.value.to_le_bytes();
            relocated[(start - guest_pc) as usize..(stop - guest_pc) as usize]
                .copy_from_slice(&value[(start - address) as usize..(stop - address) as usize]);
        }
        if let Some(offset) = relocated.iter().zip(staged).position(|(a, b)| a != b) {
            return Err(CodegenError::MaterializedWindowMismatch { guest_pc, offset });
        }
    }
    Ok(SourceWindow {
        module: selected.id(),
        original,
        relocated,
    })
}

/// Both immutable input types guarantee sorted, nonoverlapping eight-byte
/// writes. Linker append constructs a new sorted plan; materialized declaration
/// validates the complete table once. No cached index can cross source epochs.
fn overlapping_patches<'a, T>(
    patches: &'a [T],
    start: u64,
    end: u64,
    address: impl Fn(&T) -> u64,
    account: &mut Accounting,
) -> Result<&'a [T], CodegenError> {
    if let Some(last) = patches.last() {
        account.charge(1)?;
        let address = address(last);
        address
            .checked_add(8)
            .ok_or(CodegenError::SourcePatchRange { address })?;
    }
    // A write beginning up to seven bytes before the window still intersects
    // it. Saturation preserves windows at address zero without underflow.
    let first = patch_lower_bound(patches, start.saturating_sub(7), &address, account)?;
    let count = patch_lower_bound(&patches[first..], end, &address, account)?;
    account.charge(count as u64)?;
    Ok(&patches[first..first + count])
}

fn patch_lower_bound<T>(
    patches: &[T],
    target: u64,
    address: &impl Fn(&T) -> u64,
    account: &mut Accounting,
) -> Result<usize, CodegenError> {
    let mut low = 0;
    let mut high = patches.len();
    while low < high {
        account.charge(1)?;
        let middle = low + (high - low) / 2;
        if address(&patches[middle]) < target {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    Ok(low)
}

fn round_page(value: u64) -> Result<u64, CodegenError> {
    value
        .checked_add(RUNTIME_PAGE_SIZE - 1)
        .map(|v| v / RUNTIME_PAGE_SIZE * RUNTIME_PAGE_SIZE)
        .ok_or(CodegenError::InvalidAddress)
}
