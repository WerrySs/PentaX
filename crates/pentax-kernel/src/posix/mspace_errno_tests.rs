// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX atomic mspace errno fixtures; no native guest entry.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{GuestError, Limits, MemoryEffect};

struct Guest {
    inner: TestGuest,
    applies: usize,
    last_effect: Option<MemoryEffect>,
    last_address: u64,
    last_bytes: [u8; 4],
}

impl Guest {
    fn new() -> Self {
        Self {
            inner: TestGuest::new(),
            applies: 0,
            last_effect: None,
            last_address: 0,
            last_bytes: [0; 4],
        }
    }

    fn value(&self, address: u64) -> u32 {
        u32::from_le_bytes(self.inner.get(address, 4).try_into().unwrap())
    }
}

impl GuestAccess for Guest {
    fn preflight_pthread(&self) -> Result<(), Failure> {
        self.inner.preflight_pthread()
    }

    fn preflight_posix_errno(&self, address: Option<u64>) -> Result<(), Failure> {
        self.inner.preflight_posix_errno(address)
    }

    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("errno publication must not read guest payload")
    }

    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        self.applies += 1;
        self.last_effect = Some(effect);
        assert_eq!(writes.len(), 1);
        self.last_address = writes[0].address;
        self.last_bytes.copy_from_slice(writes[0].bytes);
        self.inner.apply(effect, writes)
    }
}

#[test]
fn first_mspace_errno_is_initialized_in_one_atomic_mapping_and_refusal_can_retry() {
    let mut owner = kernel(Default::default());
    let mut guest = Guest::new();
    guest.inner.refuse = true;
    assert_eq!(
        owner.publish_mspace_errno(22, &mut guest),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    let selected = guest.last_address;
    assert_eq!(guest.applies, 1);
    assert_eq!(guest.last_bytes, 22_u32.to_le_bytes());
    assert_eq!(
        guest.last_effect,
        Some(MemoryEffect::MapZeroed {
            address: selected,
            length: PAGE,
            protection: 3
        })
    );
    assert_eq!(owner.threads.active().errno, None);
    assert_eq!(owner.memory.retained_handle_count(), 0);
    assert_eq!(owner.usage().mapped_bytes, 0);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.usage().attempts, 0);
    assert_eq!(owner.usage().bytes_written, 4);
    assert_eq!(
        guest.inner.read(selected, &mut [0; 4]),
        Err(GuestError::Unmapped)
    );

    guest.inner.refuse = false;
    owner.publish_mspace_errno(12, &mut guest).unwrap();
    assert_eq!(guest.applies, 2); // One apply per call, never a zero-cell apply.
    assert_eq!(guest.last_address, selected);
    assert_eq!(guest.last_bytes, 12_u32.to_le_bytes());
    assert_eq!(owner.threads.active().errno, Some(selected));
    assert_eq!(guest.value(selected), 12);
    assert_eq!(guest.inner.get(selected + 4, 4), [0; 4]);
    assert_eq!(owner.memory.retained_handle_count(), 1);
    assert_eq!(owner.usage().mapped_bytes, PAGE);
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(owner.usage().bytes_written, 8);
    assert_eq!(owner.usage().bytes_read, 0);
    assert_eq!(owner.usage().attempts, 0);
    assert_eq!(owner.usage().scratch_peak, 4);
}

#[test]
fn existing_mspace_errno_updates_only_four_bytes_and_late_refusal_preserves_value() {
    let mut owner = kernel(Default::default());
    let mut guest = Guest::new();
    owner.publish_mspace_errno(22, &mut guest).unwrap();
    let address = owner.threads.active().errno.unwrap();
    guest
        .inner
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address: address + 4,
                bytes: &[0x93; 4],
            }],
        )
        .unwrap();
    let before = owner.usage();
    guest.inner.refuse = true;
    assert_eq!(
        owner.publish_mspace_errno(12, &mut guest),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.last_effect, Some(MemoryEffect::None));
    assert_eq!(guest.value(address), 22);
    assert_eq!(owner.threads.active().errno, Some(address));
    assert_eq!(owner.usage().mapped_bytes, before.mapped_bytes);
    assert_eq!(owner.usage().handles_live, before.handles_live);
    assert_eq!(owner.usage().bytes_written, before.bytes_written + 4);
    guest.inner.refuse = false;
    owner.publish_mspace_errno(12, &mut guest).unwrap();
    assert_eq!(guest.value(address), 12);
    assert_eq!(guest.inner.get(address + 4, 4), [0x93; 4]);
    assert_eq!(guest.applies, 3);
    assert_eq!(owner.usage().bytes_written, before.bytes_written + 8);
    assert_eq!(owner.usage().attempts, 0);
}

