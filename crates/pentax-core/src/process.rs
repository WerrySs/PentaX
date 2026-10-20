// SPDX-License-Identifier: GPL-2.0-or-later
//! Validates a complete image plan before allocating sparse guest pages.
use crate::memory::{ADDRESS_LIMIT, GuestMemory, MemoryError, PAGE_SIZE, Permissions, page_base};
use pentax_linker::{RelativePatch, RelativeRelocationPlan, SymbolicLinkPlan};
use pentax_loader::{ElfImage, LoadSegment, RuntimeLoadPlan, SegmentFlags};
use std::{collections::BTreeMap, fmt};

pub mod hle;
mod hle_strings;
pub mod startup;
pub mod tls;

#[derive(Debug, Clone)]
pub struct LoadOptions {
    pub load_bias: u64,
    pub resident_limit: u64,
    pub mapped_limit: u64,
}
impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            load_bias: 0,
            resident_limit: 256 * 1024 * 1024,
            mapped_limit: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug)]
pub struct LoadedProcess {
    pub memory: GuestMemory,
    pub entry: u64,
}

/// An image with relative relocations applied, whose remaining linking and
/// process initialization have not been completed. Only read-only diagnostics
/// are exposed; this type cannot be converted into an executable process.
#[derive(Debug)]
pub struct UnlinkedProcess {
    process: LoadedProcess,
    applied_relocations: usize,
    deferred_relocations: usize,
}

impl UnlinkedProcess {
    pub fn memory(&self) -> &GuestMemory {
        &self.process.memory
    }

    pub fn entry(&self) -> u64 {
        self.process.entry
    }

    pub fn applied_relocations(&self) -> usize {
        self.applied_relocations
    }

    pub fn deferred_relocations(&self) -> usize {
        self.deferred_relocations
    }
}

/// Memory budgets for staging a prepared module session. Image addresses and
/// load biases come exclusively from the immutable linker plan.
#[derive(Debug, Clone)]
pub struct SessionLoadLimits {
    pub resident_limit: u64,
    pub mapped_limit: u64,
}

impl Default for SessionLoadLimits {
    fn default() -> Self {
        let defaults = LoadOptions::default();
        Self {
            resident_limit: defaults.resident_limit,
            mapped_limit: defaults.mapped_limit,
        }
    }
}

/// Relocation accounting only; neither variant establishes guest startup, TLS,
/// service availability, instruction coverage or permission to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocationStatus {
    CompleteWithinPolicy,
    Deferred { count: usize },
}

/// One unpublished address space containing every module of a prepared plan.
/// Read-only diagnostics are the only capability exposed, even when every
/// relocation supported by the plan's policy has been applied.
#[derive(Debug)]
pub struct UnstartedSession {
    memory: GuestMemory,
    primary_entry: u64,
    module_count: usize,
    applied_relocations: usize,
    deferred_relocations: usize,
}

impl UnstartedSession {
    pub fn memory(&self) -> &GuestMemory {
        &self.memory
    }

    pub fn primary_entry(&self) -> u64 {
        self.primary_entry
    }

    pub fn module_count(&self) -> usize {
        self.module_count
    }

    pub fn applied_relocations(&self) -> usize {
        self.applied_relocations
    }

    pub fn deferred_relocations(&self) -> usize {
        self.deferred_relocations
    }

