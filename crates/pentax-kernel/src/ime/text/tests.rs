// SPDX-License-Identifier: GPL-2.0-only
// Original byte-layout, source-order and transactional IME examples.

use super::*;
use crate::tests::{ARG, PAGE, TestGuest};
use crate::{Limits, MemoryEffect};

const PARAM: u64 = ARG + 0x100;
const INPUT: u64 = ARG + 0x1000;
const WORK: u64 = ARG + 4 * PAGE;

struct Fixture {
    state: TextState,
    guest: TestGuest,
    budget: Budget,
}

impl Fixture {
    fn new() -> Self {
        let mut guest = TestGuest::new();
        guest
            .apply(
                MemoryEffect::MapZeroed {
                    address: WORK,
                    length: 2 * PAGE,
                    protection: 3,
                },
                &[],
            )
            .unwrap();
        let mut fixture = Self {
            state: TextState::default(),
            guest,
            budget: Budget::new(Limits::default()).unwrap(),
        };
        fixture.put(WORK, &vec![0xa5; WORK_BYTES + 2]);
        fixture.put(INPUT, &[0x7b; 100]);
        fixture.put(PARAM, &parameters(WORK));
        fixture.units(INPUT, &[0]);
        fixture
    }
    fn put(&mut self, address: u64, bytes: &[u8]) {
        self.guest
            .apply(MemoryEffect::None, &[GuestWrite { address, bytes }])
            .unwrap();
    }
    fn units(&mut self, address: u64, units: &[u16]) {
        let bytes = units
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();
        self.put(address, &bytes);
    }
    fn call(&mut self, service: Service, arguments: &[u64]) -> Result<u64, Failure> {
        let mut args = [0; 6];
        args[..arguments.len()].copy_from_slice(arguments);
        let held = self.state.metadata_bytes();
        self.state.invoke(
            service,
            CallArgs(args),
            &mut self.guest,
            &mut self.budget,
            held,
        )
    }
    fn protect_input(&mut self, protection: u32) {
        self.guest
            .apply(
                MemoryEffect::Protect {
                    address: ARG,
                    length: PAGE,
                    protection,
                },
                &[],
            )
            .unwrap();
    }
    fn open(&mut self) {
        assert_eq!(self.call(Service::ImeOpen, &[PARAM, 0]), Ok(0));
    }
}

fn parameters(work: u64) -> [u8; 96] {
    let mut bytes = [0; 96];
    bytes[..4].copy_from_slice(&7u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&4u32.to_le_bytes());
    bytes[36..40].copy_from_slice(&32u32.to_le_bytes());
    bytes[40..48].copy_from_slice(&INPUT.to_le_bytes());
    bytes[48..52].copy_from_slice(&12.5f32.to_le_bytes());
    bytes[52..56].copy_from_slice(&23.0f32.to_le_bytes());
    bytes[64..72].copy_from_slice(&work.to_le_bytes());
    bytes[72..80].copy_from_slice(&0x1111_2222_3333_4444u64.to_le_bytes());
    bytes[80..88].copy_from_slice(&0x5555_6666_7777_8888u64.to_le_bytes());
    bytes
}

#[test]
fn param_init_and_panel_outputs_preserve_tails_and_source_order_aliases() {
    let mut f = Fixture::new();
    f.put(PARAM, &[0xa5; 98]);
    assert_eq!(f.call(Service::ImeParamInit, &[0]), Ok(0));
    assert_eq!(f.call(Service::ImeParamInit, &[PARAM]), Ok(0));
    let mut initialized = [0; 98];
    initialized[..4].fill(0xff);
    initialized[96..].fill(0xa5);
    assert_eq!(f.guest.get(PARAM, 98), initialized);

    f.put(PARAM, &parameters(WORK));
    f.put(PARAM + 32, &OVER_2K.to_le_bytes());
    let output = ARG + 0x300;
    f.put(output, &[0xa5; 6]);
    // width=370; height=402 overwrites width; width*=2 ->804;
    // height*=2 reads that current804 and therefore produces1608.
    assert_eq!(
        f.call(Service::ImeGetPanelSize, &[PARAM, output, output]),
        Ok(0)
    );
    assert_eq!(
        f.guest.get(output, 6),
        [1608u32.to_le_bytes().as_slice(), &[0xa5; 2]].concat()
    );

    f.put(PARAM, &parameters(WORK));
    // The first width store overwrites type4 with370 before height chooses its
    // non-Number branch. A snapshot-only implementation incorrectly gives402.
    assert_eq!(
        f.call(Service::ImeGetPanelSize, &[PARAM, PARAM + 4, output]),
        Ok(0)
    );
    assert_eq!(f.guest.get(PARAM + 4, 4), 370u32.to_le_bytes());
    assert_eq!(f.guest.get(output, 4), 408u32.to_le_bytes());

    // GetPanelSize does not read the unrelated remainder of Param96.
    let partial = ARG + PAGE - 36;
    f.put(partial + 4, &0u32.to_le_bytes());
    f.put(partial + 32, &0u32.to_le_bytes());
    assert_eq!(
        f.call(Service::ImeGetPanelSize, &[partial, output, output + 8]),
        Ok(0)
    );
    assert_eq!(f.guest.get(output, 4), 793u32.to_le_bytes());
}

