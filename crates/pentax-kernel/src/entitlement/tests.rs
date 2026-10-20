// SPDX-License-Identifier: GPL-2.0-only
// Original local catalogue and alias fixtures; no donor title labels or PSN data.

use super::*;
use crate::MemoryEffect;
use crate::tests::{ARG, PAGE, TestGuest, kernel};

fn record(label: &[u8], kind: u32, status: u32) -> LocalAddcont {
    let mut raw = [0; 17];
    raw[..label.len()].copy_from_slice(label);
    LocalAddcont::new(raw, kind, status)
}
fn configure(owner: &mut Kernel, records: Vec<LocalAddcont>) {
    owner
        .configure_local_entitlements(LocalEntitlements::new(records).unwrap())
        .unwrap();
}
fn invoke(
    owner: &mut Kernel,
    guest: &mut impl GuestAccess,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn encoded(label: &[u8], kind: u32, status: u32) -> [u8; 28] {
    let mut bytes = [0; 28];
    bytes[..label.len()].copy_from_slice(label);
    bytes[20..24].copy_from_slice(&kind.to_le_bytes());
    bytes[24..28].copy_from_slice(&status.to_le_bytes());
    bytes
}

#[test]
fn entitlement_default_is_stateless_local_full_empty_and_opaque_inputs_are_never_read() {
    struct NoReads {
        inner: TestGuest,
    }
    impl GuestAccess for NoReads {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("opaque entitlement input read")
        }
        fn apply(
            &mut self,
            effect: MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.inner.apply(effect, writes)
        }
    }
    let mut owner = kernel(Default::default());
    let mut guest = NoReads {
        inner: TestGuest::new(),
    };
    guest.inner.put(0, &[0xa5; 128]);
    // Every query works before Initialize; SKU is explicitly local policy3.
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetSkuFlag,
            &[ARG]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.inner.get(ARG, 8),
        [3, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessInitialize,
            &[u64::MAX, ARG + 8]
        ),
        Ok(0)
    );
    assert_eq!(guest.inner.get(ARG + 8, 32), [0; 32]);
    assert_eq!(guest.inner.get(ARG + 40, 4), [0xa5; 4]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessInitialize,
            &[0, u64::MAX]
        ),
        Ok(PARAMETER)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessInitialize,
            &[u64::MAX, 0]
        ),
        Ok(PARAMETER)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetSkuFlag,
            &[0]
        ),
        Ok(PARAMETER)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[u64::MAX, ARG + 48, 0xdead_beef_0000_0002, ARG + 112]
        ),
        Ok(0)
    );
    assert_eq!(guest.inner.get(ARG + 48, 56), [0; 56]);
    assert_eq!(guest.inner.get(ARG + 104, 4), [0xa5; 4]);
    assert_eq!(guest.inner.get(ARG + 112, 4), [0; 4]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[u64::MAX, u64::MAX, ARG + 48]
        ),
        Ok(NO_ENTITLEMENT)
    );
    guest.inner.put(80, &[0xa5; 16]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetEntitlementKey,
            &[u64::MAX, u64::MAX, ARG + 80]
        ),
        Ok(NO_ENTITLEMENT)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetEntitlementKey,
            &[0, u64::MAX, u64::MAX]
        ),
        Ok(NO_ENTITLEMENT)
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetEntitlementKey,
            &[0, 0, u64::MAX]
        ),
        Ok(PARAMETER)
    );
    assert_eq!(guest.inner.get(ARG + 80, 16), [0xa5; 16]);
    assert_eq!(
        (owner.usage().bytes_read, owner.usage().handles_live),
        (0, 0)
    );
}

#[test]
fn entitlement_configured_records_preserve_order_raw_labels_fields_early_nul_and_low32_capacity() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    configure(
        &mut owner,
        vec![
            record(b"Zebra", 0x1122_3344, 0xaabb_ccdd),
            record(b"\xff0123456789abcdef", 0, u32::MAX),
            record(b"A", 7, 9),
        ],
    );
    guest.put(0x100, &[0xa5; 144]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[u64::MAX, ARG + 0x100, 0xface_0000_0000_0005, ARG + 0x80]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x80, 4), 3u32.to_le_bytes());
    assert_eq!(
        guest.get(ARG + 0x100, 28),
        encoded(b"Zebra", 0x1122_3344, 0xaabb_ccdd)
    );
    assert_eq!(
        guest.get(ARG + 0x11c, 28),
        encoded(b"\xff0123456789abcdef", 0, u32::MAX)
    );
    assert_eq!(guest.get(ARG + 0x138, 28), encoded(b"A", 7, 9));
    assert_eq!(guest.get(ARG + 0x154, 56), [0; 56]);
    assert_eq!(guest.get(ARG + 0x18c, 4), [0xa5; 4]);
    guest.put(0x200, b"\xff0123456789abcdef");
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[0xdead_beef, ARG + 0x200, ARG + 0x300]
        ),
        Ok(0)
    );
    assert_eq!(
        guest.get(ARG + 0x300, 28),
        encoded(b"\xff0123456789abcdef", 0, u32::MAX)
    );
    // Only the first two bytes exist: early NUL must not read label padding.
    guest.put(PAGE - 2, b"A\0");
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[1, ARG + PAGE - 2, ARG + 0x300]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x300, 28), encoded(b"A", 7, 9));
}

