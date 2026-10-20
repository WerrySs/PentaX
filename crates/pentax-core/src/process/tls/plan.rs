// SPDX-License-Identifier: GPL-2.0-or-later

use super::{
    StaticTlsLimits, StaticTlsPlan, TlsBudget, TlsError, TlsModuleId, TlsTemplateLayout, Work,
    charge, merge_ranges, page_range,
};
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE};
use pentax_linker::SymbolicLinkPlan;
use pentax_loader::LoadSegment;

/// Plan static blocks below TP, retaining the exact immutable source image set.
///
/// Input order and runtime IDs are explicit PentaX policy. Full initialized
/// logical source coverage and byte agreement are established before staging.
/// Zero-sized templates retain identity but require no initialized source read.
/// Relocations target ordinary image memory. Only the relocated initialized
/// prefix is copied into each TLS instance; its separate tail remains zero.
/// No TCB, DTV, guest storage, instruction execution or host FS state is created.
pub fn plan_static_tls<'plan, 'image>(
    link: &'plan SymbolicLinkPlan<'image>,
    limits: &StaticTlsLimits,
) -> Result<StaticTlsPlan<'plan, 'image>, TlsError> {
    let mut work = Work {
        used: 0,
        limit: limits.max_work_steps,
    };
    plan_layout(link, limits, &mut work)
}

/// The shared layout planner; callers continue charging the same per-call work.
pub(super) fn plan_layout<'plan, 'image>(
    link: &'plan SymbolicLinkPlan<'image>,
    limits: &StaticTlsLimits,
    work: &mut Work,
) -> Result<StaticTlsPlan<'plan, 'image>, TlsError> {
    if limits.reserved_static_bytes > limits.max_static_bytes {
        return Err(TlsError::Budget {
            kind: TlsBudget::StaticBytes,
            limit: limits.max_static_bytes,
        });
    }
    let mut modules = 0;
    charge(
        &mut modules,
        link.modules().len() as u64,
        limits.max_modules,
        TlsBudget::Modules,
    )?;
    work.charge(modules)?;
    let mut segments = 0;
    for module in link.modules() {
        charge(
            &mut segments,
            module.runtime_plan().segments.len() as u64,
            limits.max_segments,
            TlsBudget::Segments,
        )?;
    }
    work.charge(segments)?;
    work.charge(link.patch_count() as u64)?;
    let mut image_pages = Vec::new();
    let mut resident_pages = Vec::new();
    let mut templates = Vec::new();
    let mut template_count = 0;
    let mut initialized_bytes = 0;
    let mut static_extent = 0_u64;
    let mut maximum_alignment = 1;
    for module in link.modules() {
        let runtime = module.runtime_plan();
        for segment in &runtime.segments {
            if segment.mem_size == 0 {
                continue;
            }
            let start = segment
                .vaddr
                .checked_add(module.load_bias())
                .ok_or(TlsError::AddressOverflow)?;
            image_pages.push(page_range(start, segment.mem_size)?);
            if !segment.file_bytes.is_empty() {
                resident_pages.push(page_range(start, segment.file_bytes.len() as u64)?);
            }
        }
        let Some(tls) = &runtime.tls else { continue };
        charge(
            &mut template_count,
            1,
            limits.max_templates,
            TlsBudget::Templates,
        )?;
        let index = module.id().index();
        let invalid = |reason| TlsError::InvalidTemplate {
            module: index,
            reason,
        };
        let header_index = tls.initialized.program_header_index;
        let header = runtime
            .metadata
            .program_headers
            .get(header_index)
            .ok_or_else(|| invalid("program-header identity"))?;
        if header.kind != 7
            || header.mem_size != tls.mem_size
            || header.align != tls.align
            || header.file_size != tls.initialized.file_bytes.len() as u64
        {
            return Err(invalid("retained PT_TLS fields disagree"));
        }
        let initialized_size = tls.initialized.file_bytes.len() as u64;
        if initialized_size > tls.mem_size {
            return Err(invalid("initialized bytes exceed memory size"));
        }
        charge(
            &mut initialized_bytes,
            initialized_size,
            limits.max_initialized_bytes,
            TlsBudget::InitializedBytes,
        )?;
        let alignment = tls.align.max(1);
        if !alignment.is_power_of_two() {
            return Err(invalid("alignment is not a power of two"));
        }
        if alignment > limits.max_alignment {
            return Err(TlsError::Budget {
                kind: TlsBudget::Alignment,
                limit: limits.max_alignment,
            });
        }
        if alignment > PAGE_SIZE {
            return Err(invalid(
                "alignment above one guest page is outside this policy",
            ));
        }
        let source_address = header
            .vaddr
            .checked_add(module.load_bias())
            .filter(|address| *address < ADDRESS_LIMIT)
            .ok_or(TlsError::AddressOverflow)?;
        source_address
            .checked_add(initialized_size)
            .filter(|end| *end <= ADDRESS_LIMIT)
            .ok_or(TlsError::AddressOverflow)?;
        // Charge scans and byte comparisons before collecting or comparing.
        work.charge(runtime.segments.len() as u64)?;
        work.charge(initialized_size)?;
        validate_source(
            index,
            header.vaddr,
            tls.initialized.file_bytes,
            &runtime.segments,
        )?;
        let alignment_bias = header.vaddr % alignment;
        let lower = static_extent
            .checked_add(tls.mem_size)
            .ok_or(TlsError::AddressOverflow)?;
        let residue = (alignment - alignment_bias) % alignment;
        let padding = (residue + alignment - lower % alignment) % alignment;
        static_extent = lower
            .checked_add(padding)
            .ok_or(TlsError::AddressOverflow)?;
        if static_extent > limits.max_static_bytes {
            return Err(TlsError::Budget {
                kind: TlsBudget::StaticBytes,
                limit: limits.max_static_bytes,
            });
        }
        if static_extent > i64::MAX as u64 {
            return Err(invalid("signed TP displacement is out of range"));
        }
        maximum_alignment = maximum_alignment.max(alignment);
        templates.push(TlsTemplateLayout {
            module: module.id(),
            runtime_id: TlsModuleId(template_count),
            program_header_index: header_index,
            source_vaddr: header.vaddr,
            load_bias: module.load_bias(),
            source_address,
            initialized_size,
            mem_size: tls.mem_size,
            alignment,
            alignment_bias,
            tp_offset: static_extent,
        });
    }
    // A PT_TLS zero tail occupies per-thread storage, not ordinary image VAs.
    // Valid PT_LOAD relocations may share that numeric range. Template copying
    // below is bounded by p_filesz; zero-filled instance tails never copy it.
    for patch in link.patches() {
        resident_pages.push(page_range(patch.address(), 8)?);
    }
    let image_mapped_bytes = merge_ranges(&mut image_pages)?;
    let image_resident_bytes = merge_ranges(&mut resident_pages)?;
    Ok(StaticTlsPlan {
        link,
        templates,
        limits: *limits,
        static_extent,
        maximum_alignment,
        image_pages,
        image_mapped_bytes,
        image_resident_pages: resident_pages,
        image_resident_bytes,
    })
}

