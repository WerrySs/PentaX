// SPDX-License-Identifier: GPL-2.0-only
//! Original data-only device snapshots, requests and atomic guest outputs.
use super::*;

const POINTER_SERVICES: [Service; 6] = [
    Service::PadGetTriggerEffectState,
    Service::PadSetVibration,
    Service::PadRead,
    Service::PadGetControllerInformation,
    Service::PadSetTriggerEffect,
    Service::PadSetLightBar,
];

#[test]
fn disconnected_reads_serialize_exact_records_and_keep_owned_initialization_time() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    for service in POINTER_SERVICES {
        assert_eq!(call(&mut k, &mut g, service, &[2, 0, 0]), Ok(0x8092_0003));
        assert_eq!(call(&mut k, &mut g, service, &[1, 0, 1]), Ok(0x8092_0001));
    }
    for count in [0, u64::MAX, 0x8000_0000] {
        assert_eq!(
            call(&mut k, &mut g, Service::PadRead, &[1, ARG, count]),
            Ok(0x8092_0001)
        );
    }
    assert!(matches!(
        call(&mut k, &mut g, Service::PadRead, &[1, ARG, 1]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(call(&mut k, &mut g, Service::PadInit, &[]), Ok(0));
    g.put(0, &[0xa5; 244]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PadRead,
            &[0xffff_ffff_0000_0001, ARG + 1, i32::MAX as u64]
        ),
        Ok(1)
    );
    let first = g.get(ARG + 1, 120);
    let mut expected = [0; 120];
    expected[4..8].fill(128);
    expected[24..28].copy_from_slice(&1f32.to_le_bytes());
    expected[32..36].copy_from_slice(&1f32.to_le_bytes());
    let timestamp = u64::from_le_bytes(first[80..88].try_into().unwrap());
    expected[80..88].copy_from_slice(&timestamp.to_le_bytes());
    assert_eq!(first, expected);
    assert_eq!(g.get(ARG, 1), [0xa5]);
    assert_eq!(g.get(ARG + 121, 123), [0xa5; 123]);
    assert_eq!(
        call(&mut k, &mut g, Service::PadInit, &[u64::MAX; 6]),
        Ok(0)
    );
    assert_eq!(
        call(&mut k, &mut g, Service::PadRead, &[1, ARG + 122, 2]),
        Ok(1)
    );
    assert_eq!(g.get(ARG + 122, 120), expected);
    assert!(k.pad_monotonic_microseconds().unwrap() >= timestamp);
    g.put(300, &[0x5a; 30]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PadGetControllerInformation,
            &[1, ARG + 301]
        ),
        Ok(0)
    );
    let mut info = [0; 28];
    info[..4].copy_from_slice(&44.86f32.to_le_bytes());
    info[4..6].copy_from_slice(&1920u16.to_le_bytes());
    info[6..8].copy_from_slice(&1080u16.to_le_bytes());
    info[8..10].fill(2);
    assert_eq!(g.get(ARG + 301, 28), info);
    assert_eq!(
        (g.get(ARG + 300, 1), g.get(ARG + 329, 1)),
        (vec![0x5a], vec![0x5a])
    );
    assert!(!k.user_service_initialized());
    assert_eq!(k.usage().handles_live, 0);
}

