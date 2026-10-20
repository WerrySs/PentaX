// SPDX-License-Identifier: GPL-2.0-only

//! Native append with exact fallback word replacement; old pages are never replayed.
use super::{NativeGuestMemory, NativeLoadLimits};
use crate::memory::service::{StagingMemory, convert};
use crate::memory::{PAGE_SIZE, Permissions};
use crate::process::{ProcessError, SessionPlan, session_page_plan};
use pentax_linker::{Resolution, SymbolicLinkPlan};
use pentax_loader::ElfType;
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeRange, NativeTransactionLimits, NativeWordReplacement,
};

const MAX_REPLACEMENTS: usize = 16_384;

impl NativeGuestMemory<'_, '_> {
    /// Internal publication into a separately admitted dynamic window. The
    /// caller must bind the selected file/capability and current prepared source
    /// before calling. This ordinary entry refuses new TLS/deferred writes;
    /// the private relocated entry additionally binds typed TLS preparation.
    /// Only added pages are replayed. Explicit source-bound fallback transitions
    /// may replace retained words after comparing their complete old bytes.
    #[allow(dead_code)] // Retained single-append entry for internal consumers.
    pub(crate) fn append_source(
        &mut self,
        source: &SymbolicLinkPlan<'_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
    ) -> Result<(), ProcessError> {
        self.append_source_batch(source, window, limits, |_| Ok(()))
    }

    /// Publish one guest and an optional separately authenticated stop provider.
    /// The caller's final attestation runs after sealing every new page, before
    /// any retained fallback word changes. No callback runs after replacements.
    pub(crate) fn append_source_batch(
        &mut self,
        source: &SymbolicLinkPlan<'_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
        before_replacements: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
    ) -> Result<(), ProcessError> {
        self.append_in(
            SessionPlan::Ordinary(source),
            window,
            limits,
            |_| Ok(()),
            before_replacements,
        )
    }

    #[allow(dead_code)] // Retained single-append entry for internal consumers.
    pub(crate) fn append_relocated_source(
        &mut self,
        plan: &crate::process::tls::RelocatedStaticTlsPlan<'_, '_, '_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
        initialize: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
    ) -> Result<(), ProcessError> {
        self.append_relocated_source_batch(plan, window, limits, initialize, |_| Ok(()))
    }

    pub(crate) fn append_relocated_source_batch(
        &mut self,
        plan: &crate::process::tls::RelocatedStaticTlsPlan<'_, '_, '_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
        initialize: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
        before_replacements: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
    ) -> Result<(), ProcessError> {
        self.append_in(
            SessionPlan::Relocated(plan),
            window,
            limits,
            initialize,
            before_replacements,
        )
    }

