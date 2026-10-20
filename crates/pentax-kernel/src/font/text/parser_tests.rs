// SPDX-License-Identifier: GPL-2.0-only
//! Modeled callback results over original guest data. No callback is executed.

use super::*;
use crate::{
    Kernel, Limits,
    tests::{ARG, TestGuest, kernel},
};

const SOURCE: u64 = ARG + 0x100;
const DETAIL: u64 = ARG + 0x200;
const OUTPUT: u64 = ARG + 0x280;
const STACK: u64 = ARG + 3000;
const DEFAULT_FONT: u64 = 0x1111_0000;

fn words(values: &[u64]) -> CallArgs {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    CallArgs(args)
}
fn prepare() -> (Kernel, TestGuest) {
    let owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    let mut source = [0; 96];
    for (offset, value) in [
        (24, 0x7777_u64),
        (32, 0x2222),
        (48, DEFAULT_FONT),
        (56, 0x10),
    ] {
        source[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    guest.put(SOURCE - ARG, &source);
    guest.put(DETAIL - ARG, &[0; 32]);
    guest.put(OUTPUT - ARG, &[0xa5; 16]);
    guest.put(STACK - 64 - ARG, &[0xa5; 40]);
    (owner, guest)
}
fn begin(owner: &mut Kernel, guest: &mut TestGuest) -> AdmittedFontParser {
    let action = owner
        .admit_font_string(
            words(&[u64::MAX, SOURCE, DETAIL, OUTPUT]),
            Some(STACK),
            guest,
        )
        .unwrap();
    let FontParserAction::Call(request) = action else {
        panic!("expected modeled parser request");
    };
    assert_eq!(request.args(), [SOURCE, STACK - 64, STACK - 56]);
    assert_eq!(request.stack_pointer(), STACK);
    assert_eq!(guest.get(STACK - 64, 40), [0; 40]);
    request
}
fn result(guest: &mut TestGuest, order: u64, font_or_reserved: u64, code: u32) {
    let mut bytes = [0x7e; 40];
    bytes[..8].copy_from_slice(&order.to_le_bytes());
    bytes[8..16].copy_from_slice(&font_or_reserved.to_le_bytes());
    bytes[16..20].copy_from_slice(&code.to_le_bytes());
    guest.put(STACK - 64 - ARG, &bytes);
}

#[test]
fn modeled_parser_results_are_unique_consumed_dynamic_and_capture_defaults_once() {
    let (mut owner, mut guest) = prepare();
    guest.put(DETAIL + 8 - ARG, &0x2222_0000_u64.to_le_bytes());
    let first = begin(&mut owner, &mut guest);
    assert_eq!(owner.usage().attempts, 1);
    assert_eq!(owner.usage().handles_live, 1);
    let (mut other, mut other_guest) = prepare();
    let foreign = begin(&mut other, &mut other_guest);
    assert_ne!(first, foreign);
    assert!(
        other
            .complete_font_parser(&first, 0, &mut other_guest)
            .is_err()
    );
    assert_eq!(other.usage().handles_live, 1);
    // Model actual callback-side writes. The next entry is reread, whereas
    // the original default font and writing form remain captured by value.
    guest.put(SOURCE + 32 - ARG, &0x3333_u64.to_le_bytes());
    guest.put(SOURCE + 48 - ARG, &0x4444_u64.to_le_bytes());
    guest.put(SOURCE + 56 - ARG, &(-3_i64).to_le_bytes());
    guest.put(DETAIL + 8 - ARG, &0x5555_u64.to_le_bytes());
    result(&mut guest, 0xabcd, 0, 65);
    let action = owner
        .complete_font_parser(&first, 0xdead_beef_0000_0001, &mut guest)
        .unwrap();
    let FontParserAction::Call(second) = action else {
        panic!("expected second modeled request");
    };
    assert_eq!(second.entry(), 0x3333);
    assert_ne!(first, second);
    assert_eq!(guest.get(STACK - 64, 40), [0; 40]);
    assert!(owner.complete_font_parser(&first, 1, &mut guest).is_err());
    result(&mut guest, 0, 0xfeed_beef, 0x1234_5678);
    assert_eq!(
        owner.complete_font_parser(&second, 0xdead_beef_0000_0000, &mut guest),
        Ok(FontParserAction::Returned(0))
    );
    assert_eq!(owner.usage().attempts, 1);
    let strings = &owner.fonts.text.strings;
    assert_eq!(strings.len(), 1);
    assert_eq!(strings[0].form, 0x10);
    assert_eq!(strings[0].terminate_code, 0x1234_5678);
    assert_eq!(strings[0].terminate_order, 0xfeed_beef);
    assert_eq!(strings[0].chars.len(), 1);
    assert!(strings[0].chars.capacity() < MAX_CHARACTERS);
    assert_eq!(
        (
            strings[0].chars[0].font,
            strings[0].chars[0].code,
            strings[0].chars[0].order
        ),
        (0x2222_0000, 65, 0xabcd)
    );
    assert!(owner.fonts.text.pending.is_empty());
    assert_eq!(owner.usage().handles_live, 2);
    assert_eq!(guest.u64(SOURCE + 24 - ARG), 0x7777);
    assert_eq!(guest.get(OUTPUT + 8, 8), [0xa5; 8]);
    assert!(owner.complete_font_parser(&second, 0, &mut guest).is_err());
    result(&mut other_guest, 0, 0, 0);
    assert_eq!(
        other.complete_font_parser(&foreign, 2, &mut other_guest),
        Ok(FontParserAction::Returned(FAILURE))
    );
    assert_eq!(other.usage().handles_live, 0);
}

#[test]
fn modeled_terminate_fallback_reads_live_source_and_error_status_uses_only_low32() {
    let (mut owner, mut guest) = prepare();
    let request = begin(&mut owner, &mut guest);
    // The callback changes all raw text cursors; fallback starts at live.start,
    // not at a stale snapshot or the callback's unrelated current word.
    guest.put(0x600, &[b'Z', 0, 0xc2, b'!']);
    guest.put(SOURCE + 8 - ARG, &(ARG + 0x600).to_le_bytes());
    guest.put(SOURCE + 16 - ARG, &(ARG + 0x604).to_le_bytes());
    guest.put(SOURCE + 24 - ARG, &0x8888_u64.to_le_bytes());
    guest.put(SOURCE + 48 - ARG, &0x9999_u64.to_le_bytes());
    result(&mut guest, 0, 0x9876, 7);
    assert_eq!(
        owner.complete_font_parser(&request, 0, &mut guest),
        Ok(FontParserAction::Returned(0))
    );
    let string = &owner.fonts.text.strings[0];
    assert_eq!(
        string.chars.iter().map(|c| c.code).collect::<Vec<_>>(),
        [90, 0, 161]
    );
    assert!(string.chars.iter().all(|c| c.font == DEFAULT_FONT));
    assert_eq!((string.terminate_code, string.terminate_order), (7, 0x9876));
    assert_eq!(guest.u64(SOURCE + 24 - ARG), ARG + 0x604);
    assert_eq!(owner.usage().attempts, 1);
    for (status, code, expected) in [
        (0xaaaa_aaaa_ffff_ffff, 0x8123_4567, 0x8123_4567),
        (u64::MAX, 0, FAILURE),
        (0xaaaa_aaaa_0000_0002, 0x1234, FAILURE),
    ] {
        let (mut owner, mut guest) = prepare();
        let request = begin(&mut owner, &mut guest);
        result(&mut guest, 0, 0, code);
        assert_eq!(
            owner.complete_font_parser(&request, status, &mut guest),
            Ok(FontParserAction::Returned(expected))
        );
        assert_eq!(guest.u64(OUTPUT - ARG), 0);
        assert_eq!(guest.get(OUTPUT + 8, 8), [0xa5; 8]);
        assert!(owner.fonts.text.pending.is_empty());
        assert!(owner.fonts.text.strings.is_empty());
        assert_eq!(owner.usage().handles_live, 0);
        assert_eq!(owner.usage().attempts, 1);
    }
}

#[test]
fn modeled_callback_side_effects_survive_late_publication_failure_and_ticket_stays_consumed() {
    let (mut owner, mut guest) = prepare();
    let request = begin(&mut owner, &mut guest);
    guest.put(0x700, b"callback-side-effect");
    guest.put(SOURCE + 24 - ARG, &0xcafe_babe_u64.to_le_bytes());
    result(&mut guest, 0x5555, 0, 0x45);
    guest.refuse = true;
    assert!(owner.complete_font_parser(&request, 0, &mut guest).is_err());
    assert_eq!(guest.get(ARG + 0x700, 20), b"callback-side-effect");
    assert_eq!(guest.u64(SOURCE + 24 - ARG), 0xcafe_babe);
    assert_eq!(guest.get(OUTPUT, 16), [0xa5; 16]);
    assert_eq!(owner.fonts.text.pending.len(), 1);
    assert!(owner.fonts.text.pending[0].request.is_none());
    assert!(owner.fonts.text.strings.is_empty());
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(owner.memory.retained_handle_count(), 0);
    guest.refuse = false;
    assert!(owner.complete_font_parser(&request, 0, &mut guest).is_err());
    assert_eq!(owner.usage().handles_live, 1);
    assert_eq!(owner.usage().attempts, 1);
}

#[test]
fn modeled_final_callback_at_source_cap_publishes_actual_65536_characters_without_callback_loop() {
    let (mut owner, mut guest) = prepare();
    let request = begin(&mut owner, &mut guest);
    {
        let pending = &mut owner.fonts.text.pending[0];
        assert!(pending.chars.capacity() >= MAX_CHARACTERS);
        pending.iterations = MAX_CHARACTERS - 1;
        // Initialize original owned data directly in the already admitted
        // buffer. There is only ONE modeled completion, no65536-call loop.
        pending
            .chars
            .extend((0..MAX_CHARACTERS - 1).map(|index| Character {
                font: DEFAULT_FONT,
                code: 65,
                order: index as u64 + 0x1000,
                index: index as u32,
            }));
    }
    result(&mut guest, 0xbeef, 0, 90);
    assert_eq!(
        owner.complete_font_parser(&request, 1, &mut guest),
        Ok(FontParserAction::Returned(0))
    );
    assert_eq!(owner.usage().attempts, 1);
    assert!(owner.fonts.text.pending.is_empty());
    let string = &owner.fonts.text.strings[0];
    assert_eq!(string.chars.len(), MAX_CHARACTERS);
    assert_eq!(
        string
            .chars
            .last()
            .map(|c| (c.font, c.code, c.order, c.index)),
        Some((DEFAULT_FONT, 90, 0xbeef, 65_535))
    );
    assert_eq!((string.terminate_code, string.terminate_order), (0, 0));
    let last = string.character_address(MAX_CHARACTERS - 1);
    assert_eq!(guest.get(last + 24, 4), 90_u32.to_le_bytes());
    assert_eq!(guest.u64(OUTPUT - ARG), string.runtime.address);
    assert_eq!(guest.get(OUTPUT + 8, 8), [0xa5; 8]);
    assert_eq!(guest.u64(SOURCE + 24 - ARG), 0x7777);
    assert_eq!(owner.usage().handles_live, 2);
    assert_eq!(
        owner.invoke_library(Service::FontDestroyString, words(&[OUTPUT]), &mut guest),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.memory.retained_handle_count(), 0);
}
