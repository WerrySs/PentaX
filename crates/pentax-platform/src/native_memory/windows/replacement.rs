// SPDX-License-Identifier: GPL-2.0-only

//! Loader-owned word replacement without replaying pages or relaxing final rights.
use super::*;

fn charge(
    work: &mut u64,
    amount: u64,
    limits: &NativeTransactionLimits,
) -> Result<(), NativeMemoryError> {
    *work = work
        .checked_add(amount)
        .filter(|value| *value <= limits.max_preflight_steps)
        .ok_or(NativeMemoryError::BudgetExceeded)?;
    Ok(())
}

impl NativeAddressSpace {
    /// Compare all old words before changing any payload, then replace only the
    /// listed bytes and restore every page's CURRENT logical/physical rights.
    /// This trusted-owner operation is not a guest write-permission bypass.
    /// All physical pages remain NX, including temporary loader write access.
    /// Aliases of target bytes at other guest addresses are refused. Recoverable
    /// failures restore bytes and rights; failed rollback poisons the owner.
    pub fn compare_replace_words(
        &mut self,
        replacements: &[NativeWordReplacement],
        limits: &NativeTransactionLimits,
    ) -> Result<(), NativeMemoryError> {
        // This path temporarily changes physical rights without calling
        // protect(). Invalidate before even a refused attempt; sealing and all
        // rollback paths retain an empty proof until the next live span check.
        self.invalidate_stack_preflight();
        self.live()?;
        let count = replacements.len();
        if count > limits.max_outputs
            || (count as u64)
                .checked_mul(8)
                .is_none_or(|bytes| bytes > limits.max_output_bytes)
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        if count == 0 {
            return Ok(());
        }
        let page_capacity = count
            .checked_mul(2)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let scratch = page_capacity
            .checked_mul(std::mem::size_of::<Page>())
            .and_then(|bytes| {
                bytes.checked_add(count.checked_mul(std::mem::size_of::<NativeWordReplacement>())?)
            })
            .filter(|bytes| *bytes <= limits.max_scratch_bytes)
            .ok_or(NativeMemoryError::BudgetExceeded)?;
        let mut work = 0;
        charge(
            &mut work,
            (count as u64)
                .checked_mul(128)
                .and_then(|value| value.checked_add(scratch as u64))
                .ok_or(NativeMemoryError::BudgetExceeded)?,
            limits,
        )?;
        let mut words = Vec::new();
        words
            .try_reserve_exact(count)
            .map_err(|_| NativeMemoryError::Allocation)?;
        let mut pages = Vec::<Page>::new();
        pages
            .try_reserve_exact(page_capacity)
            .map_err(|_| NativeMemoryError::Allocation)?;
        if words
            .capacity()
            .checked_mul(std::mem::size_of::<NativeWordReplacement>())
            .and_then(|bytes| {
                bytes.checked_add(pages.capacity().checked_mul(std::mem::size_of::<Page>())?)
            })
            .is_none_or(|bytes| bytes > limits.max_scratch_bytes)
        {
            return Err(NativeMemoryError::BudgetExceeded);
        }
        words.extend_from_slice(replacements);
        words.sort_unstable_by_key(|word| word.address);
        for (index, word) in words.iter().enumerate() {
            let end = range(word.address, 8)?;
            self.check(word.address, 8, None)?;
            if index != 0 && words[index - 1].address + 8 > word.address {
                return Err(NativeMemoryError::Overlap);
            }
            let first = word.address / NATIVE_GUEST_PAGE * NATIVE_GUEST_PAGE;
            for page in (first..end).step_by(NATIVE_GUEST_PAGE as usize) {
                let metadata = self.pages[self
                    .index(page)
                    .map_err(|_| NativeMemoryError::Unmapped(page))?];
                pages.push(metadata);
                let at = page.max(word.address);
                let length = end.min(page + NATIVE_GUEST_PAGE) - at;
                let region = self.region(at);
                if let RegionKind::Shared { physical } = region.kind {
                    let coordinate = physical + at - region.address;
                    // Private mappings cannot alias; shared mappings must be
                    // checked against every other live view, not just outputs.
                    charge(
                        &mut work,
                        (self.pages.len() as u64)
                            .checked_mul(128)
                            .ok_or(NativeMemoryError::BudgetExceeded)?,
                        limits,
                    )?;
                    for other in &self.pages {
                        if other.address == page {
                            continue;
                        }
                        let other_region = self.region(other.address);
                        if let RegionKind::Shared {
                            physical: other_physical,
                        } = other_region.kind
                        {
                            let other_coordinate =
                                other_physical + other.address - other_region.address;
                            if coordinate < other_coordinate + NATIVE_GUEST_PAGE
                                && other_coordinate < coordinate + length
                            {
                                return Err(NativeMemoryError::Overlap);
                            }
                        }
                    }
                }
            }
        }
        pages.sort_unstable_by_key(|page| page.address);
        pages.dedup_by_key(|page| page.address);
        for (opened, page) in pages.iter().enumerate() {
            let result = self
                .operation()
                .and_then(|()| protect_page(page.address, PAGE_READWRITE));
            if let Err(error) = result {
                return self.replacement_failure(&pages[..opened], &words, 0, false, error);
            }
        }
        for word in &words {
            let mut actual = [0; 8];
            // SAFETY: Complete mapped-owner admission and successful temporary
            // RW/NX protection cover all eight bytes, including page crossings.
            // The exclusive owner prevents remapping or native entry meanwhile.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    word.address as usize as *const u8,
                    actual.as_mut_ptr(),
                    8,
                );
            }
            if actual != word.expected.to_le_bytes() {
                return self.replacement_failure(
                    &pages,
                    &words,
                    0,
                    false,
                    NativeMemoryError::ReplacementMismatch,
                );
            }
        }
        let mut copied = 0;
        for word in &words {
            if let Err(error) = self.operation() {
                return self.replacement_failure(&pages, &words, copied, false, error);
            }
            // SAFETY: The same admitted pages remain exclusively owned RW/NX;
            // safe Rust input storage cannot alias native guest allocations.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    word.replacement.to_le_bytes().as_ptr(),
                    word.address as usize as *mut u8,
                    8,
                );
            }
            copied += 1;
        }
        for page in &pages {
            let result = self
                .operation()
                .and_then(|()| protect_page(page.address, physical(page.permissions)));
            if let Err(error) = result {
                return self.replacement_failure(&pages, &words, copied, true, error);
            }
        }
        // Logical permissions, mapping lifetimes, counters and page metadata
        // never changed. No allocation or OS call follows successful sealing.
        Ok(())
    }

    fn replacement_failure(
        &mut self,
        pages: &[Page],
        words: &[NativeWordReplacement],
        copied: usize,
        reopen: bool,
        original: NativeMemoryError,
    ) -> Result<(), NativeMemoryError> {
        let mut restored = true;
        if reopen {
            for page in pages {
                if !self.rollback_operation() || protect_page(page.address, PAGE_READWRITE).is_err()
                {
                    restored = false;
                }
            }
        }
        // If reopening failed, never copy through a possibly inaccessible page.
        // Poisoning still prevents every future read, mutation or native resume.
        if restored {
            for word in &words[..copied] {
                if !self.rollback_operation() {
                    restored = false;
                    continue;
                }
                // SAFETY: Every target page remains or was restored RW/NX under
                // the original exclusive owner; these are the attested old bytes.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        word.expected.to_le_bytes().as_ptr(),
                        word.address as usize as *mut u8,
                        8,
                    );
                }
            }
        }
        for page in pages {
            if !self.rollback_operation()
                || protect_page(page.address, physical(page.permissions)).is_err()
            {
                restored = false;
            }
        }
        if !restored {
            self.poisoned = true;
        }
        if self.poisoned {
            Err(NativeMemoryError::Poisoned)
        } else {
            Err(original)
        }
    }
}

#[cfg(test)]
mod tests;
