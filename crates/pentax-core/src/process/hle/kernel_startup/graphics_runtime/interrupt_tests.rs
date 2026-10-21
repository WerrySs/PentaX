// SPDX-License-Identifier: GPL-2.0-only
//! Original owned command/shader data, real Vulkan work and live queue delivery.
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
) -> Result<u64, KernelStartupError> {
    let marker = *session
        .plan
        .factory
        .markers()
        .iter()
        .find(|m| m.registration.service == service)
        .unwrap();
    session.kernel_stop(KernelServiceStop {
        syscall_pc: session.plan.entry(marker) + 5,
        service_id: marker.service_id.into(),
        args: CallArgs(args),
    })
}
fn graphics(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    function: driver::Function,
    args: [u64; 6],
) -> Result<u64, KernelStartupError> {
    let binding = session.plan.graphics.unwrap();
    let marker = binding
        .factory
        .markers()
        .find(|m| m.function == function)
        .unwrap();
    session.service_request(NativeServiceRequest {
        owner_id: session.owner_id(),
        syscall_pc: binding.bias + marker.rva() + 5,
        next_pc: binding.bias + marker.rva() + 7,
        service_id: marker.id().into(),
        args,
        vector_args: None,
        stack_pointer: None,
    })
}
fn write(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64, bytes: &[u8]) {
    session.hle.pause().unwrap().write(address, bytes).unwrap();
}
fn words(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64, words: &[u32]) {
    write(
        session,
        address,
        &words
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect::<Vec<_>>(),
    );
}
fn read<const N: usize>(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    address: u64,
) -> [u8; N] {
    let mut output = [0; N];
    session
        .hle
        .pause()
        .unwrap()
        .read(address, &mut output)
        .unwrap();
    output
}
fn set(stream: &mut Vec<u32>, reg: u32, values: &[u32]) {
    stream.extend([0xc000_7602 | ((values.len() as u32) << 16), reg]);
    stream.extend(values);
}
fn release(stream: &mut Vec<u32>, address: u64, value: u64, context: u32, selector: u32) {
    stream.extend([
        0xc006_1060,
        0x62f,
        0x4000_0000 | (selector << 24),
        address as u32,
        (address >> 32) as u32,
        value as u32,
        (value >> 32) as u32,
        context,
    ]);
}
fn submit(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    base: u64,
    stream: &[u32],
) -> Result<u64, KernelStartupError> {
    words(session, base + 0x100, stream);
    write(session, base + 0xa00, &(base + 0x100).to_le_bytes());
    words(session, base + 0xa08, &[stream.len() as u32, 0]);
    graphics(
        session,
        driver::Function::Acb,
        [0x20, base + 0xa00, 0, 0, 0, 0],
    )
}
fn poll(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    scratch: u64,
    queue: u64,
) -> Result<u64, KernelStartupError> {
    kernel(
        session,
        Service::WaitEqueue,
        [queue, scratch + 0x100, 4, scratch + 0x30, scratch + 8, 0],
    )
}
fn setup(test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64, u64, u64)) {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let scratch = layout.entry.stack.address + 0x200;
            write(session, scratch, &[0; 64]);
            write(session, scratch + 0x80, b"original-graphics-events\0");
            assert_eq!(
                kernel(
                    session,
                    Service::CreateEqueue,
                    [scratch, scratch + 0x80, 0, 0, 0, 0]
                ),
                Ok(0)
            );
            let queue = u64::from_le_bytes(read(session, scratch));
            assert_eq!(
                kernel(
                    session,
                    Service::MapFlexible,
                    [scratch + 0x10, 0x4000, 0x33, 0, 0, 0]
                ),
                Ok(0)
            );
            let base = u64::from_le_bytes(read(session, scratch + 0x10));
            assert_eq!(
                graphics(
                    session,
                    driver::Function::AgcAddEqEvent,
                    [queue, 0x20, 0x1234, 0, 0, 0]
                ),
                Ok(0)
            );
            assert_eq!(poll(session, scratch, queue), Ok(0x8002_003c));
            test(session, scratch, queue, base);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn fenced_compute_then_label_and_interrupt_deliver_actual_ordered_coalesced_state() {
    setup(|session, scratch, queue, base| {
        let shader = base + 0x400;
        let data = base + 0x800;
        words(
            session,
            shader,
            &[
                (0x25 << 25) | (1 << 17) | 4,
                (0x1a << 25) | (2 << 17) | 130,
                0xe070_1000,
                (128 << 24) | (1 << 8) | 2,
                0xbf81_0000,
            ],
        );
        write(session, data, &[0xa5; 48]);
        write(session, base + 0x900, &[0xa5; 32]);
        let mut stream = Vec::new();
        set(&mut stream, 0x207, &[8, 1, 1]);
        set(
            &mut stream,
            0x20c,
            &[(shader >> 8) as u32, (shader >> 40) as u32],
        );
        set(&mut stream, 0x212, &[0x2c_0000, 10]);
        set(&mut stream, 0x228, &[0]);
        set(
            &mut stream,
            0x240,
            &[data as u32, (data >> 32) as u32, 48, 0, 17],
        );
        stream.extend([0xc003_1502, 1, 1, 1, 0x8005]);
        release(
            &mut stream,
            base + 0x900,
            0x1234_5678_89ab_cdef,
            0xf000_0011,
            2,
        );
        stream.extend([0xc006_4900, 0x628, 4 << 24, 0, 0, 0, 0, 19]); // Pure interrupt.
        stream.extend([
            0xc006_4902,
            0x62f,
            3 << 29,
            (base + 0x908) as u32,
            ((base + 0x908) >> 32) as u32,
            0,
            0,
            0,
        ]);
        assert_eq!(submit(session, base, &stream), Ok(0));
        let data = read::<48>(session, data);
        for i in 0..8 {
            assert_eq!(
                u32::from_le_bytes(data[i * 4..i * 4 + 4].try_into().unwrap()),
                17 + i as u32
            );
        }
        assert_eq!(&data[32..], &[0xa5; 16]);
        assert_eq!(
            u64::from_le_bytes(read(session, base + 0x900)),
            0x1234_5678_89ab_cdef
        );
        assert_ne!(u64::from_le_bytes(read(session, base + 0x908)), 0);
        assert_eq!(read::<16>(session, base + 0x910), [0xa5; 16]);
        assert_eq!(session.graphics_usage().dispatches, 1);
        assert_eq!(session.graphics_usage().interrupts, 2);
        assert_eq!(poll(session, scratch, queue), Ok(0));
        let event = read::<32>(session, scratch + 0x100);
        assert_eq!(&event[..8], &0x20_u64.to_le_bytes());
        assert_eq!(&event[8..12], &[0xf2, 0xff, 0, 0]);
        assert_eq!(&event[12..16], &2_u32.to_le_bytes());
        assert_eq!(&event[16..24], &19_u64.to_le_bytes());
        assert_eq!(&event[24..], &0x1234_u64.to_le_bytes());
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcGetEqEventType,
                [scratch + 0x100, 0, 0, 0, 0, 0]
            ),
            Ok(0x20)
        );
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcGetEqContextId,
                [scratch + 0x100, 0, 0, 0, 0, 0]
            ),
            Ok(19)
        );
        assert_eq!(poll(session, scratch, queue), Ok(0x8002_003c));
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcDeleteEqEvent,
                [queue, 0x20, 0, 0, 0, 0]
            ),
            Ok(0)
        );
    });
}

