// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn creation_publishes_entry_frame_and_handle_as_one_checked_write() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(ARG + 512).unwrap();
    owner
        .register_thread_context(ARG + 1024, ARG + 256, 700, 0x7f, &mut guest)
        .unwrap();
    guest.put(0, &[0xa5; 8]);
    guest.put(64, &[0x85; 8]);
    let frame = 0x1122_3344_5566_7788_u64.to_le_bytes();
    let create = |owner: &mut Kernel, guest: &TestGuest, output| {
        let PthreadCreateAction::Create(request) = owner
            .admit_pthread_create(
                Service::PthreadCreate,
                CallArgs([output, 0, ARG + 128, 0, 0, 0]),
                guest,
            )
            .unwrap()
        else {
            panic!("expected admitted creation");
        };
        request
    };
    let request = create(&mut owner, &guest, ARG);
    let publish = |owner: &mut Kernel, guest: &mut TestGuest, request: &AdmittedPthreadCreate| {
        owner.publish_pthread_create(
            request,
            ARG + 1024,
            ARG + 256,
            GuestWrite {
                address: ARG + 64,
                bytes: &frame,
            },
            guest,
        )
    };
    guest.refuse = true;
    assert_eq!(
        publish(&mut owner, &mut guest, &request),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    let bad_output = create(&mut owner, &guest, ARG + PAGE - 4);
    assert!(publish(&mut owner, &mut guest, &bad_output).is_err());
    let overlapping = create(&mut owner, &guest, ARG + 67);
    assert!(matches!(
        publish(&mut owner, &mut guest, &overlapping),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(guest.get(ARG, 8), [0xa5; 8]);
    assert_eq!(guest.get(ARG + 64, 8), [0x85; 8]);
    publish(&mut owner, &mut guest, &request).unwrap();
    assert_eq!(guest.u64(0), ARG + 256);
    assert_eq!(guest.get(ARG + 64, 8), frame);
}
