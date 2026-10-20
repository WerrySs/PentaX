// SPDX-License-Identifier: GPL-2.0-only
//! Original stateless helpers, exact footprints and transactional failures.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{Kernel, Limits, MemoryEffect};

fn call(
    k: &mut Kernel,
    g: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut a = [u64::MAX; 6];
    a[..values.len()].copy_from_slice(values);
    k.invoke_library(service, CallArgs(a), g)
}
fn protect(g: &mut TestGuest, protection: u32) {
    g.apply(
        MemoryEffect::Protect {
            address: ARG,
            length: PAGE,
            protection,
        },
        &[],
    )
    .unwrap();
}

#[test]
fn repeated_initialize_needs_no_guest_access_state_or_resource() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    g.refuse = true;
    let before = k.handle_usage();
    for _ in 0..3 {
        assert_eq!(
            call(&mut k, &mut g, Service::AudioOut2Initialize, &[]),
            Ok(0)
        );
    }
    assert_eq!(k.handle_usage(), before);
    assert_eq!(
        (
            k.usage().attempts,
            k.usage().bytes_read,
            k.usage().bytes_written
        ),
        (3, 0, 0)
    );
}

#[test]
fn reset_and_speaker_helpers_write_only_exact_outputs_and_preserve_canaries() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    g.put(0, &[0x5a; 256]);
    protect(&mut g, 2);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::AudioOut2ContextResetParam,
            &[ARG + 1]
        ),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::AudioOut2GetSpeakerInfo,
            &[ARG + 100, u64::MAX]
        ),
        Ok(0)
    );
    protect(&mut g, 3);
    let mut reset = [0; 64];
    for (index, value) in [256u32, 256, 0, 4, 512, 1].into_iter().enumerate() {
        reset[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(g.get(ARG + 1, 64), reset);
    assert_eq!(g.get(ARG, 1), [0x5a]);
    assert_eq!(g.get(ARG + 65, 35), [0x5a; 35]);
    let mut speaker = [0; 80];
    speaker[4] = 3;
    speaker[16..18].copy_from_slice(&(-30i16).to_le_bytes());
    speaker[20..22].copy_from_slice(&30i16.to_le_bytes());
    assert_eq!(g.get(ARG + 100, 80), speaker);
    assert_eq!(g.get(ARG + 180, 76), [0x5a; 76]);
    assert_eq!(
        (
            k.usage().bytes_read,
            k.usage().bytes_written,
            k.handle_usage().accounted()
        ),
        (0, 144, 0)
    );
}

#[test]
fn query_reads_only_queue_depth_before_aliasing_output_with_no_artificial_depth_cap() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    // Only the addressed queue_depth word is mapped; the other struct fields
    // must not be read. The output overwrites that same input after snapshot.
    for depth in [0u32, 1, 4, u32::MAX] {
        g.put(0, &[0x6b; 16]);
        g.put(0, &depth.to_le_bytes());
        assert_eq!(
            call(
                &mut k,
                &mut g,
                Service::AudioOut2ContextQueryMemory,
                &[ARG - 12, ARG]
            ),
            Ok(0)
        );
        let effective = if depth == 0 { 4 } else { depth };
        assert_eq!(g.u64(0), 0x1_0000 + u64::from(effective) * 0x590);
        assert_eq!(g.get(ARG + 8, 8), [0x6b; 8]);
    }
    assert_eq!((k.usage().bytes_read, k.usage().bytes_written), (16, 32));
    g.put(PAGE - 4, &7u32.to_le_bytes());
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::AudioOut2ContextQueryMemory,
            &[ARG + PAGE - 16, ARG + 64]
        ),
        Ok(0)
    );
    assert_eq!(g.u64(64), 0x1_0000 + 7 * 0x590);
}

#[test]
fn null_precedence_whole_span_permissions_and_budgets_refuse_without_outputs() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    let null_param = Err(Failure::Unsupported("null AudioOut2 context parameter"));
    assert_eq!(
        call(&mut k, &mut g, Service::AudioOut2ContextResetParam, &[0]),
        null_param
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::AudioOut2ContextQueryMemory,
            &[0, 0]
        ),
        null_param
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::AudioOut2ContextQueryMemory,
            &[u64::MAX, 0]
        ),
        Err(Failure::Unsupported("null AudioOut2 memory output"))
    );
    assert_eq!(
        call(&mut k, &mut g, Service::AudioOut2GetSpeakerInfo, &[0, 0]),
        Err(Failure::Unsupported("null AudioOut2 speaker info"))
    );
    assert_eq!(k.usage().bytes_read, 0);
    g.put(0, &[0x7c; 96]);
    g.put(PAGE - 63, &[0x33; 63]);
    assert!(matches!(
        call(
            &mut k,
            &mut g,
            Service::AudioOut2ContextResetParam,
            &[ARG + PAGE - 63]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(g.get(ARG + PAGE - 63, 63), [0x33; 63]);
    protect(&mut g, 1);
    assert_eq!(
        call(&mut k, &mut g, Service::AudioOut2GetSpeakerInfo, &[ARG, 0]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    protect(&mut g, 3);
    assert_eq!(g.get(ARG, 96), [0x7c; 96]);
    for limits in [
        Limits {
            max_attempts: 0,
            ..Default::default()
        },
        Limits {
            max_bytes: 63,
            ..Default::default()
        },
        Limits {
            max_work: 65,
            ..Default::default()
        },
    ] {
        let mut limited = kernel(limits);
        assert!(matches!(
            call(
                &mut limited,
                &mut g,
                Service::AudioOut2ContextResetParam,
                &[ARG]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(g.get(ARG, 96), [0x7c; 96]);
        assert_eq!(limited.handle_usage().accounted(), 0);
    }
    g.refuse = true;
    assert!(matches!(
        call(&mut k, &mut g, Service::AudioOut2ContextResetParam, &[ARG]),
        Err(Failure::Guest(_))
    ));
    g.refuse = false;
    assert_eq!(
        call(&mut k, &mut g, Service::AudioOut2ContextResetParam, &[ARG]),
        Ok(0)
    );
}