#[test]
fn refused_label_drops_notification_and_completed_prefix_cannot_be_replayed() {
    for prefix in [false, true] {
        setup(|session, scratch, queue, base| {
            write(session, base + 0x900, &[0xa5; 16]);
            let mut stream = Vec::new();
            if prefix {
                release(&mut stream, base + 0x900, 7, 11, 1);
            }
            release(&mut stream, base + 0x5000, 9, 13, 2); // Unmapped label.
            assert_eq!(
                submit(session, base, &stream),
                Err(KernelStartupError::Graphics(driver::Failure::Memory))
            );
            assert_eq!(session.graphics_usage().interrupts, u64::from(prefix));
            if prefix {
                assert_eq!(u64::from_le_bytes(read(session, base + 0x900)), 7);
                assert_eq!(poll(session, scratch, queue), Ok(0));
                assert_eq!(
                    &read::<32>(session, scratch + 0x100)[12..24],
                    &[1, 0, 0, 0, 11, 0, 0, 0, 0, 0, 0, 0]
                );
                assert_eq!(
                    submit(session, base, &stream),
                    Err(KernelStartupError::Graphics(driver::Failure::Poisoned))
                );
                assert_eq!(poll(session, scratch, queue), Ok(0x8002_003c));
            } else {
                assert_eq!(read::<16>(session, base + 0x900), [0xa5; 16]);
                assert_eq!(poll(session, scratch, queue), Ok(0x8002_003c));
                let mut valid = Vec::new();
                release(&mut valid, base + 0x900, 17, 19, 4);
                assert_eq!(submit(session, base, &valid), Ok(0));
                assert_eq!(poll(session, scratch, queue), Ok(0));
                // Public RELEASE_MEM INT_SEL4 delivers its interrupt while
                // dropping the label on both processor queues.
                assert_eq!(read::<16>(session, base + 0x900), [0xa5; 16]);
            }
        });
    }
}

