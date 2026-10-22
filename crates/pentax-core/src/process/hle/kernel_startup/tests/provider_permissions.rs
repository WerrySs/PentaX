// SPDX-License-Identifier: GPL-2.0-only
//! Original real paused-provider admission after permission and payload changes.
use super::*;
use pentax_platform::native_memory::{NativeMemoryEffect, NativeMemoryTransaction};

fn protect(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    address: u64,
    permissions: NativePermissions,
) {
    session
        .pause()
        .unwrap()
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::Protect {
                    address,
                    size: PAGE_SIZE,
                    permissions,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
}

#[test]
fn cached_permissions_never_skip_live_provider_rights_marker_bytes_or_admission_charges() {
    with_plan(
        KernelStartupLimits::default(),
        |plan, owner, _, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let charge = plan.registration_count() as u64
                + 8
                + CODE_MAPPED_BYTES / PAGE_SIZE * 3
                + plan.factory.metadata_mapped_bytes() / PAGE_SIZE * 4;
            let work = session.provider_work;
            let attempts = session.kernel_usage().attempts;
            let call = stop(plan, Service::Getpid, [0; 6]);
            assert_eq!(session.kernel_stop(call), Ok(41));
            let code_tail = plan.bias + CODE_MAPPED_BYTES - PAGE_SIZE;
            protect(&mut session, code_tail, NativePermissions::READ_WRITE);
            assert_eq!(
                session.kernel_stop(call),
                Err(KernelStartupError::InvalidProvider(
                    "kernel provider lost immutable permission"
                ))
            );
            protect(&mut session, code_tail, NativePermissions::READ_EXECUTE);
            assert_eq!(session.kernel_stop(call), Ok(41));
            let metadata_tail =
                plan.bias + METADATA_RVA + plan.factory.metadata_mapped_bytes() - PAGE_SIZE;
            protect(&mut session, metadata_tail, NativePermissions::READ_WRITE);
            assert_eq!(
                session.kernel_stop(call),
                Err(KernelStartupError::InvalidProvider(
                    "kernel metadata must remain immutable and NX"
                ))
            );
            protect(&mut session, metadata_tail, NativePermissions::READ);
            assert_eq!(session.kernel_stop(call), Ok(41));
            let marker = *plan
                .factory
                .markers()
                .iter()
                .find(|marker| marker.registration.service == Service::Getpid)
                .unwrap();
            let address = plan.entry(marker);
            let page = address / PAGE_SIZE * PAGE_SIZE;
            let expected = marker.bytes();
            let mut changed = expected;
            changed[0] ^= 1;
            protect(&mut session, page, NativePermissions::READ_WRITE);
            session.pause().unwrap().write(address, &changed).unwrap();
            protect(&mut session, page, NativePermissions::READ_EXECUTE);
            assert_eq!(
                session.kernel_stop(call),
                Err(KernelStartupError::InvalidProvider(
                    "kernel live marker bytes"
                ))
            );
            protect(&mut session, page, NativePermissions::READ_WRITE);
            session.pause().unwrap().write(address, &expected).unwrap();
            protect(&mut session, page, NativePermissions::READ_EXECUTE);
            assert_eq!(session.kernel_stop(call), Ok(41));
            assert_eq!(session.provider_work, work + 7 * charge);
            assert_eq!(session.stop_attempts, 7);
            assert_eq!(session.kernel_usage().attempts, attempts + 4);
        },
    );
}