#[test]
fn open_late_input_failure_is_atomic_and_retry_publishes_real_text_event() {
    let mut f = Fixture::new();
    f.units(INPUT, &[b'A' as u16, 0xd83d, 0xde00, 0]);
    let old_input = f.guest.get(INPUT, 100);
    let old_work = f.guest.get(WORK, WORK_BYTES + 2);
    let bytes_before = f.budget.usage().bytes_written;
    let work_before = f.budget.usage().work;
    f.protect_input(1);
    assert_eq!(
        f.call(Service::ImeOpen, &[PARAM, 0]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert!(f.state.session.is_none());
    assert_eq!((f.state.generation, f.state.revision), (0, 0));
    assert_eq!(f.state.metadata_bytes(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
    assert_eq!(f.guest.get(WORK, WORK_BYTES + 2), old_work);
    assert_eq!(f.guest.get(INPUT, 100), old_input);
    // The staged source-order query preflights both future stores before its
    // read; the denied input therefore precedes the final write-byte charge.
    assert_eq!(f.budget.usage().bytes_written, bytes_before);
    assert!(f.budget.usage().work > work_before);

    f.protect_input(3);
    f.open();
    let session = f.state.session.as_ref().unwrap();
    assert_eq!(session.text, [b'A' as u16, 0xd83d, 0xde00]);
    assert_eq!(session.cursor, 3);
    assert_eq!((session.generation, f.state.revision), (1, 1));
    assert_eq!(session.arg, 0x1111_2222_3333_4444);
    assert_eq!(session.handler, 0x5555_6666_7777_8888);
    assert_eq!(session.events.len(), 1);
    let mut event = [0; 72];
    event[8..12].copy_from_slice(&12.5f32.to_le_bytes());
    event[12..16].copy_from_slice(&23.0f32.to_le_bytes());
    event[16..20].copy_from_slice(&370u32.to_le_bytes());
    event[20..24].copy_from_slice(&402u32.to_le_bytes());
    assert_eq!(session.events[0].bytes, event);
    assert!(session.events[0].text.is_none());
    assert!(!session.event_overflow);
    let mut work = vec![0; WORK_BYTES + 2];
    work[..8].copy_from_slice(&old_input[..8]);
    work[WORK_BYTES..].fill(0xa5);
    assert_eq!(f.guest.get(WORK, WORK_BYTES + 2), work);
    assert_eq!(f.guest.get(INPUT, 100), old_input);
    assert_eq!(f.budget.usage().handles_live, 1);
}

#[test]
fn open_validation_and_param_work_alias_keep_captured_state_and_live_rect_distinct() {
    let mut f = Fixture::new();
    // Only userid is readable here; invalid userid wins before later fields.
    let partial = ARG + PAGE - 4;
    f.put(partial, &u32::MAX.to_le_bytes());
    assert_eq!(
        f.call(Service::ImeOpen, &[partial, u64::MAX]),
        Ok(INVALID_USER)
    );
    assert_eq!(f.call(Service::ImeOpen, &[0, 0]), Ok(INVALID_ADDRESS));
    let mut param = parameters(u64::MAX - 3);
    f.put(PARAM, &param);
    assert_eq!(f.call(Service::ImeOpen, &[PARAM, 0]), Ok(INVALID_PARAM));
    param = parameters(WORK);
    f.put(PARAM, &param);
    f.put(ARG + 0x300, &[0; 136]);
    f.put(ARG + 0x300 + 72, &[1]);
    assert_eq!(
        f.call(Service::ImeOpen, &[PARAM, ARG + 0x300]),
        Ok(INVALID_EXTENDED)
    );
    assert_eq!(f.state.retained_handle_count(), 0);

    param[32..36].copy_from_slice(&OVER_2K.to_le_bytes());
    let alias_param = WORK + 128;
    f.put(alias_param, &param);
    assert_eq!(f.call(Service::ImeOpen, &[alias_param, 0]), Ok(0));
    let session = f.state.session.as_ref().unwrap();
    assert_eq!(session.kind, 4);
    assert_eq!(session.option, OVER_2K);
    assert_eq!(session.arg, 0x1111_2222_3333_4444);
    assert_eq!(session.handler, 0x5555_6666_7777_8888);
    // Clear20KiB happened after capture and before source rect/type/option
    // reads, so this event uses all-zero caller fields and ordinary dimensions.
    assert_eq!(f.guest.get(alias_param, 96), [0; 96]);
    assert_eq!(&session.events[0].bytes[8..16], &[0; 8]);
    assert_eq!(
        &session.events[0].bytes[16..24],
        [793u32.to_le_bytes(), 408u32.to_le_bytes()].concat()
    );
    assert_eq!(f.call(Service::ImeOpen, &[0, u64::MAX]), Ok(BUSY));
    assert_eq!(f.call(Service::ImeClose, &[]), Ok(0));
    assert_eq!(f.call(Service::ImeClose, &[]), Ok(NOT_OPENED));
    assert_eq!((f.state.generation, f.state.revision), (1, 2));
    assert_eq!(f.budget.usage().handles_live, 0);
    f.open();
    assert_eq!((f.state.generation, f.state.revision), (2, 3));
}

#[test]
fn text_utf16_cursor_and_ignored_geometry_fields_follow_direct_call_contract() {
    let mut f = Fixture::new();
    f.units(INPUT, &[b'A' as u16, 0xd83d, 0xde00, b'B' as u16, 0]);
    f.open();
    // Only the index member at+12 is touched, despite unmapped x/y/height.
    f.put(ARG, &2u32.to_le_bytes());
    assert_eq!(f.call(Service::ImeSetCaret, &[ARG - 12]), Ok(0));
    assert_eq!(f.state.session.as_ref().unwrap().cursor, 3);
    let source = ARG + 0x300;
    f.units(source, &[b'Z' as u16, 0xd83d, 0xde01, b'Q' as u16]);
    assert_eq!(f.call(Service::ImeSetText, &[source, 4]), Ok(0));
    let session = f.state.session.as_ref().unwrap();
    // Number4 permits this direct SetText; type filtering is a host-input rule.
    assert_eq!(session.text, [b'Z' as u16, 0xd83d, 0xde01, b'Q' as u16]);
    assert_eq!(session.cursor, 3);
    assert_eq!(session.events.len(), 1);
    let revision = f.state.revision;
    f.units(ARG + PAGE - 2, &[10]);
    assert_eq!(
        f.call(Service::ImeSetText, &[ARG + PAGE - 2, 4]),
        Ok(INVALID_TEXT)
    );
    assert_eq!(f.state.revision, revision);
    assert_eq!(f.call(Service::ImeSetText, &[0, 0]), Ok(INVALID_ADDRESS));

    let geometry = ARG + PAGE - 8;
    f.put(geometry, &100.0f32.to_le_bytes());
    f.put(geometry + 4, &200.0f32.to_le_bytes());
    assert_eq!(f.call(Service::ImeSetTextGeometry, &[2, geometry]), Ok(0));
    assert_eq!(
        f.call(Service::ImeSetTextGeometry, &[1, geometry]),
        Ok(INVALID_PARAM)
    );
    assert_eq!(f.state.revision, revision);
    // Nonnull explicit-length zero does not inspect its unmapped text pointer.
    assert_eq!(f.call(Service::ImeSetText, &[u64::MAX, 0]), Ok(0));
    assert!(f.state.session.as_ref().unwrap().text.is_empty());
    assert_eq!(f.state.session.as_ref().unwrap().cursor, 0);
    assert_eq!(f.guest.get(INPUT, 2), [0; 2]);
    assert_eq!(f.guest.get(WORK, 2), [0; 2]);
}

#[test]
fn failed_text_replacement_keeps_owned_capacity_revision_and_buffers_until_retry() {
    let mut f = Fixture::new();
    f.units(INPUT, &[b'O' as u16, b'L' as u16, b'D' as u16, 0]);
    f.open();
    f.units(
        ARG + 0x300,
        &[b'N' as u16, b'E' as u16, b'W' as u16, b'!' as u16, 0],
    );
    let before = f.guest.get(WORK, WORK_BYTES);
    let input_before = f.guest.get(INPUT, 20);
    let metadata = f.state.metadata_bytes();
    let revision = f.state.revision;
    let capacity = f.state.session.as_ref().unwrap().text.capacity();
    f.protect_input(1);
    assert_eq!(
        f.call(Service::ImeSetText, &[ARG + 0x300, 4]),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(f.guest.get(WORK, WORK_BYTES), before);
    assert_eq!(f.guest.get(INPUT, 20), input_before);
    assert_eq!(f.state.metadata_bytes(), metadata);
    assert_eq!(f.state.session.as_ref().unwrap().text.capacity(), capacity);
    assert_eq!(
        f.state.session.as_ref().unwrap().text,
        [b'O' as u16, b'L' as u16, b'D' as u16]
    );
    assert_eq!(f.state.revision, revision);
    f.protect_input(3);
    let scratch_limit = f.budget.limits.max_scratch_bytes;
    f.budget.limits.max_scratch_bytes = metadata;
    assert!(matches!(
        f.call(Service::ImeSetText, &[ARG + 0x300, 4]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(f.guest.get(WORK, WORK_BYTES), before);
    assert_eq!(f.state.revision, revision);
    f.budget.limits.max_scratch_bytes = scratch_limit;
    assert_eq!(f.call(Service::ImeSetText, &[ARG + 0x300, 4]), Ok(0));
    assert_eq!(
        f.state.session.as_ref().unwrap().text,
        [b'N' as u16, b'E' as u16, b'W' as u16, b'!' as u16]
    );
    assert_eq!(f.state.session.as_ref().unwrap().cursor, 3);
    assert_eq!(f.state.revision, revision + 1);
    assert_eq!(f.guest.get(WORK + 10, WORK_BYTES - 10), before[10..]);
    assert_eq!(f.state.session.as_ref().unwrap().events.len(), 1);
    assert_eq!(f.budget.usage().handles_live, 1);
}
