// SPDX-License-Identifier: GPL-2.0-only
//! Original raw primitive metadata, source reset masks and bounded draw use.
use super::*;
fn state(words: &[u32], initial: GraphicsState) -> GraphicsState {
    plan_commands(&[&bytes(words)], initial, &Default::default())
        .unwrap()
        .final_state()
}
fn operation(op: u32) -> Vec<u32> {
    if op == 2 {
        vec![0xc001_1068, 2, 0]
    } else {
        vec![0xc003_1068, op, 0, 0, 0]
    }
}
#[test]
fn primitive_pairs_retain_all_raw_bits_and_context_history_never_restores_user_config() {
    let first = state(
        &[
            0xc001_6900,
            0x29b,
            0x8765_4321,
            0xc001_7900,
            0x25b,
            0xabcd_1234,
            0xc001_7900,
            0x262,
            0x7654_3210,
        ],
        GraphicsState::empty(),
    );
    for (space, offset, value) in [
        (RegisterSpace::Context, 0x29b, 0x8765_4321),
        (RegisterSpace::Uconfig, 0x25b, 0xabcd_1234),
        (RegisterSpace::Uconfig, 0x262, 0x7654_3210),
    ] {
        assert_eq!(first.get(space, offset), Ok(value));
    }
    let mut words = operation(1);
    words.extend([
        0xc001_6900,
        0x29b,
        3,
        0xc001_7900,
        0x25b,
        17,
        0xc001_7900,
        0x262,
        5,
    ]);
    words.extend([0xc000_1200, 0]);
    words.extend(operation(2));
    let result = state(&words, first);
    assert_eq!(result.get(RegisterSpace::Context, 0x29b), Ok(0x8765_4321));
    assert_eq!(result.get(RegisterSpace::Uconfig, 0x25b), Ok(17));
    assert_eq!(result.get(RegisterSpace::Uconfig, 0x262), Ok(5));
    let data = bytes(&words);
    let roots = [CommandRange {
        address: 0x4000,
        size: data.len() as u64,
    }];
    let mut live = OrderedCommands::prepare(&[&data], &roots, first, &Default::default()).unwrap();
    assert!(live.next_step().unwrap().is_none());
    assert_eq!(live.final_state().unwrap(), result);
    for clear in [vec![0xc000_1200, 15], operation(0), operation(3)] {
        let after = state(&clear, first);
        assert_eq!(after.get(RegisterSpace::Context, 0x29b), Ok(0));
        assert_eq!(after.get(RegisterSpace::Uconfig, 0x25b), Ok(0xabcd_1234));
        assert_eq!(after.get(RegisterSpace::Uconfig, 0x262), Ok(0x7654_3210));
    }
    let reset = state(&[0xc000_1024, 0], first);
    for (space, offset) in [
        (RegisterSpace::Context, 0x29b),
        (RegisterSpace::Uconfig, 0x25b),
        (RegisterSpace::Uconfig, 0x262),
    ] {
        assert_eq!(reset.get(space, offset), Ok(0));
    }
    assert!(
        plan_commands(
            &[&bytes(&[0xc001_7900, 0x25c, 0])],
            first,
            &Default::default()
        )
        .is_err()
    );
}
#[test]
fn primitive_profile_uses_primary_active_masks_and_refuses_effects_before_payload_reads() {
    for (space, offset, value, reason) in [
        (
            RegisterSpace::Context,
            0x29b,
            1,
            "graphics output primitive profile",
        ),
        (
            RegisterSpace::Context,
            0x29b,
            3,
            "graphics output primitive profile",
        ),
        (
            RegisterSpace::Uconfig,
            0x25b,
            1,
            "graphics primitive grouping profile",
        ),
        (
            RegisterSpace::Uconfig,
            0x25b,
            1 << 9,
            "graphics primitive grouping profile",
        ),
        (
            RegisterSpace::Uconfig,
            0x262,
            4,
            "graphics extra user VGPR profile",
        ),
    ] {
        let source = Source::new();
        let plan = plan(&commands(&[(space, offset, value)]));
        assert!(
            matches!(prepare_draw(&plan.draws()[0],&source,&Default::default()),Err(Error::Unsupported(actual)) if actual==reason)
        );
        assert!(source.reads.borrow().is_empty());
        assert_eq!(source.snapshots.get(), 0);
    }
    for primitive in [0, 2] {
        let source = Source::new();
        let plan = plan(&commands(&[
            (RegisterSpace::Context, 0x29b, primitive),
            (RegisterSpace::Uconfig, 0x25b, 0xfffc_0000),
            (RegisterSpace::Uconfig, 0x262, 0xffff_fff8),
        ]));
        let prepared = prepare_draw(&plan.draws()[0], &source, &Default::default()).unwrap();
        assert_eq!(prepared.target_address(), TARGET);
        assert_eq!(prepared.input(), source.target.as_slice());
    }
}
