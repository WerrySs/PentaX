// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;

const NAME: u64 = ARG + 0x300;
const OPTION: u64 = ARG + 0x400;
const INFO: u64 = ARG + 0x500;
const CONTEXT: u64 = ARG + 0x1000;
const STACK: u64 = ARG + 0x2000;
fn args() -> CallArgs {
    CallArgs([ARG, NAME, 0x1234_5678_1234, u64::MAX - 3, CONTEXT, 512])
}
fn extra(guest: &mut TestGuest, values: [u64; 3]) {
    for (index, value) in values.into_iter().enumerate() {
        guest.put(0x2008 + index as u64 * 8, &value.to_le_bytes());
    }
}
fn invoke(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}

#[test]
fn fiber_initialization_retains_exact_abi_context_and_full_stack_arguments() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0x300, &[0x85; 31]); // Non-UTF8, no NUL: source truncates at31 bytes.
    guest.put(0x1000, &[0xa5; 512]);
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberOptParamInitialize, &[OPTION]),
        Ok(0)
    );
    assert_eq!(guest.get(OPTION, 128)[..4], 0xbb40_e64d_u32.to_le_bytes());
    assert!(guest.get(OPTION, 128)[4..].iter().all(|byte| *byte == 0));
    extra(&mut guest, [OPTION, 0xffff_ffff_0350_0000, 0]);
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0)
    );
    assert_eq!(guest.u64(0), 0x0000_0002_def1_649c);
    assert_eq!(guest.u64(8), args().0[2]);
    assert_eq!(guest.u64(16), u64::MAX - 3);
    assert_eq!(guest.u64(24), CONTEXT);
    assert_eq!(guest.u64(32), 512);
    assert_eq!(guest.get(ARG + 40, 32), [vec![0x85; 31], vec![0]].concat());
    assert_eq!(guest.u64(72), 0);
    assert_eq!(guest.u64(80), 0x100);
    assert_eq!(guest.u64(88), CONTEXT);
    assert_eq!(guest.u64(96), CONTEXT + 512);
    assert_eq!(guest.get(ARG + 104, 96), [0; 96]);
    assert_eq!(guest.u64(200), 0xb375_92a0_0000_0000);
    assert_eq!(
        guest.get(CONTEXT, 8),
        0x7149_f2ca_7149_f2ca_u64.to_le_bytes()
    );
    assert_eq!(guest.get(CONTEXT + 8, 504), [0xa5; 504]);
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberGetInfo, &[ARG, INFO]),
        Ok(0)
    );
    assert_eq!(guest.u64(0x500), 128);
    assert_eq!(guest.get(INFO + 8, 64), guest.get(ARG + 8, 64));
    assert_eq!(guest.get(INFO + 72, 56), [0; 56]);
    // Internal flags and version occupy DIFFERENT stack arguments. Both uint32
    // parameters ignore their unspecified upper register/stack bits.
    extra(
        &mut guest,
        [0, 0xffff_ffff_0000_0042, 0xffff_ffff_034f_ffff],
    );
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitializeInternal, args(), Some(STACK), &mut guest),
        Ok(0)
    );
    assert_eq!(guest.u64(80), 0x42);
    assert_eq!(owner.usage().handles_live, 1);
}

#[test]
fn fiber_lifecycle_rename_finalization_and_reinitialization_are_real_owned_state() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0x300, b"original\0");
    extra(&mut guest, [0, 0x0350_0000, 0]);
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0)
    );
    guest.put(0x300, b"short\0");
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberRename, &[ARG, NAME]),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 40, 6), b"short\0");
    assert_eq!(guest.get(ARG + 46, 26), [0; 26]);
    let old = guest.get(ARG, 208);
    guest.refuse = true;
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberFinalize, &[ARG]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG, 208), old);
    guest.refuse = false;
    assert_eq!(invoke(&mut owner, &mut guest, FiberFinalize, &[ARG]), Ok(0));
    assert_eq!(guest.u64(0), 0x0000_0003_def1_649c);
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberFinalize, &[ARG]),
        Ok(0x8059_0006)
    );
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberGetInfo, &[ARG, INFO]),
        Ok(0)
    );
    // A copied magic/ABI record cannot manufacture another owned fiber.
    let copied = guest.get(ARG, 208);
    guest.put(0x700, &copied);
    assert_eq!(
        invoke(&mut owner, &mut guest, FiberFinalize, &[ARG + 0x700]),
        Ok(0x8059_0004)
    );
    let mut input = args();
    input.0[4..].fill(0); // Context-less initialization is valid, not a running CPU.
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, input, Some(STACK), &mut guest),
        Ok(0)
    );
    assert_eq!(guest.u64(24), 0);
    assert_eq!(guest.u64(96), 0);
    assert_eq!(owner.usage().handles_live, 1);
}