#[test]
fn graphics_event_getters_select_exact_fields_for_graphics_and_other_filters() {
    setup(|session, _, _, base| {
        let event = base + 0x4000 - 20;
        write(session, event, &0xfedc_ba98_1234_5678_u64.to_le_bytes());
        write(session, event + 8, &(-11_i16).to_le_bytes());
        write(session, event + 16, &0x89ab_cdef_u32.to_le_bytes());
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcGetEqEventType,
                [event, 0, 0, 0, 0, 0]
            ),
            Ok(0x89ab_cdef)
        );
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcGetEqContextId,
                [event, 0, 0, 0, 0, 0]
            ),
            Ok(0x1234_5678)
        );
        let event = base + 0x4000 - 12;
        write(session, event, &0x8765_4321_u64.to_le_bytes());
        write(session, event + 8, &(-14_i16).to_le_bytes());
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcGetEqEventType,
                [event, 0, 0, 0, 0, 0]
            ),
            Ok(0x8765_4321)
        );
        assert_eq!(
            graphics(
                session,
                driver::Function::AgcGetEqContextId,
                [event, 0, 0, 0, 0, 0]
            ),
            Err(KernelStartupError::Graphics(driver::Failure::Memory))
        );
    });
}

#[test]
fn completed_multi_buffer_prefix_stays_poisoned_after_later_buffer_refusal() {
    setup(|session, scratch, queue, base| {
        let mut first = Vec::new();
        release(&mut first, base + 0x900, 7, 11, 2);
        let mut second = Vec::new();
        release(&mut second, base + 0x5000, 9, 13, 2);
        words(session, base + 0x100, &first);
        words(session, base + 0x200, &second);
        write(session, base + 0xa00, &(base + 0x100).to_le_bytes());
        write(session, base + 0xa08, &(base + 0x200).to_le_bytes());
        words(session, base + 0xa20, &[8, 8]);
        let args = [0x20, base + 0xa00, base + 0xa20, 2, 0, 0];
        assert_eq!(
            graphics(session, driver::Function::MultiAcbs, args),
            Err(KernelStartupError::Graphics(driver::Failure::Memory))
        );
        assert_eq!(poll(session, scratch, queue), Ok(0));
        assert_eq!(
            graphics(session, driver::Function::MultiAcbs, args),
            Err(KernelStartupError::Graphics(driver::Failure::Poisoned))
        );
        assert_eq!(poll(session, scratch, queue), Ok(0x8002_003c));
    });
}