#[test]
fn real_input_queue_preserves_failed_delivery_and_retains_only_newest_64_changes() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    k.configure_pad_information(PadControllerInformation {
        device_class: 7,
        ..Default::default()
    })
    .unwrap();
    call(&mut k, &mut g, Service::PadInit, &[]).unwrap();
    let now = k.pad_monotonic_microseconds().unwrap() + 1;
    let mut snapshot = PadInputSnapshot {
        buttons: 0x1020,
        sticks: [1, 2, 3, 4],
        triggers: [5, 6],
        orientation: [0.0, 0.0, 1.0, 0.0],
        acceleration: [1.0, 2.0, 3.0],
        angular_velocity: [-1.0, -2.0, -3.0],
        touches: [
            PadTouch {
                x: 123,
                y: 456,
                id: 9,
            },
            PadTouch {
                x: 789,
                y: 321,
                id: 10,
            },
        ],
        touch_count: 2,
        connected: true,
        connected_count: 1,
        timestamp: now,
        extension_id: 0x1234,
        extension_length: 2,
        extension: [8; 10],
        unique_length: 3,
        unique: [7; 12],
    };
    k.publish_pad_snapshot(snapshot).unwrap();
    k.publish_pad_snapshot(snapshot).unwrap(); // No duplicate input event.
    for bad in [
        PadInputSnapshot {
            timestamp: now - 1,
            ..snapshot
        },
        PadInputSnapshot {
            touch_count: 3,
            ..snapshot
        },
        PadInputSnapshot {
            orientation: [f32::NAN; 4],
            ..snapshot
        },
    ] {
        assert!(k.publish_pad_snapshot(bad).is_err());
    }
    g.put(0, &[0xa5; 241]);
    g.refuse = true;
    assert!(call(&mut k, &mut g, Service::PadRead, &[1, ARG + 1, 2]).is_err());
    g.refuse = false;
    assert_eq!(g.get(ARG, 241), [0xa5; 241]);
    assert_eq!(
        call(&mut k, &mut g, Service::PadRead, &[1, ARG + 1, 2]),
        Ok(1)
    );
    let bytes = g.get(ARG + 1, 120);
    assert_eq!(&bytes[..10], &[0x20, 0x10, 0, 0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(&bytes[60..65], &[123, 0, 200, 1, 9]);
    assert_eq!(&bytes[68..73], &[21, 3, 65, 1, 10]);
    assert_eq!((bytes[76], bytes[104]), (1, 1));
    assert_eq!(&bytes[80..88], &now.to_le_bytes());
    assert_eq!(&bytes[94..104], &[8, 8, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[108..120], &[7, 7, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(g.get(ARG + 121, 120), [0xa5; 120]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PadGetControllerInformation,
            &[1, ARG + 300]
        ),
        Ok(0)
    );
    assert_eq!(g.get(ARG + 311, 2), [1, 1]);
    assert_eq!(g.get(ARG + 316, 4), 7i32.to_le_bytes());
    assert!(k.configure_pad_information(Default::default()).is_err());
    for index in 1..=65 {
        snapshot.timestamp = now + index;
        snapshot.buttons = index as u32;
        k.publish_pad_snapshot(snapshot).unwrap();
    }
    assert_eq!(
        call(&mut k, &mut g, Service::PadRead, &[1, ARG, 32]),
        Ok(32)
    );
    assert_eq!(g.get(ARG, 4), 2u32.to_le_bytes());
    assert_eq!(g.get(ARG + 31 * 120, 4), 33u32.to_le_bytes());
    assert_eq!(
        call(&mut k, &mut g, Service::PadRead, &[1, ARG, 64]),
        Ok(32)
    );
    assert_eq!(g.get(ARG, 4), 34u32.to_le_bytes());
    assert_eq!(g.get(ARG + 31 * 120, 4), 65u32.to_le_bytes());
    assert_eq!(call(&mut k, &mut g, Service::PadRead, &[1, ARG, 64]), Ok(1));
    assert_eq!(g.get(ARG, 4), 65u32.to_le_bytes());
}

#[test]
fn setters_keep_real_desired_outputs_and_validate_entire_trigger_input_first() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    assert!(k.pad_output().tilt_correction);
    assert_eq!(k.pad_output().tilt_reset_sequence, 0);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PadSetTiltCorrectionState,
            &[1, 0x100]
        ),
        Ok(0)
    );
    assert!(!k.pad_output().tilt_correction);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::PadSetTiltCorrectionState,
            &[1, 0x101]
        ),
        Ok(0)
    );
    assert!(k.pad_output().tilt_correction);
    assert_eq!(k.pad_output().tilt_reset_sequence, 2);
    assert_eq!(
        call(&mut k, &mut g, Service::PadSetTiltCorrectionState, &[2, 0]),
        Ok(0x8092_0003)
    );
    assert_eq!(k.pad_output().tilt_reset_sequence, 2);
    g.put(PAGE - 2, &[17, 29]);
    call(
        &mut k,
        &mut g,
        Service::PadSetVibration,
        &[1, ARG + PAGE - 2],
    )
    .unwrap();
    assert_eq!(
        (k.pad_output().vibration, k.pad_output().sequence),
        ([17, 29], 1)
    );
    call(
        &mut k,
        &mut g,
        Service::PadSetVibration,
        &[1, ARG + PAGE - 2],
    )
    .unwrap();
    assert_eq!(k.pad_output().sequence, 1);
    g.put(PAGE - 3, &[31, 37, 41]);
    call(
        &mut k,
        &mut g,
        Service::PadSetLightBar,
        &[1, ARG + PAGE - 3],
    )
    .unwrap();
    assert_eq!(
        (k.pad_output().light_bar, k.pad_output().light_bar_valid),
        ([31, 37, 41], true)
    );
    let mut command = [0; 120];
    command[0] = 3;
    command[8..12].copy_from_slice(&1u32.to_le_bytes());
    command[16..18].copy_from_slice(&[8, 2]);
    command[64..68].copy_from_slice(&2u32.to_le_bytes());
    command[72..75].copy_from_slice(&[2, 5, 8]);
    g.put(0, &command);
    call(&mut k, &mut g, Service::PadSetTriggerEffect, &[1, ARG]).unwrap();
    let output = k.pad_output();
    assert_eq!(output.sequence, 4);
    assert_eq!(
        output.triggers[0].effect,
        [0x21, 0, 3, 0, 0, 0, 9, 0, 0, 0, 0]
    );
    assert_eq!(output.triggers[0].fallback, 64);
    assert_eq!(
        output.triggers[1].effect,
        [0x25, 0x24, 0, 7, 0, 0, 0, 0, 0, 0, 0]
    );
    assert_eq!(output.triggers[1].fallback, 255);
    for (mask, right) in [(4, 2), (1, 7), (0, 7)] {
        command[0] = mask;
        command[64..68].copy_from_slice(&(right as u32).to_le_bytes());
        g.put(0, &command);
        assert_eq!(
            call(&mut k, &mut g, Service::PadSetTriggerEffect, &[1, ARG]),
            Ok(0x8092_0001)
        );
        assert_eq!(k.pad_output(), output);
    }
    g.put(200, &[0xa5; 10]);
    call(
        &mut k,
        &mut g,
        Service::PadGetTriggerEffectState,
        &[1, ARG + 201],
    )
    .unwrap();
    assert_eq!(g.get(ARG + 200, 10), [0xa5, 0, 0, 0, 0, 0, 0, 0, 0, 0xa5]);
    assert_eq!(k.pad_output(), output); // No invented acknowledgement.
    for service in [
        Service::PadSetVibration,
        Service::PadSetLightBar,
        Service::PadSetTriggerEffect,
    ] {
        assert!(call(&mut k, &mut g, service, &[1, ARG + PAGE - 1]).is_err());
        assert_eq!(k.pad_output(), output);
    }
}

