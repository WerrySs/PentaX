// SPDX-License-Identifier: GPL-2.0-only
//! Original live-capacity, output-transaction and independent-owner fixtures.
use super::*;

fn query(owner: &mut Kernel, guest: &mut impl GuestAccess, output: u64) -> Result<u64, Failure> {
    owner.invoke_library(
        Service::AvailableFlexibleMemorySize,
        CallArgs([output, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
        guest,
    )
}

#[test]
fn available_pool_tracks_live_named_and_unnamed_maps_and_successful_release() {
    let mut owner = kernel(Limits {
        flexible_memory_bytes: 3 * PAGE,
        ..Limits::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 3 * PAGE);
    // Internal errno backing and a real direct alias consume other resources.
    assert_ne!(
        call(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap(),
        0
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::AllocateMainDirect,
            &[PAGE, PAGE, 0, ARG]
        ),
        Ok(0)
    );
    let physical = guest.u64(0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapDirect,
            &[ARG + 8, PAGE, 3, 0, physical, PAGE]
        ),
        Ok(0)
    );
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 3 * PAGE);
    guest.put(64, b"original-available-pool\0");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapNamedFlexible,
            &[ARG + 16, PAGE, 3, 0, ARG + 64]
        ),
        Ok(0)
    );
    let named = guest.u64(16);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG + 24, 2 * PAGE, 3, 0]
        ),
        Ok(0)
    );
    let unnamed = guest.u64(24);
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG + 32, PAGE, 3, 0]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    guest.refuse = true;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Munmap,
            &[unnamed, 2 * PAGE]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::Munmap,
            &[unnamed, 2 * PAGE]
        ),
        Ok(0)
    );
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 2 * PAGE);
    assert_eq!(
        call(&mut owner, &mut guest, Service::Munmap, &[named, PAGE]),
        Ok(0)
    );
    assert_eq!(query(&mut owner, &mut guest, ARG + 128), Ok(0));
    assert_eq!(guest.u64(128), 3 * PAGE);
}

struct NoAccess;
impl GuestAccess for NoAccess {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("available query unexpectedly read guest memory");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("available query unexpectedly wrote guest memory");
    }
}

#[test]
fn available_pool_output_is_exact_atomic_and_null_precedes_the_mapping_census() {
    let mut owner = kernel(Limits {
        flexible_memory_bytes: 2 * PAGE,
        ..Limits::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MapFlexible,
            &[ARG, PAGE, 3, 0]
        ),
        Ok(0)
    );
    guest.regions[0].bytes.fill(0xa5);
    let resources = (owner.usage().handles_live, owner.usage().mapped_bytes);
    assert_eq!(query(&mut owner, &mut guest, ARG + 9), Ok(0));
    let mut expected = vec![0xa5; PAGE as usize];
    expected[9..17].copy_from_slice(&PAGE.to_le_bytes());
    assert_eq!(guest.regions[0].bytes, expected);
    for output in [ARG + PAGE, ARG + PAGE - 3, u64::MAX - 3] {
        let effects = guest.effects;
        let before = guest.regions[0].bytes.clone();
        assert_eq!(
            query(&mut owner, &mut guest, output),
            Err(Failure::Guest(if output == u64::MAX - 3 {
                GuestError::AddressOverflow
            } else {
                GuestError::WriteDenied
            }))
        );
        assert_eq!(guest.regions[0].bytes, before);
        assert_eq!(guest.effects, effects);
    }
    guest.regions[0].protection = 1;
    assert_eq!(
        query(&mut owner, &mut guest, ARG + 9),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.regions[0].bytes, expected);
    assert_eq!(
        (owner.usage().handles_live, owner.usage().mapped_bytes),
        resources
    );
    // Invocation and null validation fit; scanning the live map would exceed it.
    owner.budget.limits.max_work = owner.usage().work + 2;
    assert_eq!(query(&mut owner, &mut NoAccess, 0), Ok(0x8002_0016));
    let mut limited = kernel(Limits {
        max_bytes: 7,
        ..Limits::default()
    });
    assert_eq!(
        query(&mut limited, &mut NoAccess, ARG),
        Err(Failure::Limit("guest bytes"))
    );
    assert_eq!(limited.usage().bytes_written, 0);
}

#[test]
fn available_pool_is_owned_by_each_kernel_with_independent_guest_backing() {
    let mut first = kernel(Limits {
        flexible_memory_bytes: 2 * PAGE,
        ..Limits::default()
    });
    let mut second = kernel(Limits {
        flexible_memory_bytes: 3 * PAGE,
        ..Limits::default()
    });
    let mut first_guest = TestGuest::new();
    let mut second_guest = TestGuest::new();
    for (owner, guest) in [
        (&mut first, &mut first_guest),
        (&mut second, &mut second_guest),
    ] {
        assert_ne!(call(owner, guest, Service::ErrnoLocation, &[]).unwrap(), 0);
        assert_eq!(
            call(owner, guest, Service::MapFlexible, &[ARG, PAGE, 3, 0]),
            Ok(0)
        );
    }
    let mapped = first_guest.u64(0);
    assert_eq!(query(&mut first, &mut first_guest, ARG + 8), Ok(0));
    assert_eq!(query(&mut second, &mut second_guest, ARG + 8), Ok(0));
    assert_eq!((first_guest.u64(8), second_guest.u64(8)), (PAGE, 2 * PAGE));
    assert_eq!(
        call(
            &mut first,
            &mut first_guest,
            Service::Munmap,
            &[mapped, PAGE]
        ),
        Ok(0)
    );
    assert_eq!(query(&mut first, &mut first_guest, ARG + 8), Ok(0));
    assert_eq!(query(&mut second, &mut second_guest, ARG + 8), Ok(0));
    assert_eq!(
        (first_guest.u64(8), second_guest.u64(8)),
        (2 * PAGE, 2 * PAGE)
    );
}
