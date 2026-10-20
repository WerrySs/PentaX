// SPDX-License-Identifier: GPL-2.0-only
//! Original key-generation, source ABI and publication-failure checks.
use super::*;
use Service::*;
fn library(
    owner: &mut Kernel,
    guest: &mut impl GuestAccess,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}

#[test]
fn thread_context_limits_do_not_replace_the_active_thread() {
    let mut owner = kernel(Limits {
        max_thread_contexts: 2,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(owner.activate_thread_context(0x1000), Ok(7));
    assert_eq!(owner.activate_thread_context(0x2000), Ok(8));
    assert!(owner.activate_thread_context(0).is_err());
    assert!(owner.activate_thread_context(0x3000).is_err());
    assert_eq!(
        library(&mut owner, &mut guest, PthreadGetthreadid, &[]),
        Ok(8)
    );
    assert_eq!(owner.activate_thread_context(0x1000), Ok(7));
    assert_eq!(
        library(&mut owner, &mut guest, PthreadGetthreadid, &[]),
        Ok(7)
    );
    let mut exhausted = Kernel::new(
        Default::default(),
        ProcessIdentity {
            process_id: 41,
            initial_thread_id: i32::MAX as u32,
        },
        vec![],
    )
    .unwrap();
    assert_eq!(
        exhausted.activate_thread_context(0x1000),
        Ok(i32::MAX as u32)
    );
    assert!(exhausted.activate_thread_context(0x2000).is_err());
    assert_eq!(
        library(&mut exhausted, &mut guest, PthreadGetthreadid, &[]),
        Ok(i32::MAX as u64)
    );
}

#[test]
fn pthread_keys_recycle_generations_preserve_full_values_and_never_change_errno() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let errno = library(&mut owner, &mut guest, ErrnoLocation, &[]).unwrap();
    guest.copy(errno, &37_u32.to_le_bytes());
    guest.put(0, &0x1122_3344_5566_7788_u64.to_le_bytes());
    assert_eq!(
        library(&mut owner, &mut guest, KeyCreate, &[0, 0]),
        Ok(0x8002_0016)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixKeyCreate, &[0, 0]),
        Ok(22)
    );
    // Registration retains a guest callback word; creation/deletion never calls it.
    assert_eq!(
        library(&mut owner, &mut guest, KeyCreate, &[ARG, u64::MAX]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0000);
    assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[0]), Ok(0));
    let high_key = 0x1122_3344_0000_0000;
    let value = 0xfedc_ba98_7654_3210;
    assert_eq!(
        library(&mut owner, &mut guest, PosixSetSpecific, &[high_key, value]),
        Ok(0)
    );
    assert_eq!(
        library(&mut owner, &mut guest, GetSpecific, &[high_key]),
        Ok(value)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixKeyDelete, &[high_key]),
        Ok(0)
    );
    assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[0]), Ok(0));
    assert_eq!(
        library(&mut owner, &mut guest, KeyDelete, &[0]),
        Ok(0x8002_0016)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixKeyDelete, &[0]),
        Ok(22)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixSetSpecific, &[0, value]),
        Ok(22)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixKeyCreate, &[ARG, 0]),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0x1122_3344_0000_0000);
    assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[0]), Ok(0));
    assert_eq!(
        library(&mut owner, &mut guest, SetSpecific, &[0, value]),
        Ok(0)
    );
    assert_eq!(library(&mut owner, &mut guest, SetSpecific, &[0, 0]), Ok(0));
    assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[0]), Ok(0));
    for key in [256, u64::MAX] {
        assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[key]), Ok(0));
        assert_eq!(
            library(&mut owner, &mut guest, SetSpecific, &[key, value]),
            Ok(0x8002_0016)
        );
    }
    assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
    assert_eq!(owner.usage().scratch_peak, 4096);
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "mqULNdimTn0"),
        None
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "mqULNdimTn0"),
        Some(PosixKeyCreate)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libScePosix", 1, "mqULNdimTn0"),
        Some(PosixKeyCreate)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libScePosix", 2, "mqULNdimTn0"),
        None
    );
}

