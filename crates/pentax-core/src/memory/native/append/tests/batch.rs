// SPDX-License-Identifier: GPL-2.0-only

//! Actual atomic suffix backing, sealed attestation and identity refusal.
use super::*;
use pentax_linker::append_symbolic_modules_with_fallbacks;
use pentax_platform::native_memory::{NativeMemoryError, NativePermissions};

#[test]
fn batch_maps_both_inputs_and_attests_sealed_pages_or_preserves_the_previous_epoch() {
    for case in 0..4 {
        let bytes = original(PAGE_SIZE);
        let guest = original(PAGE_SIZE);
        let extra = original(PAGE_SIZE);
        let owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x80000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), base)];
        let source = plan_symbolic(&inputs, &Default::default()).unwrap();
        // Equal inputs in a distinct plan do not authorize a different epoch.
        let foreign_parent = plan_symbolic(&inputs, &Default::default()).unwrap();
        let mut memory = prepare_in(&source, &NativeLoadLimits::default(), Some(owner)).unwrap();
        memory
            .pause()
            .unwrap()
            .write(base + PAGE_SIZE, &99_u64.to_le_bytes())
            .unwrap();
        let guest_bias = base + 0x20000;
        let extra_bias = base + 0x40000;
        let next = append_symbolic_modules_with_fallbacks(
            if case == 3 { &foreign_parent } else { &source },
            &[
                ModuleInput::new(open_executable(&guest).unwrap(), guest_bias),
                ModuleInput::new(open_executable(&extra).unwrap(), extra_bias),
            ],
            &[],
            &Default::default(),
        )
        .unwrap();
        let mut attested = false;
        let window = NativeRange {
            address: guest_bias,
            size: if case == 2 { 0x20000 } else { 0x40000 },
        };
        let result =
            memory.append_source_batch(&next, window, &NativeLoadLimits::default(), |space| {
                attested = true;
                let paused = space.pause().unwrap();
                for bias in [guest_bias, extra_bias] {
                    let mut code = [0; 6];
                    paused.fetch_source(bias + 0x100, &mut code).unwrap();
                    assert_eq!(code, [0xb8, 42, 0, 0, 0, 0xc3]);
                    assert!(matches!(
                        paused.preflight_write(bias + 0x100, 1),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ));
                    let mut value = [0; 8];
                    paused.read(bias + PAGE_SIZE, &mut value).unwrap();
                    assert_eq!(u64::from_le_bytes(value), bias + 0x1234);
                }
                if case == 1 {
                    Err(ProcessError::InvalidSegment)
                } else {
                    Ok(())
                }
            });
        assert_eq!(attested, case < 2);
        let mut value = [0; 8];
        memory
            .pause()
            .unwrap()
            .read(base + PAGE_SIZE, &mut value)
            .unwrap();
        assert_eq!(u64::from_le_bytes(value), 99);
        assert!(matches!(
            memory.pause().unwrap().preflight_write(base + 0x100, 1),
            Err(NativeMemoryError::PermissionDenied(_))
        ));
        if case == 0 {
            result.unwrap();
            assert_eq!(memory.active_source_identity(), next.identity());
            assert_eq!(memory.module_count(), 3);
            assert_eq!(memory.applied_relocations(), 3);
            assert_eq!(memory.mapped_bytes(), 6 * PAGE_SIZE);
            let mut replayed = false;
            assert!(
                memory
                    .append_source_batch(&next, window, &NativeLoadLimits::default(), |_| {
                        replayed = true;
                        Ok(())
                    })
                    .is_err()
            );
            assert!(!replayed);
        } else {
            assert!(result.is_err());
            assert_eq!(memory.active_source_identity(), source.identity());
            assert_eq!(memory.module_count(), 1);
            assert_eq!(memory.applied_relocations(), 1);
            assert_eq!(memory.mapped_bytes(), 2 * PAGE_SIZE);
            for bias in [guest_bias, extra_bias] {
                memory
                    .pause()
                    .unwrap()
                    .preflight_reserved_unmapped(bias, 2 * PAGE_SIZE)
                    .unwrap();
            }
        }
    }
}

#[test]
fn batch_second_input_commit_failure_removes_the_first_input_too() {
    let bytes = original(PAGE_SIZE);
    let guest = original(PAGE_SIZE);
    let extra = original(PAGE_SIZE);
    let owner = NativeAddressSpace::reserve_any(
        NativeMemoryLimits {
            mapped_bytes: 4 * PAGE_SIZE,
            reserved_bytes: 0x80000,
        },
        0x80000,
    )
    .unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let source = plan_symbolic(
        &[ModuleInput::new(open_executable(&bytes).unwrap(), base)],
        &Default::default(),
    )
    .unwrap();
    let mut memory = prepare_in(&source, &NativeLoadLimits::default(), Some(owner)).unwrap();
    memory
        .pause()
        .unwrap()
        .write(base + PAGE_SIZE, &[51])
        .unwrap();
    let next = append_symbolic_modules_with_fallbacks(
        &source,
        &[
            ModuleInput::new(open_executable(&guest).unwrap(), base + 0x20000),
            ModuleInput::new(open_executable(&extra).unwrap(), base + 0x40000),
        ],
        &[],
        &Default::default(),
    )
    .unwrap();
    let mut attested = false;
    assert!(
        memory
            .append_source_batch(
                &next,
                NativeRange {
                    address: base + 0x20000,
                    size: 0x40000
                },
                &NativeLoadLimits::default(),
                |_| {
                    attested = true;
                    Ok(())
                }
            )
            .is_err()
    );
    assert!(!attested);
    assert_eq!(memory.active_source_identity(), source.identity());
    assert_eq!(memory.module_count(), 1);
    assert_eq!(memory.mapped_bytes(), 2 * PAGE_SIZE);
    for bias in [base + 0x20000, base + 0x40000] {
        memory
            .pause()
            .unwrap()
            .preflight_reserved_unmapped(bias, 2 * PAGE_SIZE)
            .unwrap();
    }
    let mut value = [0];
    memory
        .pause()
        .unwrap()
        .read(base + PAGE_SIZE, &mut value)
        .unwrap();
    assert_eq!(value, [51]);
    // A successful permission change proves the owner remains usable.
    memory
        .space
        .protect(base + PAGE_SIZE, PAGE_SIZE, NativePermissions::READ)
        .unwrap();
}
