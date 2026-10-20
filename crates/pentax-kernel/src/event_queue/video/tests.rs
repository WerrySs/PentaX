// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::tests::{ARG, TestGuest, kernel};
fn setup() -> (crate::Kernel, TestGuest, u64) {
    let mut kernel = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(64, &[0; 4]);
    guest.put(128, b"original-video-events\0");
    assert_eq!(
        kernel.invoke_library(
            Service::CreateEqueue,
            CallArgs([ARG, ARG + 128, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0)
    );
    let queue = guest.u64(0);
    (kernel, guest, queue)
}
fn poll(kernel: &mut crate::Kernel, guest: &mut TestGuest, queue: u64) -> Result<u64, Failure> {
    kernel.invoke_library(
        Service::WaitEqueue,
        CallArgs([queue, ARG + 512, 2, ARG + 32, ARG + 64, 0]),
        guest,
    )
}
#[test]
fn real_video_subscriptions_coalesce_exact_payload_and_keep_source_active_order() {
    let (mut kernel, mut guest, queue) = setup();
    kernel
        .add_video_event(queue, 1, 9, VideoEventKind::Flip, 17)
        .unwrap();
    kernel
        .add_video_event(queue, 1, 9, VideoEventKind::Vblank, 27)
        .unwrap();
    {
        let _uncommitted = kernel
            .prepare_video_interrupt(1, 9, VideoEventKind::Flip, 99, 0x3456)
            .unwrap();
    }
    assert_eq!(poll(&mut kernel, &mut guest, queue), Ok(0x8002_003c));
    kernel
        .prepare_video_interrupt(1, 9, VideoEventKind::Flip, u64::MAX - 6, 0x3456)
        .unwrap()
        .complete();
    kernel
        .prepare_video_interrupt(1, 9, VideoEventKind::Vblank, 41, 0x4567)
        .unwrap()
        .complete();
    for _ in 0..20 {
        kernel
            .prepare_video_interrupt(1, 9, VideoEventKind::Flip, u64::MAX - 6, 0x5678)
            .unwrap()
            .complete();
    }
    kernel
        .add_video_event(queue, 1, 9, VideoEventKind::Flip, 37)
        .unwrap();
    assert_eq!(poll(&mut kernel, &mut guest, queue), Ok(0));
    assert_eq!(guest.get(ARG + 32, 4), 2u32.to_le_bytes());
    let flip = guest.get(ARG + 512, 32);
    let blank = guest.get(ARG + 544, 32);
    assert_eq!(u64::from_le_bytes(flip[..8].try_into().unwrap()), 0);
    assert_eq!(i16::from_le_bytes(flip[8..10].try_into().unwrap()), -13);
    assert_eq!(u32::from_le_bytes(flip[12..16].try_into().unwrap()), 15);
    assert_eq!(
        u64::from_le_bytes(flip[16..24].try_into().unwrap()),
        0xffff_ffff_fff9_f678
    );
    assert_eq!(u64::from_le_bytes(flip[24..].try_into().unwrap()), 37);
    assert_eq!(u64::from_le_bytes(blank[..8].try_into().unwrap()), 1);
    assert_eq!(
        u64::from_le_bytes(blank[16..24].try_into().unwrap()),
        (41 << 16) | 0x1567
    );
    assert_eq!(poll(&mut kernel, &mut guest, queue), Ok(0x8002_003c));
}
#[test]
fn output_quota_generation_and_close_refusals_do_not_consume_or_reorder_notifications() {
    let (mut kernel, mut guest, queue) = setup();
    kernel
        .add_video_event(queue, 1, 9, VideoEventKind::Flip, 17)
        .unwrap();
    let handles = kernel.usage().handles_live;
    assert!(matches!(
        kernel.add_video_event(queue, 2, 9, VideoEventKind::Flip, 99),
        Err(Failure::Unsupported("video event queue port collision"))
    ));
    assert_eq!(kernel.usage().handles_live, handles);
    kernel
        .prepare_video_interrupt(1, 10, VideoEventKind::Flip, 90, 1)
        .unwrap()
        .complete();
    assert_eq!(poll(&mut kernel, &mut guest, queue), Ok(0x8002_003c));
    kernel
        .prepare_video_interrupt(1, 9, VideoEventKind::Flip, 7, 2)
        .unwrap()
        .complete();
    guest.put(512, &[0xa7; 32]);
    guest.refuse = true;
    assert!(poll(&mut kernel, &mut guest, queue).is_err());
    assert_eq!(guest.get(ARG + 512, 32), [0xa7; 32]);
    guest.refuse = false;
    assert_eq!(poll(&mut kernel, &mut guest, queue), Ok(0));
    let before = kernel.budget.limits.max_work;
    kernel.budget.limits.max_work = kernel.usage().work;
    assert!(
        kernel
            .prepare_video_interrupt(1, 9, VideoEventKind::Flip, 8, 3)
            .is_err()
    );
    kernel.budget.limits.max_work = before;
    assert_eq!(poll(&mut kernel, &mut guest, queue), Ok(0x8002_003c));
    {
        let _uncommitted = kernel.prepare_video_close(1, 9).unwrap();
    }
    assert_eq!(kernel.usage().handles_live, handles);
    kernel.prepare_video_close(1, 9).unwrap().complete();
    assert_eq!(kernel.usage().handles_live, handles - 1);
    assert_eq!(
        kernel.delete_video_event(queue, VideoEventKind::Flip),
        Ok(0)
    );
}
