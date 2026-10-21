// SPDX-License-Identifier: GPL-2.0-only
//! Original actual-owner capture and retained queue composition, no OS display.
use super::super::KernelServiceStop;
use super::*;
use crate::process::hle::{
    NativeImageLimits, unavailable::tests::original_input, with_native_image_session,
};
use pentax_kernel::{CallArgs, Service};
fn kernel(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    service: Service,
    args: [u64; 6],
) -> u64 {
    let marker = *session
        .plan
        .factory
        .markers()
        .iter()
        .find(|m| m.registration.service == service)
        .unwrap();
    session
        .kernel_stop(KernelServiceStop {
            syscall_pc: session.plan.entry(marker) + 5,
            service_id: marker.service_id.into(),
            args: CallArgs(args),
        })
        .unwrap()
}
fn call(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    function: driver::Function,
    args: [u64; 6],
    stack: Option<u64>,
) -> u64 {
    let binding = session.plan.graphics.unwrap();
    let marker = binding
        .factory
        .markers()
        .find(|m| m.function == function)
        .unwrap();
    session
        .service_request(NativeServiceRequest {
            owner_id: session.owner_id(),
            syscall_pc: binding.bias + marker.rva() + 5,
            next_pc: binding.bias + marker.rva() + 7,
            service_id: marker.id().into(),
            args,
            vector_args: None,
            stack_pointer: stack,
        })
        .unwrap()
}
fn read<const N: usize>(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    address: u64,
) -> [u8; N] {
    let mut bytes = [0; N];
    session.pause().unwrap().read(address, &mut bytes).unwrap();
    bytes
}
fn poll(session: &mut PreparedNativeKernelStartup<'_, '_>, scratch: u64, queue: u64) -> u64 {
    kernel(
        session,
        Service::WaitEqueue,
        [queue, scratch + 384, 1, scratch + 64, scratch + 72, 0],
    )
}
#[test]
fn actual_owned_flip_completion_delivers_video_events_once_after_ack_and_preserves_vblank_separation()
 {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let scratch = layout.entry.stack.address + 1024;
            session.pause().unwrap().write(scratch, &[0; 1024]).unwrap();
            session
                .pause()
                .unwrap()
                .write(scratch + 128, b"original-video-events\0")
                .unwrap();
            assert_eq!(
                kernel(
                    session,
                    Service::CreateEqueue,
                    [scratch, scratch + 128, 0, 0, 0, 0]
                ),
                0
            );
            let queue = u64::from_le_bytes(read(session, scratch));
            assert_eq!(session.video_registration_counts(), (0, 0, 0));
            assert_eq!(
                session.graphics_video_flip_observations(),
                crate::gpu::video::FlipObservations::default()
            );
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoOpen,
                    [255, 0, 0, 0, 0, 0],
                    None
                ),
                1
            );
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoAddFlipEvent,
                    [queue, 1, 17, 0, 0, 0],
                    None
                ),
                0
            );
            assert_eq!(session.video_registration_counts(), (1, 0, 0));
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoAddVblankEvent,
                    [queue, 1, 27, 0, 0, 0],
                    None
                ),
                0
            );
            assert_eq!(session.video_registration_counts(), (1, 0, 1));
            assert_eq!(
                kernel(
                    session,
                    Service::MapFlexible,
                    [scratch + 16, 131072, 0x33, 0, 0, 0]
                ),
                0
            );
            let base = u64::from_le_bytes(read(session, scratch + 16));
            let target = (base + 65535) & !65535;
            let mut descriptor = [0; 32];
            descriptor[..8].copy_from_slice(&target.to_le_bytes());
            let mut attribute = [0; 80];
            attribute[12..16].copy_from_slice(&2u32.to_le_bytes());
            attribute[16..20].copy_from_slice(&2u32.to_le_bytes());
            attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000u64.to_le_bytes());
            session
                .pause()
                .unwrap()
                .write(scratch + 192, &descriptor)
                .unwrap();
            session
                .pause()
                .unwrap()
                .write(scratch + 256, &attribute)
                .unwrap();
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoRegisterBuffers2,
                    [1, 0, 0, scratch + 192, 1, scratch + 256],
                    Some(scratch + 512)
                ),
                0
            );
            assert_eq!(session.video_registration_counts(), (1, 1, 1));
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoSubmitFlip,
                    [1, 0, 1, (-7i64) as u64, 0, 0],
                    None
                ),
                0
            );
            let metadata = session.graphics_video_peek_ready_frame().unwrap().unwrap();
            assert_eq!(
                session
                    .graphics_function_profile()
                    .count(driver::Function::VideoSubmitFlip),
                1
            );
            assert_eq!(
                session.graphics_function_profile().last(),
                "VideoSubmitFlip"
            );
            assert_eq!(
                session.graphics_video_flip_observations().ready_with_frame,
                1
            );
            assert_eq!(metadata.handle(), 1);
            assert_eq!(poll(session, scratch, queue), 0x8002_003c); // Capture is no event.
            session
                .graphics_video_observe_vblank(1, metadata.generation(), 10)
                .unwrap();
            assert!(
                session
                    .graphics_video_observe_vblank(1, metadata.generation(), 10)
                    .is_err()
            );
            assert_eq!(poll(session, scratch, queue), 0);
            assert_eq!(u64::from_le_bytes(read(session, scratch + 384)), 1);
            let frame = session.graphics_video_take_frame(10).unwrap().unwrap();
            assert_eq!(session.graphics_video_flip_observations().presenting, 1);
            let mut completion = Some(session.graphics_video_acknowledge_frame(&frame).unwrap());
            assert_eq!(session.graphics_video_flip_observations().acknowledged, 1);
            assert_eq!(session.graphics_video_flip_observations().presenting, 0);
            assert_eq!(poll(session, scratch, queue), 0x8002_003c); // Ack alone does not deliver.
            session
                .graphics_video_deliver_completion(&mut completion)
                .unwrap();
            assert!(completion.is_none());
            assert!(
                session
                    .graphics_video_deliver_completion(&mut completion)
                    .is_err()
            );
            assert_eq!(poll(session, scratch, queue), 0);
            let event = read::<32>(session, scratch + 384);
            assert_eq!(u64::from_le_bytes(event[..8].try_into().unwrap()), 0);
            assert_eq!(i16::from_le_bytes(event[8..10].try_into().unwrap()), -13);
            assert_eq!(u64::from_le_bytes(event[24..].try_into().unwrap()), 17);
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoGetEventData,
                    [scratch + 384, scratch + 80, 0, 0, 0, 0],
                    None
                ),
                0
            );
            assert_eq!(i64::from_le_bytes(read(session, scratch + 80)), -7);
            assert_eq!(poll(session, scratch, queue), 0x8002_003c);
            assert_eq!(session.video_registration_counts(), (1, 1, 1));
            assert_eq!(
                call(
                    session,
                    driver::Function::VideoClose,
                    [1, 0, 0, 0, 0, 0],
                    None
                ),
                0
            );
            assert_eq!(session.video_registration_counts(), (0, 0, 0));
            assert_eq!(poll(session, scratch, queue), 0x8002_003c);
            assert_eq!(
                session.graphics_video_flip_observations(),
                crate::gpu::video::FlipObservations::default()
            );
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