#[test]
fn controller_budget_refusals_preserve_output_state_and_queued_input() {
    for (limits, service) in [
        (
            Limits {
                max_work: 1,
                ..Default::default()
            },
            Service::PadSetTiltCorrectionState,
        ),
        (
            Limits {
                max_bytes: 1,
                ..Default::default()
            },
            Service::PadSetVibration,
        ),
    ] {
        let mut k = kernel(limits);
        let mut g = TestGuest::new();
        let before = k.pad_output();
        assert!(call(&mut k, &mut g, service, &[1, ARG]).is_err());
        assert_eq!(k.pad_output(), before);
    }
    let mut k = kernel(Limits {
        max_scratch_bytes: 119,
        max_io_bytes: 119,
        max_name_bytes: 119,
        ..Default::default()
    });
    let mut g = TestGuest::new();
    call(&mut k, &mut g, Service::PadInit, &[]).unwrap();
    let timestamp = k.pad_monotonic_microseconds().unwrap() + 1;
    k.publish_pad_snapshot(PadInputSnapshot {
        buttons: 42,
        timestamp,
        ..Default::default()
    })
    .unwrap();
    g.put(0, &[0xa5; 120]);
    assert!(matches!(
        call(&mut k, &mut g, Service::PadRead, &[1, ARG, 1]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(g.get(ARG, 120), [0xa5; 120]);
}
