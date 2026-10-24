// SPDX-License-Identifier: GPL-2.0-only
//! Original ordered LOD-request data consumers; no GPU results are invented.
use super::*;

fn encoded(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

#[test]
fn lod_requests_retain_exact_packet_order_and_policy_without_memory_reads() {
    let bytes = encoded(&[
        0xc003_8e00,
        0,
        0,
        0,
        0,
        0xc003_8e00,
        65,
        0x4000,
        0,
        0x2008_d008,
        0xc003_8e00,
        0,
        0,
        0,
        1 << 18,
    ]);
    let plan = plan_commands(&[&bytes], GraphicsState::empty(), &Default::default()).unwrap();
    assert!(plan.draws().is_empty());
    assert_eq!(plan.actions().len(), 3);
    assert_eq!(plan.usage().write_bytes, 65);
    let [
        Action::LodStats(probe),
        Action::LodStats(report),
        Action::LodStats(reset),
    ] = plan.actions()
    else {
        panic!("expected three source-ordered LOD actions");
    };
    assert!(probe.is_empty_probe());
    assert_eq!(
        (report.destination(), report.completion_bytes()),
        (0x4000, 65)
    );
    assert_eq!(
        (
            report.reset_count(),
            report.interval(),
            report.cache_policy()
        ),
        (52, 2, 2)
    );
    assert!(report.report_and_reset());
    assert!(reset.force_reset());
    let range = CommandRange {
        address: 0x1000,
        size: bytes.len() as u64,
    };
    let mut ordered = OrderedCommands::prepare(
        &[&bytes],
        &[range],
        GraphicsState::empty(),
        &Default::default(),
    )
    .unwrap();
    for expected in [*probe, *report, *reset] {
        let Some(OrderedStep::LodStats(actual)) = ordered.next_step().unwrap() else {
            panic!("expected live LOD action");
        };
        assert_eq!(actual, expected);
    }
    assert!(ordered.next_step().unwrap().is_none());
    ordered.final_state().unwrap();
}

#[test]
fn malformed_lod_suffix_never_returns_a_plan_with_an_executable_write_prefix() {
    for suffix in [
        vec![0xc003_8e00, 2112, 0x4001, 0, 0],
        vec![0xc003_8e00, 2112, 0x4000, 0, 1 << 20],
        vec![0xc002_8e00, 2112, 0x4000, 0],
        vec![0xc003_8e00, 2112, 0x4000, 0],
    ] {
        let mut words = vec![0xc003_3700, 0x100, 0x5000, 0, 0x1234_5678];
        words.extend(suffix);
        let bytes = encoded(&words);
        assert!(plan_commands(&[&bytes], GraphicsState::empty(), &Default::default()).is_err());
        let ranges = [CommandRange {
            address: 0x1000,
            size: bytes.len() as u64,
        }];
        assert!(
            OrderedCommands::prepare(
                &[&bytes],
                &ranges,
                GraphicsState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
fn lod_actions_consume_the_existing_action_and_packet_budgets() {
    let bytes = encoded(&[0xc003_8e00, 2112, 0x4000, 0, 0]);
    let no_actions = CommandLimits {
        max_draws: 0,
        max_dispatches: 0,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&[&bytes], GraphicsState::empty(), &no_actions),
        Err(Error::Limit("command actions"))
    ));
    let no_packets = CommandLimits {
        max_packets: 0,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&[&bytes], GraphicsState::empty(), &no_packets),
        Err(Error::Limit("command packets"))
    ));
}
