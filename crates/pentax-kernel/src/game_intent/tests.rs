// SPDX-License-Identifier: GPL-2.0-only
//! Original pinned no-event semantics, preserved layout and refusal checks.
use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{Kernel, Limits, MemoryEffect};

const BASE: u64 = ARG + 2 * PAGE;
const SIZE: usize = 16_704;

fn fixture() -> (TestGuest, Vec<u8>) {
    let mut guest = TestGuest::new();
    guest
        .apply(
            MemoryEffect::MapZeroed {
                address: BASE,
                length: 2 * PAGE,
                protection: 3,
            },
            &[],
        )
        .unwrap();
    let mut original = (0..SIZE + 8)
        .map(|i| (i % 251 + 1) as u8)
        .collect::<Vec<_>>();
    original[..8].copy_from_slice(&0_u64.to_le_bytes()); // Source ignores size.
    guest.put(BASE - ARG, &original);
    (guest, original)
}
fn receive(owner: &mut Kernel, guest: &mut TestGuest, base: u64) -> Result<u64, Failure> {
    owner.invoke_library(
        Service::GameIntentReceiveIntent,
        CallArgs([base, 0, 0, 0, 0, 0]),
        guest,
    )
}

#[test]
fn game_intent_no_event_preserves_size_reserved_padding_tail_and_has_no_lifecycle_gate() {
    let (mut guest, original) = fixture();
    let mut owner = kernel(Default::default());
    let empty = owner.handle_usage();
    assert_eq!(receive(&mut owner, &mut guest, 0), Ok(INVALID_ARGUMENT));
    assert_eq!(owner.usage().bytes_written, 0);
    assert_eq!(receive(&mut owner, &mut guest, BASE), Ok(INTENT_NOT_FOUND));
    let mut expected = original.clone();
    expected[8..12].fill(0xff);
    expected[12..45].fill(0);
    expected[308..16_700].fill(0);
    assert_eq!(guest.get(BASE, expected.len()), expected);
    assert_eq!(owner.usage().bytes_read, 0);
    assert_eq!(owner.usage().bytes_written, 16_429);
    for service in [
        Service::GameIntentInitialize,
        Service::GameIntentInitialize,
        Service::GameIntentTerminate,
        Service::GameIntentTerminate,
    ] {
        assert_eq!(
            owner.invoke_library(service, CallArgs([u64::MAX; 6]), &mut guest),
            Ok(0)
        );
    }
    assert_eq!(owner.usage().bytes_written, 16_429);
    assert_eq!(owner.handle_usage(), empty);
    assert_eq!(receive(&mut owner, &mut guest, BASE), Ok(INTENT_NOT_FOUND));
    assert_eq!(guest.get(BASE, expected.len()), expected);
}

#[test]
fn game_intent_property_only_compares_opaque_inputs_and_writes_one_nul_byte() {
    let mut guest = TestGuest::new();
    guest.put(0, &[0xa7; PAGE as usize]);
    let mut owner = kernel(Limits {
        max_bytes: 1,
        ..Default::default()
    });
    let output = ARG + PAGE - 1;
    for missing in 0..4 {
        let mut a = [u64::MAX, u64::MAX, output, u64::MAX, 0, 0];
        a[missing] = 0;
        assert_eq!(
            owner.invoke_library(
                Service::GameIntentGetPropertyValueString,
                CallArgs(a),
                &mut guest
            ),
            Ok(INVALID_ARGUMENT)
        );
        assert_eq!(owner.usage().bytes_written, 0);
    }
    assert_eq!(guest.get(output - 1, 2), [0xa7, 0xa7]);
    assert_eq!(
        owner.invoke_library(
            Service::GameIntentGetPropertyValueString,
            CallArgs([u64::MAX, u64::MAX, output, u64::MAX, 0, 0]),
            &mut guest
        ),
        Ok(VALUE_NOT_FOUND)
    );
    assert_eq!(guest.get(output - 1, 2), [0xa7, 0]);
    assert_eq!(
        (
            owner.usage().bytes_read,
            owner.usage().bytes_written,
            owner.usage().scratch_peak
        ),
        (0, 1, 0)
    );
    let mut other = kernel(Default::default());
    assert_eq!(
        other.invoke_library(
            Service::GameIntentGetPropertyValueString,
            CallArgs([1, 1, u64::MAX, 1, 0, 0]),
            &mut guest
        ),
        Err(Failure::Guest(GuestError::AddressOverflow))
    );
}

#[test]
fn game_intent_refused_outputs_and_budgets_keep_every_byte_and_allow_owned_retry() {
    let (mut guest, original) = fixture();
    for limits in [
        Limits {
            max_scratch_bytes: 16_428,
            max_io_bytes: 16_428,
            ..Default::default()
        },
        Limits {
            max_bytes: 16_428,
            ..Default::default()
        },
        Limits {
            max_work: 16_428,
            ..Default::default()
        },
    ] {
        let mut owner = kernel(limits);
        assert!(matches!(
            receive(&mut owner, &mut guest, BASE),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.get(BASE, original.len()), original);
        assert_eq!(owner.handle_usage().live, 0);
    }
    let mut owner = kernel(Default::default());
    guest.refuse = true;
    assert_eq!(
        receive(&mut owner, &mut guest, BASE),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(BASE, original.len()), original);
    assert_eq!(
        (owner.usage().attempts, owner.usage().bytes_written),
        (1, 16_429)
    );
    guest.refuse = false;
    assert_eq!(receive(&mut owner, &mut guest, BASE), Ok(INTENT_NOT_FOUND));
    assert_eq!(
        (owner.usage().attempts, owner.usage().bytes_written),
        (2, 32_858)
    );
    let mut other = kernel(Default::default());
    assert_eq!(
        receive(&mut other, &mut guest, u64::MAX - 16),
        Err(Failure::Guest(GuestError::AddressOverflow))
    );
    assert_eq!(other.usage().scratch_peak, 0);
}
