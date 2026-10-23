// SPDX-License-Identifier: GPL-2.0-only
//! Original source-state/planner consumers; no guest data or device execution.
use super::*;
use crate::compute::{CommandRange, Memory, MemoryError};
use crate::graphics::{CommandLimits, OrderedCommands, OrderedStep, plan_commands};
use std::cell::Cell;

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn ordered(words: &[u32], initial: GraphicsState) -> OrderedCommands {
    let data = bytes(words);
    OrderedCommands::prepare(
        &[&data],
        &[CommandRange {
            address: 0x4000,
            size: data.len() as u64,
        }],
        initial,
        &CommandLimits::default(),
    )
    .unwrap()
}

#[test]
fn trusted_five_defaults_admit_one_instance_without_initializing_detached_state() {
    let fresh = GraphicsState::for_new_processor();
    assert_eq!(fresh.instances, Some(1));
    assert_eq!(fresh.index_type, Some(IndexType::U16));
    assert_eq!(fresh.index_count, Some(0));
    assert_eq!(fresh.index_base, Some(0));
    assert_eq!(fresh.draw_indirect_base, Some(0));
    let mut detached = fresh;
    detached.instances = None;
    detached.index_type = None;
    detached.index_count = None;
    detached.index_base = None;
    detached.draw_indirect_base = None;
    // In particular no Context, SH, primitive or GPU address authority appears.
    assert_eq!(detached, GraphicsState::empty());
    assert_eq!(GraphicsState::default(), detached);
    let draw = bytes(&[0xc001_2d00, 3, 2]);
    assert!(matches!(
        plan_commands(&[&draw], detached, &CommandLimits::default()),
        Err(Error::Unsupported("unknown NUM_INSTANCES"))
    ));
    let plan = plan_commands(&[&draw], fresh, &CommandLimits::default()).unwrap();
    assert_eq!(plan.draws().len(), 1);
    assert_eq!(plan.draws()[0].instance_count(), 1);
    assert_eq!(plan.usage().vertices, 3);
}

#[test]
fn explicit_values_context_clear_and_trusted_reinitialization_have_distinct_scope() {
    let fresh = GraphicsState::for_new_processor();
    let explicit = bytes(&[0xc000_2f00, 7, 0xc001_2d00, 3, 2]);
    let first = plan_commands(&[&explicit], fresh, &CommandLimits::default()).unwrap();
    assert_eq!(first.draws()[0].instance_count(), 7);
    let draw = bytes(&[0xc001_2d00, 3, 2]);
    let next = plan_commands(&[&draw], first.final_state(), &CommandLimits::default()).unwrap();
    assert_eq!(next.draws()[0].instance_count(), 7); // Normal Dcb does not reset.
    let zero = bytes(&[0xc000_2f00, 0, 0xc001_2d00, 3, 2]);
    let no_draw = plan_commands(&[&zero], fresh, &CommandLimits::default()).unwrap();
    assert!(no_draw.draws().is_empty());
    assert_eq!(no_draw.usage().zero_draws, 1); // Retain explicit guest-zero policy.
    let mut dirty = first.final_state();
    dirty.index_type = Some(IndexType::U32);
    dirty.index_count = Some(9);
    dirty.index_base = Some(0x18000);
    dirty.draw_indirect_base = Some(0x20000);
    dirty.write(RegisterSpace::Context, 0x202, 0x321).unwrap();
    let clear = [0xc000_1200, 0];
    let cleared = plan_commands(&[&bytes(&clear)], dirty, &CommandLimits::default())
        .unwrap()
        .final_state();
    let mut live = ordered(&clear, dirty);
    assert!(live.next_step().unwrap().is_none());
    assert_eq!(live.final_state().unwrap(), cleared);
    assert_eq!(
        (
            cleared.instances,
            cleared.index_type,
            cleared.index_count,
            cleared.index_base,
            cleared.draw_indirect_base
        ),
        (
            dirty.instances,
            dirty.index_type,
            dirty.index_count,
            dirty.index_base,
            dirty.draw_indirect_base
        )
    );
    assert!(cleared.get(RegisterSpace::Context, 0x202).is_err());
    // The trusted host boundary selects the factory. This pure fixture makes
    // no frame/presentation claim and does not substitute that factory for a
    // guest processor-reset event with its separately retained contract.
    let reinitialized = GraphicsState::for_new_processor();
    assert_eq!(reinitialized, fresh);
    assert_ne!(reinitialized, cleared);
    let after = plan_commands(&[&draw], reinitialized, &CommandLimits::default()).unwrap();
    assert_eq!(after.draws()[0].instance_count(), 1);
    let mut packet_reset = dirty;
    packet_reset.reset(crate::queue_reset::Reset::Processor);
    assert_eq!(
        (packet_reset.instances, packet_reset.index_base),
        (dirty.instances, dirty.index_base)
    );
}

struct RefusedMemory(Cell<usize>);
impl Memory for RefusedMemory {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), MemoryError> {
        self.0.set(self.0.get() + 1);
        Err(MemoryError::Unavailable)
    }
}
#[test]
fn qualified_zero_bases_still_refuse_index_and_indirect_memory_authority() {
    let fresh = GraphicsState::for_new_processor();
    // Nonempty indexed OFFSET draw at the known zero base is still invalid.
    let indexed = bytes(&[0xc003_3500, 3, 0, 3, 0]);
    assert!(plan_commands(&[&indexed], fresh, &CommandLimits::default()).is_err());
    let mut live = ordered(&[0xc003_2500, 0, 0x8c, 0x8d, 0], fresh);
    assert!(matches!(
        live.next_step(),
        Ok(Some(OrderedStep::IndirectDraw))
    ));
    let memory = RefusedMemory(Cell::new(0));
    assert!(matches!(
        live.resolve_indirect(&memory),
        Err(Error::Unsupported("unknown draw indirect base"))
    ));
    assert_eq!(memory.0.get(), 0);
    // Even an explicit nonzero base confers no current owned GPU-read right.
    let mut explicit = fresh;
    explicit.draw_indirect_base = Some(0x20000);
    let mut live = ordered(&[0xc003_2500, 0, 0x8c, 0x8d, 0], explicit);
    assert!(matches!(
        live.next_step(),
        Ok(Some(OrderedStep::IndirectDraw))
    ));
    assert!(matches!(
        live.resolve_indirect(&memory),
        Err(Error::Memory(MemoryError::Unavailable))
    ));
    assert_eq!(memory.0.get(), 1);
}
