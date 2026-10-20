// SPDX-License-Identifier: GPL-2.0-only
//! Opaque service tokens backed by one authenticated internal RW/NX mapping.

use super::*;

#[derive(Clone, Copy)]
pub(crate) struct RuntimeObject {
    pub(crate) address: u64,
    identity: u64,
    length: u64,
}

impl MemoryState {
    pub(crate) fn runtime_object(
        &mut self,
        size: usize,
        held_scratch: usize,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        prepare: impl FnOnce(u64, &mut [u8]) -> Result<RuntimeDataOutputs, Failure>,
    ) -> Result<RuntimeObject, Failure> {
        let identity = self.next_mapping;
        let address = self.runtime_data(size, held_scratch, guest, budget, prepare)?;
        // runtime_data admitted the rounded length before publishing its mapping.
        let length = ((size as u64) + PAGE - 1) & !(PAGE - 1);
        Ok(RuntimeObject {
            address,
            identity,
            length,
        })
    }

    fn runtime_object_index(
        &self,
        object: RuntimeObject,
        budget: &mut Budget,
    ) -> Result<usize, Failure> {
        budget.work(self.mappings.len() as u64 * 4 + 1)?;
        let mut found = None;
        for (index, mapping) in self.mappings.iter().enumerate() {
            if mapping.identity != object.identity {
                continue;
            }
            if found.is_some()
                || mapping.start != object.address
                || mapping.length != object.length
                || mapping.protection != 3
                || !mapping.internal
                || mapping.physical.is_some()
                || mapping.pthread.is_some()
            {
                return Err(Failure::Unsupported("changed runtime object mapping"));
            }
            found = Some(index);
        }
        found.ok_or(Failure::Unsupported("released runtime object mapping"))
    }

    pub(crate) fn check_runtime_object(
        &self,
        object: RuntimeObject,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        self.runtime_object_index(object, budget).map(|_| ())
    }

    pub(crate) fn release_runtime_object(
        &mut self,
        object: RuntimeObject,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        self.release_runtime_object_with_outputs(object, guest, budget, &[])
    }

