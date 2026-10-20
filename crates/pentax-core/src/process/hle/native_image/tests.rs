// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::unavailable::tests::original_input;

fn initializer_input(entries: &[u64]) -> Vec<u8> {
    let mut bytes = original_input(0);
    let size = (11 + entries.len()) as u64 * 16;
    for offset in [152, 160] {
        bytes[offset..offset + 8].copy_from_slice(&size.to_le_bytes());
    }
    for (index, value) in entries.iter().enumerate() {
        let offset = 0x2a0 + index * 16;
        bytes[offset..offset + 8].copy_from_slice(&12_i64.to_le_bytes());
        bytes[offset + 8..offset + 16].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn explicit_initializer_is_source_bound_and_its_metadata_cannot_select_another_module_or_padding() {
    let primary = original_input(0);
    let limits = NativeImageLimits {
        startup_libc: Some(1),
        ..Default::default()
    };
    for entries in [vec![], vec![0], vec![0x100]] {
        let library = initializer_input(&entries);
        let result =
            with_native_image_session(&[&primary, &library], &limits, |session, layout| {
                assert_eq!(layout.initializer.is_some(), entries == [0x100]);
                if let Some(start) = layout.initializer {
                    assert_eq!(start.owner_id(), session.owner_id());
                    assert!(std::ptr::eq(
                        start.source_plan(),
                        session.plan().link_plan()
                    ));
                    assert_eq!(start.module_index(), 1);
                    assert_eq!(
                        start.entry(),
                        session.plan().link_plan().modules()[1].load_bias() + 0x100
                    );
                    assert_eq!(*start.process_entry(), layout.entry);
                    assert!(!format!("{start:?}").contains("original-module"));
                }
                Ok::<_, ()>(())
            });
        assert!(result.is_ok());
    }
    for (library, expected) in [
        (initializer_input(&[0x100, 0x100]), "duplicate-initializer"),
        (initializer_input(&[PAGE_SIZE]), "initializer-target"),
        (initializer_input(&[u64::MAX]), "initializer-target"),
    ] {
        let error =
            with_native_image_session(&[&primary, &library], &limits, |_, _| -> Result<(), ()> {
                panic!("invalid initializer reached callback")
            })
            .unwrap_err();
        assert_eq!(error.code(), expected);
    }
    let mut executable = initializer_input(&[0x100]);
    executable[16..18].copy_from_slice(&0xfe10_u16.to_le_bytes());
    assert_eq!(
        with_native_image_session(&[&primary, &executable], &limits, |_, _| Ok::<_, ()>(()))
            .unwrap_err()
            .code(),
        "initializer-module"
    );
    for index in [0, 2, usize::MAX] {
        let limits = NativeImageLimits {
            startup_libc: Some(index),
            ..limits.clone()
        };
        assert_eq!(
            with_native_image_session(&[&primary, &executable], &limits, |_, _| Ok::<_, ()>(()))
                .unwrap_err()
                .code(),
            "initializer-input-index"
        );
    }
}

#[test]
fn image_admission_refuses_input_and_layout_budgets_before_callback() {
    let bytes = original_input(0);
    for limits in [
        NativeImageLimits {
            max_input_bytes: 1,
            ..Default::default()
        },
        NativeImageLimits {
            max_images: 0,
            ..Default::default()
        },
        NativeImageLimits {
            max_layout_work_steps: 0,
            ..Default::default()
        },
        NativeImageLimits {
            stack_bytes: 1,
            ..Default::default()
        },
        NativeImageLimits {
            native: NativeLoadLimits {
                reserved_limit: 0,
                ..Default::default()
            },
            ..Default::default()
        },
    ] {
        let result = with_native_image_session(&[&bytes], &limits, |_, _| -> Result<(), ()> {
            panic!("must reject before callback")
        });
        assert!(result.is_err());
    }
    let mut fixed = bytes;
    fixed[16..18].copy_from_slice(&2_u16.to_le_bytes());
    assert!(matches!(
        with_native_image_session(&[&fixed], &Default::default(), |_, _| Ok::<_, ()>(())),
        Err(NativeImageError::Invalid("fixed-address-image-unsupported"))
    ));
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn supplied_image_admission_keeps_typed_owner_entry_and_unavailable_manifest_alive() {
    for count in [0, 1] {
        let bytes = original_input(count);
        let before = bytes.clone();
        let limits = NativeImageLimits {
            stack_bytes: PAGE_SIZE,
            kernel_window_bytes: 65_536,
            ..Default::default()
        };
        let result = with_native_image_session(&[&bytes], &limits, |session, layout| {
            assert_eq!(layout.supplied_modules, 1);
            assert_eq!(layout.unavailable_functions, count);
            assert_eq!(session.plan().unavailable_functions(), count);
            // Original image plus kernel, graphics, JSON, unavailable and libc.
            assert_eq!(session.memory().module_count(), 6);
            assert_eq!(session.native_service_requests(), 0);
            let thread = session.prepare_main_thread().unwrap();
            assert_eq!(thread.owner_id(), session.owner_id());
            let source = session.prepare_native_source(&Default::default()).unwrap();
            assert_eq!(source.owner_id(), session.owner_id());
            assert_eq!(
                layout.entry.entry,
                session.plan().link_plan().primary_entry()
            );
            let paused = session.pause().unwrap();
            let mut argc = [0; 8];
            paused.read(layout.entry.arguments, &mut argc).unwrap();
            assert_eq!(u64::from_le_bytes(argc), 1);
            for guard in layout.entry.guards {
                paused
                    .preflight_reserved_unmapped(guard.address, guard.size)
                    .unwrap();
            }
            Err::<(), _>(()) // Preparation test, never native execution.
        });
        assert!(matches!(result, Err(NativeImageError::Callback(()))));
        assert_eq!(bytes, before);
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn image_admission_places_two_page_tls_with_both_owned_guards_and_initialized_source() {
    use crate::process::hle::provider::{program, put};
    let mut bytes = original_input(0);
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 64 + 40, &(2 * PAGE_SIZE).to_le_bytes());
    program(&mut bytes, 2, 7, 4, [0x180, 0x180, 8, PAGE_SIZE + 1, 8]);
    put(&mut bytes, 0x180, &37_u64.to_le_bytes());
    let result = with_native_image_session(
        &[&bytes],
        &NativeImageLimits {
            stack_bytes: PAGE_SIZE,
            kernel_window_bytes: 65_536,
            ..Default::default()
        },
        |session, _| {
            let instance = &session.tls().instances()[0];
            let first = instance.blocks[0].address;
            let guards = instance.guard_pages.unwrap();
            let mut value = [0; 8];
            let paused = session.pause().unwrap();
            paused.read(first, &mut value).unwrap();
            assert_eq!(u64::from_le_bytes(value), 37);
            for guard in guards {
                paused
                    .preflight_reserved_unmapped(guard, PAGE_SIZE)
                    .unwrap();
            }
            Err::<(), _>(())
        },
    );
    assert!(
        matches!(result, Err(NativeImageError::Callback(()))),
        "{result:?}"
    );
}
