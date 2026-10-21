// SPDX-License-Identifier: GPL-2.0-only
//! Original loaded-image tests; no native block or guest instruction executes.
use super::*;
use crate::process::hle::{NativeImageLimits, with_native_image_session};

fn with_loaded(test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64, u64)) {
    with_loaded_limits(&NativeImageLimits::default(), test);
}

fn with_loaded_limits(
    limits: &NativeImageLimits,
    test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64, u64),
) {
    let primary = super::super::super::unavailable::tests::original_input(0);
    let provider =
        build_kernel_provider(KernelStartupLimits::default().max_kernel_provider_bytes).unwrap();
    with_native_image_session(&[&primary, provider.image()], limits, |session, layout| {
        assert_eq!(session.supplied_module_handle(0), Some(1));
        assert_eq!(session.supplied_module_handle(1), Some(2));
        assert_eq!(session.supplied_module_handle(2), None);
        let marker = provider
            .markers()
            .iter()
            .find(|marker| marker.registration.service == Service::GetProcParam)
            .unwrap();
        let expected = session.plan().link_plan().modules()[1].load_bias() + marker.rva;
        test(session, layout.entry.stack.address + 0x100, expected);
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
fn lifetime_provider_ledger_preserves_per_lookup_refusal_and_output_on_retry() {
    let mut limits = NativeImageLimits::default();
    limits.startup.max_provider_work = u64::MAX;
    limits.startup.max_provider_call_work = Some(0);
    limits.startup.kernel.max_attempts = u64::MAX;
    limits.startup.startup.hle.services.max_attempts = u64::MAX;
    for request_cap in [None, Some(u64::MAX)] {
        limits.startup.max_native_service_requests = request_cap;
        with_loaded_limits(&limits, |session, scratch, _| {
            session
                .pause()
                .unwrap()
                .write(scratch + 128, &73_u64.to_le_bytes())
                .unwrap();
            for _ in 0..2 {
                let work = session.provider_work;
                let attempts = session.kernel_usage().attempts;
                session
                    .pause()
                    .unwrap()
                    .write(scratch, b"sceKernelGetProcParam\0")
                    .unwrap();
                let query = stop(
                    session.plan(),
                    Service::Dlsym,
                    [2, scratch, scratch + 128, 0, 0, 0],
                );
                let result = session.service_request(super::super::NativeServiceRequest {
                    owner_id: session.owner_id(),
                    syscall_pc: query.syscall_pc,
                    next_pc: query.syscall_pc + 2,
                    service_id: query.service_id,
                    args: query.args.0,
                    vector_args: None,
                    stack_pointer: None,
                });
                assert_eq!(word(session, scratch + 128), 73);
                if request_cap.is_none() {
                    assert_eq!(
                        result,
                        Err(KernelStartupError::Budget("native request attempts"))
                    );
                    assert_eq!(session.native_service_requests, 0);
                    assert_eq!(session.provider_work, work);
                    assert_eq!(session.kernel_usage().attempts, attempts);
                    break;
                }
                assert_eq!(
                    result,
                    Err(KernelStartupError::Kernel(Failure::Limit(
                        "module lookup work"
                    )))
                );
                assert!(session.provider_work > work); // Authentication remains charged.
                assert_eq!(session.kernel_usage().attempts, attempts + 1);
                assert_eq!(session.native_service_requests, attempts + 1);
            }
        });
    }
}

fn lookup(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    scratch: u64,
    handle: u64,
    name: &[u8],
) -> Result<u64, KernelStartupError> {
    session.pause().unwrap().write(scratch, name).unwrap();
    let query = stop(
        session.plan(),
        Service::Dlsym,
        [handle, scratch, scratch + 128, 0, 0, 0],
    );
    session.kernel_stop(query)
}

#[test]
fn loaded_module_lookup_publishes_actual_target_and_never_selects_other_or_generated_images() {
    with_loaded(|session, scratch, expected| {
        assert_eq!(
            lookup(session, scratch, 2, b"sceKernelGetProcParam\0"),
            Ok(0)
        );
        assert_eq!(word(session, scratch + 128), expected);
        let mut opcode = [0];
        session
            .pause()
            .unwrap()
            .read(expected, &mut opcode)
            .unwrap();
        assert_eq!(opcode, [0xb8]);
        let absent = u64::from(Errno::NoSuchResource.kernel_code() as u32);
        for (handle, name) in [
            (1, b"sceKernelGetProcParam\0".as_slice()),
            (3, b"sceKernelGetProcParam\0"),
            (2, b"original_missing\0"),
        ] {
            session
                .pause()
                .unwrap()
                .write(scratch + 128, &u64::MAX.to_le_bytes())
                .unwrap();
            assert_eq!(lookup(session, scratch, handle, name), Ok(absent));
            assert_eq!(word(session, scratch + 128), 0);
        }
        assert_eq!(session.kernel_usage().attempts, 4);
    });
}

#[test]
fn loaded_lookup_refuses_live_writable_code_and_cumulative_exhaustion_without_zeroing_output() {
    with_loaded(|session, scratch, expected| {
        let page = expected & !(PAGE_SIZE - 1);
        session
            .pause()
            .unwrap()
            .apply_transaction(
                pentax_platform::native_memory::NativeMemoryTransaction {
                    effect: pentax_platform::native_memory::NativeMemoryEffect::Protect {
                        address: page,
                        size: PAGE_SIZE,
                        permissions: NativePermissions::new(true, true, true),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
        session
            .pause()
            .unwrap()
            .write(scratch + 128, &73_u64.to_le_bytes())
            .unwrap();
        assert!(matches!(
            lookup(session, scratch, 2, b"sceKernelGetProcParam\0"),
            Err(KernelStartupError::Kernel(Failure::Unsupported(_)))
        ));
        assert_eq!(word(session, scratch + 128), 73);
    });
    with_loaded(|session, scratch, _| {
        session
            .pause()
            .unwrap()
            .write(scratch + 128, &73_u64.to_le_bytes())
            .unwrap();
        let maximum = session.plan().limits.max_provider_work;
        // Admit this exact kernel stop, including its independent RO/NX
        // metadata-page checks, but leave no work for the export lookup.
        // Exhausting stop admission instead would test a different boundary.
        let stop_work = session.plan().registration_count() as u64
            + 8
            + CODE_MAPPED_BYTES / PAGE_SIZE * 3
            + session.plan().factory.metadata_mapped_bytes() / PAGE_SIZE * 4;
        session.provider_work = maximum.checked_sub(stop_work).unwrap();
        let kernel_attempts = session.kernel_usage().attempts;
        assert_eq!(
            lookup(session, scratch, 2, b"sceKernelGetProcParam\0"),
            Err(KernelStartupError::Kernel(Failure::Limit(
                "module lookup work"
            )))
        );
        assert_eq!(word(session, scratch + 128), 73);
        assert_eq!(session.provider_work, maximum);
        assert_eq!(session.kernel_usage().attempts, kernel_attempts + 1);
        // The exhausted cumulative budget now rejects the following stop
        // before another kernel attempt, without clearing the same output.
        assert_eq!(
            lookup(session, scratch, 2, b"sceKernelGetProcParam\0"),
            Err(KernelStartupError::Budget("provider work"))
        );
        assert_eq!(session.kernel_usage().attempts, kernel_attempts + 1);
        assert_eq!(session.provider_work, maximum);
        assert_eq!(word(session, scratch + 128), 73);
    });
}
