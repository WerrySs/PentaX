// SPDX-License-Identifier: GPL-2.0-only
//! Original owned-memory fixtures. No game/font asset discovery or native code.

use super::*;
use crate::tests::{ARG, TestGuest};

#[allow(clippy::too_many_arguments)]
fn run(
    state: &mut WritingState,
    strings: &[text::FontString],
    objects: &Objects,
    memory: &mut MemoryState,
    guest: &mut TestGuest,
    budget: &mut Budget,
    service: Service,
    args: &[u64],
) -> Result<u64, Failure> {
    let mut registers = [0; 6];
    registers[..args.len()].copy_from_slice(args);
    let held = state.metadata_bytes()
        + objects.metadata_bytes()
        + core::mem::size_of_val(strings)
        + strings
            .iter()
            .map(|string| capacity(&string.chars))
            .sum::<usize>();
    state.invoke(
        FontCall {
            service,
            args: CallArgs(registers),
            stack_pointer: None,
            vector_args: None,
        },
        strings,
        objects,
        memory,
        guest,
        budget,
        held,
    )
}

fn number(guest: &TestGuest, address: u64) -> u64 {
    u64::from_le_bytes(guest.get(address, 8).try_into().unwrap())
}

fn encoded(values: &[f32; 6]) -> [u8; 24] {
    let mut bytes = [0; 24];
    floats(values, &mut bytes);
    bytes
}

#[test]
fn writing_initialization_and_replacement_publish_exact_views_atomically() {
    let limits = crate::Limits::default();
    let mut memory = MemoryState::new(limits).unwrap();
    let mut budget = Budget::new(limits).unwrap();
    let objects = Objects::default();
    let mut guest = TestGuest::new();
    let mut state = WritingState::default();
    let owner = ARG + 0x100;
    guest.put(0x100, &[0xa5; 264]);
    guest.refuse = true;
    assert!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingInit,
            &[owner, 0, 0]
        )
        .is_err()
    );
    assert_eq!(
        (state.metadata_bytes(), state.retained_handle_count()),
        (0, 0)
    );
    assert_eq!(budget.usage().handles_live, 0);
    assert_eq!(guest.get(owner, 264), vec![0xa5; 264]);
    guest.refuse = false;
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingInit,
            &[owner, 0, 0]
        )
        .unwrap(),
        0
    );
    let runtime = number(&guest, owner);
    assert_eq!(guest.get(owner, 256), writing_view(runtime, 0));
    assert_eq!(guest.get(owner + 256, 8), [0xa5; 8]);
    assert_eq!(number(&guest, owner + 8), runtime + STEP);
    assert_eq!(number(&guest, owner + 24), runtime + LETTER);
    assert_eq!(number(&guest, owner + 32), runtime + METRICS);
    assert_eq!(guest.get(runtime + METRICS, 24), encoded(&fixed_metrics(0)));
    assert_eq!(guest.get(runtime + STEP + 8, 4), 8.0_f32.to_le_bytes());
    let glyph_width = f32::from_le_bytes(guest.get(runtime + STEP + 40, 4).try_into().unwrap());
    assert!(glyph_width > 0.0); // Actual fallback metrics, not zeroed glyph work.
    assert_eq!(
        (state.retained_handle_count(), budget.usage().handles_live),
        (1, 2)
    );
    let old_view = guest.get(owner, 256);
    let old_payload = guest.get(runtime, WRITING_BYTES);
    guest.refuse = true;
    assert!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingInit,
            &[owner, 0, 0]
        )
        .is_err()
    );
    assert_eq!(guest.get(owner, 256), old_view);
    assert_eq!(guest.get(runtime, WRITING_BYTES), old_payload);
    assert_eq!(budget.usage().handles_live, 2);
    guest.refuse = false;
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingInit,
            &[owner, 0, 0]
        )
        .unwrap(),
        0
    );
    assert_eq!(number(&guest, owner), runtime); // Reuse admits one atomic replacement.
    assert_eq!(budget.usage().handles_live, 2);
    assert!(!state.depends_on_font(0));
    assert!(!state.depends_on_string(0));
}