#[test]
fn pthread_key_source_capacity_is_256_and_lowest_deleted_index_is_reused_without_old_data() {
    let mut owner = kernel(Limits {
        max_handles: 300,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    for index in 0..256_u64 {
        assert_eq!(library(&mut owner, &mut guest, KeyCreate, &[ARG, 0]), Ok(0));
        assert_eq!(guest.u64(0), index);
    }
    assert_eq!(owner.usage().handles_live, 256);
    assert_eq!(
        library(&mut owner, &mut guest, KeyCreate, &[ARG, 0]),
        Ok(0x8002_0023)
    );
    assert_eq!(
        library(&mut owner, &mut guest, PosixKeyCreate, &[ARG, 0]),
        Ok(35)
    );
    assert_eq!(guest.u64(0), 255);
    assert_eq!(
        library(&mut owner, &mut guest, SetSpecific, &[17, 42]),
        Ok(0)
    );
    assert_eq!(library(&mut owner, &mut guest, KeyDelete, &[17]), Ok(0));
    assert_eq!(library(&mut owner, &mut guest, KeyDelete, &[29]), Ok(0));
    for index in [17, 29] {
        assert_eq!(library(&mut owner, &mut guest, KeyCreate, &[ARG, 0]), Ok(0));
        assert_eq!(guest.u64(0), index);
        assert_eq!(
            library(&mut owner, &mut guest, GetSpecific, &[index]),
            Ok(0)
        );
    }
    assert_eq!(owner.usage().handles_live, 256);
    let mut other = kernel(Default::default());
    let mut other_guest = TestGuest::new();
    assert_eq!(
        library(&mut other, &mut other_guest, KeyCreate, &[ARG, 0]),
        Ok(0)
    );
    assert_eq!(
        library(&mut other, &mut other_guest, GetSpecific, &[0]),
        Ok(0)
    );
    assert_eq!(
        library(&mut other, &mut other_guest, SetSpecific, &[0, 73]),
        Ok(0)
    );
    assert_eq!(
        library(&mut other, &mut other_guest, GetSpecific, &[0]),
        Ok(73)
    );
    assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[0]), Ok(0));
}

#[test]
fn pthread_key_failed_output_resources_and_lazy_storage_never_publish_partial_state() {
    let mut owner = kernel(Limits {
        max_handles: 1,
        max_scratch_bytes: 2048,
        max_io_bytes: 2048,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.refuse = true;
    assert_eq!(
        library(&mut owner, &mut guest, PosixKeyCreate, &[ARG, 0]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.u64(0), 0);
    assert_eq!(owner.usage().handles_live, 0);
    guest.refuse = false;
    // Only four output bytes are required, including at the backing boundary.
    assert_eq!(
        library(&mut owner, &mut guest, KeyCreate, &[ARG + PAGE - 4, 0]),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + PAGE - 4, 4), 0_u32.to_le_bytes());
    assert_eq!(library(&mut owner, &mut guest, SetSpecific, &[0, 0]), Ok(0));
    assert_eq!(owner.usage().scratch_peak, 0);
    assert!(matches!(
        library(&mut owner, &mut guest, PosixSetSpecific, &[0, 42]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(library(&mut owner, &mut guest, GetSpecific, &[0]), Ok(0));
    assert_eq!(owner.usage().scratch_peak, 0);
    assert!(matches!(
        library(&mut owner, &mut guest, KeyCreate, &[ARG, 0]),
        Err(Failure::Limit(_))
    ));
    assert!(matches!(
        library(&mut owner, &mut guest, MutexAttrInit, &[ARG]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(library(&mut owner, &mut guest, KeyDelete, &[0]), Ok(0));
    assert_eq!(library(&mut owner, &mut guest, KeyCreate, &[ARG, 0]), Ok(0));
    assert_eq!(guest.u64(0), 0);
    assert_eq!(owner.usage().handles_live, 1);
    struct Unbound;
    impl GuestAccess for Unbound {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("unbound key read")
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("unbound key write")
        }
    }
    let mut unbound = kernel(Default::default());
    for service in [
        KeyCreate,
        KeyDelete,
        SetSpecific,
        GetSpecific,
        PosixKeyCreate,
    ] {
        assert!(matches!(
            library(&mut unbound, &mut Unbound, service, &[ARG, 42]),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(
        unbound.usage().bytes_read + unbound.usage().bytes_written,
        0
    );
    assert_eq!(unbound.usage().handles_live, 0);
}