    pub fn relocation_status(&self) -> RelocationStatus {
        match self.deferred_relocations {
            0 => RelocationStatus::CompleteWithinPolicy,
            count => RelocationStatus::Deferred { count },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessError {
    NativeDeferredRelocations {
        count: usize,
    },
    NativeUnsupportedTls {
        module: usize,
    },
    Memory(MemoryError),
    NoLoadSegments,
    AddressOverflow,
    OverlappingSegments,
    EntryNotExecutable,
    ImageTooLarge,
    InvalidSegment,
    ConflictingPagePermissions,
    RelocationBiasMismatch {
        planned: u64,
        requested: u64,
    },
    SharedModulePage {
        address: u64,
    },
    /// Valid partial-page RELRO semantics are outside the first staging policy.
    UnsupportedRelroAlignment {
        module: usize,
        address: u64,
        size: u64,
    },
    RelroOutsideModule {
        module: usize,
        address: u64,
    },
}
impl From<MemoryError> for ProcessError {
    fn from(e: MemoryError) -> Self {
        Self::Memory(e)
    }
}
impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeDeferredRelocations { count } => write!(
                f,
                "native memory requires complete relocations; {count} deferred"
            ),
            Self::NativeUnsupportedTls { module } => write!(
                f,
                "native memory module {module} requires unsupported TLS preparation"
            ),
            Self::Memory(e) => write!(f, "{e}"),
            Self::NoLoadSegments => write!(f, "image has no nonempty loadable segments"),
            Self::AddressOverflow => write!(f, "image exceeds the guest address space"),
            Self::OverlappingSegments => write!(f, "loadable byte ranges overlap"),
            Self::EntryNotExecutable => {
                write!(f, "entry point does not belong to an executable segment")
            }
            Self::ImageTooLarge => write!(f, "image exceeds configured memory limits"),
            Self::InvalidSegment => write!(f, "invalid segment metadata"),
            Self::ConflictingPagePermissions => write!(
                f,
                "segments sharing a guest page would implicitly make it writable and executable"
            ),
            Self::RelocationBiasMismatch { planned, requested } => write!(
                f,
                "relocation load bias {planned:#x} differs from requested bias {requested:#x}"
            ),
            Self::SharedModulePage { address } => write!(
                f,
                "separate modules share guest page {address:#x}; shared module pages are unsupported"
            ),
            Self::UnsupportedRelroAlignment {
                module,
                address,
                size,
            } => write!(
                f,
                "module {module} RELRO range {address:#x} + {size:#x} does not cover complete 16 KiB pages; partial-page RELRO is unsupported"
            ),
            Self::RelroOutsideModule { module, address } => write!(
                f,
                "module {module} RELRO range includes guest page {address:#x} outside its mappings"
            ),
        }
    }
}
impl std::error::Error for ProcessError {}

/// Maps file bytes and zero-filled tails, then applies final page permissions.
/// This does not relocate dynamic images, resolve imports or start guest code.
pub fn load(image: &ElfImage<'_>, options: &LoadOptions) -> Result<LoadedProcess, ProcessError> {
    map_segments(image.entry, &image.segments, &[], options)
}

/// Maps the loader's explicitly required runtime segments without resolving
/// imports, applying relocations, allocating TLS or executing guest instructions.
pub fn load_runtime(
    plan: &RuntimeLoadPlan<'_>,
    options: &LoadOptions,
) -> Result<LoadedProcess, ProcessError> {
    map_segments(plan.metadata.entry, &plan.segments, &[], options)
}

/// Copies an immutable executable plan into owned guest staging memory, applies
/// its checked relative patches and then installs the original page permissions.
/// Any failure drops the staging image. Symbol and other relocations remain
/// deferred, and no stack, TLS, imports or guest execution are initialized.
pub fn load_relative(
    plan: &RelativeRelocationPlan<'_>,
    options: &LoadOptions,
) -> Result<UnlinkedProcess, ProcessError> {
    if options.load_bias != plan.load_bias() {
        return Err(ProcessError::RelocationBiasMismatch {
            planned: plan.load_bias(),
            requested: options.load_bias,
        });
    }
    let runtime = plan.runtime_plan();
    let process = map_segments(
        runtime.metadata.entry,
        &runtime.segments,
        plan.patches(),
        options,
    )?;
    Ok(UnlinkedProcess {
        process,
        applied_relocations: plan.patches().len(),
        deferred_relocations: plan.deferred().len(),
    })
}

/// Stage the complete, image-bound module set into one owned address space.
/// Original payloads are copied before any relative or symbolic patch, and
/// final permissions are installed only after all writes succeed. Any error
/// drops the complete unpublished image; no mutable or executable process is
/// returned. Provider entries need not be executable; only module zero is the
/// primary image and its entry must belong to its own executable logical bytes.
/// The staging address policy additionally requires 16 KiB-aligned retained
/// biases and all module mappings within the core's lower 47-bit address space.
/// A pure plan can pass its wider u64 checks and still fail these restrictions.
///
/// The initial RELRO policy revokes WRITE on complete 16 KiB pages, preserving
/// original READ/EXECUTE. Nonempty partial-page ranges fail explicitly without
/// rounding or protecting adjacent bytes; empty ranges are no-ops.
pub fn load_symbolic(
    plan: &SymbolicLinkPlan<'_>,
    limits: &SessionLoadLimits,
) -> Result<UnstartedSession, ProcessError> {
    stage_session(SessionPlan::Ordinary(plan), limits)
}