#[test]
fn writing_retains_the_selected_character_owner_and_refuses_foreign_view_clones() {
    let limits = crate::Limits::default();
    let mut memory = MemoryState::new(limits).unwrap();
    let mut budget = Budget::new(limits).unwrap();
    let objects = Objects::default();
    let mut guest = TestGuest::new();
    let string = memory
        .runtime_object(104, 0, &mut guest, &mut budget, |address, bytes| {
            bytes[64..72].copy_from_slice(&address.to_le_bytes());
            bytes[88..92].copy_from_slice(&u32::from(b'A').to_le_bytes());
            bytes[96..104].copy_from_slice(&(ARG + 0x900).to_le_bytes());
            Ok(RuntimeDataOutputs::new())
        })
        .unwrap();
    let strings = [text::FontString {
        runtime: string,
        chars: vec![text::Character {
            font: 0,
            code: u32::from(b'A'),
            order: ARG + 0x900,
            index: 0,
        }],
        form: 0x10,
        terminate_code: 0,
        terminate_order: 0,
    }];
    let mut state = WritingState::default();
    let owner = ARG + 0x100;
    let character = string.address + 64;
    assert_eq!(
        run(
            &mut state,
            &strings,
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingInit,
            &[owner, 0, character]
        )
        .unwrap(),
        0
    );
    let runtime = number(&guest, owner);
    assert!(state.depends_on_string(string.address));
    assert_eq!(number(&guest, owner + 16), character);
    assert_eq!(guest.get(runtime + STEP + 24, 4), 1_u32.to_le_bytes());
    assert_eq!(
        guest.get(runtime + STEP + 28, 4),
        u32::from(b'A').to_le_bytes()
    );
    assert_eq!(guest.get(runtime + LETTER + 16, 4), 1_u32.to_le_bytes());
    assert_eq!(
        run(
            &mut state,
            &strings,
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingRefersRenderStepCharacter,
            &[owner, ARG + 0x700]
        )
        .unwrap(),
        character
    );
    assert_eq!(number(&guest, ARG + 0x700), runtime + LETTER);
    assert_eq!(
        run(
            &mut state,
            &strings,
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingGetRenderMetrics,
            &[owner, ARG + 0x800]
        )
        .unwrap(),
        0
    );
    assert_eq!(guest.get(ARG + 0x800, 24), encoded(&fixed_metrics(1)));
    assert_eq!(
        run(
            &mut state,
            &strings,
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingSetMaskInvisible,
            &[owner, 1]
        )
        .unwrap(),
        0
    );
    assert_eq!(guest.get(runtime + STEP + 24, 4), 1_u32.to_le_bytes()); // Mask is stored only.
    guest.put(0x600, &runtime.to_le_bytes());
    let before = guest.get(runtime, WRITING_BYTES);
    assert!(matches!(
        run(
            &mut state,
            &strings,
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingSetMaskInvisible,
            &[ARG + 0x600, 2]
        ),
        Err(Failure::Unsupported("foreign font writing view"))
    ));
    assert_eq!(guest.get(runtime, WRITING_BYTES), before);
    guest.put(0x100, &0_u64.to_le_bytes());
    assert_eq!(
        run(
            &mut state,
            &strings,
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingRefersRenderStep,
            &[owner]
        )
        .unwrap(),
        0
    );
    assert!(state.depends_on_string(string.address)); // Corrupting a view cannot drop ownership.
}

