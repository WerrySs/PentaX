// SPDX-License-Identifier: GPL-2.0-only

//! Source return domains and backing/policy separation, without native entry.

use super::*;

fn invoke(
    kernel: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    kernel.invoke_library(service, CallArgs(args), guest)
}

#[test]
fn direct_library_errno_widths_are_source_values_and_raw_invoke_stays_typed() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    for (service, args, expected) in [
        (Service::Open, [0; 6], 0x8002_0016_u64),
        (Service::Close, [0; 6], 0x8002_0001),
        (Service::Fstat, [0; 6], 0x8002_0001),
        (Service::Stat, [0; 6], 0x8002_0016),
        (Service::CheckReachability, [0; 6], 0x8002_0016),
        (Service::Read, [0; 6], 0xffff_ffff_8002_0001),
        (Service::Pread, [3, 0, 8, 0, 0, 0], 0xffff_ffff_8002_000e),
        (Service::Lseek, [0; 6], 0xffff_ffff_8002_0009),
        (Service::Pwrite, [0; 6], 0xffff_ffff_8002_0001),
        (Service::Ftruncate, [0; 6], 0x8002_0009),
        (Service::Fsync, [0; 6], 0x8002_0009),
        (Service::Mkdir, [0; 6], 0x8002_0016),
        (Service::Rmdir, [0; 6], 0x8002_0016),
        (Service::Rename, [0; 6], 0x8002_0016),
        (Service::Unlink, [0; 6], 0x8002_0016),
        (Service::AllocateDirect, [0; 6], 0x8002_0016),
        (Service::ClockGettime, [3, ARG, 0, 0, 0, 0], 0x8002_0016),
        (Service::Gettimeofday, [0; 6], 0x8002_000e),
        (Service::Nanosleep, [0; 6], 0x8002_000e),
    ] {
        assert!(matches!(
            kernel.invoke(service, CallArgs(args), &mut guest),
            Err(Failure::Errno(_))
        ));
        assert_eq!(
            kernel.invoke_library(service, CallArgs(args), &mut guest),
            Ok(expected),
            "{service:?}"
        );
    }
    assert_eq!(kernel.usage().attempts, 38);
    assert_eq!(kernel.usage().handles_live, 0);
    assert_eq!(guest.effects, 0);
}

#[test]
fn library_error_results_keep_source_sync_outputs_and_do_not_consume_failed_counts() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"original\0");
    invoke(
        &mut kernel,
        &mut guest,
        Service::CreateSema,
        &[ARG + 16, ARG, 1, 0, 2, 0],
    )
    .unwrap();
    let sema = guest.u64(16);
    assert_eq!(
        invoke(&mut kernel, &mut guest, Service::PollSema, &[sema, 1]),
        Ok(0x8002_0010)
    );
    invoke(&mut kernel, &mut guest, Service::SignalSema, &[sema, 1]).unwrap();
    assert_eq!(
        invoke(&mut kernel, &mut guest, Service::PollSema, &[sema, 1]),
        Ok(0)
    );
    invoke(
        &mut kernel,
        &mut guest,
        Service::CreateEventFlag,
        &[ARG + 24, ARG, 0x11, 0x10],
    )
    .unwrap();
    let flag = guest.u64(24);
    guest.put(32, &0x55_u64.to_le_bytes());
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::PollEventFlag,
            &[flag, 0x20, 1, ARG + 32]
        ),
        Ok(0x8002_0010)
    );
    assert_eq!(guest.u64(32), 0x10);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::PollEventFlag,
            &[flag, 0x10, 0x21, ARG + 32]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(32), 0x10);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::PollEventFlag,
            &[flag, 0x10, 1, ARG + 32]
        ),
        Ok(0x8002_0010)
    );
    assert_eq!(guest.u64(32), 0);
}

#[test]
fn library_return_mapping_never_hides_backing_policy_or_unbound_domain_failures() {
    let mut kernel = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"original\0");
    guest.refuse = true;
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::CreateSema,
            &[ARG + 16, ARG, 1, 0, 2]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(kernel.usage().handles_live, 0);
    assert_eq!(guest.u64(16), 0);
    assert!(matches!(
        invoke(&mut kernel, &mut guest, Service::ReadTsc, &[]),
        Err(Failure::Unsupported(_))
    ));
    guest.refuse = false;
    invoke(
        &mut kernel,
        &mut guest,
        Service::MapFlexible,
        &[ARG + 8, PAGE, 3, 0],
    )
    .unwrap();
    let address = guest.u64(8);
    let before = guest.get(ARG + 16, 24);
    assert_eq!(
        invoke(
            &mut kernel,
            &mut guest,
            Service::QueryProtection,
            &[address, ARG + 16, ARG + 16, ARG + 32]
        ),
        Err(Failure::Unsupported("overlapping guest outputs"))
    );
    assert_eq!(guest.get(ARG + 16, 24), before);
    let mut exhausted = super::kernel(Limits {
        max_attempts: 0,
        ..Default::default()
    });
    assert_eq!(
        invoke(&mut exhausted, &mut guest, Service::Close, &[0]),
        Err(Failure::Limit("attempts"))
    );

    struct UnknownDomain;
    impl GuestAccess for UnknownDomain {
        fn read(&self, _: u64, output: &mut [u8]) -> Result<(), GuestError> {
            output.fill(0);
            Ok(())
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("unbound error must not publish output")
        }
        fn module_symbol(&mut self, _: i32, _: &str) -> Result<Option<u64>, Failure> {
            Err(Failure::Errno(Errno::NoMemory))
        }
    }
    assert_eq!(
        kernel.invoke_library(
            Service::Dlsym,
            CallArgs([1, ARG, ARG + 8, 0, 0, 0]),
            &mut UnknownDomain
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
}
