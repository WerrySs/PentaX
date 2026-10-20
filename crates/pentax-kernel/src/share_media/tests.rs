// SPDX-License-Identifier: GPL-2.0-only
//! Original silent Share behavior, output atomicity and error precedence.
use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{GuestError, Limits, MemoryEffect};

struct NoAccess;
impl GuestAccess for NoAccess {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("opaque Share input read");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("unexpected Share side effect");
    }
}
struct WriteOnly(TestGuest);
impl GuestAccess for WriteOnly {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("Share output payload read");
    }
    fn preflight_write(&self, address: u64, length: u64) -> Result<(), GuestError> {
        self.0.preflight_write(address, length)
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        self.0.apply(effect, writes)
    }
}
fn call(
    k: &mut crate::Kernel,
    g: &mut impl GuestAccess,
    s: Service,
    args: [u64; 6],
) -> Result<u64, Failure> {
    k.invoke_library(s, CallArgs(args), g)
}
const CAPTURES: [Service; 4] = [
    Service::ShareCaptureScreenshot,
    Service::ShareCaptureScreenshotExtended,
    Service::ShareCaptureVideoClip,
    Service::ShareCaptureVideoClipExtended,
];

#[test]
fn share_media_opaque_calls_low_word_masks_and_no_callback_or_device_retention() {
    let mut k = kernel(Limits {
        max_handles: 1,
        ..Limits::default()
    });
    let census = k.handle_usage();
    for word in [0, u64::MAX, 1, 0x8888_0000_0000_0000, 0x1111_1111_8000_0000] {
        for s in [
            Service::ShareRegisterContentEventCallback,
            Service::ShareUnregisterContentEventCallback,
            Service::ShareSetScreenshotOverlayImage,
        ] {
            assert_eq!(call(&mut k, &mut NoAccess, s, [word; 6]), Ok(0));
        }
        for s in [
            Service::ShareFeaturePermit,
            Service::ShareFeatureProhibit,
            Service::ShareSetCaptureSource,
        ] {
            assert_eq!(
                call(&mut k, &mut NoAccess, s, [word; 6]),
                Ok(if word as u32 == 0 { INVALID } else { 0 })
            );
        }
        for s in [
            Service::ShareSetContentParam,
            Service::ShareSetContentParamForApplicationTitle,
        ] {
            assert_eq!(
                call(&mut k, &mut NoAccess, s, [word; 6]),
                Ok(if word == 0 { INVALID } else { 0 })
            );
        }
        assert_eq!(
            call(
                &mut k,
                &mut NoAccess,
                Service::ShareOpenMenuForContent,
                [word; 6]
            ),
            Ok(NOT_SUPPORTED)
        );
    }
    for s in CAPTURES {
        assert_eq!(
            call(&mut k, &mut NoAccess, s, [u64::MAX, 0, 0, 0, 0, 0]),
            Ok(NOT_SUPPORTED)
        );
    }
    assert_eq!(k.handle_usage(), census);
    assert_eq!(
        (
            k.usage().bytes_read,
            k.usage().bytes_written,
            k.usage().handles_live
        ),
        (0, 0, 0)
    );
}

