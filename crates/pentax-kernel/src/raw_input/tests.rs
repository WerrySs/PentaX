// SPDX-License-Identifier: GPL-2.0-only
// Original stateless disconnected-device examples; no host device or game data.

use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{
    CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Kernel, Limits, MemoryEffect, Service,
};

fn call(
    owner: &mut Kernel,
    guest: &mut impl GuestAccess,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [u64::MAX; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}

struct NoGuest;
impl GuestAccess for NoGuest {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("opaque raw-input read");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("opaque raw-input write");
    }
}

struct WriteOnlyGuest(TestGuest);
impl GuestAccess for WriteOnlyGuest {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("raw-input output payload read");
    }
    fn preflight_write(&self, address: u64, length: u64) -> Result<(), GuestError> {
        self.0.preflight_write(address, length)
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        self.0.apply(effect, writes)
    }
}

#[test]
fn raw_open_close_and_init_are_stateless_signed_low_words_with_opaque_inputs() {
    let mut owner = kernel(Limits::default());
    for (init, open, close, argument_error, handle_error) in [
        (
            Service::MouseInit,
            Service::MouseOpen,
            Service::MouseClose,
            0x80df0001,
            0x80df0003,
        ),
        (
            Service::KeyboardInit,
            Service::KeyboardOpen,
            Service::KeyboardClose,
            0x80da0001,
            0x80da0003,
        ),
    ] {
        // A positive synthetic handle can close before Init or Open. No
        // registry/lifecycle requirement exists in the pinned implementation.
        assert_eq!(call(&mut owner, &mut NoGuest, close, &[97]), Ok(0));
        assert_eq!(
            call(
                &mut owner,
                &mut NoGuest,
                open,
                &[
                    u64::MAX,
                    0xaaaa_aaaa_0000_0000,
                    0xbbbb_bbbb_0000_0001,
                    u64::MAX
                ]
            ),
            Ok(1)
        );
        assert_eq!(
            call(&mut owner, &mut NoGuest, close, &[0xcccc_cccc_0000_0001]),
            Ok(0)
        );
        assert_eq!(call(&mut owner, &mut NoGuest, init, &[]), Ok(0));
        assert_eq!(call(&mut owner, &mut NoGuest, init, &[]), Ok(0));
        assert_eq!(call(&mut owner, &mut NoGuest, open, &[0, 0, 0, 0]), Ok(1));
        assert_eq!(
            call(&mut owner, &mut NoGuest, open, &[0, 1, 0, 0]),
            Ok(argument_error)
        );
        assert_eq!(
            call(
                &mut owner,
                &mut NoGuest,
                open,
                &[0, 0, 0x0000_0000_ffff_ffff, 0]
            ),
            Ok(argument_error)
        );
        assert_eq!(
            call(&mut owner, &mut NoGuest, open, &[0, 0, 2, 0]),
            Ok(argument_error)
        );
        assert_eq!(
            call(&mut owner, &mut NoGuest, close, &[0x0000_0000_ffff_ffff]),
            Ok(handle_error)
        );
        assert_eq!(
            call(&mut owner, &mut NoGuest, close, &[0x1000_0000_0000_0000]),
            Ok(handle_error)
        );
        assert_eq!(call(&mut owner, &mut NoGuest, close, &[97]), Ok(0));
    }
    assert_eq!(owner.usage().bytes_read + owner.usage().bytes_written, 0);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.usage().mapped_bytes, 0);
    assert_eq!(owner.handle_usage().accounted(), 0);
}

#[test]
fn disconnected_output_clears_exact_record_padding_and_tails_without_payload_reads() {
    let mut owner = kernel(Limits::default());
    let mut guest = WriteOnlyGuest(TestGuest::new());
    let output = ARG + 0x200;
    let cases = [
        // Mouse has no source16-record cap; this64-record request also needs
        // the fallible owned buffer beyond the ordinary1536-byte static table.
        (Service::MouseRead, [97, output, 64, 0, 0, 0], 40 * 64),
        (Service::KeyboardRead, [97, output, 16, 0, 0, 0], 96 * 16),
        (Service::KeyboardReadState, [97, output, 0, 0, 0, 0], 96),
        // Sixth GP is the output. Modifier/LED/key values are ignored.
        (
            Service::KeyboardGetKey2Char,
            [97, 1, u64::MAX, u64::MAX, u64::MAX, output],
            20,
        ),
    ];
    let mut written = 0;
    for (service, arguments, length) in cases {
        guest.0.put(output - ARG - 1, &vec![0xa5; length + 2]);
        assert_eq!(
            owner.invoke_library(service, CallArgs(arguments), &mut guest),
            Ok(0)
        );
        let mut expected = vec![0; length + 2];
        expected[0] = 0xa5;
        expected[length + 1] = 0xa5;
        assert_eq!(guest.0.get(output - 1, length + 2), expected);
        written += length as u64;
    }
    assert_eq!(owner.usage().bytes_written, written);
    assert_eq!(owner.usage().bytes_read, 0);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.handle_usage().accounted(), 0);
}