/// The only staging inputs: an ordinary plan, or a retained relocated TLS plan
/// whose base, extra writes and zero remaining count all derive from itself.
pub(crate) enum SessionPlan<'a, 'binding, 'plan, 'image> {
    Ordinary(&'a SymbolicLinkPlan<'image>),
    Relocated(&'a tls::RelocatedStaticTlsPlan<'binding, 'plan, 'image>),
}

impl<'image> SessionPlan<'_, '_, '_, 'image> {
    pub(crate) fn base(&self) -> &SymbolicLinkPlan<'image> {
        match self {
            Self::Ordinary(plan) => plan,
            Self::Relocated(plan) => plan.bindings().base_plan(),
        }
    }

    pub(crate) fn tls_patches(&self) -> &[tls::TlsRelocationPatch] {
        match self {
            Self::Ordinary(_) => &[],
            Self::Relocated(plan) => plan.tls_patches(),
        }
    }

    pub(crate) fn deferred_count(&self) -> usize {
        match self {
            Self::Ordinary(plan) => plan.deferred_count(),
            Self::Relocated(plan) => plan.bindings().remaining_deferred().len(),
        }
    }
}

fn stage_session(
    source: SessionPlan<'_, '_, '_, '_>,
    limits: &SessionLoadLimits,
) -> Result<UnstartedSession, ProcessError> {
    let (memory, metadata) = stage_session_with(source, limits, |_| {
        Ok(GuestMemory::new(limits.resident_limit, limits.mapped_limit))
    })?;
    Ok(UnstartedSession {
        memory,
        primary_entry: metadata.primary_entry,
        module_count: metadata.module_count,
        applied_relocations: metadata.applied_relocations,
        deferred_relocations: metadata.deferred_relocations,
    })
}

#[derive(Debug)]
pub(crate) struct SessionMetadata {
    pub primary_entry: u64,
    pub module_count: usize,
    pub applied_relocations: usize,
    pub deferred_relocations: usize,
}

pub(crate) struct SessionPagePlan {
    pub(crate) pages: BTreeMap<u64, (usize, Permissions)>,
    pub(crate) primary_entry: u64,
}

