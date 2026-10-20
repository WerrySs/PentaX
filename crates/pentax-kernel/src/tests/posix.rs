// SPDX-License-Identifier: GPL-2.0-only

//! Original bounded errno state over owned backing; no native entry here.

use super::*;

fn invoke(
    kernel: &mut Kernel,
    guest: &mut impl GuestAccess,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    kernel.invoke_library(service, CallArgs(args), guest)
}
fn error_value(guest: &TestGuest, address: u64) -> u32 {
    u32::from_le_bytes(guest.get(address, 4).try_into().unwrap())
}

#[test]
fn posix_errno_has_stable_independent_backing_and_success_does_not_clear_it() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let address = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    assert_eq!(error_value(&guest, address), 0);
    assert_eq!(owner.usage().mapped_bytes, PAGE);
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]),
        Ok(address)
    );
    assert_eq!(owner.usage().mapped_bytes, PAGE);
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address,
                bytes: &37_u32.to_le_bytes(),
            }],
        )
        .unwrap();
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixClose, &[0]),
        Ok(0)
    );
    assert_eq!(error_value(&guest, address), 37);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixClose, &[u64::MAX]),
        Ok(0xffff_ffff)
    );
    assert_eq!(error_value(&guest, address), 9);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixRead, &[0, 0, 8]),
        Ok(u64::MAX)
    );
    assert_eq!(error_value(&guest, address), 14);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Close, &[0]),
        Ok(0x8002_0001)
    );
    assert_eq!(error_value(&guest, address), 14); // Kernel ABI does not set POSIX errno.
    assert_eq!(owner.usage().attempts, 6);
    assert!(matches!(
        invoke(
            &mut owner,
            &mut guest,
            Service::Mprotect,
            &[address, PAGE, 1]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        invoke(&mut owner, &mut guest, Service::Munmap, &[address, PAGE]),
        Err(Failure::Unsupported(_))
    ));
    let mut other = kernel(Default::default());
    let mut other_guest = TestGuest::new();
    let other_address = invoke(&mut other, &mut other_guest, Service::ErrnoLocation, &[]).unwrap();
    assert_eq!(error_value(&other_guest, other_address), 0);
    assert_eq!(error_value(&guest, address), 14);
}

#[test]
fn posix_widths_wrappers_and_unavailable_paths_keep_the_source_distinctions() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let address = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    for (service, args, result, errno) in [
        (Service::PosixOpen, [0; 6], 0xffff_ffff_u64, 22),
        (
            Service::PosixClose,
            [u64::MAX, 0, 0, 0, 0, 0],
            0xffff_ffff,
            9,
        ),
        (Service::PosixRead, [1, ARG, 0, 0, 0, 0], u64::MAX, 9),
        (Service::PosixPread, [3, 0, 8, 0, 0, 0], u64::MAX, 14),
        (Service::PosixLseek, [0; 6], u64::MAX, 9),
        (Service::PosixFstat, [0; 6], u64::MAX, 1),
        (Service::PosixStat, [0; 6], 0xffff_ffff, 22),
        (
            Service::PosixClockGettime,
            [3, ARG, 0, 0, 0, 0],
            0xffff_ffff,
            22,
        ),
        (Service::PosixClockGetres, [0; 6], 0xffff_ffff, 14),
        (Service::PosixNanosleep, [0; 6], 0xffff_ffff, 14),
    ] {
        assert_eq!(
            owner.invoke_library(service, CallArgs(args), &mut guest),
            Ok(result),
            "{service:?}"
        );
        assert_eq!(error_value(&guest, address), errno);
    }
    assert!(matches!(
        invoke(&mut owner, &mut guest, Service::PosixRead, &[0, ARG, 8]),
        Err(Failure::Unsupported("POSIX standard input"))
    ));
    assert!(matches!(
        invoke(
            &mut owner,
            &mut guest,
            Service::PosixClockGettime,
            &[14, ARG]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(error_value(&guest, address), 14);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixRead, &[0, ARG, 0]),
        Ok(0)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PageSize, &[]),
        Ok(0x4000)
    );
    assert_eq!(error_value(&guest, address), 14);
}

#[test]
fn posix_unbound_bad_backing_and_budgets_never_publish_fake_errno_results() {
    struct Unbound;
    impl GuestAccess for Unbound {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("unbound POSIX input read")
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("unbound POSIX output")
        }
    }
    let mut owner = kernel(Default::default());
    assert!(matches!(
        invoke(&mut owner, &mut Unbound, Service::PosixOpen, &[ARG, 0, 0]),
        Err(Failure::Unsupported(_))
    ));
    assert!(matches!(
        invoke(&mut owner, &mut Unbound, Service::ErrnoLocation, &[]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(owner.usage().bytes_read + owner.usage().bytes_written, 0);
    assert_eq!(owner.usage().mapped_bytes, 0);

    let mut guest = TestGuest::new();
    let address = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address,
                bytes: &37_u32.to_le_bytes(),
            }],
        )
        .unwrap();
    guest.refuse = true;
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixClose, &[u64::MAX]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(error_value(&guest, address), 37);
    guest.refuse = false;
    let index = guest
        .regions
        .iter()
        .position(|region| region.start == address)
        .unwrap();
    guest.regions[index].protection = 1;
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(error_value(&guest, address), 37);
    guest.regions[index].protection = 7;
    assert!(matches!(
        invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]),
        Err(Failure::Unsupported(_))
    ));
    let mut exhausted = kernel(Limits {
        max_mapped_bytes: 0,
        ..Default::default()
    });
    assert!(matches!(
        invoke(&mut exhausted, &mut guest, Service::ErrnoLocation, &[]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(exhausted.usage().handles_live, 0);
    assert_eq!(exhausted.usage().mapped_bytes, 0);
}