#[test]
fn fiber_source_errors_and_backing_or_handle_failure_do_not_publish_partial_state() {
    let mut owner = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    let mut guest = TestGuest::new();
    guest.put(0x300, b"original\0");
    extra(&mut guest, [0, 0x0350_0000, 0]);
    for (register, value, expected) in [
        (0, 0, 1),
        (1, 0, 1),
        (2, 0, 1),
        (0, ARG + 1, 2),
        (4, CONTEXT + 1, 2),
        (5, 16, 3),
        (5, 513, 4),
        (4, 0, 4),
    ] {
        let mut input = args();
        input.0[register] = value;
        assert_eq!(
            owner.invoke_library_with_stack(FiberInitialize, input, Some(STACK), &mut guest),
            Ok(0x8059_0000 + expected)
        );
    }
    extra(&mut guest, [OPTION + 1, 0, 0]);
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0x8059_0002)
    );
    extra(&mut guest, [OPTION, 0, 0]);
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0x8059_0004)
    );
    extra(&mut guest, [0, 0x0350_0000, 0]);
    guest.put(0, &[0xa5; 208]);
    guest.put(0x1000, &[0x85; 16]);
    guest.refuse = true;
    assert!(
        owner
            .invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest)
            .is_err()
    );
    guest.refuse = false;
    let mut invalid_context = args();
    invalid_context.0[4] = ARG + PAGE;
    assert!(
        owner
            .invoke_library_with_stack(FiberInitialize, invalid_context, Some(STACK), &mut guest)
            .is_err()
    );
    let mut overlapping = args();
    overlapping.0[4] = ARG;
    assert!(matches!(
        owner.invoke_library_with_stack(FiberInitialize, overlapping, Some(STACK), &mut guest),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(guest.get(ARG, 208), [0xa5; 208]);
    assert_eq!(guest.get(CONTEXT, 16), [0x85; 16]);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, args(), Some(STACK), &mut guest),
        Ok(0)
    );
    let mut second = args();
    second.0[0] = ARG + 0x700;
    assert!(matches!(
        owner.invoke_library_with_stack(FiberInitialize, second, Some(STACK), &mut guest),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.get(ARG + 0x700, 208), [0; 208]);
    assert_eq!(owner.usage().handles_live, 1);
}

#[test]
fn fiber_names_observe_zeroing_aliases_and_missing_or_truncated_call_frames_refuse() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    extra(&mut guest, [0, 0, 0]);
    let mut input = args();
    input.0[1] = ARG + 40;
    guest.put(40, b"erased by initialization\0");
    assert_eq!(
        owner.invoke_library_with_stack(FiberInitialize, input, Some(STACK), &mut guest),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 40, 32), [0; 32]);
    assert!(matches!(
        owner.invoke_library(FiberInitialize, input, &mut guest),
        Err(Failure::Unsupported(_))
    ));
    let before = guest.get(ARG, 208);
    assert!(
        owner
            .invoke_library_with_stack(
                FiberInitializeInternal,
                input,
                Some(ARG + PAGE - 24),
                &mut guest
            )
            .is_err()
    );
    assert_eq!(guest.get(ARG, 208), before);
    for module in ["Fiber", "libSceFiber"] {
        for library in ["Fiber", "libSceFiber"] {
            assert_eq!(
                resolve(module, (1, 1), library, 1, "hVYD7Ou2pCQ"),
                Some(FiberInitialize)
            );
            assert_eq!(resolve(module, (1, 2), library, 1, "hVYD7Ou2pCQ"), None);
            assert_eq!(resolve(module, (1, 1), library, 2, "hVYD7Ou2pCQ"), None);
        }
    }
    assert_eq!(
        resolve("libkernel", (1, 1), "libkernel", 1, "hVYD7Ou2pCQ"),
        None
    );
}
