// SPDX-License-Identifier: GPL-2.0-only
//! Original loaded-image tests; no native block or guest instruction executes.
use super::*;
use crate::process::hle::{NativeImageLimits, with_native_image_session};

fn with_loaded(test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64, u64)) {
    let primary = super::super::super::unavailable::tests::original_input(0);
    let provider =
        build_kernel_provider(KernelStartupLimits::default().max_kernel_provider_bytes).unwrap();
    with_native_image_session(
        &[&primary, provider.image()],
        &NativeImageLimits::default(),
        |session, layout| {
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
        },
    )
    .unwrap();
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
        session.provider_work = maximum
            - (session.plan().registration_count() as u64 + 8 + CODE_MAPPED_BYTES / PAGE_SIZE * 3);
        assert!(matches!(
            lookup(session, scratch, 2, b"sceKernelGetProcParam\0"),
            Err(KernelStartupError::Kernel(Failure::Limit(_)))
        ));
        assert_eq!(word(session, scratch + 128), 73);
        assert_eq!(session.provider_work, maximum);
    });
}
