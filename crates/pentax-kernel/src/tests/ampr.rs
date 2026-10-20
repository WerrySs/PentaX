// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use Service::*;

fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn code(error: Errno) -> Result<u64, Failure> {
    Ok(u64::from(error.kernel_code() as u32))
}
fn set(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    object: u64,
    buffer: u64,
    size: u64,
) -> Result<u64, Failure> {
    call(owner, guest, AmprSetBuffer, &[object, buffer, size])
}

#[test]
fn ampr_retained_binding_reset_live_queries_and_clear_follow_actual_headers() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let object = ARG + 64;
    guest.put(56, &[0xa5; 48]);
    assert_eq!(
        call(&mut owner, &mut guest, AmprConstruct, &[object]),
        Ok(0)
    );
    assert_eq!(guest.get(object, 24), [0; 24]);
    assert_eq!(guest.get(object - 8, 8), [0xa5; 8]);
    assert_eq!(guest.get(object + 24, 16), [0xa5; 16]);
    guest.put(64, &0xfedc_ba98_u32.to_le_bytes());
    assert_eq!(
        set(&mut owner, &mut guest, object, ARG + 512, (1 << 40) | 64),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, 1);
    guest.put(68, &7_u32.to_le_bytes());
    guest.put(72, &9_u32.to_le_bytes());
    guest.put(76, &96_u32.to_le_bytes());
    guest.put(80, &(ARG + 1024).to_le_bytes());
    for (service, expected) in [
        (AmprGetType, 0xfedc_ba98),
        (AmprGetOffset, 7),
        (AmprGetNum, 9),
        (AmprGetSize, 96),
        (AmprGetBase, ARG + 1024),
    ] {
        assert_eq!(
            call(&mut owner, &mut guest, service, &[object]),
            Ok(expected)
        );
    }
    let before = guest.get(object, 24);
    guest.refuse = true;
    assert_eq!(
        call(&mut owner, &mut guest, AmprReset, &[object]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(guest.get(object, 24), before);
    assert_eq!(call(&mut owner, &mut guest, AmprReset, &[object]), Ok(0));
    assert_eq!(guest.u64(72), 64 << 32);
    assert_eq!(guest.u64(80), ARG + 512);
    assert_eq!(guest.u64(64), 0xfedc_ba98);
    guest.refuse = true;
    assert_eq!(
        call(&mut owner, &mut guest, AmprConstruct, &[object]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(owner.usage().handles_live, 1);
    let before = guest.get(object, 24);
    assert_eq!(call(&mut owner, &mut guest, AmprDestruct, &[object]), Ok(0));
    assert_eq!(guest.get(object, 24), before);
    assert_eq!(owner.usage().handles_live, 0);
    guest.put(80, &(ARG + 1024).to_le_bytes());
    guest.put(76, &96_u32.to_le_bytes());
    assert_eq!(call(&mut owner, &mut guest, AmprReset, &[object]), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, AmprAprDestruct, &[object]),
        Ok(0)
    );
    guest.put(80, &(ARG + 512).to_le_bytes());
    assert_eq!(call(&mut owner, &mut guest, AmprReset, &[object]), Ok(0));
    assert_eq!(guest.u64(80), ARG + 1024); // APR destructor is the source no-op.
    guest.put(68, &7_u32.to_le_bytes());
    guest.put(72, &9_u32.to_le_bytes());
    guest.put(1024, &[0x37; 96]);
    let prefix = guest.get(object, 12);
    assert_eq!(
        call(&mut owner, &mut guest, AmprClearBuffer, &[object]),
        Ok(ARG + 1024)
    );
    assert_eq!(guest.get(object, 12), prefix);
    assert_eq!(guest.get(object + 12, 12), [0; 12]);
    assert_eq!(guest.get(ARG + 1024, 96), [0x37; 96]);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(
        call(&mut owner, &mut guest, AmprClearBuffer, &[object]),
        Ok(0)
    );
}

#[test]
fn ampr_alias_replacement_erasure_and_empty_snapshot_retention_do_not_revive_stale_owners() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let buffer = ARG + 512;
    assert_eq!(set(&mut owner, &mut guest, ARG, buffer, 64), Ok(0));
    assert_eq!(set(&mut owner, &mut guest, ARG + 64, buffer, 128), Ok(0));
    assert_eq!(call(&mut owner, &mut guest, AmprReset, &[buffer]), Ok(0));
    assert_eq!(guest.u64(512 + 8), 128 << 32);
    assert_eq!(guest.u64(512 + 16), buffer);
    assert_eq!(owner.usage().handles_live, 3);
    // Erasing the backing-address object removes its alias key as well.
    assert_eq!(call(&mut owner, &mut guest, AmprDestruct, &[buffer]), Ok(0));
    guest.put(512, &[0; 24]);
    assert_eq!(
        call(&mut owner, &mut guest, AmprReset, &[buffer]),
        code(Errno::Perm)
    );
    guest.put(512 + 16, &(ARG + 2048).to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, AmprReset, &[buffer]),
        code(Errno::Perm)
    );
    assert_eq!(owner.usage().handles_live, 3);
    let mut other = kernel(Default::default());
    assert_eq!(call(&mut other, &mut guest, AmprReset, &[buffer]), Ok(0));
    assert_eq!(guest.u64(512 + 16), ARG + 2048); // Isolated owner loads actual header.
    assert_eq!(
        call(&mut owner, &mut guest, AmprConstruct, &[buffer]),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, 2);
    assert_eq!(
        call(&mut owner, &mut guest, AmprClearBuffer, &[ARG + 64]),
        Ok(buffer)
    );
    assert_eq!(owner.usage().handles_live, 1);
}