#[test]
fn line_steps_metrics_overlap_cursor_limits_and_destruction_use_real_owned_backing() {
    let limits = crate::Limits::default();
    let mut memory = MemoryState::new(limits).unwrap();
    let mut budget = Budget::new(limits).unwrap();
    let objects = Objects::default();
    let mut guest = TestGuest::new();
    let mut state = WritingState::default();
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontCreateWritingLine,
            &[0, 0x10, u64::MAX, ARG]
        )
        .unwrap(),
        0
    );
    let line = number(&guest, ARG);
    assert_eq!(
        budget.usage().mapped_bytes,
        (LINE_BYTES as u64).div_ceil(0x4000) * 0x4000
    );
    assert_eq!(
        (state.retained_handle_count(), budget.usage().handles_live),
        (1, 2)
    );
    guest.put(0x800, &encoded(&[5.0, 2.0, -3.0, 7.0, -1.0, 6.0]));
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineWritesOrder,
            &[line, u64::MAX, ARG + 0x800, 0x1122]
        )
        .unwrap(),
        0
    );
    let first_step = guest.get(line + LINE_HEADER as u64, 64);
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineWritesOrder,
            &[line, 0, 0, 0x3344]
        )
        .unwrap(),
        0
    );
    assert_eq!(guest.get(line + LINE_HEADER as u64, 64), first_step);
    assert_eq!(number(&guest, line + LINE_HEADER as u64 + 24), 0x1122);
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineGetRenderMetrics,
            &[line, ARG + 0x900]
        )
        .unwrap(),
        0
    );
    assert_eq!(
        guest.get(ARG + 0x900, 24),
        encoded(&[13.0, 2.0, -12.0, 7.0, -1.0, 13.0])
    );
    guest.put(0xa00, &[0xcc; 16]);
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineGetOrderingSpace,
            &[line, ARG + 0xa00, 0, ARG + 0xa04, 0]
        )
        .unwrap(),
        0
    );
    assert_eq!(
        guest.get(ARG + 0xa00, 8),
        [13.0_f32.to_le_bytes(), 13.0_f32.to_le_bytes()].concat()
    );
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineGetOrderingSpace,
            &[line, ARG + 0xa00, ARG + 0xa00, ARG + 0xa00, ARG + 0xa00]
        )
        .unwrap(),
        0
    );
    assert_eq!(guest.get(ARG + 0xa00, 4), [0; 4]);
    assert_eq!(guest.get(ARG + 0xa08, 8), [0xcc; 8]);
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineRefersRenderStep,
            &[line]
        )
        .unwrap(),
        line + 64
    );
    guest.refuse = true;
    assert!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineRefersRenderStep,
            &[line]
        )
        .is_err()
    );
    assert_eq!(state.lines[0].cursor, 1);
    guest.refuse = false;
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineRefersRenderStep,
            &[line]
        )
        .unwrap(),
        line + 128
    );
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineRefersRenderStep,
            &[line]
        )
        .unwrap(),
        0
    );
    state.lines[0].count = LINE_MAX_STEPS;
    let header = guest.get(line, 64);
    assert!(matches!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineWritesOrder,
            &[line, 0, u64::MAX, 0]
        ),
        Err(Failure::Limit("font writing line steps"))
    ));
    assert_eq!(guest.get(line, 64), header);
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineClear,
            &[line]
        )
        .unwrap(),
        0
    );
    assert_eq!(state.lines[0].metrics, fixed_metrics(0));
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontWritingLineRefersRenderStep,
            &[line]
        )
        .unwrap(),
        0
    );
    guest.refuse = true;
    assert!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontDestroyWritingLine,
            &[ARG]
        )
        .is_err()
    );
    assert_eq!(number(&guest, ARG), line);
    assert_eq!(budget.usage().handles_live, 2);
    guest.refuse = false;
    assert_eq!(
        run(
            &mut state,
            &[],
            &objects,
            &mut memory,
            &mut guest,
            &mut budget,
            Service::FontDestroyWritingLine,
            &[ARG]
        )
        .unwrap(),
        0
    );
    assert_eq!(number(&guest, ARG), 0);
    assert_eq!(budget.usage().handles_live, 0);
    let mut byte = [0];
    assert!(guest.read(line, &mut byte).is_err());
}
