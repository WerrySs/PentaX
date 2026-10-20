// SPDX-License-Identifier: GPL-2.0-only

//! Original atomic blob/caller-output requests through the public kernel API.
use super::*;
use crate::tests::{ARG, TestGuest, kernel};
use crate::{GuestError, Kernel};

struct Guest {
    inner: TestGuest,
    calls: usize,
    outputs: usize,
}

impl Guest {
    fn new() -> Self {
        Self {
            inner: TestGuest::new(),
            calls: 0,
            outputs: 0,
        }
    }
}

impl GuestAccess for Guest {
    fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), GuestError> {
        self.inner.read(address, bytes)
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        self.calls += 1;
        self.outputs = writes.len();
        self.inner.apply(effect, writes)
    }
}

fn empty(k: &Kernel) {
    assert_eq!(k.usage().handles_live, 0);
    assert_eq!(k.usage().mapped_bytes, 0);
    assert_eq!(k.memory.mappings.len(), 0);
    assert_eq!(k.memory.next_mapping, 1);
}

#[test]
fn four_outputs_publish_one_blob_and_three_copied_caller_spans_in_one_effect() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    g.inner.put(0, &[0x6b; 98]);
    let address = k
        .publish_runtime_data(17, 13, &mut g, |base, bytes| {
            bytes[..8].copy_from_slice(&(base + 8).to_le_bytes());
            bytes[8..].fill(0x42);
            let mut outputs = RuntimeDataOutputs::new();
            let mut copied = [0x57; 32];
            outputs.push(ARG + 1, &copied)?;
            copied.fill(0);
            outputs.push(ARG + 33, &base.to_le_bytes())?;
            outputs.push(ARG + 65, &(base + 8).to_le_bytes())?;
            Ok(outputs)
        })
        .unwrap();
    assert_eq!(g.calls, 1);
    assert_eq!(g.outputs, 4);
    assert_eq!(g.inner.get(ARG, 1), [0x6b]);
    assert_eq!(g.inner.get(ARG + 1, 32), [0x57; 32]);
    assert_eq!(g.inner.get(ARG + 33, 8), address.to_le_bytes());
    assert_eq!(g.inner.get(ARG + 65, 8), (address + 8).to_le_bytes());
    assert_eq!(g.inner.get(ARG + 73, 1), [0x6b]);
    assert_eq!(g.inner.get(address, 8), (address + 8).to_le_bytes());
    assert_eq!(
        g.inner.get(address + 8, 10),
        [0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0]
    );
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (1, PAGE));
    assert_eq!(k.usage().scratch_peak, 30);
    assert_eq!(k.memory.next_mapping, 2);
    assert_eq!(k.usage().attempts, 0);
}

#[test]
fn late_external_write_refusal_preserves_all_caller_bytes_and_retry_reuses_address_and_identity() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    g.inner.put(0, &[0xa7; 80]);
    let capacity = k.memory.mappings.capacity();
    let mut selected = 0;
    let result = k.publish_runtime_data(8, 0, &mut g, |base, bytes| {
        selected = base;
        bytes.copy_from_slice(&base.to_le_bytes());
        let mut outputs = RuntimeDataOutputs::new();
        outputs.push(ARG, &[1; 32])?;
        outputs.push(ARG + 32, &[2; 32])?;
        outputs.push(ARG + PAGE, &[3; 8])?; // Late unbacked caller span.
        Ok(outputs)
    });
    assert_eq!(result, Err(Failure::Guest(GuestError::WriteDenied)));
    empty(&k);
    assert_eq!(k.memory.mappings.capacity(), capacity);
    assert_eq!(g.calls, 1);
    assert_eq!(g.outputs, 4);
    assert_eq!(g.inner.get(ARG, 80), [0xa7; 80]);
    assert_eq!(g.read(selected, &mut [0; 1]), Err(GuestError::Unmapped));
    let work = k.usage().work;
    let address = k
        .publish_runtime_data(8, 0, &mut g, |base, bytes| {
            assert_eq!(base, selected);
            bytes.copy_from_slice(&base.to_le_bytes());
            let mut outputs = RuntimeDataOutputs::new();
            outputs.push(ARG, &[1; 32])?;
            outputs.push(ARG + 32, &[2; 32])?;
            outputs.push(ARG + 64, &[3; 8])?;
            Ok(outputs)
        })
        .unwrap();
    assert_eq!(address, selected);
    assert_eq!(g.calls, 2);
    assert_eq!(g.inner.get(ARG, 32), [1; 32]);
    assert_eq!(g.inner.get(ARG + 32, 32), [2; 32]);
    assert_eq!(g.inner.get(ARG + 64, 8), [3; 8]);
    assert_eq!(k.memory.mappings.len(), 1);
    assert_eq!(k.memory.mappings[0].identity, 1);
    assert!(k.usage().work > work);
}

