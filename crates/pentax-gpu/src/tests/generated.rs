// SPDX-License-Identifier: GPL-2.0-or-later

//! Deterministic generated inputs. Every generated stream must be inspected
//! without panicking, within its configured bounds, with a consistent
//! contiguous prefix and an identical result on repetition.

use super::{CLEAR_STATE, CONTEXT, INDIRECT, NOP, PAD, SH, UCONFIG, bytes, type3};
use crate::{InspectError, LimitKind, Limits, Outcome, PacketBody, RECORD_STORAGE_BYTES, inspect};

/// Xorshift64; quality is irrelevant, determinism is not.
struct Generator(u64);

impl Generator {
    fn advance(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.advance() % bound
    }

    fn word(&mut self) -> u32 {
        (self.advance() >> 32) as u32
    }

    fn pick<T: Copy>(&mut self, choices: &[T]) -> T {
        choices[self.below(choices.len() as u64) as usize]
    }

    /// Mostly plausible headers and offsets, with arbitrary words mixed in.
    fn command_word(&mut self) -> u32 {
        match self.below(8) {
            0 => PAD,
            1 => self.word(),
            2 => self.below(0x1_0040) as u32,
            3 => 0x8000_0000 | self.below(4) as u32,
            _ => {
                let random_opcode = self.word() as u8;
                let opcode = self.pick(&[
                    NOP,
                    CONTEXT,
                    SH,
                    UCONFIG,
                    INDIRECT,
                    CLEAR_STATE,
                    random_opcode,
                ]);
                let random_count = self.below(0x4000) as u16;
                let count = self.pick(&[0, 1, 2, 3, 0x3ffe, 0x3fff, random_count]);
                let flags = if self.below(3) == 0 {
                    self.word() as u8
                } else {
                    0
                };
                type3(opcode, count, flags)
            }
        }
    }

    fn limit(&mut self, default: u64, scale: u64) -> u64 {
        match self.below(4) {
            0 => default,
            1 => 0,
            2 => self.below(scale + 1),
            _ => u64::MAX,
        }
    }

    fn limits(&mut self) -> Limits {
        let default = Limits::DEFAULT;
        Limits {
            max_input_bytes: self.limit(default.max_input_bytes, 256),
            max_records: self.limit(default.max_records, 48),
            max_record_bytes: self.limit(default.max_record_bytes, 48 * RECORD_STORAGE_BYTES),
            max_visited_dwords: self.limit(default.max_visited_dwords, 72),
        }
    }
}

fn check(input: &[u8], limits: &Limits) {
    let length = input.len() as u64;
    let inspection = match inspect(input, limits) {
        Ok(inspection) => inspection,
        Err(error) => {
            let expected = if !length.is_multiple_of(4) {
                InspectError::UnalignedLength { bytes: length }
            } else {
                InspectError::LimitExceeded {
                    limit: LimitKind::InputBytes,
                    requested: length,
                    maximum: limits.max_input_bytes,
                }
            };
            assert_eq!(error, expected, "{error}");
            assert!(!length.is_multiple_of(4) || length > limits.max_input_bytes);
            return;
        }
    };
    assert!(length.is_multiple_of(4) && length <= limits.max_input_bytes);
    assert_eq!(inspection.input(), input);

    let accounting = inspection.accounting();
    let capacity = accounting.record_capacity() as u64;
    assert!(accounting.input_capacity_bytes() <= limits.max_input_bytes);
    assert!(accounting.record_capacity_bytes() <= limits.max_record_bytes);
    assert!(capacity <= limits.max_records);
    assert!(capacity <= inspection.input_dwords() as u64);
    assert!(inspection.committed_packets() <= accounting.record_capacity());
    assert!(inspection.visited_dwords() <= limits.max_visited_dwords);
    assert!(inspection.visited_dwords() >= inspection.committed_dwords() as u64);

    let mut next = 0;
    for packet in inspection.packets() {
        let range = packet.dword_range();
        assert_eq!((packet.at(), range.start), (next, next));
        assert_eq!(packet.bytes(), &input[range.start * 4..range.end * 4]);
        let header = packet.header();
        match packet.body() {
            PacketBody::Padding => assert_eq!(header.raw(), PAD),
            PacketBody::HeaderOnlyNop => assert_eq!(header.raw(), type3(NOP, 0x3fff, 0)),
            PacketBody::Nop(payload) => {
                assert_eq!((header.opcode(), header.low_byte()), (NOP, 0));
                assert_eq!(payload.dword_range(), range.start + 1..range.end);
            }
            PacketBody::RegisterWrite(write) => {
                assert_eq!(write.space().opcode(), header.opcode());
                assert!(!write.is_empty() && write.relative_indices().end <= 0x1_0000);
                assert!(!header.predicate() && header.reserved_bits() == 0);
                assert_eq!(write.value_range(), range.start + 2..range.end);
                assert_eq!(write.values().len(), write.len());
            }
        }
        next = range.end;
    }
    assert_eq!(next, inspection.committed_dwords());

    match *inspection.outcome() {
        Outcome::CompleteFlatSubset => assert_eq!(next, inspection.input_dwords()),
        Outcome::Stopped { at, reason } => {
            assert_eq!(at, next);
            assert!(at < inspection.input_dwords());
            assert!(!reason.to_string().is_empty());
            if let Some(header) = reason.header() {
                let word = &input[at * 4..at * 4 + 4];
                assert_eq!(header.raw().to_le_bytes(), word);
            }
        }
    }
    assert_eq!(inspect(input, limits).ok().as_ref(), Some(&inspection));
}

#[test]
fn generated_command_streams_never_panic_and_stay_bounded() {
    let mut generator = Generator(0x9e37_79b9_7f4a_7c15);
    for _ in 0..4000 {
        let length = generator.below(48) as usize;
        let words: Vec<u32> = (0..length).map(|_| generator.command_word()).collect();
        let limits = generator.limits();
        check(&bytes(&words), &limits);
    }
}

#[test]
fn generated_arbitrary_bytes_are_classified_before_ownership() {
    let mut generator = Generator(0x0123_4567_89ab_cdef);
    for _ in 0..2000 {
        let length = generator.below(80) as usize;
        let input: Vec<u8> = (0..length).map(|_| generator.word() as u8).collect();
        check(&input, &Limits::default());
        let limits = generator.limits();
        check(&input, &limits);
    }
}
