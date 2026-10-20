// SPDX-License-Identifier: GPL-2.0-or-later

//! Original synthetic fixtures; no upstream or game command bytes.

mod generated;
mod grammar;
mod indirect;
mod limits;
mod stops;

use crate::{Inspection, Limits, Outcome, StopReason, inspect};

const PAD: u32 = 0x8000_0000;
const NOP: u8 = 0x10;
const CONTEXT: u8 = 0x69;
const SH: u8 = 0x76;
const UCONFIG: u8 = 0x79;
const INDIRECT: u8 = 0x3f;
const CLEAR_STATE: u8 = 0x12;
const REGISTER_OPCODES: [u8; 3] = [CONTEXT, SH, UCONFIG];

fn type3(opcode: u8, count: u16, flags: u8) -> u32 {
    0xc000_0000 | (u32::from(count) << 16) | (u32::from(opcode) << 8) | u32::from(flags)
}

/// A direct register write whose count equals the number of values.
fn write(opcode: u8, flags: u8, offset_word: u32, values: &[u32]) -> Vec<u32> {
    let count = u16::try_from(values.len()).unwrap();
    [&[type3(opcode, count, flags), offset_word][..], values].concat()
}

fn indirect(low: u32, high: u32, control: u32) -> [u32; 4] {
    [type3(INDIRECT, 2, 0), low, high, control]
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn run_with(words: &[u32], limits: &Limits) -> Inspection {
    inspect(&bytes(words), limits).expect("aligned input within the input limit")
}

fn run(words: &[u32]) -> Inspection {
    run_with(words, &Limits::default())
}

fn stopped(inspection: &Inspection) -> (usize, StopReason) {
    match *inspection.outcome() {
        Outcome::Stopped { at, reason } => (at, reason),
        Outcome::CompleteFlatSubset => panic!("expected a stop, got {inspection:?}"),
    }
}

/// Owner-independent view of the committed prefix: position and original
/// bytes of each packet.
fn committed(inspection: &Inspection) -> Vec<(usize, Vec<u8>)> {
    inspection
        .packets()
        .map(|packet| (packet.at(), packet.bytes().to_vec()))
        .collect()
}

/// Inspects `words`, asserts a stop at dword `at` whose committed prefix is
/// exactly that of `words[..at]` inspected alone, and returns the reason.
fn terminal(words: &[u32], at: usize) -> StopReason {
    let full = run(words);
    let (stop, reason) = stopped(&full);
    assert_eq!(stop, at, "{reason}");
    assert_eq!(full.committed_dwords(), at);
    let prefix = run(&words[..at]);
    assert!(prefix.is_complete(), "{prefix:?}");
    assert_eq!(committed(&full), committed(&prefix));
    assert_eq!(full.committed_packets(), prefix.committed_packets());
    assert_eq!(&full.input()[..at * 4], prefix.input());
    reason
}
