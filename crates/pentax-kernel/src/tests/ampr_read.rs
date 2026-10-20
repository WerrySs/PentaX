// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{Kernel, Limits, Service};

const OBJECT: u64 = ARG + 0x100;
const BUFFER: u64 = ARG + 0x800;
const DESTINATION: u64 = ARG + 0x1800;
const STACK: u64 = ARG + 0x3000;

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
fn set(owner: &mut Kernel, guest: &mut TestGuest, object: u64, buffer: u64, size: u64) {
    assert_eq!(
        call(
            owner,
            guest,
            Service::AmprSetBuffer,
            &[object, buffer, size]
        ),
        Ok(0)
    );
}
fn append(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    object: u64,
    id: u64,
    destination: u64,
    size: u64,
    offset: u64,
) -> Result<u64, Failure> {
    let refused = guest.refuse;
    guest.refuse = false;
    guest.put(STACK + 8 - ARG, &offset.to_le_bytes());
    guest.refuse = refused;
    owner.invoke_library_with_stack(
        Service::AmprReadFile,
        CallArgs([object, u64::MAX, u64::MAX, id, destination, size]),
        Some(STACK),
        guest,
    )
}
fn binding(owner: &Kernel, object: u64) -> &Binding {
    owner
        .ampr
        .bindings
        .iter()
        .find(|entry| entry.object == object)
        .unwrap()
}
fn error(error: Errno) -> Result<u64, Failure> {
    Ok(u64::from(error.kernel_code() as u32))
}

#[test]
fn ampr_read_records_retain_exact_arguments_and_footprints_then_reset_and_clear() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0x100, &0x8000_0002_u32.to_le_bytes());
    set(&mut owner, &mut guest, OBJECT, BUFFER, 64);
    guest.put(0x108, &u32::MAX.to_le_bytes());
    guest.put(0x800, &[0xa5; 56]);
    guest.put(0x1800, &[0x5a; 16]);
    assert_eq!(
        append(
            &mut owner,
            &mut guest,
            OBJECT,
            (1 << 40) | 99,
            DESTINATION,
            8,
            0
        ),
        Ok(0)
    );
    assert_eq!(
        append(
            &mut owner,
            &mut guest,
            OBJECT,
            u64::MAX,
            0,
            0x1_0000_0000,
            0xff_ffff_ffff
        ),
        Ok(0)
    );
    let mut expected = [0xa5; 56];
    expected[..44].fill(0);
    expected[0] = 0x17;
    expected[20] = 0x17;
    assert_eq!(guest.get(BUFFER, 56), expected);
    assert_eq!(guest.get(DESTINATION, 16), [0x5a; 16]);
    assert_eq!(guest.u64(0x100), (44_u64 << 32) | 0x8001_0002);
    assert_eq!(guest.u64(0x108), (64_u64 << 32) | 1);
    assert_eq!(
        binding(&owner, OBJECT).reads,
        [
            ReadCommand {
                record_offset: 0,
                file_id: 99,
                destination: DESTINATION,
                size: 8,
                file_offset: 0
            },
            ReadCommand {
                record_offset: 20,
                file_id: u32::MAX,
                destination: 0,
                size: 0x1_0000_0000,
                file_offset: 0xff_ffff_ffff
            },
        ]
    );
    assert_eq!(
        binding(&owner, OBJECT).gather,
        Some((u32::MAX, 0x1_0000_0000, 0x100_ffff_ffff))
    );
    assert_eq!(owner.usage().ampr_commands, 2);
    // Retained metadata wins over later direct header edits.
    guest.put(0x104, &99_u32.to_le_bytes());
    guest.put(0x10c, &4_u32.to_le_bytes());
    guest.put(0x110, &(ARG + 0x2000).to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprReset, &[OBJECT]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x100), 0x8001_0002);
    assert_eq!(guest.u64(0x108), 64_u64 << 32);
    assert_eq!(guest.u64(0x110), BUFFER);
    assert!(binding(&owner, OBJECT).reads.is_empty());
    assert_eq!(binding(&owner, OBJECT).gather, None);
    assert_eq!(owner.usage().ampr_commands, 0);
    assert_eq!(guest.get(BUFFER, 56), expected);
    assert_eq!(append(&mut owner, &mut guest, OBJECT, 7, 0, 1, 0), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprClearBuffer, &[OBJECT]),
        Ok(BUFFER)
    );
    assert_eq!(owner.usage().ampr_commands, 0);
    assert_eq!(owner.usage().handles_live, 0);
}

#[test]
fn ampr_read_header_overlays_and_queued_object_alias_precedence_follow_source_order() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    set(&mut owner, &mut guest, OBJECT, OBJECT, 64);
    assert_eq!(append(&mut owner, &mut guest, OBJECT, 1, 0, 1, 0), Ok(0));
    assert_eq!(guest.u64(0x100), (20_u64 << 32) | 0x1_0017);
    assert_eq!(guest.u64(0x108), 1); // Record zeroed size/count before live increment.
    assert_eq!(guest.u64(0x110), 0);
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprReset, &[OBJECT]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x100), 0x1_0017);
    assert_eq!(guest.u64(0x108), 64_u64 << 32);
    assert_eq!(guest.u64(0x110), OBJECT);

    let exact = ARG + 0x400;
    let other = ARG + 0x200;
    set(&mut owner, &mut guest, exact, BUFFER, 64);
    assert_eq!(append(&mut owner, &mut guest, exact, 7, 0, 1, 0), Ok(0));
    set(&mut owner, &mut guest, other, exact, 128);
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprReset, &[exact]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x410), BUFFER); // Queued exact object beats other.alias.
    assert_eq!(binding(&owner, other).alias, Some(exact));
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprReset, &[exact]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x410), exact); // Empty exact object now follows the alias.
    assert_eq!(guest.u64(0x408), 128_u64 << 32);
    assert_eq!(binding(&owner, other).alias, Some(exact)); // No self alias replaces it.
}