#[test]
fn share_media_outputs_exact_request_id_status_padding_and_canaries_without_reads() {
    let mut k = kernel(Limits::default());
    let mut g = WriteOnly(TestGuest::new());
    let out = ARG + 0x200;
    for s in CAPTURES {
        g.0.put(out - ARG - 1, &[0xa5; 6]);
        assert_eq!(
            call(&mut k, &mut g, s, [out, out, 0, 0, 0, 0]),
            Ok(NOT_SUPPORTED)
        );
        assert_eq!(g.0.get(out - 1, 6), [0xa5, 0xff, 0xff, 0xff, 0xff, 0xa5]);
    }
    g.0.put(out - ARG - 1, &[0xa5; 18]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::ShareGetCurrentStatus,
            [0x3333_3333_8000_0000, out, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        g.0.get(out - 1, 18),
        [&[0xa5][..], &[0; 16], &[0xa5]].concat()
    );
    g.0.put(out - ARG - 1, &[0xa5; 6]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::ShareGetRunningStatus,
            [out, 0, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert_eq!(g.0.get(out - 1, 6), [0xa5, 0, 0, 0, 0, 0xa5]);
    assert_eq!(
        (
            k.usage().bytes_read,
            k.usage().bytes_written,
            k.usage().handles_live
        ),
        (0, 36, 0)
    );
}

#[test]
fn share_media_validation_before_output_atomic_refusal_quotas_and_actual_errno() {
    let mut k = kernel(Limits::default());
    let mut g = TestGuest::new();
    let out = ARG + PAGE - 8;
    g.put(PAGE - 16, &[0xa5; 16]);
    assert_eq!(
        call(
            &mut k,
            &mut NoAccess,
            Service::ShareGetCurrentStatus,
            [0, u64::MAX, 0, 0, 0, 0]
        ),
        Ok(INVALID)
    );
    assert_eq!(
        call(
            &mut k,
            &mut NoAccess,
            Service::ShareGetCurrentStatus,
            [1, 0, 0, 0, 0, 0]
        ),
        Ok(INVALID)
    );
    assert_eq!(
        call(
            &mut k,
            &mut NoAccess,
            Service::ShareGetRunningStatus,
            [0; 6]
        ),
        Ok(INVALID)
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::ShareGetCurrentStatus,
            [1, out, 0, 0, 0, 0]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(g.get(ARG + PAGE - 16, 16), [0xa5; 16]);
    for (s, args) in [
        (
            Service::ShareCaptureScreenshot,
            [0, u64::MAX - 1, 0, 0, 0, 0],
        ),
        (
            Service::ShareGetRunningStatus,
            [u64::MAX - 1, 0, 0, 0, 0, 0],
        ),
    ] {
        assert_eq!(
            call(&mut k, &mut g, s, args),
            Err(Failure::Guest(GuestError::AddressOverflow))
        );
    }
    for (s, args, limit) in [
        (Service::ShareGetCurrentStatus, [1, ARG, 0, 0, 0, 0], 15),
        (Service::ShareCaptureVideoClip, [0, ARG, 0, 0, 0, 0], 3),
        (Service::ShareGetRunningStatus, [ARG, 0, 0, 0, 0, 0], 3),
    ] {
        let mut limited = kernel(Limits {
            max_bytes: limit,
            ..Limits::default()
        });
        g.put(0, &[0xa5; 16]);
        assert!(matches!(
            call(&mut limited, &mut g, s, args),
            Err(Failure::Limit(_))
        ));
        assert_eq!(g.get(ARG, 16), [0xa5; 16]);
    }
    let mut work = kernel(Limits {
        max_work: 1,
        ..Limits::default()
    });
    assert_eq!(
        call(
            &mut work,
            &mut NoAccess,
            Service::ShareRegisterContentEventCallback,
            [u64::MAX; 6]
        ),
        Err(Failure::Limit("work"))
    );
    let errno = call(&mut k, &mut g, Service::ErrnoLocation, [0; 6]).unwrap();
    g.apply(
        MemoryEffect::None,
        &[GuestWrite {
            address: errno,
            bytes: &37_u32.to_le_bytes(),
        }],
    )
    .unwrap();
    let census = k.handle_usage();
    for (s, args, expected) in [
        (Service::ShareFeaturePermit, [0; 6], INVALID),
        (Service::ShareOpenMenuForContent, [0; 6], NOT_SUPPORTED),
        (Service::ShareCaptureVideoClip, [0; 6], NOT_SUPPORTED),
        (Service::ShareRegisterContentEventCallback, [u64::MAX; 6], 0),
    ] {
        assert_eq!(call(&mut k, &mut g, s, args), Ok(expected));
        assert_eq!(g.get(errno, 4), 37_u32.to_le_bytes());
        assert_eq!(k.handle_usage(), census);
    }
}
