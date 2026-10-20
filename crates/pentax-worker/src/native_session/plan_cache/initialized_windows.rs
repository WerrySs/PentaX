// SPDX-License-Identifier: GPL-2.0-only
//! Exact immutable-metadata results for one borrowed source epoch.
//!
//! This retains no bytes, live mapping/permission proof or evaluated operand.
//! Only the unchanged complete scanner can populate a successful entry. An
//! exact PC and exact maximum are required; a result grants no segment-wide
//! uniqueness. Every caller still charges the original metadata walk and
//! performs its current live fetch, byte comparisons and checked dispatch.
use super::*;

#[derive(Clone, Copy)]
struct Entry {
    pc: u64,
    maximum: u64,
    module: usize,
    length: u64, // Zero is empty; the scanner never succeeds with zero length.
}
const EMPTY: Entry = Entry {
    pc: 0,
    maximum: 0,
    module: 0,
    length: 0,
};

pub(super) struct Memo<'view, 'plan, 'image> {
    source: &'view MaterializedSource<'plan, 'image>,
    entries: Vec<Entry>,
    bytes: u64,
    #[cfg(test)]
    scans: u64,
}
impl<'view, 'plan, 'image> Memo<'view, 'plan, 'image> {
    pub(super) fn new(
        source: &'view MaterializedSource<'plan, 'image>,
        maximum_bytes: u64,
        work: &mut u64,
        work_limit: u64,
    ) -> Self {
        let mut memo = Self {
            source,
            entries: Vec::new(),
            bytes: 0,
            #[cfg(test)]
            scans: 0,
        };
        let Some(requested) = (SLOTS as u64)
            .checked_mul(std::mem::size_of::<Entry>() as u64)
            .filter(|bytes| *bytes <= maximum_bytes)
        else {
            return memo;
        };
        // The one-time construction cost is additional to unchanged per-block
        // charges. An exact old work budget can disable this optimization.
        if charge(work, requested, work_limit, "admission work").is_err() {
            return memo;
        }
        let mut entries = Vec::new();
        if entries.try_reserve_exact(SLOTS).is_err() {
            return memo;
        }
        let Some(actual) = (entries.capacity() as u64)
            .checked_mul(std::mem::size_of::<Entry>() as u64)
            .filter(|bytes| *bytes <= maximum_bytes)
        else {
            return memo;
        };
        if actual > requested
            && charge(work, actual - requested, work_limit, "admission work").is_err()
        {
            return memo;
        }
        entries.resize(SLOTS, EMPTY);
        memo.entries = entries;
        memo.bytes = actual;
        memo
    }

    pub(super) fn yield_to_plans(&mut self, retained: u64, incoming: u64, maximum: u64) {
        if retained
            .checked_add(incoming)
            .and_then(|bytes| bytes.checked_add(self.bytes))
            .is_none_or(|bytes| bytes > maximum)
        {
            // Disable for the rest of this epoch, avoiding repeated allocation
            // attempts and preserving all space previously available to plans.
            self.entries = Vec::new();
            self.bytes = 0;
        }
    }

    pub(super) fn get(
        &mut self,
        pc: u64,
        maximum: u64,
    ) -> Result<(usize, u64), NativeSessionError> {
        if self.entries.is_empty() {
            #[cfg(test)]
            {
                self.scans = self.scans.saturating_add(1);
            }
            return initialized_window(self.source, pc, maximum);
        }
        let index = ((pc ^ (pc >> 8) ^ (pc >> 16) ^ (pc >> 32)) as usize) & (SLOTS - 1);
        if let Some(entry) = self.entries.get(index)
            && entry.length != 0
            && entry.pc == pc
            && entry.maximum == maximum
        {
            return Ok((entry.module, entry.length));
        }
        #[cfg(test)]
        {
            self.scans = self.scans.saturating_add(1);
        }
        let result = initialized_window(self.source, pc, maximum)?;
        if let Some(entry) = self.entries.get_mut(index) {
            *entry = Entry {
                pc,
                maximum,
                module: result.0,
                length: result.1,
            };
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