#[test]
fn prepare_and_builder_failures_publish_nothing_and_builder_refusals_do_not_add_slots() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    let before = k.memory.mappings.capacity();
    assert_eq!(
        k.publish_runtime_data(8, 0, &mut g, |_, bytes| {
            bytes.fill(0xff);
            let mut outputs = RuntimeDataOutputs::new();
            outputs.push(ARG, &[1; 8])?;
            Err(Failure::Unsupported("original staged preparation failure"))
        }),
        Err(Failure::Unsupported("original staged preparation failure"))
    );
    empty(&k);
    assert_eq!(g.calls, 0);
    assert_eq!(g.inner.get(ARG, 8), [0; 8]);
    assert_eq!(k.memory.mappings.capacity(), before);
    let address = k
        .publish_runtime_data(1, 0, &mut g, |_, bytes| {
            bytes[0] = 42;
            let mut outputs = RuntimeDataOutputs::new();
            assert_eq!(
                outputs.push(ARG, &[0; 257]),
                Err(Failure::Limit("runtime data output bytes"))
            );
            assert_eq!(
                outputs.push(u64::MAX, &[1]),
                Err(Failure::Guest(GuestError::AddressOverflow))
            );
            outputs.push(ARG, &[1])?;
            outputs.push(ARG + 1, &[2])?;
            outputs.push(ARG + 2, &[3])?;
            assert_eq!(
                outputs.push(ARG + 3, &[4]),
                Err(Failure::Limit("runtime data outputs"))
            );
            Ok(outputs)
        })
        .unwrap();
    assert_eq!(g.calls, 1);
    assert_eq!(g.outputs, 4);
    assert_eq!(g.inner.get(ARG, 4), [1, 2, 3, 0]);
    assert_eq!(g.inner.get(address, 1), [42]);
}

#[test]
fn held_scratch_is_combined_with_the_actual_buffer_and_overflow_refuses_before_prepare() {
    let limits = Limits {
        max_scratch_bytes: 64,
        max_name_bytes: 1,
        max_io_bytes: 64,
        ..Default::default()
    };
    for (size, held, expected) in [
        (8, 57, Failure::Limit("scratch")),
        (1, usize::MAX, Failure::Limit("scratch")),
        (usize::MAX, 0, Failure::Limit("runtime data size")),
        (0, 0, Failure::Limit("empty runtime data")),
    ] {
        let mut k = kernel(limits);
        let mut g = Guest::new();
        let mut prepared = false;
        assert_eq!(
            k.publish_runtime_data(size, held, &mut g, |_, _| {
                prepared = true;
                Ok(RuntimeDataOutputs::new())
            }),
            Err(expected)
        );
        assert!(!prepared);
        assert_eq!(g.calls, 0);
        empty(&k);
    }
    let mut k = kernel(limits);
    let mut g = Guest::new();
    k.publish_runtime_data(8, 56, &mut g, |_, bytes| {
        bytes.fill(7);
        Ok(RuntimeDataOutputs::new())
    })
    .unwrap();
    assert_eq!(k.usage().scratch_peak, 64);
    assert_eq!(g.calls, 1);
}

#[test]
fn handles_mapping_identity_capacity_and_work_are_admitted_before_any_effect() {
    for limits in [
        Limits {
            max_handles: 0,
            ..Default::default()
        },
        Limits {
            max_mapped_bytes: 0,
            ..Default::default()
        },
        Limits {
            max_work: 0,
            ..Default::default()
        },
        Limits {
            max_bytes: 0,
            ..Default::default()
        },
    ] {
        let mut k = kernel(limits);
        let mut g = Guest::new();
        assert!(
            k.publish_runtime_data(8, 0, &mut g, |_, _| Ok(RuntimeDataOutputs::new()))
                .is_err()
        );
        assert_eq!(g.calls, 0);
        empty(&k);
    }
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    k.memory.next_mapping = u64::MAX;
    assert_eq!(
        k.publish_runtime_data(8, 0, &mut g, |_, _| Ok(RuntimeDataOutputs::new())),
        Err(Failure::Limit("mapping identities"))
    );
    assert_eq!(g.calls, 0);
    assert_eq!(k.memory.mappings.len(), 0);
    assert_eq!(k.memory.next_mapping, u64::MAX);
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (0, 0));
    // An exhausted internal metadata allocation cannot become a fallible push
    // after backing publication, even if a future owner incorrectly shrinks it.
    let mut k = kernel(Default::default());
    k.memory.mappings.shrink_to_fit();
    assert_eq!(
        k.publish_runtime_data(8, 0, &mut g, |_, _| Ok(RuntimeDataOutputs::new())),
        Err(Failure::Limit("mapping metadata"))
    );
    assert_eq!(g.calls, 0);
    empty(&k);
}

#[test]
fn overlapping_external_or_blob_outputs_refuse_before_guest_apply_without_losing_a_mapping_slot() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    for blob_overlap in [false, true] {
        assert!(matches!(
            k.publish_runtime_data(8, 0, &mut g, |base, _| {
                let mut outputs = RuntimeDataOutputs::new();
                outputs.push(if blob_overlap { base } else { ARG }, &[1; 8])?;
                if !blob_overlap {
                    outputs.push(ARG + 4, &[2; 8])?;
                }
                Ok(outputs)
            }),
            Err(Failure::Unsupported("overlapping guest outputs"))
        ));
        empty(&k);
        assert_eq!(g.calls, 0);
    }
    assert!(
        k.allocate_runtime_data(8, &mut g, |_, bytes| {
            bytes.fill(9);
            Ok(())
        })
        .is_ok()
    );
    assert_eq!(g.calls, 1);
    assert_eq!(g.outputs, 1);
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (1, PAGE));
}