/// Complete source/RELRO/relocation preflight without touching live backing.
pub(crate) fn session_page_plan(
    source: &SessionPlan<'_, '_, '_, '_>,
    limits: &SessionLoadLimits,
) -> Result<SessionPagePlan, ProcessError> {
    let plan = source.base();
    let tls_patches = source.tls_patches();
    let primary = plan.modules().first().ok_or(ProcessError::NoLoadSegments)?;
    let primary_entry = primary
        .runtime_plan()
        .metadata
        .entry
        .checked_add(primary.load_bias())
        .filter(|entry| *entry < ADDRESS_LIMIT)
        .ok_or(ProcessError::AddressOverflow)?;
    let mut ranges = Vec::new();
    let mut page_ranges = Vec::new();
    let mut primary_entry_executable = false;
    for (module_index, module) in plan.modules().iter().enumerate() {
        let bias = module.load_bias();
        if module.id().index() != module_index || !bias.is_multiple_of(PAGE_SIZE) {
            return Err(ProcessError::InvalidSegment);
        }
        for segment in &module.runtime_plan().segments {
            if segment.file_bytes.len() as u64 > segment.mem_size {
                return Err(ProcessError::InvalidSegment);
            }
            if segment.mem_size == 0 {
                continue;
            }
            let start = segment
                .vaddr
                .checked_add(bias)
                .ok_or(ProcessError::AddressOverflow)?;
            let end = start
                .checked_add(segment.mem_size)
                .filter(|end| *end <= ADDRESS_LIMIT)
                .ok_or(ProcessError::AddressOverflow)?;
            let page_start = page_base(start);
            let page_end = end
                .checked_add(PAGE_SIZE - 1)
                .map(page_base)
                .ok_or(ProcessError::AddressOverflow)?;
            if page_end - page_start > limits.mapped_limit {
                return Err(ProcessError::ImageTooLarge);
            }
            primary_entry_executable |= module_index == 0
                && start <= primary_entry
                && primary_entry < end
                && segment.flags.executable();
            ranges.push((start, end, module_index));
            page_ranges.push((
                page_start,
                page_end,
                module_index,
                permissions(segment.flags),
            ));
        }
    }
    if ranges.is_empty() {
        return Err(ProcessError::NoLoadSegments);
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(ProcessError::OverlappingSegments);
    }
    if !primary_entry_executable {
        return Err(ProcessError::EntryNotExecutable);
    }
    let mut pages = BTreeMap::<u64, (usize, Permissions)>::new();
    for (start, end, module, permission) in page_ranges {
        for base in (start..end).step_by(PAGE_SIZE as usize) {
            if let Some(&(previous_module, previous)) = pages.get(&base) {
                if previous_module != module {
                    return Err(ProcessError::SharedModulePage { address: base });
                }
                let combined = previous.union(permission);
                if previous != permission
                    && combined.contains(Permissions::WRITE)
                    && combined.contains(Permissions::EXECUTE)
                {
                    return Err(ProcessError::ConflictingPagePermissions);
                }
                pages.insert(base, (module, combined));
            } else {
                pages.insert(base, (module, permission));
            }
            if pages.len() as u64 > limits.mapped_limit / PAGE_SIZE {
                return Err(ProcessError::ImageTooLarge);
            }
        }
    }
    // Compute the final page plan before allocating guest memory. No partial
    // RELRO interpretation or ownership/protection failure can start staging.
    for (module_index, module) in plan.modules().iter().enumerate() {
        for range in &module.runtime_plan().relro {
            if range.size == 0 {
                continue;
            }
            let start = range
                .address
                .checked_add(module.load_bias())
                .ok_or(ProcessError::AddressOverflow)?;
            let end = start
                .checked_add(range.size)
                .filter(|end| *end <= ADDRESS_LIMIT)
                .ok_or(ProcessError::AddressOverflow)?;
            if !start.is_multiple_of(PAGE_SIZE) || !range.size.is_multiple_of(PAGE_SIZE) {
                return Err(ProcessError::UnsupportedRelroAlignment {
                    module: module_index,
                    address: start,
                    size: range.size,
                });
            }
            for base in (start..end).step_by(PAGE_SIZE as usize) {
                let (_, permissions) = pages
                    .get_mut(&base)
                    .filter(|(owner, _)| *owner == module_index)
                    .ok_or(ProcessError::RelroOutsideModule {
                        module: module_index,
                        address: base,
                    })?;
                *permissions = Permissions::new(
                    permissions.contains(Permissions::READ),
                    false,
                    permissions.contains(Permissions::EXECUTE),
                );
            }
        }
    }
    for patch in plan.patches() {
        validate_session_patch(&ranges, patch.consumer().index(), patch.address())?;
    }
    for patch in tls_patches {
        validate_session_patch(&ranges, patch.consumer().index(), patch.address())?;
    }
    Ok(SessionPagePlan {
        pages,
        primary_entry,
    })
}

pub(crate) fn stage_session_with<M: crate::memory::service::StagingMemory>(
    source: SessionPlan<'_, '_, '_, '_>,
    limits: &SessionLoadLimits,
    create: impl FnOnce(u64) -> Result<M, ProcessError>,
) -> Result<(M, SessionMetadata), ProcessError> {
    let layout = session_page_plan(&source, limits)?;
    let plan = source.base();
    let tls_patches = source.tls_patches();
    let pages = layout.pages;
    let primary_entry = layout.primary_entry;
    let mut memory = create(pages.len() as u64 * PAGE_SIZE)?;
    // Coalesce contiguous staging pages. Final per-page source/RELRO policy
    // remains unchanged; native reservations need not be rebuilt page by page.
    let mut mapped_pages = pages.keys().copied().peekable();
    while let Some(start) = mapped_pages.next() {
        let mut end = start + PAGE_SIZE;
        while mapped_pages.peek().is_some_and(|next| *next == end) {
            mapped_pages.next();
            end += PAGE_SIZE;
        }
        memory.map(start, end - start, Permissions::READ_WRITE)?;
    }
    for module in plan.modules() {
        for segment in &module.runtime_plan().segments {
            if !segment.file_bytes.is_empty() {
                let start = segment
                    .vaddr
                    .checked_add(module.load_bias())
                    .ok_or(ProcessError::AddressOverflow)?;
                memory.write(start, segment.file_bytes)?;
            }
        }
    }
    // The linker proved the complete ordinary-plus-TLS write set nonoverlapping,
    // so this order cannot decide any value. Protection follows every write.
    for patch in plan.patches() {
        memory.write(patch.address(), &patch.value().to_le_bytes())?;
    }
    for patch in tls_patches {
        memory.write(patch.address(), &patch.value().to_le_bytes())?;
    }
    for (&base, &(_, permissions)) in &pages {
        memory.protect(base, PAGE_SIZE, permissions)?;
    }
    Ok((
        memory,
        SessionMetadata {
            primary_entry,
            module_count: plan.modules().len(),
            applied_relocations: plan.patch_count() + tls_patches.len(),
            deferred_relocations: source.deferred_count(),
        },
    ))
}