#[test]
fn invalid_handle_wins_then_pointer_count_and_mapping_errors_have_no_outputs() {
    let mut owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let output = ARG + 0x200;
    guest.put(output - ARG, &[0xa5; 1600]);
    for (service, arguments, expected) in [
        (Service::MouseRead, [0, 0, 0, 0, 0, 0], 0x80df0003),
        (Service::MouseRead, [1, 0, 1, 0, 0, 0], 0x80df0001),
        (Service::MouseRead, [1, output, 0, 0, 0, 0], 0x80df0001),
        (
            Service::MouseRead,
            [1, output, 0xffff_ffff, 0, 0, 0],
            0x80df0001,
        ),
        (
            Service::KeyboardRead,
            [0xffff_ffff, 0, 17, 0, 0, 0],
            0x80da0003,
        ),
        (Service::KeyboardRead, [1, 0, 1, 0, 0, 0], 0x80da0001),
        (Service::KeyboardRead, [1, output, 0, 0, 0, 0], 0x80da0001),
        (Service::KeyboardRead, [1, output, 17, 0, 0, 0], 0x80da0001),
        (
            Service::KeyboardRead,
            [1, output, 0xffff_ffff, 0, 0, 0],
            0x80da0001,
        ),
        (Service::KeyboardReadState, [0, 0, 0, 0, 0, 0], 0x80da0003),
        (Service::KeyboardReadState, [1, 0, 0, 0, 0, 0], 0x80da0001),
        (Service::KeyboardGetKey2Char, [0, 2, 0, 0, 0, 0], 0x80da0003),
        (Service::KeyboardGetKey2Char, [1, 0, 0, 0, 0, 0], 0x80da0001),
        (
            Service::KeyboardGetKey2Char,
            [1, 2, 0, 0, 0, output],
            0x80da0001,
        ),
        (
            Service::KeyboardGetKey2Char,
            [1, 0xffff_ffff, 0, 0, 0, output],
            0x80da0001,
        ),
    ] {
        assert_eq!(
            owner.invoke_library(service, CallArgs(arguments), &mut guest),
            Ok(expected)
        );
    }
    assert_eq!(guest.get(output, 1600), [0xa5; 1600]);
    assert_eq!(owner.usage().bytes_written + owner.usage().bytes_read, 0);
    assert_eq!(owner.usage().handles_live, 0);
}

#[test]
fn raw_zeroing_refuses_whole_span_and_budget_without_partial_prefix_or_resources() {
    let mut owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.put(PAGE - 1600, &[0xa5; 1600]);
    let old_tail = guest.get(ARG + PAGE - 1600, 1600);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::MouseRead,
            &[97, ARG + PAGE - 20, 1]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::KeyboardRead,
            &[97, ARG + PAGE - 1500, 16]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + PAGE - 1600, 1600), old_tail);
    let too_many = 1024 * 1024 / 40 + 1;
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::MouseRead,
            &[97, ARG, too_many]
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG + PAGE - 1600, 1600), old_tail);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.usage().mapped_bytes, 0);

    let mut limited = kernel(Limits {
        max_bytes: 39,
        ..Limits::default()
    });
    guest.put(0, &[0xa5; 42]);
    assert!(matches!(
        call(
            &mut limited,
            &mut guest,
            Service::MouseRead,
            &[1, ARG + 1, 1]
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG, 42), [0xa5; 42]);
    assert_eq!(limited.usage().handles_live, 0);
    let mut no_attempts = kernel(Limits {
        max_attempts: 0,
        ..Limits::default()
    });
    assert!(matches!(
        call(&mut no_attempts, &mut NoGuest, Service::MouseInit, &[]),
        Err(Failure::Limit(_))
    ));
}

#[test]
fn library_raw_results_preserve_actual_errno_cell_and_disjoint_resource_census() {
    let mut owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let errno = call(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    guest
        .apply(
            MemoryEffect::None,
            &[GuestWrite {
                address: errno,
                bytes: &37u32.to_le_bytes(),
            }],
        )
        .unwrap();
    let census = owner.handle_usage();
    let usage = owner.usage();
    let output = ARG + 0x200;
    for (service, arguments, result) in [
        (Service::MouseInit, [0; 6], 0),
        (Service::MouseOpen, [u64::MAX, 0, 1, u64::MAX, 0, 0], 1),
        (Service::MouseClose, [97, 0, 0, 0, 0, 0], 0),
        (Service::MouseRead, [97, output, 1, 0, 0, 0], 0),
        (Service::MouseRead, [0, output, 1, 0, 0, 0], 0x80df0003),
        (Service::KeyboardInit, [0; 6], 0),
        (Service::KeyboardOpen, [u64::MAX, 0, 0, u64::MAX, 0, 0], 1),
        (Service::KeyboardClose, [97, 0, 0, 0, 0, 0], 0),
        (Service::KeyboardRead, [97, output, 1, 0, 0, 0], 0),
        (Service::KeyboardReadState, [97, output, 0, 0, 0, 0], 0),
        (Service::KeyboardGetKey2Char, [97, 0, 0, 0, 0, output], 0),
        (
            Service::KeyboardGetKey2Char,
            [97, 2, 0, 0, 0, output],
            0x80da0001,
        ),
    ] {
        assert_eq!(
            owner.invoke_library(service, CallArgs(arguments), &mut guest),
            Ok(result)
        );
        assert_eq!(guest.get(errno, 4), 37u32.to_le_bytes());
        assert_eq!(owner.handle_usage(), census);
    }
    assert_eq!(owner.usage().handles_live, usage.handles_live);
    assert_eq!(owner.usage().mapped_bytes, usage.mapped_bytes);
    assert_eq!(owner.usage().bytes_read, usage.bytes_read);
}