#[test]
fn errno_permission_and_byte_budget_refusals_precede_any_new_output() {
    let mut owner = kernel(Limits {
        max_bytes: 7,
        ..Default::default()
    });
    let mut guest = Guest::new();
    owner.publish_mspace_errno(22, &mut guest).unwrap();
    let address = owner.threads.active().errno.unwrap();
    guest
        .inner
        .apply(
            MemoryEffect::Protect {
                address,
                length: PAGE,
                protection: 1,
            },
            &[],
        )
        .unwrap();
    assert_eq!(
        owner.publish_mspace_errno(12, &mut guest),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(owner.usage().bytes_written, 4);
    assert_eq!(guest.applies, 1);
    guest
        .inner
        .apply(
            MemoryEffect::Protect {
                address,
                length: PAGE,
                protection: 3,
            },
            &[],
        )
        .unwrap();
    assert_eq!(
        owner.publish_mspace_errno(12, &mut guest),
        Err(Failure::Limit("guest bytes"))
    );
    assert_eq!(guest.value(address), 22);
    assert_eq!(guest.applies, 1);
    assert_eq!(owner.threads.active().errno, Some(address));
    assert_eq!(owner.usage().mapped_bytes, PAGE);
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(owner.usage().attempts, 0);
}

#[test]
fn errno_numbers_outside_pinned_mspace_profile_refuse_without_effects_or_budget() {
    let mut owner = kernel(Default::default());
    let mut guest = Guest::new();
    let before = owner.usage();
    for number in [0, 1, 13, 1062, u32::MAX] {
        assert_eq!(
            owner.publish_mspace_errno(number, &mut guest),
            Err(Failure::Unsupported("mspace errno number"))
        );
    }
    assert_eq!(owner.usage(), before);
    assert_eq!(owner.threads.active().errno, None);
    assert_eq!(guest.applies, 0);
}

#[test]
fn mspace_errno_uses_the_selected_registered_threads_independent_actual_cells() {
    let mut owner = kernel(Default::default());
    let mut guest = Guest::new();
    owner.activate_thread_context(0x1010).unwrap();
    owner.publish_mspace_errno(22, &mut guest).unwrap();
    let first = owner.threads.active().errno.unwrap();
    owner
        .register_thread_context(0x2020, ARG, 700, 0x7f, &mut guest)
        .unwrap();
    owner.activate_thread_context(0x2020).unwrap();
    assert_eq!(owner.threads.active().errno, None);
    owner.publish_mspace_errno(12, &mut guest).unwrap();
    let second = owner.threads.active().errno.unwrap();
    assert_ne!(first, second);
    assert_eq!(guest.value(first), 22);
    assert_eq!(guest.value(second), 12);
    owner.activate_thread_context(0x1010).unwrap();
    assert_eq!(owner.threads.active().errno, Some(first));
    owner.publish_mspace_errno(12, &mut guest).unwrap();
    owner.activate_thread_context(0x2020).unwrap();
    assert_eq!(owner.threads.active().errno, Some(second));
    owner.publish_mspace_errno(22, &mut guest).unwrap();
    assert_eq!(guest.value(first), 12);
    assert_eq!(guest.value(second), 22);
    assert_eq!(owner.usage().mapped_bytes, 2 * PAGE);
    assert_eq!(owner.usage().handles_live, 2);
    assert_eq!(owner.usage().attempts, 0);
}
