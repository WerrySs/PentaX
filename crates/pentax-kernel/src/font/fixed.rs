// SPDX-License-Identifier: GPL-2.0-only
//! Small ordered stores without reading or clobbering intervening guest bytes.

use super::*;

pub(crate) struct FixedWrites {
    addresses: [u64; 8],
    lengths: [usize; 8],
    bytes: [[u8; 256]; 8],
    count: usize,
}
impl FixedWrites {
    pub(crate) fn new() -> Self {
        Self {
            addresses: [0; 8],
            lengths: [0; 8],
            bytes: [[0; 256]; 8],
            count: 0,
        }
    }
    pub(crate) fn push(&mut self, address: u64, data: &[u8]) -> Result<(), Failure> {
        if data.len() > 256 || self.count == 8 {
            return Err(Failure::Limit("font fixed outputs"));
        }
        let mut first = address;
        let mut last = address
            .checked_add(data.len() as u64)
            .ok_or(GuestError::AddressOverflow)?;
        if data.is_empty() {
            return Ok(());
        }
        let mut included = [false; 8];
        loop {
            let mut changed = false;
            for (index, slot) in included.iter_mut().enumerate().take(self.count) {
                let end = self.addresses[index] + self.lengths[index] as u64;
                if !*slot && self.addresses[index] < last && first < end {
                    *slot = true;
                    first = first.min(self.addresses[index]);
                    last = last.max(end);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let length =
            usize::try_from(last - first).map_err(|_| Failure::Limit("font fixed output span"))?;
        if length > 256 {
            return Err(Failure::Limit("font fixed output span"));
        }
        let mut merged = [0; 256];
        for (index, slot) in included.iter().enumerate().take(self.count) {
            if *slot {
                let at = (self.addresses[index] - first) as usize;
                merged[at..at + self.lengths[index]]
                    .copy_from_slice(&self.bytes[index][..self.lengths[index]]);
            }
        }
        let at = (address - first) as usize;
        merged[at..at + data.len()].copy_from_slice(data);
        let mut next = 0;
        for (index, slot) in included.iter().enumerate().take(self.count) {
            if !*slot {
                self.addresses[next] = self.addresses[index];
                self.lengths[next] = self.lengths[index];
                self.bytes[next] = self.bytes[index];
                next += 1;
            }
        }
        self.addresses[next] = first;
        self.lengths[next] = length;
        self.bytes[next] = merged;
        self.count = next + 1;
        Ok(())
    }
    pub(super) fn writes(&self) -> [GuestWrite<'_>; 8] {
        std::array::from_fn(|index| GuestWrite {
            address: self.addresses[index],
            bytes: &self.bytes[index][..self.lengths[index]],
        })
    }
    pub(crate) fn apply(
        &self,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        budget.work(8 * 256)?;
        budget.write(guest, &self.writes()[..self.count])
    }
    pub(super) fn outputs(&self) -> Result<RuntimeDataOutputs, Failure> {
        let mut outputs = RuntimeDataOutputs::new();
        for write in &self.writes()[..self.count] {
            outputs.push(write.address, write.bytes)?;
        }
        Ok(outputs)
    }
}
