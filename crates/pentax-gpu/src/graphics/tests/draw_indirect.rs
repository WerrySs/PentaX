// SPDX-License-Identifier: GPL-2.0-only
//! Original live argument/state consumers; structural admission reads no payload.
use super::*;
use crate::compute::MemoryError;
struct Args {
    value: Cell<[u32; 5]>,
    reads: Cell<usize>,
    deny: Cell<bool>,
}
impl Args {
    fn new(value: [u32; 5]) -> Self {
        Self {
            value: Cell::new(value),
            reads: Cell::new(0),
            deny: Cell::new(false),
        }
    }
}
impl Memory for Args {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.reads.set(self.reads.get() + 1);
        assert_eq!((address, output.len()), (0x8020, 20));
        if self.deny.get() {
            return Err(MemoryError::ReadDenied);
        }
        output.copy_from_slice(&bytes(&self.value.get()));
        Ok(())
    }
}
fn initial() -> GraphicsState {
    let mut state = GraphicsState::empty();
    state.index_type = Some(IndexType::U16);
    state.index_base = Some(0xa000);
    state.index_count = Some(4);
    state.instances = Some(99);
    state
}
fn stream(patches: [u32; 2]) -> Vec<u32> {
    vec![
        0xc002_1100,
        1,
        0x8007,
        0,
        0xc003_2500,
        0x20,
        patches[0],
        patches[1],
        0,
    ]
}
fn live(words: &[u32], state: GraphicsState) -> OrderedCommands {
    let data = bytes(words);
    OrderedCommands::prepare(
        &[&data],
        &[CommandRange {
            address: 0x4000,
            size: data.len() as u64,
        }],
        state,
        &Default::default(),
    )
    .unwrap()
}
#[test]
fn live_indexed_arguments_patch_current_user_data_and_clamp_count_without_double_index_offset() {
    let args = Args::new([999, 999, 999, 999, 999]);
    let words = stream([0x008e_008c, 0x1000_008d]);
    assert!(plan_commands(&[&bytes(&words)], initial(), &Default::default()).is_err());
    let mut plan = live(&words, initial());
    assert_eq!(args.reads.get(), 0);
    assert!(matches!(
        plan.next_step(),
        Ok(Some(OrderedStep::IndirectDraw))
    ));
    assert!(plan.next_step().is_err()); // Cannot skip the unresolved live read.
    args.value.set([9, 2, 3, (-2i32) as u32, 7]);
    let draw = plan.resolve_indirect(&args).unwrap().unwrap();
    assert_eq!((draw.vertex_count(), draw.instance_count()), (4, 2));
    assert_eq!(draw.indices.unwrap().address, 0xa006);
    let offsets = draw.native_offsets.unwrap();
    assert_eq!(
        (
            offsets.base_vertex(),
            offsets.first_instance(),
            offsets.first_index()
        ),
        (-2, 7, 3)
    );
    assert_eq!(draw.state.get(RegisterSpace::Sh, 0x8c), Ok((-2i32) as u32));
    assert_eq!(draw.state.get(RegisterSpace::Sh, 0x8d), Ok(7));
    assert_eq!(draw.state.get(RegisterSpace::Sh, 0x8e), Ok(3));
    assert!(plan.next_step().unwrap().is_none());
    assert_eq!(plan.final_state().unwrap().instances, Some(2));
    assert_eq!(args.reads.get(), 1);
}
#[test]
fn live_zero_draw_keeps_patch_order_and_later_direct_actions_and_noncontext_history() {
    let args = Args::new([3, 0, 6, (-2i32) as u32, 7]);
    // All three destinations alias: source's final first-index write wins.
    let mut words = stream([0x008c_008c, 0x1000_008c]);
    words.extend([0xc001_2d00, 3, 2, 0xc000_2f00, 2, 0xc001_2d00, 3, 2]);
    let mut plan = live(&words, initial());
    assert!(matches!(
        plan.next_step().unwrap(),
        Some(OrderedStep::IndirectDraw)
    ));
    assert!(plan.resolve_indirect(&args).unwrap().is_none());
    // The first AUTO retires as zero; its reserved action is still consumed.
    let Some(OrderedStep::Draw(draw)) = plan.next_step().unwrap() else {
        panic!("actual later draw");
    };
    assert_eq!(draw.instance_count(), 2);
    assert_eq!(draw.state.get(RegisterSpace::Sh, 0x8c), Ok(6));
    assert!(plan.next_step().unwrap().is_none());
    let final_state = plan.final_state().unwrap();
    assert_eq!(final_state.draw_indirect_base, Some(0x8000));
    let mut history = final_state;
    history
        .context_operation(crate::context_state::Operation::Push)
        .unwrap();
    history
        .context_operation(crate::context_state::Operation::Clear)
        .unwrap();
    history
        .context_operation(crate::context_state::Operation::Pop)
        .unwrap();
    assert_eq!(history.draw_indirect_base, Some(0x8000));
    history.reset(crate::queue_reset::Reset::Processor);
    assert_eq!(history.draw_indirect_base, Some(0));
    let mut noop = live(&stream([0x280, 0x280]), initial());
    assert!(matches!(
        noop.next_step().unwrap(),
        Some(OrderedStep::IndirectDraw)
    ));
    args.value.set([3, 1, 0, 123, 456]);
    let draw = noop.resolve_indirect(&args).unwrap().unwrap();
    assert!(draw.state.get(RegisterSpace::Sh, 0x8c).is_err());
}
#[test]
fn indirect_argument_refusals_and_action_capacity_are_admitted_without_payload_or_extra_allocation()
{
    let args = Args::new([3, 1, 0, 0, 0]);
    let words = stream([0x280, 0x280]);
    let data = bytes(&words);
    let mut capped = vec![0xc000_4200, 0, 0xc000_4200, 0];
    capped.extend(&words);
    let capped = bytes(&capped);
    let limits = CommandLimits {
        max_draws: 1,
        max_dispatches: 0,
        ..Default::default()
    };
    assert!(matches!(
        OrderedCommands::prepare(
            &[&capped],
            &[CommandRange {
                address: 0x4000,
                size: capped.len() as u64
            }],
            initial(),
            &limits
        ),
        Err(Error::Limit("command actions"))
    ));
    let limited = CommandLimits {
        max_work: 1,
        ..Default::default()
    };
    assert!(
        OrderedCommands::prepare(
            &[&data],
            &[CommandRange {
                address: 0x4000,
                size: data.len() as u64
            }],
            initial(),
            &limited
        )
        .is_err()
    );
    assert_eq!(args.reads.get(), 0);
    let mut denied = live(&words, initial());
    assert!(matches!(
        denied.next_step().unwrap(),
        Some(OrderedStep::IndirectDraw)
    ));
    args.deny.set(true);
    assert!(matches!(
        denied.resolve_indirect(&args),
        Err(Error::Memory(MemoryError::ReadDenied))
    ));
    assert!(denied.next_step().is_err());
    assert!(denied.resolve_indirect(&args).is_err());
    assert_eq!(args.reads.get(), 1);
    for patches in [[0x10c, 0x280], [0x8c, 0x2000_0280], [0x8c, 0x8d]] {
        let mut bad = stream(patches);
        if patches == [0x8c, 0x8d] {
            bad[8] = 1;
        }
        let data = bytes(&bad);
        assert!(
            OrderedCommands::prepare(
                &[&data],
                &[CommandRange {
                    address: 0x4000,
                    size: data.len() as u64
                }],
                initial(),
                &Default::default()
            )
            .is_err()
        );
    }
}
