// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_cache_checks_source_and_emission_across_scratch_boundaries() {
    let mut code = [0xb8, 1, 0, 0, 0].repeat(53);
    code.push(0xc3);
    with_source(Some(&code), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        assert!(planned.bytes().len() > 256);
        assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
        assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
        for offset in [128, 264] {
            let mut original = [0];
            engine.machine.cache.read(offset, &mut original).unwrap();
            engine.machine.cache.arm_writable().unwrap();
            engine
                .machine
                .cache
                .write(offset, &[original[0] ^ 0xff])
                .unwrap();
            engine.machine.cache.seal_executable().unwrap();
            assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
        }
        assert_eq!(engine.usage().cache_publications, 3);
        assert_eq!(engine.usage().cache_reuses, 1);
        let page = source.source_plan().modules()[0].load_bias();
        for offset in [128, 265] {
            arena
                .protect(page, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            arena
                .pause()
                .unwrap()
                .write(state.guest_pc + offset as u64, &[code[offset] ^ 0xff])
                .unwrap();
            arena
                .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                .unwrap();
            assert_eq!(
                engine.publish(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::SourceMismatch)
            );
            arena
                .protect(page, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            arena
                .pause()
                .unwrap()
                .write(state.guest_pc + offset as u64, &[code[offset]])
                .unwrap();
            arena
                .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                .unwrap();
        }
        assert_eq!(engine.usage().cache_publications, 3);
        assert_eq!(engine.usage().cache_reuses, 1);
        assert_eq!(engine.usage().native_entries, 0);
    });
}

#[test]
fn native_cache_partial_protection_keeps_neighbor_pages_and_write_guards() {
    with_source(None, |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let cache = &mut engine.machine.cache;
        let page = NATIVE_GUEST_PAGE as usize;
        cache.seal_executable_range(0, page).unwrap();
        assert!(cache.range_executable(0, page).unwrap());
        assert!(!cache.range_executable(page, page).unwrap());
        assert!(cache.write(0, &[1]).is_err());
        assert!(cache.write(page - 1, &[1, 2]).is_err());
        cache.write(page, &[7]).unwrap();
        cache.seal_executable_range(page, page).unwrap();
        cache.arm_writable_range(0, page).unwrap();
        assert!(!cache.range_executable(0, page).unwrap());
        assert!(cache.range_executable(page, page).unwrap());
        cache.write(0, &[9]).unwrap();
        assert!(cache.write(page, &[9]).is_err());
        let mut neighbor = [0];
        cache.read(page, &mut neighbor).unwrap();
        assert_eq!(neighbor, [7]);
        for (offset, length) in [
            (1, page),
            (0, 1),
            (0, 0),
            (cache.len, page),
            (usize::MAX, page),
        ] {
            assert!(cache.arm_writable_range(offset, length).is_err());
            assert!(cache.seal_executable_range(offset, length).is_err());
        }
        cache.seal_executable_range(0, page).unwrap();
        cache.pages[0] = super::super::super::CachePage::NeedsFlush;
        assert!(!cache.range_executable(0, page).unwrap());
        assert!(cache.write(0, &[1]).is_err());
    });
}

#[test]
fn native_cache_slots_retain_exact_bytes_and_replace_collisions_or_corruption() {
    let code = [0xb8, 1, 0, 0, 0, 0xc3, 0xb8, 2, 0, 0, 0, 0xc3];
    with_source(Some(&code), |source, arena, stack, state| {
        let limits = NativeDispatchLimits {
            cache_slots: 4,
            ..Default::default()
        };
        let mut engine =
            NativeBlockStepper::new(source, &mut arena.pause().unwrap(), state, stack, &limits)
                .unwrap();
        let first = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.slot_placement(0),
        );
        let second = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc + 6,
            engine.slot_placement(1),
        );
        let collision = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc + 6,
            engine.slot_placement(0),
        );
        for planned in [&first, &second, &first] {
            assert!(engine.publish(&arena.pause().unwrap(), planned).unwrap());
        }
        assert_eq!(engine.usage().cache_publications, 2);
        assert_eq!(engine.usage().cache_reuses, 1);
        for planned in [&collision, &first] {
            assert!(engine.publish(&arena.pause().unwrap(), planned).unwrap());
        }
        assert_eq!(engine.usage().cache_publications, 4);
        // Every reuse checks actual code, native tail and trailer cell, not a hash.
        let body = first.bytes().len() - 2;
        let cell = engine.machine.cache.len / limits.cache_slots - TAIL_CELL;
        for offset in [0, body, cell] {
            let mut original = [0];
            engine.machine.cache.read(offset, &mut original).unwrap();
            engine.machine.cache.arm_writable().unwrap();
            engine
                .machine
                .cache
                .write(offset, &[original[0] ^ 0xff])
                .unwrap();
            engine.machine.cache.seal_executable().unwrap();
            assert!(engine.publish(&arena.pause().unwrap(), &first).unwrap());
        }
        assert_eq!(engine.usage().cache_publications, 7);
        assert_eq!(engine.usage().cache_reuses, 1);
        assert_eq!(engine.usage().native_entries, 0); // Publication never executes code.
        assert_eq!(engine.state(), &state);
    });
}

#[test]
fn native_cache_refuses_non_slot_placements_and_rechecks_live_source_on_hits() {
    with_source(None, |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let placement = engine.placement();
        for invalid in [
            CachePlacement {
                address: placement.address + 1,
                ..placement
            },
            CachePlacement {
                address: placement.address + engine.machine.cache.len as u64,
                ..placement
            },
            CachePlacement {
                capacity: placement.capacity - 1,
                ..placement
            },
        ] {
            let planned = block(source, &arena.pause().unwrap(), state.guest_pc, invalid);
            assert_eq!(
                engine.publish(&arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::EmissionMismatch)
            );
        }
        assert_eq!(engine.usage().cache_publications, 0);
        let planned = block(source, &arena.pause().unwrap(), state.guest_pc, placement);
        assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
        assert!(engine.publish(&arena.pause().unwrap(), &planned).unwrap());
        let page = source.source_plan().modules()[0].load_bias();
        arena
            .protect(page, PAGE, NativePermissions::new(true, true, true))
            .unwrap();
        assert_eq!(
            engine.publish(&arena.pause().unwrap(), &planned),
            Err(NativeDispatchError::WritableSource)
        );
        arena
            .protect(page, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        arena
            .pause()
            .unwrap()
            .write(state.guest_pc, &[0x90])
            .unwrap();
        arena
            .protect(page, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        assert_eq!(
            engine.publish(&arena.pause().unwrap(), &planned),
            Err(NativeDispatchError::SourceMismatch)
        );
        assert_eq!(engine.usage().cache_publications, 1);
        assert_eq!(engine.usage().cache_reuses, 1);
    });
}

#[test]
fn native_cache_exit_only_plans_attest_source_without_executable_publication() {
    with_source(Some(&[0xc3]), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        let planned = block(
            source,
            &arena.pause().unwrap(),
            state.guest_pc,
            engine.placement(),
        );
        assert!(!engine.publish(&arena.pause().unwrap(), &planned).unwrap());
        assert!(!engine.machine.cache.executable);
        assert_eq!(engine.usage().cache_publications, 0);
        let page = source.source_plan().modules()[0].load_bias();
        arena
            .protect(page, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        arena
            .pause()
            .unwrap()
            .write(state.guest_pc, &[0x90])
            .unwrap();
        arena
            .protect(page, PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        assert_eq!(
            engine.publish(&arena.pause().unwrap(), &planned),
            Err(NativeDispatchError::SourceMismatch)
        );
    });
}