#[test]
fn ampr_read_cold_failure_retains_snapshot_and_refused_transactions_preserve_records() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        append(&mut owner, &mut guest, OBJECT, 1, 0, 1, 0),
        error(Errno::Fault)
    );
    assert_eq!(owner.usage().handles_live, 1);
    guest.put(0x10c, &64_u32.to_le_bytes());
    guest.put(0x110, &BUFFER.to_le_bytes());
    assert_eq!(
        append(&mut owner, &mut guest, OBJECT, 1, 0, 1, 0),
        error(Errno::Fault)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprReset, &[OBJECT]),
        error(Errno::Perm)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprConstruct, &[OBJECT]),
        Ok(0)
    );
    set(&mut owner, &mut guest, OBJECT, BUFFER, 64);
    guest.put(0x800, &[0xa5; 64]);
    let header = guest.get(OBJECT, 24);
    guest.refuse = true;
    assert_eq!(
        append(&mut owner, &mut guest, OBJECT, 1, 0, 1, 0),
        Err(Failure::Guest(crate::GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(guest.get(OBJECT, 24), header);
    assert_eq!(guest.get(BUFFER, 64), [0xa5; 64]);
    assert_eq!(owner.usage().ampr_commands, 0);
    assert_eq!(binding(&owner, OBJECT).snapshot.offset, 0);
    assert_eq!(binding(&owner, OBJECT).gather, None);

    let mut owner = kernel(Default::default());
    set(&mut owner, &mut guest, OBJECT + 64, ARG + PAGE - 16, 64);
    guest.put(PAGE - 16, &[0x5a; 16]);
    let before = guest.get(OBJECT + 64, 24);
    assert_eq!(
        append(&mut owner, &mut guest, OBJECT + 64, 1, 0, 1, 0),
        // TestGuest::apply reports all invalid output spans as WriteDenied.
        Err(Failure::Guest(crate::GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + PAGE - 16, 16), [0x5a; 16]);
    assert_eq!(guest.get(OBJECT + 64, 24), before);
    assert_eq!(owner.usage().ampr_commands, 0);
}

#[test]
fn ampr_read_measure_stack_bounds_and_record_limits_have_exact_error_domains() {
    let mut owner = kernel(Limits {
        max_ampr_commands: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    for (destination, size, offset, expected) in [
        (0, 1, 0, Ok(20)),
        (0, 0x1_0000_0000, 0xff_ffff_ffff, Ok(24)),
        (0x0000_f000_0000_0000 - 1, 1, 0, Ok(20)),
        (0, 0, 0, error(Errno::Invalid)),
        (0, 0x1_0000_0001, 0, error(Errno::Invalid)),
        (0, 1, 0x100_0000_0000, error(Errno::Invalid)),
        (0x0000_f000_0000_0000, 1, 0, error(Errno::Invalid)),
        (u64::MAX, 1, 0, error(Errno::Invalid)),
    ] {
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::AmprMeasureReadFile,
                &[u64::MAX, destination, size, offset]
            ),
            expected
        );
        if expected.is_ok_and(|value| value == u64::from(Errno::Invalid.kernel_code() as u32)) {
            assert_eq!(
                append(&mut owner, &mut guest, OBJECT, 1, destination, size, offset),
                expected
            );
        }
    }
    assert_eq!(owner.usage().handles_live, 0);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::AmprReadFile,
            &[OBJECT, 0, 0, 1, 0, 1]
        ),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(
        owner.invoke_library_with_stack(
            Service::AmprReadFile,
            CallArgs([OBJECT, 0, 0, 1, 0, 1]),
            Some(ARG + PAGE - 8),
            &mut guest
        ),
        Err(Failure::Guest(crate::GuestError::Unmapped))
    );
    set(&mut owner, &mut guest, OBJECT, BUFFER, 64);
    assert_eq!(append(&mut owner, &mut guest, OBJECT, 1, 0, 1, 0), Ok(0));
    let before = guest.get(OBJECT, 24);
    assert!(matches!(
        append(&mut owner, &mut guest, OBJECT, 2, 0, 1, 0),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(OBJECT, 24), before);
    assert_eq!(owner.usage().ampr_commands, 1);
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprReset, &[OBJECT]),
        Ok(0)
    );
    assert_eq!(append(&mut owner, &mut guest, OBJECT, 2, 0, 1, 0), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, Service::AmprDestruct, &[OBJECT]),
        Ok(0)
    );
    assert_eq!(owner.usage().ampr_commands, 0);

    let mut limited = kernel(Limits {
        max_scratch_bytes: 35,
        max_name_bytes: 1,
        max_io_bytes: 0,
        ..Default::default()
    });
    guest.put(0x110, &[0; 8]);
    set(&mut limited, &mut guest, OBJECT, BUFFER, 64);
    assert!(matches!(
        append(&mut limited, &mut guest, OBJECT, 1, 0, 1, 0),
        Err(Failure::Limit(_))
    ));
    assert_eq!(limited.usage().ampr_commands, 0);
}
