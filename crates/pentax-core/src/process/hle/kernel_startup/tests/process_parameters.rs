// SPDX-License-Identifier: GPL-2.0-only

//! Original data-only image/service tests. No native instruction executes.
use super::*;

const PARAM: u64 = OUTPUT + 0x80;
fn consumer(kind: u32, size: u64) -> Vec<u8> {
    let mut bytes = original_consumer();
    put(&mut bytes, 16, &0xfe10_u16.to_le_bytes());
    put(&mut bytes, 56, &7_u16.to_le_bytes());
    program(&mut bytes, 6, kind, 4, [PARAM, PARAM, size, size, 8]);
    put(&mut bytes, PARAM as usize, &42_u64.to_le_bytes());
    bytes
}

#[test]
fn process_parameter_query_uses_the_actual_primary_address_without_copy_or_content_guesses() {
    with_consumer(
        consumer(0x6100_0001, 16),
        Default::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let query = stop(plan, Service::GetProcParam, [u64::MAX; 6]);
            assert_eq!(session.kernel_stop(query), Ok(base + PARAM));
            assert_eq!(word(&mut session, base + PARAM), 42);
            session
                .pause()
                .unwrap()
                .write(base + PARAM, &77_u64.to_le_bytes())
                .unwrap();
            assert_eq!(session.kernel_stop(query), Ok(base + PARAM));
            assert_eq!(word(&mut session, base + PARAM), 77);
            assert_eq!(session.kernel_usage().attempts, 2);
            assert_eq!(session.kernel_usage().bytes_read, 0);
            assert_eq!(session.kernel_usage().bytes_written, 0);
            assert_eq!(session.kernel_usage().handles_live, 0);
        },
    );
    for bytes in [original_consumer(), consumer(0x6100_0002, 16)] {
        with_consumer(bytes, Default::default(), |plan, owner, _, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            assert_eq!(
                session.kernel_stop(stop(plan, Service::GetProcParam, [0; 6])),
                Ok(0)
            );
        });
    }
}

#[test]
fn parameter_query_preflights_the_whole_live_extent_and_recovers_only_after_real_permission_restoration()
 {
    let mut bytes = consumer(0x6100_0001, 16);
    bytes.resize(0xc010, 0);
    let parameter = OUTPUT + PAGE_SIZE - 8;
    program(
        &mut bytes,
        2,
        1,
        6,
        [OUTPUT, OUTPUT, PAGE_SIZE + 16, PAGE_SIZE * 2, PAGE_SIZE],
    );
    program(
        &mut bytes,
        6,
        0x6100_0001,
        4,
        [parameter, parameter, 16, 16, 8],
    );
    with_consumer(bytes, Default::default(), |plan, owner, base, pointers| {
        let mut session = prepared(plan, owner, &pointers);
        let query = stop(plan, Service::GetProcParam, [0; 6]);
        assert_eq!(session.kernel_stop(query), Ok(base + parameter));
        for (permissions, succeeds) in [
            (NativePermissions::new(false, false, false), false),
            (NativePermissions::READ_WRITE, true),
        ] {
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    pentax_platform::native_memory::NativeMemoryTransaction {
                        effect: pentax_platform::native_memory::NativeMemoryEffect::Protect {
                            address: base + OUTPUT + PAGE_SIZE,
                            size: PAGE_SIZE,
                            permissions,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            if succeeds {
                assert_eq!(session.kernel_stop(query), Ok(base + parameter));
            } else {
                assert!(matches!(
                    session.kernel_stop(query),
                    Err(KernelStartupError::Kernel(Failure::Guest(
                        GuestError::ReadDenied
                    )))
                ));
            }
        }
        assert_eq!(session.kernel_usage().attempts, 3);
    });
}

#[test]
fn unsupported_parameter_shapes_and_query_work_never_become_a_null_success() {
    for (mut bytes, shared) in [
        (consumer(0x6100_0001, 0), false),
        (consumer(0x6100_0001, 16), true),
    ] {
        if shared {
            put(&mut bytes, 16, &3_u16.to_le_bytes());
        }
        with_consumer(bytes, Default::default(), |plan, owner, _, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            assert!(matches!(
                session.kernel_stop(stop(plan, Service::GetProcParam, [0; 6])),
                Err(KernelStartupError::Kernel(Failure::Unsupported(_)))
            ));
        });
    }
    with_consumer(
        consumer(0x6100_0001, 16),
        Default::default(),
        |plan, owner, _, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            session.provider_work =
                plan.limits.max_provider_work - (plan.registration_count() as u64 + 11);
            assert!(matches!(
                session.kernel_stop(stop(plan, Service::GetProcParam, [0; 6])),
                Err(KernelStartupError::Budget("provider work"))
            ));
            assert_eq!(session.kernel_usage().attempts, 0);
        },
    );
}
