// SPDX-License-Identifier: GPL-2.0-only
//! Original exact tagged packets and ordered draw authority boundaries.
use super::*;

#[test]
fn exact_gpu_flip_actions_preserve_draw_order_signed_payloads_and_fresh_cycles() {
    let mut words = commands(&[]);
    words.extend([0xc004_105c, 1, 3, 2, 0x7654_3210, 0xfedc_ba98]);
    set(&mut words, RegisterSpace::Sh, 0x0c, &[0.25f32.to_bits()]);
    words.extend([0xc001_2d00, 6, 2, 0xc004_105c, 2, 0, 1, 7, 0]);
    let p = plan(&words);
    assert_eq!(p.draws().len(), 2);
    assert_eq!(p.actions().len(), 4);
    assert_eq!(p.usage().flips, 2);
    assert_eq!(p.actions()[0], Action::Draw(0));
    assert_eq!(p.actions()[2], Action::Draw(1));
    let Action::Flip(first) = p.actions()[1] else {
        panic!("first flip missing");
    };
    let Action::Flip(second) = p.actions()[3] else {
        panic!("second flip missing");
    };
    assert_eq!(
        (
            first.handle(),
            first.buffer(),
            first.mode(),
            first.argument()
        ),
        (1, 3, 2, 0xfedc_ba98_7654_3210u64 as i64)
    );
    assert_eq!(
        (
            second.handle(),
            second.buffer(),
            second.mode(),
            second.argument()
        ),
        (2, 0, 1, 7)
    );
    assert_eq!(p.draws()[0].vertex_count(), 3);
    assert_eq!(p.draws()[1].vertex_count(), 6);
    assert_eq!(
        p.draws()[1].state.get(RegisterSpace::Sh, 0x0c).unwrap(),
        0.25f32.to_bits()
    );
}

#[test]
fn tagged_flip_header_and_source_authority_refusals_never_return_an_executable_prefix() {
    for header in [0xc004_105d, 0xc004_1058, 0xc003_105c, 0xc005_105c] {
        let mut words = commands(&[]);
        words.extend([header, 1, 0, 1, 0, 0]);
        assert!(
            plan_commands(
                &[&bytes(&words)],
                GraphicsState::empty(),
                &CommandLimits::default()
            )
            .is_err()
        );
    }
    let exact = [0xc004_105c, 1, 0, 1, 0, 0];
    assert!(
        plan_commands(
            &[&bytes(&exact)],
            GraphicsState::empty(),
            &CommandLimits::default()
        )
        .is_err()
    );
    let mut repeated = commands(&[]);
    repeated.extend(exact);
    repeated.extend(exact);
    assert!(
        plan_commands(
            &[&bytes(&repeated)],
            GraphicsState::empty(),
            &CommandLimits::default()
        )
        .is_err()
    );
    let mut truncated = commands(&[]);
    truncated.extend(&exact[..5]);
    assert!(
        plan_commands(
            &[&bytes(&truncated)],
            GraphicsState::empty(),
            &CommandLimits::default()
        )
        .is_err()
    );
    for marker in [0x6875_0777, 0x6875_0778, 0x6875_0781] {
        let mut words = commands(&[]);
        words.extend([0xc000_1000, marker]);
        assert!(matches!(
            plan_commands(
                &[&bytes(&words)],
                GraphicsState::empty(),
                &CommandLimits::default()
            ),
            Err(Error::Unsupported(_))
        ));
    }
}
