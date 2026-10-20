// SPDX-License-Identifier: GPL-2.0-only

use super::*;

fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    a: u64,
    b: u64,
) -> Result<u64, Failure> {
    owner.invoke_library(service, CallArgs([a, b, 0, 0, 0, 0]), guest)
}

#[test]
fn affinity_targets_owned_threads_and_preserves_full_width_masks_across_selection() {
    use Service::*;
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(1).unwrap();
    let main = call(&mut owner, &mut guest, PthreadSelf, 0, 0).unwrap();
    owner
        .register_thread_context(2, ARG + 256, 700, 1u64 << 63, &mut guest)
        .unwrap();
    guest.put(32, &[0xa5; 16]);
    assert_eq!(
        call(&mut owner, &mut guest, PthreadGetaffinity, main, ARG + 32),
        Ok(0)
    );
    assert_eq!(guest.u64(32), 0x7f);
    assert_eq!(guest.u64(40), 0xa5a5_a5a5_a5a5_a5a5);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            PthreadGetaffinity,
            ARG + 256,
            ARG + 32
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(32), 1u64 << 63);
    let mask = 0xfedc_ba98_7654_3210;
    assert_eq!(
        call(&mut owner, &mut guest, PthreadSetaffinity, ARG + 256, mask),
        Ok(0)
    );
    owner.activate_thread_context(2).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, PthreadSelf, 0, 0),
        Ok(ARG + 256)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            PthreadGetaffinity,
            ARG + 256,
            ARG + 32
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(32), mask);
    assert_eq!(
        call(&mut owner, &mut guest, PthreadSetaffinity, main, 0),
        Ok(0)
    );
    owner.activate_thread_context(1).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, PthreadGetaffinity, main, ARG + 32),
        Ok(0)
    );
    assert_eq!(guest.u64(32), 0);
    assert_eq!(owner.threads.active().affinity, 0);
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "bt3CTBKmGyI"),
        Some(PthreadSetaffinity)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "rcrVFJsQWRY"),
        Some(PthreadGetaffinity)
    );
    assert_eq!(
        resolve("libkernel", (1, 1), "Posix", 1, "bt3CTBKmGyI"),
        None
    );
    assert_eq!(
        resolve("libkernel", (1, 2), "libkernel", 1, "rcrVFJsQWRY"),
        None
    );
}

#[test]
fn affinity_errors_output_faults_and_work_refusal_do_not_mutate_thread_state() {
    use Service::*;
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    let main = call(&mut owner, &mut guest, PthreadSelf, 0, 0).unwrap();
    for service in [PthreadSetaffinity, PthreadGetaffinity] {
        assert_eq!(call(&mut owner, &mut guest, service, 0, 0), Ok(0x8002_0003));
        assert!(matches!(
            call(&mut owner, &mut guest, service, u64::MAX, ARG),
            Err(Failure::Unsupported(_))
        ));
    }
    assert_eq!(
        call(&mut owner, &mut guest, PthreadGetaffinity, main, 0),
        Ok(0x8002_0016)
    );
    guest.put(32, &[0xa5; 16]);
    guest.refuse = true;
    assert_eq!(
        call(&mut owner, &mut guest, PthreadGetaffinity, main, ARG + 32),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    guest.refuse = false;
    assert_eq!(guest.u64(32), 0xa5a5_a5a5_a5a5_a5a5);
    assert_eq!(owner.threads.active().affinity, 0x7f);
    let mut foreign = kernel(Default::default());
    assert!(matches!(
        call(&mut foreign, &mut guest, PthreadSetaffinity, main, 3),
        Err(Failure::Unsupported(_))
    ));
    let used = owner.usage().work;
    owner
        .budget
        .work(Limits::default().max_work - used)
        .unwrap();
    assert!(matches!(
        call(&mut owner, &mut guest, PthreadSetaffinity, main, 3),
        Err(Failure::Limit(_))
    ));
    assert_eq!(owner.threads.active().affinity, 0x7f);
}
