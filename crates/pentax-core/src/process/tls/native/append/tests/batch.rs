// SPDX-License-Identifier: GPL-2.0-only

//! Original A-export/B-import guest plus a factory-built B stop provider.
use super::replacement::{consumer, read_word};
use super::*;
use pentax_linker::{
    DT_SCE_EXPORT_LIB, DT_SCE_MODULE_INFO, FallbackPolicy, Resolution,
    append_symbolic_modules_with_fallbacks, plan_missing_functions,
    plan_missing_functions_for_extension,
};
use pentax_platform::native_memory::NativePermissions;

fn guest_exporting_a_and_importing_b() -> Vec<u8> {
    let mut bytes = consumer();
    let strings_size = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap()) as usize;
    let import = u32::from_le_bytes(bytes[0x418..0x41c].try_into().unwrap()) as usize;
    bytes[0x500 + import] = b'B';
    bytes[0x500 + strings_size..0x500 + strings_size + 16].copy_from_slice(b"AAAAAAAAAAA#D#C\0");
    bytes[0x218..0x220].copy_from_slice(&((strings_size + 16) as u64).to_le_bytes());
    bytes[0x238..0x240].copy_from_slice(&72_u64.to_le_bytes());
    bytes[0x430..0x434].copy_from_slice(&(strings_size as u32).to_le_bytes());
    bytes[0x434] = 0x12;
    bytes[0x435] = 3;
    bytes[0x436..0x438].copy_from_slice(&1_u16.to_le_bytes());
    bytes[0x438..0x440].copy_from_slice(&0x100_u64.to_le_bytes());
    bytes[0x440..0x448].copy_from_slice(&1_u64.to_le_bytes());
    let module = u64::from_le_bytes(bytes[0x258..0x260].try_into().unwrap());
    let library = u64::from_le_bytes(bytes[0x268..0x270].try_into().unwrap());
    for (at, value) in [
        (152, 13_u64 * 16),
        (160, 13 * 16),
        (0x2a0, DT_SCE_MODULE_INFO as u64),
        (0x2a8, module),
        (0x2b0, DT_SCE_EXPORT_LIB as u64),
        (0x2b8, library),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn with_forbidden_provider_tls(image: &[u8]) -> Vec<u8> {
    let mut bytes = image.to_vec();
    bytes[56..58].copy_from_slice(&3_u16.to_le_bytes());
    bytes[176..180].copy_from_slice(&7_u32.to_le_bytes());
    bytes[180..184].copy_from_slice(&4_u32.to_le_bytes());
    for (at, value) in [
        (184, 0x100_u64),
        (192, 0x100),
        (208, 8),
        (216, 32),
        (224, 16),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn batch_guest_tls_and_new_stop_provider_commit_together_after_sealed_attestation() {
    // Success, failed post-seal attestation, changed retained slot, forbidden
    // provider TLS, and failure after the actual new-template copies.
    for case in 0..5 {
        let bytes = consumer();
        let guest = guest_exporting_a_and_importing_b();
        let owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10_0000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), base)];
        let fallback = crate::process::hle::build_unavailable_provider(
            plan_missing_functions(&inputs, &Default::default(), 1).unwrap(),
            &Default::default(),
        )
        .unwrap();
        let mut supplied = inputs.to_vec();
        supplied.push(ModuleInput::new(
            open_executable(fallback.image()).unwrap(),
            base + 0x10000,
        ));
        let source = plan_symbolic(&supplied, &Default::default()).unwrap();
        let old = source.patches()[0];
        let Resolution::Definition(reference) = old.resolution() else {
            panic!("expected A fallback");
        };
        let grant = FallbackPolicy::new(
            &source,
            reference.module,
            fallback.image(),
            &[reference],
            &Default::default(),
        )
        .unwrap();
        let policies = [grant];
        let guest_bias = base + 0x20000;
        let delta_bias = base + 0x30000;
        let delta = crate::process::hle::build_unavailable_provider(
            plan_missing_functions_for_extension(
                &source,
                ModuleInput::new(open_executable(&guest).unwrap(), guest_bias),
                &policies,
                &Default::default(),
                1,
            )
            .unwrap(),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(delta.function_count(), 1);
        assert_eq!(
            delta.function(0).unwrap().first_consumer().module.index(),
            2
        );
        let forbidden_delta = with_forbidden_provider_tls(delta.image());
        let delta_image = if case == 3 {
            &forbidden_delta[..]
        } else {
            delta.image()
        };
        let next = append_symbolic_modules_with_fallbacks(
            &source,
            &[
                ModuleInput::new(open_executable(&guest).unwrap(), guest_bias),
                ModuleInput::new(open_executable(delta_image).unwrap(), delta_bias),
            ],
            &policies,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(next.parent_identity(), Some(source.identity()));
        assert_eq!(next.appended_guest().unwrap().index(), 2);
        assert_eq!(next.fallback_transitions().len(), 1);
        let replacement = next.fallback_transitions()[0].new_patch().value();
        assert_eq!(replacement, guest_bias + 0x100);
        let b_value = next
            .patches()
            .iter()
            .find(|patch| patch.consumer().index() == 2)
            .unwrap()
            .value();
        assert_eq!(b_value, delta_bias + delta.exit_callback_rva() + 16);
        let bindings = plan_tls_bindings(&source, &Default::default()).unwrap();
        let policy = StaticTlsLimits {
            reserved_static_bytes: PAGE_SIZE,
            thread_control: ThreadControlBlockPolicy::KytyObservedSelfPointerV1,
            max_instances: 3,
            max_storage_bytes: 6 * PAGE_SIZE,
            ..Default::default()
        };
        let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
        let tp = base + 0x68000;
        let mut session = super::super::super::prepare_native_in(
            &layout,
            &StaticTlsStorage {
                thread_pointers: &[tp],
            },
            &NativeLoadLimits::default(),
            Some(owner),
        )
        .unwrap();
        let live = session.instances()[0].blocks[0].address;
        session
            .pause()
            .unwrap()
            .write(live, &99_u64.to_le_bytes())
            .unwrap();
        // Distinct spare bytes prove rollback restores rather than zeroes them.
        session
            .pause()
            .unwrap()
            .write(tp - 64, &107_u64.to_le_bytes())
            .unwrap();
        if case == 2 {
            let page = old.address() / PAGE_SIZE * PAGE_SIZE;
            session
                .memory
                .space
                .protect(page, PAGE_SIZE, NativePermissions::READ_WRITE)
                .unwrap();
            session
                .pause()
                .unwrap()
                .write(old.address(), &0xfeed_u64.to_le_bytes())
                .unwrap();
            session
                .memory
                .space
                .protect(page, PAGE_SIZE, NativePermissions::READ_EXECUTE)
                .unwrap();
        }
        let next_bindings = plan_tls_bindings(&next, &Default::default()).unwrap();
        let next_layout = plan_relocated_static_tls(&next_bindings, &policy).unwrap();
        session.memory.fail_append_after_initializer = case == 4;
        let mut attested = false;
        let result = session.append_batch(
            &next_layout,
            NativeRange {
                address: guest_bias,
                size: 0x20000,
            },
            &NativeLoadLimits::default(),
            |space| {
                attested = true;
                let paused = space.pause().unwrap();
                assert_eq!(read_word(&paused, tp - 64), b_value);
                assert_eq!(read_word(&paused, live), 99);
                assert_eq!(
                    read_word(&paused, old.address()),
                    if case == 2 { 0xfeed } else { old.value() }
                );
                for address in [guest_bias + 0x100, b_value] {
                    assert!(matches!(
                        paused.preflight_write(address, 1),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                    paused.preflight_fetch_source(address, 8).unwrap();
                }
                let mut marker = [0; 8];
                paused.fetch_source(b_value, &mut marker).unwrap();
                let at = (delta.exit_callback_rva() + 16) as usize;
                assert_eq!(&marker, &delta.image()[at..at + 8]);
                if case == 1 {
                    Err(ProcessError::InvalidSegment)
                } else {
                    Ok(())
                }
            },
        );
        assert_eq!(attested, case < 3);
        assert_eq!(read_word(&session.pause().unwrap(), live), 99);
        assert!(matches!(
            session.pause().unwrap().preflight_write(old.address(), 8),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
        if case == 0 {
            result.unwrap();
            assert_eq!(session.memory.active_source_identity(), next.identity());
            assert_eq!(session.memory.module_count(), 4);
            assert_eq!(session.templates().len(), 2);
            assert_eq!(session.templates()[1].module.index(), 2);
            assert_eq!(
                read_word(&session.pause().unwrap(), old.address()),
                replacement
            );
            assert_eq!(read_word(&session.pause().unwrap(), tp - 64), b_value);
            let fresh = session.append_thread_instance(base + 0x88000).unwrap();
            let a_block = session.instances()[fresh].blocks[0].address;
            let b_block = session.instances()[fresh].blocks[1].address;
            assert_eq!(read_word(&session.pause().unwrap(), a_block), replacement);
            assert_eq!(read_word(&session.pause().unwrap(), b_block), b_value);
            assert_eq!(read_word(&session.pause().unwrap(), live), 99);
        } else {
            if case == 2 {
                assert!(matches!(
                    result,
                    Err(TlsError::Staging(ProcessError::Memory(
                        MemoryError::Native(NativeMemoryError::ReplacementMismatch)
                    )))
                ));
            } else {
                assert!(result.is_err());
            }
            assert_eq!(session.memory.active_source_identity(), source.identity());
            assert_eq!(session.memory.module_count(), 2);
            assert_eq!(session.templates().len(), 1);
            assert_eq!(session.initializers.len(), 1);
            assert_eq!(
                u64::from_le_bytes(session.initializers[0][..8].try_into().unwrap()),
                old.value()
            );
            assert_eq!(
                read_word(&session.pause().unwrap(), old.address()),
                if case == 2 { 0xfeed } else { old.value() }
            );
            assert_eq!(read_word(&session.pause().unwrap(), tp - 64), 107);
            session
                .pause()
                .unwrap()
                .preflight_reserved_unmapped(guest_bias, 2 * PAGE_SIZE)
                .unwrap();
            session
                .pause()
                .unwrap()
                .preflight_reserved_unmapped(delta_bias, PAGE_SIZE)
                .unwrap();
        }
    }
}
