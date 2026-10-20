// SPDX-License-Identifier: GPL-2.0-only

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
fn output_preserves_binary_streams_errno_and_releases_retained_budget() {
    let mut owner = kernel(Limits {
        max_output_bytes: 8,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0, b"a\0bc");
    let errno = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    for (service, descriptor, length) in [
        (Service::Write, 1, 4),
        (Service::PosixWrite, 2, 2),
        (Service::PosixWrite, 0, 2),
    ] {
        assert_eq!(
            invoke(&mut owner, &mut guest, service, &[descriptor, ARG, length]),
            Ok(length)
        );
    }
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(
        owner.output_usage(),
        OutputUsage {
            accepted_bytes: 8,
            retained_bytes: 8
        }
    );
    let retained = owner.output_usage();
    assert_eq!(owner.output_tail(OutputStream::Stdout, 2), b"bc");
    assert_eq!(
        owner.output_tail(OutputStream::Stdout, usize::MAX),
        b"a\0bc"
    );
    assert!(owner.output_tail(OutputStream::Stdout, 0).is_empty());
    assert_eq!(owner.output_usage(), retained);
    assert_eq!(owner.take_output(OutputStream::Stdout), b"a\0bc");
    assert_eq!(owner.output_usage().retained_bytes, 4);
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[1, ARG, 4]),
        Ok(4)
    );
    assert_eq!(owner.take_output(OutputStream::Stderr), b"a\0");
    assert_eq!(owner.take_output(OutputStream::StdinCompatibility), b"a\0");
    assert_eq!(owner.take_output(OutputStream::Stdout), b"a\0bc");
    assert!(owner.take_output(OutputStream::Stdout).is_empty());
    assert_eq!(
        owner.output_usage(),
        OutputUsage {
            accepted_bytes: 12,
            retained_bytes: 0
        }
    );
    assert_eq!(owner.usage().attempts, 5);
}

#[test]
fn output_failures_preserve_pending_bytes_and_use_exact_return_domains() {
    let mut owner = kernel(Limits {
        max_output_bytes: 8,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0, b"keep");
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[1, ARG, 4]),
        Ok(4)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[2, ARG, 5]),
        Err(Failure::Limit("output bytes"))
    );
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::Write,
            &[2, ARG + PAGE - 1, 2]
        ),
        Err(Failure::Guest(GuestError::Unmapped))
    );
    guest.regions[0].protection = 2;
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[2, ARG, 1]),
        Err(Failure::Guest(GuestError::ReadDenied))
    );
    guest.regions[0].protection = 3;
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[2, ARG, u64::MAX]),
        Err(Failure::Limit("output transfer"))
    );
    // Direct KernelWrite checks null before descriptors, including empty writes.
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[99, 0, 0]),
        Ok(0xffff_ffff_8002_000e)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[0, ARG, 1]),
        Ok(0xffff_ffff_8002_0009)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixWrite, &[1, 0, 0]),
        Ok(0)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixWrite, &[1, 0, 1]),
        Ok(u64::MAX)
    );
    let errno = invoke(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    assert_eq!(guest.get(errno, 4), 14_u32.to_le_bytes());
    assert_eq!(
        invoke(
            &mut owner,
            &mut guest,
            Service::PosixWrite,
            &[u64::MAX, 0, 1]
        ),
        Ok(u64::MAX)
    );
    assert_eq!(guest.get(errno, 4), 9_u32.to_le_bytes());
    assert_eq!(
        owner.output_usage(),
        OutputUsage {
            accepted_bytes: 4,
            retained_bytes: 4
        }
    );
    assert_eq!(owner.take_output(OutputStream::Stdout), b"keep");
    assert!(owner.take_output(OutputStream::Stderr).is_empty());
}

#[test]
fn output_cannot_mutate_read_only_file_descriptors() {
    let original = OriginalFile::new();
    let mut owner = Kernel::new(
        Limits::default(),
        ProcessIdentity {
            process_id: 100,
            initial_thread_id: 1,
        },
        vec![original.root()],
    )
    .unwrap();
    let mut guest = TestGuest::new();
    guest.put(0, b"changed");
    guest.put(64, b"/original/data.txt\0");
    let fd = invoke(&mut owner, &mut guest, Service::Open, &[ARG + 64, 0, 0]).unwrap();
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::Write, &[fd, ARG, 7]),
        Ok(0xffff_ffff_8002_0009)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, Service::PosixWrite, &[fd, ARG, 7]),
        Ok(u64::MAX)
    );
    assert_eq!(
        std::fs::read(&original.path).unwrap(),
        b"original-data-0123456789"
    );
    assert_eq!(owner.output_usage(), OutputUsage::default());
}