#[test]
fn ampr_reserved_words_exact_query_spans_and_source_parameter_order_preserve_atomicity() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    for service in [
        AmprConstruct,
        AmprAprConstruct,
        AmprAprDestruct,
        AmprDestruct,
        AmprClearBuffer,
        AmprGetType,
        AmprGetNum,
        AmprGetSize,
        AmprGetOffset,
        AmprGetBase,
    ] {
        assert_eq!(
            call(&mut owner, &mut guest, service, &[0, u64::MAX, u64::MAX]),
            Ok(0)
        );
    }
    assert_eq!(
        call(&mut owner, &mut guest, AmprReset, &[0]),
        code(Errno::Perm)
    );
    assert_eq!(set(&mut owner, &mut guest, 0, 0, 0), code(Errno::Invalid));
    assert_eq!(
        set(&mut owner, &mut guest, u64::MAX - 8, 0, 0),
        code(Errno::Fault)
    );
    assert_eq!(
        set(&mut owner, &mut guest, ARG + PAGE, 3, 1),
        code(Errno::Invalid)
    );
    assert_eq!(
        set(&mut owner, &mut guest, ARG + PAGE, ARG + 512, 64),
        Err(Failure::Guest(GuestError::Unmapped))
    );
    guest.put(128, &[0xa5; 48]);
    assert_eq!(
        call(&mut owner, &mut guest, AmprAprConstruct, &[ARG + 128]),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 128, 24), [0xa5; 24]);
    assert_eq!(guest.get(ARG + 152, 16), [0; 16]);
    assert_eq!(guest.get(ARG + 168, 8), [0xa5; 8]);
    for (first, second, length) in [(64, 64, 8), (64, 68, 12), (72, 64, 16)] {
        guest.put(64, &[0xa5; 24]);
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                AmprAprConstruct,
                &[ARG, ARG + first, ARG + second]
            ),
            Ok(0)
        );
        assert_eq!(guest.get(ARG + 64, length), vec![0; length]);
        assert_eq!(guest.get(ARG + 64 + length as u64, 1), [0xa5]);
    }
    guest.put(64, &[0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            AmprAprConstruct,
            &[ARG, ARG + 64, ARG + PAGE - 4]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + 64, 8), [0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            AmprAprConstruct,
            &[ARG, ARG + 64, u64::MAX - 3]
        ),
        code(Errno::Fault)
    );
    guest.put(PAGE - 4, &0xfedc_ba98_u32.to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, AmprGetType, &[ARG + PAGE - 4]),
        Ok(0xfedc_ba98)
    );
    assert_eq!(
        call(&mut owner, &mut guest, AmprGetBase, &[ARG + PAGE - 20]),
        Err(Failure::Guest(GuestError::Unmapped))
    );
}

#[test]
fn ampr_binding_and_work_scratch_limits_refuse_before_guest_or_retained_publication() {
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, AmprReset, &[ARG]),
        code(Errno::Perm)
    );
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(set(&mut owner, &mut guest, ARG, ARG + 512, 64), Ok(0));
    assert_eq!(
        set(&mut owner, &mut guest, ARG, ARG + 512, 64),
        code(Errno::Busy)
    );
    let original = guest.get(ARG + 64, 24);
    assert_eq!(
        set(&mut owner, &mut guest, ARG + 64, ARG + 1024, 64),
        Err(Failure::Limit("handles"))
    );
    assert_eq!(guest.get(ARG + 64, 24), original);
    assert_eq!(call(&mut owner, &mut guest, AmprDestruct, &[ARG]), Ok(0));
    assert_eq!(set(&mut owner, &mut guest, ARG + 64, ARG + 1024, 64), Ok(0));
    for size in [0, 1, 64 * 1024 * 1024 + 4] {
        assert_eq!(
            set(&mut owner, &mut guest, ARG + 64, ARG + 1024, size),
            code(Errno::Invalid)
        );
    }
    assert_eq!(
        call(&mut owner, &mut guest, AmprClearBuffer, &[ARG + 64]),
        Ok(ARG + 1024)
    );
    assert_eq!(
        set(
            &mut owner,
            &mut guest,
            ARG + 64,
            ARG + 1024,
            64 * 1024 * 1024
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(64 + 8), (64_u64 * 1024 * 1024) << 32);
    assert_eq!(owner.usage().handles_live, 1);
    guest.put(128, &[0xa5; 24]);
    for limits in [
        Limits {
            max_work: 1,
            ..Default::default()
        },
        Limits {
            max_scratch_bytes: 1,
            max_name_bytes: 1,
            max_io_bytes: 0,
            ..Default::default()
        },
    ] {
        let mut limited = kernel(limits);
        assert!(matches!(
            call(&mut limited, &mut guest, AmprConstruct, &[ARG + 128]),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.get(ARG + 128, 24), [0xa5; 24]);
        assert_eq!(limited.usage().handles_live, 0);
    }
}