#[test]
fn entitlement_configuration_rejects_equivalent_duplicates_capacity_work_and_late_replacement_atomically()
 {
    let mut owner = kernel(Default::default());
    configure(&mut owner, vec![record(b"Keep", 1, 2)]);
    let mut first = [0; 17];
    first[..4].copy_from_slice(b"Same");
    first[5] = 0xaa;
    let mut second = first;
    second[5] = 0xbb;
    let duplicate = LocalEntitlements::new(vec![
        LocalAddcont::new(first, 3, 4),
        LocalAddcont::new(second, 5, 6),
    ])
    .unwrap();
    assert_eq!(
        owner.configure_local_entitlements(duplicate),
        Err(Failure::Unsupported("duplicate local entitlement label"))
    );
    assert_eq!(owner.entitlements.len(), 1);
    assert_eq!(owner.entitlements.records[0].label[..4], *b"Keep");
    let oversized = Vec::with_capacity(MAX_RECORDS + 1);
    assert!(matches!(
        LocalEntitlements::new(oversized),
        Err(Failure::Limit(_))
    ));
    let next = LocalEntitlements::new(vec![record(b"New", 8, 9)]).unwrap();
    let old_limit = owner.budget.limits.max_scratch_bytes;
    owner.budget.limits.max_scratch_bytes =
        owner.entitlements.metadata_bytes() + next.metadata_bytes() - 1;
    assert!(matches!(
        owner.configure_local_entitlements(next),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_scratch_bytes = old_limit;
    let old_work = owner.budget.limits.max_work;
    owner.budget.limits.max_work = owner.usage().work;
    assert!(matches!(
        owner.configure_local_entitlements(LocalEntitlements::default()),
        Err(Failure::Limit("work"))
    ));
    owner.budget.limits.max_work = old_work;
    let mut guest = TestGuest::new();
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, 0, 0, ARG]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG, 4), 1u32.to_le_bytes());
    assert!(matches!(
        owner.configure_local_entitlements(LocalEntitlements::default()),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.entitlements.len(), 1);
}

#[test]
fn entitlement_list_hit_alias_reread_and_final_copy_are_staged_in_source_order() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    configure(
        &mut owner,
        vec![record(b"Alpha", 1, 2), record(b"Beta", 3, 4)],
    );
    guest.put(0x100, &[0xa5; 64]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG + 0x100, 2, ARG + 0x104]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x100, 56), [0; 56]); // Clear overwrote total before reread: no copies.
    assert_eq!(guest.get(ARG + 0x138, 4), [0xa5; 4]);
    guest.put(0x100, &[0xa5; 64]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG + 0x101, 2, ARG + 0x100]
        ),
        Ok(0)
    );
    let mut expected = vec![2];
    expected.extend_from_slice(&encoded(b"Alpha", 1, 2));
    expected.extend_from_slice(&encoded(b"Beta", 3, 4));
    assert_eq!(guest.get(ARG + 0x100, 57), expected);
    assert_eq!(guest.get(ARG + 0x100, 4), [2, b'A', b'l', b'p']); // Final record copy overwrites hit's high bytes.
    guest.put(0x100, &[0xa5; 64]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG + 0x100, 2, ARG + 0x136]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x100, 58), [0; 58]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, u64::MAX, 0xffff_ffff_0000_0000, ARG]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG, 4), 2u32.to_le_bytes());
}

#[test]
fn entitlement_single_zero_overlay_changes_later_label_comparison_without_overreading_early_nul() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    configure(&mut owner, vec![record(b"AB", 1, 2), record(b"", 7, 9)]);
    guest.put(0x100, &[0xa5; 40]);
    guest.put(0x100, b"ABnotmatch\0");
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[0, ARG + 0x100, ARG + 0x102]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x102, 28), encoded(b"AB", 1, 2));
    guest.put(0x100, &[0xa5; 32]);
    guest.put(0x100, b"AB\0");
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[u64::MAX, ARG + 0x100, ARG + 0x100]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 0x100, 28), encoded(b"", 7, 9));
    assert_eq!(guest.get(ARG + 0x11c, 4), [0xa5; 4]);
}