    fn append_in(
        &mut self,
        admission: SessionPlan<'_, '_, '_, '_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
        initialize: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
        before_replacements: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
    ) -> Result<(), ProcessError> {
        let source = admission.base();
        let module_index = self.metadata.module_count;
        let added_count = source
            .modules()
            .len()
            .checked_sub(module_index)
            .filter(|count| (1..=2).contains(count))
            .ok_or(ProcessError::InvalidSegment)?;
        if source.parent_identity() != Some(self.active_source)
            || source
                .appended_guest()
                .is_none_or(|guest| guest.index() != module_index)
        {
            return Err(ProcessError::InvalidSegment);
        }
        let added = &source.modules()[module_index..];
        for module in added {
            if !matches!(
                module.runtime_plan().metadata.elf_type,
                ElfType::SharedObject | ElfType::SceSharedObject
            ) {
                return Err(ProcessError::InvalidSegment);
            }
            if module.runtime_plan().tls.is_some()
                && (module.id().index() != module_index
                    || matches!(&admission, SessionPlan::Ordinary(_)))
            {
                return Err(ProcessError::NativeUnsupportedTls {
                    module: module.id().index(),
                });
            }
        }
        if admission.deferred_count() != 0 {
            return Err(ProcessError::NativeDeferredRelocations {
                count: admission.deferred_count(),
            });
        }
        let retained_count = source
            .patches()
            .iter()
            .filter(|patch| patch.consumer().index() < module_index)
            .count();
        if retained_count != self.ordinary_patches {
            return Err(ProcessError::InvalidSegment);
        }
        let transitions = source.fallback_transitions();
        if transitions.len() > MAX_REPLACEMENTS {
            return Err(ProcessError::ImageTooLarge);
        }
        let mut replacements = Vec::new();
        replacements
            .try_reserve_exact(transitions.len())
            .map_err(|_| ProcessError::Memory(crate::memory::MemoryError::BudgetExceeded))?;
        for transition in transitions {
            let old = transition.old_patch();
            let new = transition.new_patch();
            if transition.parent_identity() != self.active_source
                || transition.next_identity() != source.identity()
                || old.consumer().index() >= module_index
                || new.consumer() != old.consumer()
                || new.address() != old.address()
                || new.source() != old.source()
                || old.resolution() != Resolution::Definition(transition.old_provider())
                || new.resolution() != Resolution::Definition(transition.new_provider())
                || transition.old_provider().module.index() >= module_index
                || transition.new_provider().module.index() != module_index
            {
                return Err(ProcessError::InvalidSegment);
            }
            replacements.push(NativeWordReplacement {
                address: old.address(),
                expected: old.value(),
                replacement: new.value(),
            });
        }
        let retained_tls = self
            .metadata
            .applied_relocations
            .checked_sub(retained_count)
            .ok_or(ProcessError::InvalidSegment)?;
        if admission
            .tls_patches()
            .iter()
            .filter(|patch| patch.consumer().index() < module_index)
            .count()
            != retained_tls
        {
            return Err(ProcessError::InvalidSegment);
        }
        let applied = source
            .patch_count()
            .checked_add(admission.tls_patches().len())
            .ok_or(ProcessError::AddressOverflow)?;
        let window_end = window
            .address
            .checked_add(window.size)
            .filter(|_| window.size != 0)
            .ok_or(ProcessError::AddressOverflow)?;
        let layout = session_page_plan(&admission, &limits.session)?;
        let mut pages = Vec::new();
        let count = layout
            .pages
            .values()
            .filter(|(owner, _)| *owner >= module_index)
            .count();
        pages
            .try_reserve_exact(count)
            .map_err(|_| ProcessError::Memory(crate::memory::MemoryError::BudgetExceeded))?;
        let mut module_pages = [false; 2];
        for (&address, &(owner, permissions)) in &layout.pages {
            if owner < module_index {
                continue;
            }
            if address < window.address
                || address
                    .checked_add(PAGE_SIZE)
                    .is_none_or(|end| end > window_end)
            {
                return Err(ProcessError::InvalidSegment);
            }
            module_pages[owner - module_index] = true;
            pages.push((address, permissions));
        }
        if module_pages[..added_count].iter().any(|present| !present) {
            return Err(ProcessError::NoLoadSegments);
        }
        let added_bytes = (pages.len() as u64)
            .checked_mul(PAGE_SIZE)
            .ok_or(ProcessError::ImageTooLarge)?;
        if self
            .mapped_bytes()
            .checked_add(added_bytes)
            .is_none_or(|total| {
                total
                    > limits
                        .session
                        .mapped_limit
                        .min(limits.session.resident_limit)
            })
        {
            return Err(ProcessError::ImageTooLarge);
        }
        {
            let paused = self
                .space
                .pause()
                .map_err(|error| ProcessError::Memory(convert(error)))?;
            for &(address, _) in &pages {
                paused
                    .preflight_reserved_unmapped(address, PAGE_SIZE)
                    .map_err(|error| ProcessError::Memory(convert(error)))?;
            }
        }
        let mut runs: Vec<(u64, u64)> = Vec::new();
        runs.try_reserve_exact(pages.len())
            .map_err(|_| ProcessError::Memory(crate::memory::MemoryError::BudgetExceeded))?;
        for &(address, _) in &pages {
            if let Some(last) = runs.last_mut().filter(|last| last.0 + last.1 == address) {
                last.1 += PAGE_SIZE;
            } else {
                runs.push((address, PAGE_SIZE));
            }
        }
        let mut mapped = 0;
        let result = (|| {
            for &(address, size) in &runs {
                self.space.map(address, size, Permissions::READ_WRITE)?;
                mapped += 1;
            }
            for module in added {
                for segment in &module.runtime_plan().segments {
                    if !segment.file_bytes.is_empty() {
                        let address = module
                            .load_bias()
                            .checked_add(segment.vaddr)
                            .ok_or(ProcessError::AddressOverflow)?;
                        self.space.write(address, segment.file_bytes)?;
                    }
                }
            }
            for patch in source
                .patches()
                .iter()
                .filter(|patch| patch.consumer().index() >= module_index)
            {
                self.space
                    .write(patch.address(), &patch.value().to_le_bytes())?;
            }
            for patch in admission
                .tls_patches()
                .iter()
                .filter(|patch| patch.consumer().index() >= module_index)
            {
                self.space
                    .write(patch.address(), &patch.value().to_le_bytes())?;
            }
            initialize(&mut self.space)?;
            #[cfg(test)]
            if std::mem::take(&mut self.fail_append_after_initializer) {
                return Err(ProcessError::InvalidSegment);
            }
            for &(address, permissions) in &pages {
                StagingMemory::protect(&mut self.space, address, PAGE_SIZE, permissions)?;
            }
            before_replacements(&mut self.space)?;
            // Last fallible stage: the platform attests all retained old words
            // and restores current page rights, or rolls back every replacement.
            // Failure still removes new runs; the outer TLS owner restores its
            // staged new blocks. No old module/page payload is replayed.
            self.space
                .compare_replace_words(
                    &replacements,
                    &NativeTransactionLimits {
                        max_outputs: MAX_REPLACEMENTS,
                        max_output_bytes: MAX_REPLACEMENTS as u64 * 8,
                        max_preflight_steps: 64 * 1024 * 1024,
                        max_scratch_bytes: 4 * 1024 * 1024,
                    },
                )
                .map_err(|error| ProcessError::Memory(convert(error)))?;
            Ok::<_, ProcessError>(())
        })();
        if let Err(error) = result {
            for &(address, size) in runs[..mapped].iter().rev() {
                if self.space.unmap(address, size).is_err() {
                    self.space.abandon();
                    return Err(ProcessError::Memory(convert(
                        pentax_platform::native_memory::NativeMemoryError::Poisoned,
                    )));
                }
            }
            return Err(error);
        }
        self.active_source = source.identity();
        self.ordinary_patches = source.patch_count();
        self.metadata.module_count = source.modules().len();
        self.metadata.applied_relocations = applied;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Actual page publication/rollback with original inputs; no guest executes.
    use super::*;
    use crate::memory::native::prepare_in;
    use pentax_linker::{ModuleInput, append_symbolic_module, plan_symbolic};
    use pentax_loader::open_executable;
    use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryLimits};

