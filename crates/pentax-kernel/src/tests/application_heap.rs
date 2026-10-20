// SPDX-License-Identifier: GPL-2.0-only
//! Original guest tables; no callback is executed by these state tests.
use super::*;

fn table(guest: &mut TestGuest, malloc: u64, free: u64, memalign: u64) {
    let mut bytes = [0xa5; 80];
    for (slot, pointer) in [(0, malloc), (1, free), (6, memalign)] {
        bytes[slot * 8..slot * 8 + 8].copy_from_slice(&pointer.to_le_bytes());
    }
    guest.put(0, &bytes);
}
fn register(state: &mut Kernel, guest: &mut TestGuest, address: u64) -> Result<u64, Failure> {
    state.invoke_library(
        Service::SetApplicationHeapApi,
        CallArgs([address, 0, 0, 0, 0, 0]),
        guest,
    )
}
#[test]
fn application_heap_registration_copies_exact_slots_and_allows_replacement_and_disable() {
    let mut state = kernel(Limits::default());
    let mut guest = TestGuest::new();
    assert!(state.application_heap_callbacks().is_none());
    table(&mut guest, 0x1000, 0x2000, 0x3000);
    register(&mut state, &mut guest, ARG).unwrap();
    let first = state.application_heap_callbacks().unwrap();
    assert_eq!(
        (first.malloc, first.free, first.posix_memalign),
        (0x1000, 0x2000, Some(0x3000))
    );
    table(&mut guest, 0x4000, 0x5000, 0);
    assert!(state.application_heap_callbacks() == Some(first));
    register(&mut state, &mut guest, ARG).unwrap();
    let second = state.application_heap_callbacks().unwrap();
    assert_eq!(
        (second.malloc, second.free, second.posix_memalign),
        (0x4000, 0x5000, None)
    );
    register(&mut state, &mut guest, 0).unwrap();
    assert!(state.application_heap_callbacks().is_none());
    for (malloc, free) in [(0, 0x2000), (0x1000, 0)] {
        table(&mut guest, 0x1000, 0x2000, 0x3000);
        register(&mut state, &mut guest, ARG).unwrap();
        table(&mut guest, malloc, free, 0x3000);
        register(&mut state, &mut guest, ARG).unwrap();
        assert!(state.application_heap_callbacks().is_none());
    }
    assert!(
        kernel(Limits::default())
            .application_heap_callbacks()
            .is_none()
    );
    assert_eq!(state.usage().bytes_written, 0);
}
#[test]
fn application_heap_bad_tables_and_budget_failures_leave_previous_callbacks_intact() {
    let mut state = kernel(Limits::default());
    let mut guest = TestGuest::new();
    table(&mut guest, 0x1000, 0x2000, 0x3000);
    register(&mut state, &mut guest, ARG).unwrap();
    let original = state.application_heap_callbacks();
    for address in [ARG + PAGE - 72, u64::MAX - 79] {
        assert!(matches!(
            register(&mut state, &mut guest, address),
            Err(Failure::Guest(_))
        ));
        assert!(state.application_heap_callbacks() == original);
    }
    guest.regions[0].protection = 0;
    assert_eq!(
        register(&mut state, &mut guest, ARG),
        Err(Failure::Guest(GuestError::ReadDenied))
    );
    assert!(state.application_heap_callbacks() == original);
    guest.regions[0].protection = 3;
    let mut limited = kernel(Limits {
        max_bytes: 80,
        ..Default::default()
    });
    register(&mut limited, &mut guest, ARG).unwrap();
    table(&mut guest, 0x4000, 0x5000, 0x6000);
    assert_eq!(
        register(&mut limited, &mut guest, ARG),
        Err(Failure::Limit("guest bytes"))
    );
    assert!(limited.application_heap_callbacks() == original);
    register(&mut limited, &mut guest, 0).unwrap();
    assert!(limited.application_heap_callbacks().is_none());
}
