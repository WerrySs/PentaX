// SPDX-License-Identifier: GPL-2.0-only
// Original UDS calls and guest bytes; no input title or host pointer fixtures.

use super::*;
use crate::Limits;
use crate::tests::{ARG, PAGE, TestGuest};

struct Fixture {
    state: UdsState,
    memory: MemoryState,
    trophies: TrophyState,
    files: FileState,
    guest: TestGuest,
    budget: Budget,
}
impl Fixture {
    fn new(limits: Limits) -> Self {
        let mut budget = Budget::new(limits).unwrap();
        let files = FileState::new(limits, Vec::new(), &mut budget).unwrap();
        Self {
            state: UdsState::default(),
            memory: MemoryState::new(limits).unwrap(),
            trophies: TrophyState::default(),
            files,
            guest: TestGuest::new(),
            budget,
        }
    }
    fn call(&mut self, service: Service, values: &[u64]) -> Result<u64, Failure> {
        let mut a = [0; 6];
        a[..values.len()].copy_from_slice(values);
        self.state.invoke(
            service,
            CallArgs(a),
            UdsResources {
                memory: &mut self.memory,
                trophies: &mut self.trophies,
                files: &self.files,
                storage: None,
                language: 1,
            },
            &mut self.guest,
            &mut self.budget,
        )
    }
    fn pointer(&mut self, array: bool, offset: u64) -> u64 {
        let service = if array {
            Service::UdsCreateEventPropertyArray
        } else {
            Service::UdsCreateEventPropertyObject
        };
        assert_eq!(self.call(service, &[ARG + offset]), Ok(0));
        self.guest.u64(offset)
    }
    fn event(&mut self, properties: u64) -> (u64, u64) {
        assert_eq!(
            self.call(
                Service::UdsCreateEvent,
                &[ARG + 0x200, properties, ARG + 0x20, ARG + 0x28]
            ),
            Ok(0)
        );
        (self.guest.u64(0x20), self.guest.u64(0x28))
    }
    fn values(&self, address: u64) -> Vec<(&[u8], UdsInteger)> {
        let table = if let Some(object) = self
            .state
            .objects
            .iter()
            .find(|object| object.runtime.address == address)
        {
            &object.properties
        } else {
            &self
                .state
                .events
                .iter()
                .find(|event| event.runtime.address + 8 == address)
                .unwrap()
                .properties
        };
        table
            .iter()
            .map(|property| (property.key.as_slice(), property.value))
            .collect()
    }
    fn census(&self) -> usize {
        self.state.retained_handle_count() + self.memory.retained_handle_count()
    }
}

#[test]
fn uds_integer_variants_replace_exact_widths_and_noninteger_setters_erase_raw_byte_keys() {
    let mut f = Fixture::new(Limits::default());
    let object = f.pointer(false, 0);
    f.guest.put(0x100, b"\xffraw\0");
    f.guest.put(0x120, b"\0");
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[object, ARG + 0x100, 0x1_ffff_ffff]
        ),
        Ok(0)
    );
    assert_eq!(
        f.values(object),
        [(b"\xffraw".as_slice(), UdsInteger::I32(-1))]
    );
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetUInt32,
            &[object, ARG + 0x100, 0xfeed_1234_0000_0000]
        ),
        Ok(0)
    );
    assert_eq!(f.values(object)[0].1, UdsInteger::U32(0));
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetUInt64,
            &[object, ARG + 0x100, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(f.values(object)[0].1, UdsInteger::U64(u64::MAX));
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[object, ARG + 0x120, 5]
        ),
        Ok(0)
    );
    assert_eq!(f.values(object)[0], (b"".as_slice(), UdsInteger::I32(5)));
    for service in [
        Service::UdsEventPropertyObjectSetString,
        Service::UdsEventPropertyObjectSetInt64,
        Service::UdsEventPropertyObjectSetFloat32,
        Service::UdsEventPropertyObjectSetFloat64,
        Service::UdsEventPropertyObjectSetBool,
        Service::UdsEventPropertyObjectSetBinary,
    ] {
        assert_eq!(
            f.call(
                Service::UdsEventPropertyObjectSetUInt64,
                &[object, ARG + 0x100, 9]
            ),
            Ok(0)
        );
        // Ignored values need no guest span, UTF8 conversion or fake XMM value.
        assert_eq!(
            f.call(service, &[object, ARG + 0x100, u64::MAX, u64::MAX]),
            Ok(0)
        );
        assert_eq!(f.values(object), [(b"".as_slice(), UdsInteger::I32(5))]);
    }
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetString,
            &[object, ARG + 0x120, 0]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetBinary,
            &[object, ARG + 0x120, 0, 1]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(f.values(object)[0].1, UdsInteger::I32(5));
    assert_eq!(f.census(), f.budget.usage().handles_live);
}

