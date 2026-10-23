// SPDX-License-Identifier: GPL-2.0-only
//! Transfer is a writing action, never a synchronization or a draw receipt.
use super::*;

#[test]
fn graphics_static_captured_and_live_transfers_invalidate_prior_draw_even_for_zero_dma() {
    for transfer in [
        vec![0xc004_4000, 0x101, 0x8000, 0, 0x9000, 0],
        vec![0xc005_5000, 0x4000_0000, 0x0123_4567, 0, 0x9000, 0, 4096],
        vec![0xc005_5000, u32::MAX, 0, u32::MAX, 1, u32::MAX, 0xfc00_0000],
    ] {
        let mut words = commands(&[]);
        words.extend(&transfer);
        let data = bytes(&words);
        let plan = plan_commands(&[&data], GraphicsState::empty(), &Default::default()).unwrap();
        assert!(matches!(plan.memory_action(0), MemoryAction::Transfer(_)));
        let roots = [CommandRange {
            address: 0x4000,
            size: data.len() as u64,
        }];
        let mut ordered = OrderedCommands::prepare(
            &[&data],
            &roots,
            GraphicsState::empty(),
            &Default::default(),
        )
        .unwrap();
        assert!(matches!(
            ordered.next_step().unwrap(),
            Some(OrderedStep::Draw(_))
        ));
        assert!(matches!(
            ordered.next_step().unwrap(),
            Some(OrderedStep::Memory(0))
        ));
        assert!(matches!(
            ordered.memory_action(0),
            MemoryAction::Transfer(_)
        ));
        assert!(ordered.next_step().unwrap().is_none());
        struct Nested {
            root: Vec<u8>,
            child: Vec<u8>,
        }
        impl Memory for Nested {
            fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
                let input = match address {
                    0x4000 => &self.root,
                    0x8000 => &self.child,
                    _ => return Err(MemoryError::Unavailable),
                };
                if input.len() != output.len() {
                    return Err(MemoryError::Unavailable);
                }
                output.copy_from_slice(input);
                Ok(())
            }
        }
        let source = Nested {
            root: bytes(&[0xc002_3f00, 0x8000, 0, 0x0f20_0000 | words.len() as u32]),
            child: data.clone(),
        };
        let mut capture = CapturedCommands::capture(
            &[CommandRange {
                address: 0x4000,
                size: 16,
            }],
            &source,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(capture.command_ranges().len(), 2);
        let captured = capture
            .plan(GraphicsState::empty(), &Default::default())
            .unwrap();
        assert_eq!(captured.draws().len(), 1);
        assert!(matches!(
            captured.memory_action(0),
            MemoryAction::Transfer(_)
        ));
        words.extend([0xc004_105c, 1, 0, 1, 77, 0]);
        let data = bytes(&words);
        assert!(plan_commands(&[&data], GraphicsState::empty(), &Default::default()).is_err());
        assert!(
            OrderedCommands::prepare(
                &[&data],
                &[CommandRange {
                    address: 0x4000,
                    size: data.len() as u64
                }],
                GraphicsState::empty(),
                &Default::default()
            )
            .is_err()
        );
    }
}
