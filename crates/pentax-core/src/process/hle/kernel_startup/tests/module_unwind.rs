// SPDX-License-Identifier: GPL-2.0-only
//! Original bound provider requests and live mappings; no guest CPU executes.
use super::*;

fn image() -> Vec<u8> {
    let mut bytes = original_consumer();
    put(&mut bytes, 56, &7u16.to_le_bytes());
    program(&mut bytes, 6, 0x6474_e550, 4, [0x700, 0x700, 8, 8, 4]);
    put(&mut bytes, 0x700, &[1, 0x1b, 0xff, 0xff]);
    put(&mut bytes, 0x704, &(-0x104i32).to_le_bytes());
    bytes
}
fn query(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    address: u64,
    output: u64,
) -> Result<u64, KernelStartupError> {
    let marker = stop(
        session.plan(),
        Service::ModuleInfoForUnwind,
        [address, 0, output, 0, 0, 0],
    );
    session.service_request(NativeServiceRequest {
        owner_id: session.owner_id(),
        syscall_pc: marker.syscall_pc,
        next_pc: marker.syscall_pc + 2,
        service_id: marker.service_id,
        args: marker.args.0,
        vector_args: None,
        stack_pointer: None,
    })
}
fn initialize(session: &mut PreparedNativeKernelStartup<'_, '_>, output: u64) {
    session
        .pause()
        .unwrap()
        .write(output, &[0xa5; 312])
        .unwrap();
    session
        .pause()
        .unwrap()
        .write(output, &304u64.to_le_bytes())
        .unwrap();
}

#[test]
fn unwind_native_provider_returns_checked_loaded_guest_metadata_and_excludes_generated_code() {
    with_consumer(
        image(),
        Default::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let output = base + OUTPUT;
            initialize(&mut session, output);
            assert_eq!(query(&mut session, base + ENTRY as u64, output), Ok(0));
            for (offset, expected) in [
                (0, 304),
                (264, base + 0x700),
                (272, base + 0x600),
                (280, 0x100),
                (288, base),
                (296, 0xc000),
            ] {
                assert_eq!(word(&mut session, output + offset), expected);
            }
            assert_eq!(word(&mut session, output + 304), 0xa5a5_a5a5_a5a5_a5a5);
            for address in [
                1,
                u64::MAX,
                base + 0x10000,
                plan.entry(plan.factory.markers()[0]),
            ] {
                initialize(&mut session, output);
                assert_eq!(query(&mut session, address, output), Ok(0x8002_0003));
                assert_eq!(word(&mut session, output + 264), 0xa5a5_a5a5_a5a5_a5a5);
            }
        },
    );
}

#[test]
fn unwind_live_unmap_and_changed_header_never_publish_stale_module_metadata() {
    with_consumer(
        image(),
        Default::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let output = base + OUTPUT;
            initialize(&mut session, output);
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    pentax_platform::native_memory::NativeMemoryTransaction {
                        effect: pentax_platform::native_memory::NativeMemoryEffect::Protect {
                            address: base,
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
                .write(base + 0x704, &0i32.to_le_bytes())
                .unwrap();
            assert!(matches!(
                query(&mut session, base + ENTRY as u64, output),
                Err(KernelStartupError::Kernel(Failure::Unsupported(
                    "changed module unwind header"
                )))
            ));
            assert_eq!(word(&mut session, output + 264), 0xa5a5_a5a5_a5a5_a5a5);
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    pentax_platform::native_memory::NativeMemoryTransaction {
                        effect: pentax_platform::native_memory::NativeMemoryEffect::Unmap {
                            address: base + 0x4000,
                            size: PAGE_SIZE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            // The candidate belongs to the source, but one of its real mappings is
            // gone. It must not receive a fresh success from retained metadata.
            assert_eq!(
                query(&mut session, base + ENTRY as u64, output),
                Ok(0x8002_0003)
            );
            assert_eq!(word(&mut session, output + 264), 0xa5a5_a5a5_a5a5_a5a5);
        },
    );
}

#[test]
fn unwind_real_readonly_output_and_malformed_extent_preserve_all_output_bytes() {
    with_consumer(
        image(),
        Default::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let output = base + OUTPUT;
            initialize(&mut session, output);
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    pentax_platform::native_memory::NativeMemoryTransaction {
                        effect: pentax_platform::native_memory::NativeMemoryEffect::Protect {
                            address: output,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::new(true, false, false),
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            assert_eq!(
                query(&mut session, base + ENTRY as u64, output),
                Err(KernelStartupError::Kernel(Failure::Guest(
                    GuestError::WriteDenied
                )))
            );
            assert_eq!(word(&mut session, output), 304);
            assert_eq!(word(&mut session, output + 264), 0xa5a5_a5a5_a5a5_a5a5);
        },
    );
    let mut malformed = image();
    put(&mut malformed, 0x704, &i32::MIN.to_le_bytes());
    with_consumer(
        malformed,
        Default::default(),
        |plan, owner, base, pointers| {
            let mut session = prepared(plan, owner, &pointers);
            let output = base + OUTPUT;
            initialize(&mut session, output);
            assert_eq!(
                query(&mut session, base + ENTRY as u64, output),
                Err(KernelStartupError::Kernel(Failure::Unsupported(
                    "checked module unwind metadata"
                )))
            );
            assert_eq!(word(&mut session, output + 264), 0xa5a5_a5a5_a5a5_a5a5);
        },
    );
}