/// Two original virtual windows sharing exactly the same physical byte array.
struct Aliases {
    bytes: [u8; 128],
    writes: usize,
}
impl Aliases {
    fn coordinate(address: u64, length: usize) -> Result<usize, GuestError> {
        for base in [ARG, ARG + PAGE] {
            if let Some(offset) = address.checked_sub(base)
                && offset
                    .checked_add(length as u64)
                    .is_some_and(|end| end <= 128)
            {
                return Ok(offset as usize);
            }
        }
        Err(GuestError::Unmapped)
    }
}
impl GuestAccess for Aliases {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        let at = Self::coordinate(address, output.len())?;
        output.copy_from_slice(&self.bytes[at..at + output.len()]);
        Ok(())
    }
    fn read_after_u64_write_byte(
        &self,
        address: u64,
        destination: u64,
        value: u64,
    ) -> Result<u8, GuestError> {
        let at = Self::coordinate(address, 1)?;
        let start = Self::coordinate(destination, 8)?;
        for (index, value) in value.to_le_bytes().into_iter().enumerate() {
            if start + index == at {
                if destination + index as u64 != address {
                    return Err(GuestError::UnsupportedMapping);
                }
                return Ok(value);
            }
        }
        Ok(self.bytes[at])
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        if effect != MemoryEffect::None {
            return Err(GuestError::UnsupportedMapping);
        }
        for (index, write) in writes.iter().enumerate() {
            let at = Self::coordinate(write.address, write.bytes.len())?;
            for prior in &writes[..index] {
                let other = Self::coordinate(prior.address, prior.bytes.len())?;
                if at < other + prior.bytes.len() && other < at + write.bytes.len() {
                    return Err(GuestError::UnsupportedMapping);
                }
            }
        }
        for write in writes {
            let at = Self::coordinate(write.address, write.bytes.len())?;
            self.bytes[at..at + write.bytes.len()].copy_from_slice(write.bytes);
        }
        self.writes += 1;
        Ok(())
    }
}

#[test]
fn entitlement_caps_late_faults_and_distinct_physical_aliases_publish_no_partial_outputs() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    configure(&mut owner, vec![record(b"AB", 1, 2)]);
    guest.put(0x100, &[0xa5; 64]);
    guest.put(0x80, &[0xa5; 4]);
    assert!(matches!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG + 0x100, u64::MAX, ARG + 0x80]
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG + 0x100, 64), [0xa5; 64]);
    assert_eq!(guest.get(ARG + 0x80, 4), [0xa5; 4]);
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG + 0x100, 2, ARG + PAGE]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + 0x100, 64), [0xa5; 64]);
    let limit = owner.budget.limits.max_scratch_bytes;
    owner.budget.limits.max_scratch_bytes = owner.entitlements.metadata_bytes() + 55;
    assert!(matches!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG + 0x100, 2, ARG + 0x80]
        ),
        Err(Failure::Limit(_))
    ));
    owner.budget.limits.max_scratch_bytes = limit;
    guest.put(0x200, b"AB\0");
    guest.put(PAGE - 16, &[0xa5; 16]);
    assert!(
        invoke(
            &mut owner,
            &mut guest,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[0, ARG + 0x200, ARG + PAGE - 16]
        )
        .is_err()
    );
    assert_eq!(guest.get(ARG + PAGE - 16, 16), [0xa5; 16]);
    let mut alias = Aliases {
        bytes: [0xa5; 128],
        writes: 0,
    };
    alias.bytes[..3].copy_from_slice(b"AB\0");
    let before = alias.bytes;
    assert_eq!(
        invoke(
            &mut owner,
            &mut alias,
            Service::EntitlementAccessGetAddcontEntitlementInfo,
            &[0, ARG + PAGE, ARG]
        ),
        Err(Failure::Guest(GuestError::UnsupportedMapping))
    );
    assert_eq!(alias.bytes, before);
    assert_eq!(alias.writes, 0);
    assert_eq!(
        invoke(
            &mut owner,
            &mut alias,
            Service::EntitlementAccessGetAddcontEntitlementInfoList,
            &[0, ARG, 1, ARG + PAGE]
        ),
        Err(Failure::Guest(GuestError::UnsupportedMapping))
    );
    assert_eq!(alias.bytes, before);
    assert_eq!(alias.writes, 0);
    assert_eq!(owner.entitlements.len(), 1);
    assert_eq!(owner.usage().handles_live, 0);
}
