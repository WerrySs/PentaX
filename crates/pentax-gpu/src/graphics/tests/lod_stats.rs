// SPDX-License-Identifier: GPL-2.0-only
//! A producer packet alone cannot publish mip feedback.
use super::*;

#[test]
fn lod_stats_packets_refuse_before_an_executable_prefix_is_exposed() {
    for control in [0, 0x300f_fffc] {
        let words = [
            0xc003_3700u32,
            0x100,
            0x5000,
            0,
            0x1234_5678,
            0xc003_8e00,
            2112,
            0x6000,
            0,
            control,
        ];
        let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        let ranges = [CommandRange {
            address: 0x1000,
            size: bytes.len() as u64,
        }];
        assert!(matches!(
            plan_commands(&[&bytes], GraphicsState::empty(), &Default::default()),
            Err(Error::Unsupported(
                "LOD statistics feedback is not supported"
            ))
        ));
        assert!(matches!(
            OrderedCommands::prepare(
                &[&bytes],
                &ranges,
                GraphicsState::empty(),
                &Default::default()
            ),
            Err(Error::Unsupported(
                "LOD statistics feedback is not supported"
            ))
        ));
    }
}