fn validate_session_patch(
    ranges: &[(u64, u64, usize)],
    module: usize,
    address: u64,
) -> Result<(), ProcessError> {
    let end = address
        .checked_add(8)
        .filter(|end| *end <= ADDRESS_LIMIT)
        .ok_or(ProcessError::AddressOverflow)?;
    let first = ranges.partition_point(|range| range.1 <= address);
    let mut cursor = address;
    for &(start, range_end, owner) in &ranges[first..] {
        if start > cursor || owner != module {
            break;
        }
        cursor = range_end.min(end);
        if cursor == end {
            return Ok(());
        }
    }
    Err(ProcessError::InvalidSegment)
}

fn map_segments(
    original_entry: u64,
    segments: &[LoadSegment<'_>],
    patches: &[RelativePatch],
    options: &LoadOptions,
) -> Result<LoadedProcess, ProcessError> {
    let entry = original_entry
        .checked_add(options.load_bias)
        .ok_or(ProcessError::AddressOverflow)?;
    if !options.load_bias.is_multiple_of(PAGE_SIZE) {
        return Err(ProcessError::InvalidSegment);
    }
    let mut ranges = Vec::new();
    let mut plan = BTreeMap::<u64, Permissions>::new();
    let mut page_ranges = Vec::new();
    let mut executable_entry = false;
    for segment in segments {
        if segment.file_bytes.len() as u64 > segment.mem_size {
            return Err(ProcessError::InvalidSegment);
        }
        if segment.mem_size == 0 {
            continue;
        }
        let start = segment
            .vaddr
            .checked_add(options.load_bias)
            .ok_or(ProcessError::AddressOverflow)?;
        let end = start
            .checked_add(segment.mem_size)
            .ok_or(ProcessError::AddressOverflow)?;
        if end > ADDRESS_LIMIT {
            return Err(ProcessError::AddressOverflow);
        }
        let page_end = end
            .checked_add(PAGE_SIZE - 1)
            .map(page_base)
            .ok_or(ProcessError::AddressOverflow)?;
        let page_start = page_base(start);
        // Each segment must fit before any range is expanded into descriptors.
        if page_end - page_start > options.mapped_limit {
            return Err(ProcessError::ImageTooLarge);
        }
        executable_entry |= start <= entry && entry < end && segment.flags.executable();
        ranges.push((start, end));
        page_ranges.push((page_start, page_end, permissions(segment.flags)));
    }
    if ranges.is_empty() {
        return Err(ProcessError::NoLoadSegments);
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|r| r[0].1 > r[1].0) {
        return Err(ProcessError::OverlappingSegments);
    }
    if !executable_entry {
        return Err(ProcessError::EntryNotExecutable);
    }
    // Adjacent byte ranges can share a boundary page. Charge that page once.
    // Expand only after rejecting byte overlaps, which also bounds duplicate work.
    for (start, end, permission) in page_ranges {
        for base in (start..end).step_by(PAGE_SIZE as usize) {
            if let Some(&previous) = plan.get(&base) {
                let combined = previous.union(permission);
                if previous != permission
                    && combined.contains(Permissions::WRITE)
                    && combined.contains(Permissions::EXECUTE)
                {
                    return Err(ProcessError::ConflictingPagePermissions);
                }
                plan.insert(base, combined);
            } else {
                plan.insert(base, permission);
            }
            if plan.len() as u64 > options.mapped_limit / PAGE_SIZE {
                return Err(ProcessError::ImageTooLarge);
            }
        }
    }
    let mut memory = GuestMemory::new(options.resident_limit, options.mapped_limit);
    for &base in plan.keys() {
        memory.map(base, PAGE_SIZE, Permissions::READ_WRITE)?;
    }
    for segment in segments {
        if !segment.file_bytes.is_empty() {
            memory.write(segment.vaddr + options.load_bias, segment.file_bytes)?;
        }
    }
    // Patches refer to the same immutable image that supplied these segments.
    // Staging remains owned locally and writable until every write succeeds.
    for patch in patches {
        memory.write(patch.address, &patch.value.to_le_bytes())?;
    }
    for (&base, &permissions) in &plan {
        memory.protect(base, PAGE_SIZE, permissions)?;
    }
    Ok(LoadedProcess { memory, entry })
}

fn permissions(flags: SegmentFlags) -> Permissions {
    Permissions::new(flags.readable(), flags.writable(), flags.executable())
}

#[cfg(test)]
mod relative_tests;

#[cfg(test)]
mod symbolic_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn runtime_plan_maps_original_bytes_at_the_biased_entry() {
        let bytes = demo::elf();
        let view = pentax_loader::open_executable(&bytes).unwrap();
        let plan = view.runtime_plan().unwrap();
        let options = LoadOptions {
            load_bias: 0x400000,
            ..LoadOptions::default()
        };
        let process = load_runtime(&plan, &options).unwrap();
        assert_eq!(process.entry, 0x800000);
        let mut code = [0; 5];
        process.memory.fetch(process.entry, &mut code).unwrap();
        assert_eq!(code, [0xb8, 3, 0, 0, 0]);
        let mut empty = [1; 8];
        process
            .memory
            .read(process.entry + 128, &mut empty)
            .unwrap();
        assert_eq!(empty, [0; 8]);
    }

    #[test]
    fn disjoint_segments_share_one_page_budget() {
        let bytes = demo::elf();
        let mut image = pentax_loader::parse(&bytes).unwrap();
        image.segments[0].mem_size = 0x100;
        let mut second = image.segments[0];
        second.vaddr += 0x200;
        second.file_bytes = &[];
        second.flags = SegmentFlags::from_raw(4);
        image.segments.push(second);
        let options = LoadOptions {
            mapped_limit: PAGE_SIZE,
            ..LoadOptions::default()
        };
        let process = load(&image, &options).unwrap();
        assert_eq!(process.memory.mapped_bytes(), PAGE_SIZE);
    }

    #[test]
    fn separate_rx_and_rw_segments_cannot_implicitly_create_rwx_pages() {
        let bytes = demo::elf();
        let mut image = pentax_loader::parse(&bytes).unwrap();
        image.segments[0].mem_size = 0x100;
        let mut second = image.segments[0];
        second.vaddr += 0x200;
        second.file_bytes = &[];
        second.flags = SegmentFlags::from_raw(6);
        image.segments.push(second);
        assert!(matches!(
            load(&image, &LoadOptions::default()),
            Err(ProcessError::ConflictingPagePermissions)
        ));
    }

    #[test]
    fn file_and_bss_are_loaded_and_execute_page_is_not_writable() {
        let bytes = demo::elf();
        let image = pentax_loader::parse(&bytes).unwrap();
        let mut process = load(&image, &LoadOptions::default()).unwrap();
        let mut data = [0xff; 8];
        process.memory.read(process.entry + 128, &mut data).unwrap();
        assert_eq!(data, [0; 8]);
        assert!(process.memory.write(process.entry, &[0]).is_err());
    }

    #[test]
    fn overlap_invalid_entry_budget_and_bias_are_rejected() {
        let bytes = demo::elf();
        let mut image = pentax_loader::parse(&bytes).unwrap();
        image.segments.push(image.segments[0]);
        assert!(matches!(
            load(&image, &LoadOptions::default()),
            Err(ProcessError::OverlappingSegments)
        ));
        image.segments.pop();
        image.entry = 0;
        assert!(matches!(
            load(&image, &LoadOptions::default()),
            Err(ProcessError::EntryNotExecutable)
        ));
        image.entry = image.segments[0].vaddr;
        let options = LoadOptions {
            mapped_limit: 1,
            ..LoadOptions::default()
        };
        assert!(matches!(
            load(&image, &options),
            Err(ProcessError::ImageTooLarge)
        ));
        let options = LoadOptions {
            load_bias: ADDRESS_LIMIT,
            ..LoadOptions::default()
        };
        assert!(matches!(
            load(&image, &options),
            Err(ProcessError::AddressOverflow)
        ));
    }
}
