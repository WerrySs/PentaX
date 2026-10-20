// SPDX-License-Identifier: GPL-2.0-only

//! Owned zeroed pthread prefixes, with atomic page/slot publication and release.
//! Private host state remains in MutexState, never in the guest allocation.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PthreadObjectKind {
    Mutex,
    Attribute,
    Condition,
    ConditionAttribute,
    Rwlock,
    RwlockAttribute,
    ThreadAttribute,
}

impl PthreadObjectKind {
    pub(crate) const fn prefix_bytes(self) -> u64 {
        // Pinned pthread.cpp mutex/condition objects and their attributes.
        match self {
            Self::Mutex | Self::Condition | Self::Rwlock => 256,
            Self::Attribute
            | Self::ConditionAttribute
            | Self::RwlockAttribute
            | Self::ThreadAttribute => 64,
        }
    }
}

impl MemoryState {
    pub(crate) fn owns_pthread_object(
        &self,
        address: u64,
        budget: &mut Budget,
    ) -> Result<bool, Failure> {
        budget.work(self.mappings.len() as u64 + 1)?;
        Ok(self
            .mappings
            .iter()
            .any(|mapping| mapping.start == address && mapping.pthread.is_some()))
    }

    pub(crate) fn create_pthread_object(
        &mut self,
        kind: PthreadObjectKind,
        slot: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.can_add_handle()?;
        budget.can_map(PAGE)?;
        let address = self.choose_address(self.next_pthread_address, PAGE, PAGE, 0, budget)?;
        let next_address = address
            .checked_add(PAGE)
            .filter(|end| *end <= 1 << 47)
            .ok_or(Errno::NoMemory)?;
        let next_mapping = self
            .next_mapping
            .checked_add(1)
            .ok_or(Failure::Limit("mapping identities"))?;
        budget.work(PAGE)?;
        // A private page makes destruction revoke every access to the prefix.
        // Both the source-sized prefix and page padding start zero and remain
        // RW/NX; neither contains a host mutex, pointer or allocator metadata.
        debug_assert!(kind.prefix_bytes() <= PAGE);
        budget.apply(
            guest,
            MemoryEffect::MapZeroed {
                address,
                length: PAGE,
                protection: 3,
            },
            &[GuestWrite {
                address: slot,
                bytes: &address.to_le_bytes(),
            }],
        )?;
        // The object's one shared handle also accounts for its mapping record.
        // Both vectors reserved max_handles, so publication cannot allocate here.
        self.mappings.push(Mapping {
            identity: self.next_mapping,
            start: address,
            length: PAGE,
            protection: 3,
            physical: None,
            internal: true,
            pthread: Some(kind),
            _name: Vec::new(),
        });
        self.next_mapping = next_mapping;
        // Never reissue an old pthread address, even after its page is released.
        // A stale copied slot cannot accidentally identify a newer object.
        self.next_pthread_address = next_address;
        budget.added_handle();
        budget.mapped(PAGE);
        Ok(address)
    }

    pub(crate) fn destroy_pthread_object(
        &mut self,
        kind: PthreadObjectKind,
        address: u64,
        slot: u64,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        budget.work(self.mappings.len() as u64 + 1)?;
        let index = self
            .mappings
            .iter()
            .position(|mapping| {
                mapping.start == address
                    && mapping.length == PAGE
                    && mapping.internal
                    && mapping.pthread == Some(kind)
            })
            .ok_or(Failure::Unsupported("unowned pthread object backing"))?;
        let slot_end = slot
            .checked_add(8)
            .ok_or(crate::GuestError::AddressOverflow)?;
        if slot < address + PAGE && address < slot_end {
            return Err(Failure::Unsupported(
                "pthread output overlaps released object",
            ));
        }
        budget.apply(
            guest,
            MemoryEffect::Unmap {
                address,
                length: PAGE,
            },
            &[GuestWrite {
                address: slot,
                bytes: &0_u64.to_le_bytes(),
            }],
        )?;
        self.mappings.swap_remove(index);
        budget.removed_handle();
        budget.unmapped(PAGE);
        Ok(())
    }
}
