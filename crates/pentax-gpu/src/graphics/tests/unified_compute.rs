// SPDX-License-Identifier: GPL-2.0-only
//! Original unified-processor streams; no GPU effect or imported shader bytes.
use super::*;
use crate::compute::{ComputeState, MemoryError};

fn set(first: u32, values: &[u32]) -> Vec<u32> {
    let mut out = vec![0xc000_7600 | ((values.len() as u32) << 16), first];
    out.extend_from_slice(values);
    out
}
fn direct() -> [u32; 5] {
    [0xc003_1500, 1, 1, 1, 0x41]
}

#[test]
fn unified_cs_snapshots_and_retired_state_preserve_distinct_graphics_and_user_banks() {
    let mut words = set(0x08, &[101]);
    words.extend(set(0x240, &(0..16).map(|v| v + 17).collect::<Vec<_>>()));
    words.extend(direct());
    words.extend(set(0x240, &[71]));
    words.extend(direct());
    let plan = plan_commands_with_compute(
        &[&bytes(&words)],
        GraphicsState::empty(),
        ComputeState::for_new_queue(),
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(plan.actions(), &[Action::Dispatch(0), Action::Dispatch(1)]);
    assert_eq!(plan.usage().dispatch_packets, 2);
    assert_eq!(plan.draws().len(), 0);
    assert_eq!(plan.final_state().get(RegisterSpace::Sh, 0x08), Ok(101));
    assert!(plan.final_state().get(RegisterSpace::Sh, 0x240).is_err());
    let mut first = ComputeState::for_new_queue();
    for reg in 0..16 {
        first.write(0x240 + reg, reg + 17).unwrap();
    }
    assert_eq!(plan.dispatches()[0].diagnostic_state(), first);
    first.write(0x240, 71).unwrap();
    assert_eq!(plan.dispatches()[1].diagnostic_state(), first);
    assert_eq!(plan.final_compute_state(), first);
    let later = plan_commands_with_compute(
        &[&bytes(&direct())],
        plan.final_state(),
        plan.final_compute_state(),
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(later.dispatches()[0].diagnostic_state(), first);
    // Detached entrypoints never borrow an asynchronous queue or make START known.
    let detached = plan_commands(
        &[&bytes(&direct())],
        GraphicsState::empty(),
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(
        detached.dispatches()[0].diagnostic_state(),
        ComputeState::empty()
    );
    let clear = plan_commands_with_compute(
        &[&bytes(&[0xc000_1200, 2])],
        plan.final_state(),
        first,
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(clear.final_compute_state(), first);
}

struct ArraySource {
    words: RefCell<[u32; 2]>,
    reads: Cell<usize>,
}
impl crate::compute::Memory for ArraySource {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        if address != 0x4000 || output.len() != 8 {
            return Err(MemoryError::Unavailable);
        }
        self.reads.set(self.reads.get() + 1);
        output.copy_from_slice(&bytes(&*self.words.borrow()));
        Ok(())
    }
}
#[test]
fn ordered_cs_array_is_read_at_its_cursor_and_later_writes_do_not_rewrite_prior_dispatch() {
    let source = ArraySource {
        words: RefCell::new([0x240, 7]),
        reads: Cell::new(0),
    };
    let mut words = set(0x08, &[101]);
    words.extend([0xc003_6300, 0x4000, 0, 0x8000_0000, 1]);
    words.extend(direct());
    words.extend(set(0x240, &[29]));
    words.extend(direct());
    let data = bytes(&words);
    let mut plan = OrderedCommands::prepare_with_compute(
        &[&data],
        &[crate::compute::CommandRange {
            address: 0x8000,
            size: data.len() as u64,
        }],
        GraphicsState::empty(),
        ComputeState::for_new_queue(),
        &CommandLimits::default(),
    )
    .unwrap();
    assert_eq!(source.reads.get(), 0);
    assert!(plan.final_compute_state().is_err());
    assert!(matches!(plan.next_step(), Ok(Some(OrderedStep::Array))));
    source.words.borrow_mut()[1] = 23;
    plan.resolve_array(&source).unwrap();
    assert_eq!(source.reads.get(), 1);
    let Some(OrderedStep::Dispatch(first)) = plan.next_step().unwrap() else {
        panic!("first dispatch missing")
    };
    let mut expected = ComputeState::for_new_queue();
    expected.write(0x240, 23).unwrap();
    assert_eq!(first.diagnostic_state(), expected);
    source.words.borrow_mut()[1] = 99;
    let Some(OrderedStep::Dispatch(second)) = plan.next_step().unwrap() else {
        panic!("second dispatch missing")
    };
    expected.write(0x240, 29).unwrap();
    assert_eq!(second.diagnostic_state(), expected);
    let mut first_expected = expected;
    first_expected.write(0x240, 23).unwrap();
    assert_eq!(first.diagnostic_state(), first_expected);
    assert_eq!(source.reads.get(), 1);
    assert!(plan.next_step().unwrap().is_none());
    assert_eq!(plan.final_compute_state().unwrap(), expected);
    assert_eq!(
        plan.final_state().unwrap().get(RegisterSpace::Sh, 0x08),
        Ok(101)
    );
}

#[test]
fn cs_unknown_user_boundary_and_dispatch_host_work_quotas_refuse_without_executable_prefix() {
    for first in [0x218, 0x250] {
        assert!(
            plan_commands(
                &[&bytes(&set(first, &[1]))],
                GraphicsState::empty(),
                &CommandLimits::default()
            )
            .is_err()
        );
    }
    assert!(
        plan_commands(
            &[&bytes(&set(0x24f, &[1, 2]))],
            GraphicsState::empty(),
            &CommandLimits::default()
        )
        .is_err()
    );
    let mut limits = CommandLimits {
        max_draws: 0,
        max_dispatches: 1,
        ..Default::default()
    };
    let data = bytes(&direct());
    let plan = plan_commands(&[&data], GraphicsState::empty(), &limits).unwrap();
    assert_eq!(plan.dispatches().len(), 1); // Compute-only does not need a draw quota.
    limits.max_host_bytes = plan.usage().host_bytes - 1;
    assert!(matches!(
        plan_commands(&[&data], GraphicsState::empty(), &limits),
        Err(Error::Limit(_))
    ));
    limits = CommandLimits {
        max_draws: 0,
        max_dispatches: 1,
        ..Default::default()
    };
    limits.max_work = plan.usage().work - 1;
    assert!(matches!(
        plan_commands(&[&data], GraphicsState::empty(), &limits),
        Err(Error::Limit(_))
    ));
    limits = CommandLimits {
        max_dispatches: 0,
        ..Default::default()
    };
    assert!(matches!(
        plan_commands(&[&data], GraphicsState::empty(), &limits),
        Err(Error::Limit("command dispatches"))
    ));
}

#[test]
fn dispatch_consumes_fresh_draw_and_no_dispatch_stream_reserves_no_cs_snapshot_vector() {
    let mut words = commands(&[]);
    words.extend([0xc003_1500, 0, 0, 0, 0x41]);
    words.extend([0xc004_105c, 1, 0, 1, 0, 0]);
    assert!(matches!(
        plan_commands(
            &[&bytes(&words)],
            GraphicsState::empty(),
            &CommandLimits::default()
        ),
        Err(Error::Unsupported("GPU flip needs a fresh draw"))
    ));
    let empty = bytes(&[0x8000_0000]);
    let zero = CommandLimits {
        max_draws: 0,
        max_dispatches: 0,
        ..Default::default()
    };
    let with = CommandLimits {
        max_dispatches: 128,
        ..zero
    };
    let a = plan_commands(&[&empty], GraphicsState::empty(), &zero).unwrap();
    let b = plan_commands(&[&empty], GraphicsState::empty(), &with).unwrap();
    // Only the explicitly enlarged action pool is retained; no CS vector is allocated.
    let actions = 128 * std::mem::size_of::<Action>();
    assert_eq!(b.usage().host_bytes - a.usage().host_bytes, actions);
    assert_eq!(b.usage().work - a.usage().work, actions as u64);
    assert!(b.dispatches().is_empty());
}