    mod batch;

    fn original(data: u64) -> Vec<u8> {
        let mut bytes = vec![0; 4096];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        for (offset, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 3)] {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        for (offset, value) in [(24, 0x100_u64), (32, 64)] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        for (at, kind, flags, offset, address, file, memory, align) in [
            (
                64, 1_u32, 5_u32, 0_u64, 0_u64, 4096_u64, PAGE_SIZE, PAGE_SIZE,
            ),
            (120, 1, 6, 0, data, 0, PAGE_SIZE, PAGE_SIZE),
            (176, 2, 4, 0x200, 0x200, 80, 80, 8),
        ] {
            bytes[at..at + 4].copy_from_slice(&kind.to_le_bytes());
            bytes[at + 4..at + 8].copy_from_slice(&flags.to_le_bytes());
            for (field, value) in [
                (8, offset),
                (16, address),
                (32, file),
                (40, memory),
                (48, align),
            ] {
                bytes[at + field..at + field + 8].copy_from_slice(&value.to_le_bytes());
            }
        }
        bytes[0x100..0x106].copy_from_slice(&[0xb8, 42, 0, 0, 0, 0xc3]);
        for (index, (tag, value)) in [(7_i64, 0x300_u64), (8, 24), (9, 24), (12, 0x100), (0, 0)]
            .into_iter()
            .enumerate()
        {
            bytes[0x200 + index * 16..0x208 + index * 16].copy_from_slice(&tag.to_le_bytes());
            bytes[0x208 + index * 16..0x210 + index * 16].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [(0x300, data), (0x308, 8), (0x310, 0x1234)] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
    #[test]
    fn append_maps_and_relocates_only_new_pages_while_retaining_existing_guest_writes() {
        let limits = NativeLoadLimits::default();
        let space =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x80000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        let old = original(PAGE_SIZE);
        let source = plan_symbolic(
            &[ModuleInput::new(open_executable(&old).unwrap(), base)],
            &Default::default(),
        )
        .unwrap();
        let mut memory = prepare_in(&source, &limits, Some(space)).unwrap();
        let owner = memory.owner_id();
        let reserved = memory.reserved_bytes();
        let sentinel = 0x1122_3344_5566_7788_u64;
        memory
            .pause()
            .unwrap()
            .write(base + PAGE_SIZE, &sentinel.to_le_bytes())
            .unwrap();
        let input = original(PAGE_SIZE);
        let window = NativeRange {
            address: base + 0x40000,
            size: 0x20000,
        };
        let next = append_symbolic_module(
            &source,
            ModuleInput::new(open_executable(&input).unwrap(), window.address),
            &Default::default(),
        )
        .unwrap();
        memory.append_source(&next, window, &limits).unwrap();
        assert_eq!(memory.owner_id(), owner);
        assert_eq!(memory.reserved_bytes(), reserved);
        assert_eq!(memory.active_source_identity(), next.identity());
        assert_eq!(memory.source_plan().identity(), source.identity());
        assert_eq!(memory.module_count(), 2);
        assert_eq!(memory.applied_relocations(), 2);
        let mut word = [0; 8];
        memory
            .pause()
            .unwrap()
            .read(base + PAGE_SIZE, &mut word)
            .unwrap();
        assert_eq!(u64::from_le_bytes(word), sentinel);
        memory
            .pause()
            .unwrap()
            .read(window.address + PAGE_SIZE, &mut word)
            .unwrap();
        assert_eq!(u64::from_le_bytes(word), window.address + 0x1234);
        assert!(
            memory
                .pause()
                .unwrap()
                .preflight_fetch_source(window.address + 0x100, 1)
                .is_ok()
        );
        assert!(
            memory
                .pause()
                .unwrap()
                .preflight_write(window.address + 0x100, 1)
                .is_err()
        );
        assert!(memory.append_source(&next, window, &limits).is_err());
    }

    #[test]
    fn failed_append_rolls_back_new_native_runs_and_keeps_original_epoch_and_data() {
        let limits = NativeLoadLimits::default();
        let space = NativeAddressSpace::reserve_any(
            NativeMemoryLimits {
                mapped_bytes: 3 * PAGE_SIZE,
                reserved_bytes: 0x80000,
            },
            0x80000,
        )
        .unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        let old = original(PAGE_SIZE);
        let source = plan_symbolic(
            &[ModuleInput::new(open_executable(&old).unwrap(), base)],
            &Default::default(),
        )
        .unwrap();
        let mut memory = prepare_in(&source, &limits, Some(space)).unwrap();
        memory
            .pause()
            .unwrap()
            .write(base + PAGE_SIZE, &[51])
            .unwrap();
        // Separate runs: the first fits the actual narrower owner limit; the
        // second cannot commit, requiring removal of that first new run.
        let input = original(2 * PAGE_SIZE);
        let window = NativeRange {
            address: base + 0x40000,
            size: 0x20000,
        };
        let next = append_symbolic_module(
            &source,
            ModuleInput::new(open_executable(&input).unwrap(), window.address),
            &Default::default(),
        )
        .unwrap();
        assert!(matches!(
            memory.append_source(&next, window, &limits),
            Err(ProcessError::Memory(
                crate::memory::MemoryError::BudgetExceeded
            ))
        ));
        assert_eq!(memory.active_source_identity(), source.identity());
        assert_eq!(memory.module_count(), 1);
        assert_eq!(memory.applied_relocations(), 1);
        assert_eq!(memory.mapped_bytes(), 2 * PAGE_SIZE);
        memory
            .pause()
            .unwrap()
            .preflight_reserved_unmapped(window.address, 3 * PAGE_SIZE)
            .unwrap();
        let mut retained = [0];
        memory
            .pause()
            .unwrap()
            .read(base + PAGE_SIZE, &mut retained)
            .unwrap();
        assert_eq!(retained, [51]);
    }
}