fn validate_source(
    module: usize,
    start: u64,
    expected: &[u8],
    segments: &[LoadSegment<'_>],
) -> Result<(), TlsError> {
    if expected.is_empty() {
        return Ok(());
    }
    let end = start
        .checked_add(expected.len() as u64)
        .ok_or(TlsError::AddressOverflow)?;
    let mut initialized: Vec<_> = segments
        .iter()
        .filter(|segment| !segment.file_bytes.is_empty())
        .collect();
    initialized.sort_unstable_by_key(|segment| segment.vaddr);
    let mut cursor = start;
    for segment in initialized {
        let source_end = segment
            .vaddr
            .checked_add(segment.file_bytes.len() as u64)
            .ok_or(TlsError::AddressOverflow)?;
        if source_end <= cursor {
            continue;
        }
        if segment.vaddr > cursor {
            break;
        }
        if !segment.flags.readable() {
            return Err(TlsError::SourceNotReadable {
                module,
                address: cursor,
            });
        }
        let count = (source_end.min(end) - cursor) as usize;
        let source_offset = (cursor - segment.vaddr) as usize;
        let expected_offset = (cursor - start) as usize;
        if segment.file_bytes[source_offset..source_offset + count]
            != expected[expected_offset..expected_offset + count]
        {
            return Err(TlsError::TemplateBytesMismatch { module });
        }
        cursor += count as u64;
        if cursor == end {
            return Ok(());
        }
    }
    Err(TlsError::SourceNotInitialized {
        module,
        address: cursor,
    })
}
