// SPDX-License-Identifier: GPL-2.0-only
//! Original exact wait packets and bounded static/live action order.
use super::*;

#[test]
fn selected_wait_packets_are_sealed_exactly_and_consume_static_and_live_actions_once() {
    let words = [
        0xc005_1018,
        2,
        (-2i32) as u32,
        0,
        0,
        0,
        0,
        0xc005_1018,
        1,
        15,
        0,
        0,
        0,
        0,
    ];
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    let plan = plan_commands(&[&bytes], GraphicsState::empty(), &Default::default()).unwrap();
    let Action::WaitFlip(first) = plan.actions()[0] else {
        panic!("first exact wait missing")
    };
    assert_eq!((first.handle(), first.buffer()), (2, -2));
    assert_eq!(plan.actions().len(), 2);
    let mut live = OrderedCommands::prepare(
        &[&bytes],
        &[CommandRange {
            address: 0x1000,
            size: bytes.len() as u64,
        }],
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    for expected in [(2, -2), (1, 15)] {
        let Some(OrderedStep::WaitFlip(wait)) = live.next_step().unwrap() else {
            panic!("live wait missing")
        };
        assert_eq!((wait.handle(), wait.buffer()), expected);
    }
    assert!(live.next_step().unwrap().is_none());
    assert!(live.final_state().is_ok());
    for index in 3..7 {
        let mut bad = bytes.clone();
        bad[index * 4] = 1;
        assert!(plan_commands(&[&bad], GraphicsState::empty(), &Default::default()).is_err());
        assert!(
            OrderedCommands::prepare(
                &[&bad],
                &[CommandRange {
                    address: 0x1000,
                    size: bad.len() as u64
                }],
                GraphicsState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
    assert!(plan_commands(&[&bytes[..24]], GraphicsState::empty(), &Default::default()).is_err());
    for header in [0xc004_1018u32, 0xc006_1018, 0xc005_1019] {
        let mut bad = bytes.clone();
        bad[..4].copy_from_slice(&header.to_le_bytes());
        assert!(plan_commands(&[&bad], GraphicsState::empty(), &Default::default()).is_err());
        assert!(
            OrderedCommands::prepare(
                &[&bad],
                &[CommandRange {
                    address: 0x1000,
                    size: bad.len() as u64
                }],
                GraphicsState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
    let mut collision = bytes.clone();
    collision[4..8].copy_from_slice(&0x6875_0777u32.to_le_bytes());
    assert!(plan_commands(&[&collision], GraphicsState::empty(), &Default::default()).is_ok());
    // Numeric port data is not a legacy untyped flip marker or port authority.
}

#[test]
fn wait_action_capacity_and_fresh_draw_authority_are_admitted_before_any_executable_prefix() {
    let mut limits = CommandLimits::default();
    limits.max_draws = 1;
    let words = [
        0xc003_3700,
        0x100,
        0x30000,
        0,
        1, // First real owned-write action.
        0xc005_1018,
        1,
        0,
        0,
        0,
        0,
        0, // Second action fills2*max_draws.
        0xc005_1018,
        1,
        0,
        0,
        0,
        0,
        0,
    ];
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    assert!(matches!(
        plan_commands(&[&bytes], GraphicsState::empty(), &limits),
        Err(Error::Limit("command actions"))
    ));
    let mut words = commands(&[]);
    words.extend([0xc005_1018, 1, 0, 0, 0, 0, 0, 0xc004_105c, 1, 0, 1, 0, 0]);
    let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    assert!(matches!(
        plan_commands(&[&bytes], GraphicsState::empty(), &Default::default()),
        Err(Error::Unsupported("GPU flip needs a fresh draw"))
    ));
}