#[test]
fn uds_events_deep_copy_properties_expose_live_owned_views_and_release_both_only_after_unmap() {
    let mut f = Fixture::new(Limits::default());
    let object = f.pointer(false, 0);
    f.guest.put(0x100, b"key\0");
    f.guest.put(0x200, b"OriginalEvent\0");
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[object, ARG + 0x100, 7]
        ),
        Ok(0)
    );
    f.guest.put(0x20, &[0xa5; 16]);
    f.guest.refuse = true;
    assert_eq!(
        f.call(
            Service::UdsCreateEvent,
            &[ARG + 0x200, object, ARG + 0x20, ARG + 0x28]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert!(f.state.events.is_empty());
    assert_eq!(f.memory.retained_handle_count(), 1);
    assert_eq!(f.guest.get(ARG + 0x20, 16), [0xa5; 16]);
    f.guest.refuse = false;
    assert_eq!(
        f.call(
            Service::UdsCreateEvent,
            &[ARG + 0x200, object, ARG + 0x20, ARG + 0x20]
        ),
        Err(Failure::Guest(GuestError::UnsupportedMapping))
    );
    assert!(f.state.events.is_empty());
    assert_eq!(f.memory.retained_handle_count(), 1);
    let (event, view) = f.event(object);
    assert_eq!(view, event + 8);
    assert_ne!(view, object);
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[object, ARG + 0x100, 8]
        ),
        Ok(0)
    );
    assert_eq!(f.values(view)[0].1, UdsInteger::I32(7));
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetUInt64,
            &[view, ARG + 0x100, 99]
        ),
        Ok(0)
    );
    assert_eq!(f.values(view)[0].1, UdsInteger::U64(99));
    assert_eq!(f.values(object)[0].1, UdsInteger::I32(8));
    assert!(matches!(
        f.call(Service::UdsDestroyEventPropertyObject, &[view]),
        Err(Failure::Unsupported(_))
    ));
    f.guest.refuse = true;
    assert_eq!(
        f.call(Service::UdsDestroyEvent, &[event]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(f.values(view)[0].1, UdsInteger::U64(99));
    assert_eq!(f.memory.retained_handle_count(), 2);
    f.guest.refuse = false;
    assert_eq!(f.call(Service::UdsDestroyEvent, &[event]), Ok(0));
    assert!(matches!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[view, ARG + 0x100, 1]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert!(f.guest.read(event, &mut [0; 1]).is_err());
    assert_eq!(
        f.call(Service::UdsDestroyEventPropertyObject, &[object]),
        Ok(0)
    );
    assert_eq!(
        (
            f.census(),
            f.budget.usage().handles_live,
            f.budget.usage().mapped_bytes
        ),
        (0, 0, 0)
    );
    assert_eq!(f.call(Service::UdsDestroyEvent, &[0]), Ok(0));
}

#[test]
fn uds_nested_pointer_outputs_are_detached_and_late_failure_preserves_key_owner_and_mapping() {
    let mut f = Fixture::new(Limits::default());
    let object = f.pointer(false, 0);
    f.guest.put(0x100, b"key\0");
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetUInt32,
            &[object, ARG + 0x100, 3]
        ),
        Ok(0)
    );
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetObject,
            &[object, ARG + 0x100, 0, ARG + PAGE]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(f.values(object)[0].1, UdsInteger::U32(3));
    assert_eq!(f.census(), 2);
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetObject,
            &[object, ARG + 0x100, 0, ARG + 8]
        ),
        Ok(0)
    );
    let child = f.guest.u64(8);
    assert!(f.values(object).is_empty());
    assert!(f.values(child).is_empty());
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetArray,
            &[object, ARG + 0x100, u64::MAX, ARG + 16]
        ),
        Ok(0)
    );
    assert_eq!(f.guest.u64(16), u64::MAX);
    assert!(matches!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[u64::MAX, ARG + 0x100, 3]
        ),
        Err(Failure::Unsupported(_))
    ));
    let array = f.pointer(true, 24);
    for service in [
        Service::UdsEventPropertyArraySetString,
        Service::UdsEventPropertyArraySetInt32,
        Service::UdsEventPropertyArraySetUInt32,
        Service::UdsEventPropertyArraySetInt64,
        Service::UdsEventPropertyArraySetUInt64,
        Service::UdsEventPropertyArraySetFloat32,
        Service::UdsEventPropertyArraySetFloat64,
        Service::UdsEventPropertyArraySetBool,
        Service::UdsEventPropertyArraySetBinary,
    ] {
        assert_eq!(f.call(service, &[array, u64::MAX, u64::MAX]), Ok(0));
    }
    assert_eq!(
        f.call(
            Service::UdsEventPropertyArraySetObject,
            &[array, 0, ARG + 32]
        ),
        Ok(0)
    );
    let detached = f.guest.u64(32);
    assert_eq!(
        f.call(Service::UdsDestroyEventPropertyArray, &[array]),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::UdsDestroyEventPropertyObject, &[object]),
        Ok(0)
    );
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[child, ARG + 0x100, 4]
        ),
        Ok(0)
    );
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[detached, ARG + 0x100, 5]
        ),
        Ok(0)
    );
    for pointer in [child, detached] {
        assert_eq!(
            f.call(Service::UdsDestroyEventPropertyObject, &[pointer]),
            Ok(0)
        );
    }
    assert_eq!(f.census(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
}

#[test]
fn uds_contexts_allow_duplicate_pairs_preserve_error_order_and_keep_source_noop_lifetimes() {
    let mut f = Fixture::new(Limits::default());
    f.guest.put(0, &[0xa5; 16]);
    assert_eq!(
        f.call(Service::UdsCreateContext, &[ARG + PAGE, 1, 2, 0]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(f.state.next_context, 1);
    let mut ids = Vec::new();
    for _ in 0..8 {
        assert_eq!(f.call(Service::UdsCreateContext, &[ARG, 1, 2, 0]), Ok(0));
        ids.push(u32::from_le_bytes(f.guest.get(ARG, 4).try_into().unwrap()));
    }
    assert_eq!(ids, (1..=8).collect::<Vec<_>>());
    assert_eq!(f.guest.get(ARG + 4, 4), [0xa5; 4]);
    assert_eq!(
        f.call(Service::UdsCreateContext, &[0, u64::MAX, 2, 1]),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        f.call(Service::UdsCreateContext, &[ARG, u64::MAX, 2, 0]),
        Ok(INVALID_USER)
    );
    assert_eq!(
        f.call(Service::UdsCreateContext, &[ARG, 1, 2, 0]),
        Ok(MAX_CONTEXTS)
    );
    assert_eq!(
        f.call(Service::UdsRegisterContext, &[99, 0, 1]),
        Ok(INVALID_CONTEXT)
    );
    assert_eq!(
        f.call(Service::UdsRegisterContext, &[1, u64::MAX, 0]),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::UdsRegisterContext, &[1, 0, 1]),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        f.call(Service::UdsRegisterContext, &[1, 0, 0]),
        Ok(ALREADY_REGISTERED)
    );
    assert_eq!(f.call(Service::UdsTerminate, &[]), Ok(0));
    assert!(f.state.contexts[0].registered);
    assert_eq!(f.call(Service::UdsDestroyContext, &[1]), Ok(0));
    assert_eq!(f.call(Service::UdsCreateContext, &[ARG, 1, 2, 0]), Ok(0));
    assert_eq!(f.guest.get(ARG, 4), 9i32.to_le_bytes());
    assert_eq!(f.call(Service::UdsCreateHandle, &[ARG]), Ok(0));
    assert_eq!(f.guest.get(ARG, 4), 1i32.to_le_bytes());
    assert_eq!(f.call(Service::UdsAbortHandle, &[u64::MAX]), Ok(0));
    assert_eq!(f.call(Service::UdsDestroyHandle, &[0]), Ok(0));
    assert_eq!(f.census(), 8);
    f.guest.put(0x100, &[0xee; 64]);
    assert_eq!(f.call(Service::UdsGetMemoryStat, &[ARG + 0x100]), Ok(0));
    assert_eq!(f.guest.get(ARG + 0x100, 24), [0; 24]);
    assert_eq!(f.guest.get(ARG + 0x118, 4), [0xee; 4]);
    assert_eq!(
        f.call(Service::UdsGetStorageStat, &[u64::MAX, ARG + 0x100]),
        Ok(0)
    );
    assert_eq!(f.guest.get(ARG + 0x100, 56), [0; 56]);
    assert_eq!(f.guest.get(ARG + 0x138, 8), [0xee; 8]);
    assert_eq!(f.call(Service::UdsInitialize, &[ARG + 0x100]), Ok(0));
    assert_eq!(f.call(Service::UdsInitialize, &[0]), Ok(INVALID_ARGUMENT));
    assert_eq!(f.budget.usage().handles_live, f.census());
}

#[test]
fn uds_global_quota_accounts_real_mapping_and_owner_and_refuses_scratch_or_work_without_partial_state()
 {
    let mut f = Fixture::new(Limits {
        max_handles: 3,
        ..Default::default()
    });
    f.memory
        .runtime_data(8, 0, &mut f.guest, &mut f.budget, |_, _| {
            Ok(RuntimeDataOutputs::new())
        })
        .unwrap();
    let object = f.pointer(false, 0);
    assert_eq!(f.census(), 3);
    f.guest.put(8, &[0xa5; 8]);
    assert!(matches!(
        f.call(Service::UdsCreateEventPropertyArray, &[ARG + 8]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(f.guest.u64(8), 0xa5a5_a5a5_a5a5_a5a5);
    assert_eq!(f.census(), 3);
    f.guest.put(0x100, b"key\0");
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetInt32,
            &[object, ARG + 0x100, 7]
        ),
        Ok(0)
    );
    f.budget.limits.max_scratch_bytes = 0;
    assert!(matches!(
        f.call(
            Service::UdsEventPropertyObjectSetUInt32,
            &[object, ARG + 0x100, 8]
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(f.values(object)[0].1, UdsInteger::I32(7));
    f.budget.limits.max_scratch_bytes = 64 * 1024 * 1024;
    let maximum = f.budget.limits.max_work;
    f.budget.limits.max_work = f.budget.usage().work;
    assert_eq!(
        f.call(Service::UdsDestroyEventPropertyObject, &[object]),
        Err(Failure::Limit("work"))
    );
    assert_eq!(f.census(), 3);
    f.budget.limits.max_work = maximum;
    assert_eq!(
        f.call(Service::UdsDestroyEventPropertyObject, &[object]),
        Ok(0)
    );
    assert_eq!(f.census(), 1);
    let mut one = Fixture::new(Limits {
        max_handles: 1,
        ..Default::default()
    });
    assert!(matches!(
        one.call(Service::UdsCreateEventPropertyObject, &[ARG]),
        Err(Failure::Limit(_))
    ));
    assert_eq!((one.census(), one.budget.usage().mapped_bytes), (0, 0));
}

#[test]
fn uds_literal_event_diagnostics_preserve_truncation_canaries_and_atomic_alias_or_late_refusal() {
    let mut f = Fixture::new(Limits::default());
    assert_eq!(
        f.call(Service::UdsEventEstimateSize, &[u64::MAX, ARG]),
        Ok(0)
    );
    assert_eq!(f.guest.u64(0), 3);
    for (length, expected) in [
        (0, b"".as_slice()),
        (1, b"\0".as_slice()),
        (2, b"{\0".as_slice()),
        (3, b"{}\0".as_slice()),
        (99, b"{}\0".as_slice()),
    ] {
        f.guest.put(0x100, &[0xa5; 16]);
        f.guest.put(0, &[0xa5; 8]);
        assert_eq!(
            f.call(
                Service::UdsEventToString,
                &[u64::MAX, ARG + 0x100, length, ARG]
            ),
            Ok(0)
        );
        assert_eq!(f.guest.u64(0), 3);
        assert_eq!(f.guest.get(ARG + 0x100, expected.len()), expected);
        assert_eq!(f.guest.get(ARG + 0x100 + expected.len() as u64, 1), [0xa5]);
    }
    f.guest.put(0, &[0xa5; 8]);
    assert_eq!(
        f.call(Service::UdsEventToString, &[1, ARG, 3, ARG]),
        Err(Failure::Guest(GuestError::UnsupportedMapping))
    );
    assert_eq!(f.guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(
        f.call(Service::UdsEventToString, &[1, ARG + PAGE, 3, ARG]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(f.guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(
        f.call(
            Service::UdsEventToString,
            &[0, u64::MAX, u64::MAX, u64::MAX]
        ),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(f.state.retained_handle_count(), 0);
}

#[test]
fn uds_post_checks_null_context_registration_then_owner_and_does_not_consume_empty_events() {
    let mut f = Fixture::new(Limits::default());
    assert_eq!(
        f.call(Service::UdsPostEvent, &[99, 0, 0, 0]),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        f.call(Service::UdsPostEvent, &[99, 0, u64::MAX, 0]),
        Ok(INVALID_CONTEXT)
    );
    assert_eq!(f.call(Service::UdsCreateContext, &[ARG, 1, 2, 0]), Ok(0));
    assert_eq!(
        f.call(Service::UdsPostEvent, &[1, 0, u64::MAX, 0]),
        Ok(NOT_REGISTERED)
    );
    assert_eq!(f.call(Service::UdsRegisterContext, &[1, 0, 0]), Ok(0));
    assert!(matches!(
        f.call(Service::UdsPostEvent, &[1, 0, u64::MAX, 0]),
        Err(Failure::Unsupported(_))
    ));
    f.guest.put(0x200, b"OriginalEmpty\0");
    let (event, view) = f.event(0);
    for _ in 0..2 {
        assert_eq!(
            f.call(Service::UdsPostEvent, &[1, u64::MAX, event, u64::MAX]),
            Ok(0)
        );
    }
    assert_eq!(f.state.events.len(), 1);
    assert!(f.values(view).is_empty());
    assert_eq!(f.call(Service::UdsDestroyEvent, &[event]), Ok(0));
    f.guest.put(0x200, b"_UnlockTrophy\0");
    f.guest.put(0x100, b"_trophy_id\0");
    let (event, view) = f.event(0);
    assert_eq!(
        f.call(Service::UdsPostEvent, &[1, 0, event, 0]),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(
        f.call(
            Service::UdsEventPropertyObjectSetUInt32,
            &[view, ARG + 0x100, 2]
        ),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::UdsPostEvent, &[1, 0, event, 0]),
        Ok(INVALID_ARGUMENT)
    );
    assert_eq!(f.state.events.len(), 1);
    assert_eq!(f.values(view)[0].1, UdsInteger::U32(2));
    assert_eq!(f.census(), f.budget.usage().handles_live);
}