    /// Release only this authenticated mapping and publish caller outputs in
    /// the same backing transaction. Failed output admission retains the live
    /// object and every caller byte; no fallible state edit follows commit.
    pub(crate) fn release_runtime_object_with_outputs(
        &mut self,
        object: RuntimeObject,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
        writes: &[GuestWrite<'_>],
    ) -> Result<(), Failure> {
        let index = self.runtime_object_index(object, budget)?;
        budget.work(object.length / PAGE + self.mappings.len() as u64)?;
        budget.apply(
            guest,
            MemoryEffect::Unmap {
                address: object.address,
                length: object.length,
            },
            writes,
        )?;
        // Only the authenticated complete mapping is removed. No allocation or
        // fallible lookup follows the actual backing transaction.
        self.mappings.remove(index);
        budget.removed_handle();
        budget.unmapped(object.length);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{ARG, TestGuest};

    #[test]
    fn uds_runtime_objects_publish_and_release_atomically_and_reject_reused_address_identity() {
        let limits = Limits::default();
        let mut memory = MemoryState::new(limits).unwrap();
        let mut budget = Budget::new(limits).unwrap();
        let mut guest = TestGuest::new();
        guest.refuse = true;
        let create = |memory: &mut MemoryState, guest: &mut TestGuest, budget: &mut Budget| {
            memory.runtime_object(16, 0, guest, budget, |address, bytes| {
                bytes.copy_from_slice(&[0x5a; 16]);
                let mut outputs = RuntimeDataOutputs::new();
                outputs.push(ARG, &address.to_le_bytes())?;
                Ok(outputs)
            })
        };
        assert!(create(&mut memory, &mut guest, &mut budget).is_err());
        assert_eq!(memory.retained_handle_count(), 0);
        assert_eq!(budget.usage().handles_live, 0);
        let mut output = [0; 8];
        guest.read(ARG, &mut output).unwrap();
        assert_eq!(output, [0; 8]);
        guest.refuse = false;
        let first = create(&mut memory, &mut guest, &mut budget).unwrap();
        guest.read(ARG, &mut output).unwrap();
        assert_eq!(u64::from_le_bytes(output), first.address);
        let mut payload = [0; 16];
        guest.read(first.address, &mut payload).unwrap();
        assert_eq!(payload, [0x5a; 16]);
        guest.refuse = true;
        assert!(
            memory
                .release_runtime_object(first, &mut guest, &mut budget)
                .is_err()
        );
        assert_eq!(memory.retained_handle_count(), 1);
        memory.check_runtime_object(first, &mut budget).unwrap();
        guest.refuse = false;
        let zero = 0u64.to_le_bytes();
        let bad_output = [GuestWrite {
            address: ARG + crate::tests::PAGE - 4,
            bytes: &zero,
        }];
        assert!(
            memory
                .release_runtime_object_with_outputs(first, &mut guest, &mut budget, &bad_output)
                .is_err()
        );
        assert_eq!(memory.retained_handle_count(), 1);
        assert_eq!(budget.usage().handles_live, 1);
        guest.read(first.address, &mut payload).unwrap();
        assert_eq!(payload, [0x5a; 16]);
        guest.read(ARG, &mut output).unwrap();
        assert_eq!(u64::from_le_bytes(output), first.address);
        memory
            .release_runtime_object_with_outputs(
                first,
                &mut guest,
                &mut budget,
                &[GuestWrite {
                    address: ARG,
                    bytes: &zero,
                }],
            )
            .unwrap();
        guest.read(ARG, &mut output).unwrap();
        assert_eq!(output, zero);
        assert!(guest.read(first.address, &mut payload).is_err());
        let second = create(&mut memory, &mut guest, &mut budget).unwrap();
        assert_eq!(first.address, second.address);
        assert!(memory.check_runtime_object(first, &mut budget).is_err());
        assert!(
            memory
                .release_runtime_object(first, &mut guest, &mut budget)
                .is_err()
        );
        memory.check_runtime_object(second, &mut budget).unwrap();
        memory
            .release_runtime_object(second, &mut guest, &mut budget)
            .unwrap();
        assert_eq!(memory.retained_handle_count(), 0);
        assert_eq!(budget.usage().handles_live, 0);
        assert_eq!(budget.usage().mapped_bytes, 0);
    }

    #[test]
    fn font_writing_sized_runtime_outputs_are_copied_and_publish_with_one_owned_mapping() {
        let limits = Limits::default();
        let mut memory = MemoryState::new(limits).unwrap();
        let mut budget = Budget::new(limits).unwrap();
        let mut guest = TestGuest::new();
        let mut outputs = RuntimeDataOutputs::new();
        assert!(outputs.push(ARG + 64, &[0x91; 257]).is_err());
        assert_eq!(outputs.len(), 0);
        let mut caller = [0x5a; 256];
        outputs.push(ARG + 64, &caller).unwrap();
        caller.fill(0);
        let object = memory
            .runtime_object(224, 0, &mut guest, &mut budget, |address, bytes| {
                bytes.fill(0x37);
                outputs.push(ARG, &address.to_le_bytes())?;
                outputs.push(ARG + 8, &0x1234u64.to_le_bytes())?;
                assert!(outputs.push(ARG + 16, &[0x99]).is_err());
                Ok(outputs)
            })
            .unwrap();
        assert_eq!(guest.get(ARG + 64, 256), [0x5a; 256]);
        assert_eq!(guest.get(ARG, 8), object.address.to_le_bytes());
        assert_eq!(guest.get(ARG + 8, 8), 0x1234u64.to_le_bytes());
        assert_eq!(guest.get(ARG + 16, 1), [0]);
        assert_eq!(guest.get(object.address, 224), [0x37; 224]);
        assert_eq!(budget.usage().handles_live, 1);
        memory
            .release_runtime_object(object, &mut guest, &mut budget)
            .unwrap();
        assert_eq!(budget.usage().handles_live, 0);
    }
}
