// SPDX-License-Identifier: GPL-2.0-or-later

use super::{
    CLEAR_STATE, CONTEXT, INDIRECT, NOP, PAD, REGISTER_OPCODES, SH, UCONFIG, indirect, run,
    stopped, terminal, type3, write,
};
use crate::{Header, StopCategory, StopReason};

/// Four committed dwords placed before each failing packet.
fn prefix() -> Vec<u32> {
    [vec![PAD], write(CONTEXT, 0, 1, &[2])].concat()
}

fn after_prefix(packet: &[u32]) -> Vec<u32> {
    [prefix(), packet.to_vec()].concat()
}

#[test]
fn types_zero_and_one_are_unsupported_not_invalid() {
    for raw in [0x0000_0000, 0x0123_4567, 0x4000_0000, 0x7fff_ffff] {
        let reason = terminal(&after_prefix(&[raw, PAD]), 4);
        assert_eq!(
            reason,
            StopReason::UnsupportedPacketType {
                header: Header::new(raw)
            }
        );
        assert_eq!(reason.category(), StopCategory::Unsupported);
    }
}

#[test]
fn inexact_type2_words_are_subset_stops() {
    for raw in [0x8000_0001, 0x8000_1000, 0x8010_0000, 0xbfff_ffff] {
        let reason = terminal(&after_prefix(&[raw]), 4);
        assert_eq!(
            reason,
            StopReason::UnsupportedPadding {
                header: Header::new(raw)
            }
        );
        assert_eq!(reason.category(), StopCategory::Unsupported);
    }
}

#[test]
fn every_nonzero_nop_low_byte_is_an_unsupported_profile() {
    for flags in 1..=u8::MAX {
        for count in [0, 2, 0x3fff] {
            let header = type3(NOP, count, flags);
            let words = after_prefix(&[header, 0, 0, 0]);
            assert_eq!(
                terminal(&words, 4),
                StopReason::UnsupportedNopProfile {
                    header: Header::new(header)
                }
            );
            // Rejected at the header, before any framing or body charge.
            assert_eq!(run(&words).visited_dwords(), 5);
        }
    }
}

#[test]
fn every_opcode_outside_the_subset_is_unsupported() {
    let admitted = [NOP, CONTEXT, SH, UCONFIG, INDIRECT];
    for opcode in (0..=u8::MAX).filter(|opcode| !admitted.contains(opcode)) {
        for count in [0, 0x3fff] {
            let header = type3(opcode, count, 0);
            let words = after_prefix(&[header, 0]);
            let reason = terminal(&words, 4);
            assert_eq!(
                reason,
                StopReason::UnsupportedOpcode {
                    header: Header::new(header)
                }
            );
            assert_eq!(reason.category(), StopCategory::Unsupported);
            assert_eq!(run(&words).visited_dwords(), 5);
        }
    }
}

#[test]
fn clear_state_commands_have_no_reset_effect() {
    // Every four-bit command value, including push, pop and push-clear forms.
    for command in 0..16 {
        let header = type3(CLEAR_STATE, 0, 0);
        let words = after_prefix(&[header, command]);
        assert_eq!(
            terminal(&words, 4),
            StopReason::UnsupportedOpcode {
                header: Header::new(header)
            }
        );
        let inspection = run(&words);
        let committed = inspection.packet(1).unwrap();
        assert_eq!(committed.dword_range(), 1..4);
    }
}

#[test]
fn predicated_register_writes_stop_before_any_intent() {
    for opcode in REGISTER_OPCODES {
        for flags in [0b001, 0b011, 0b101, 0b111] {
            let packet = write(opcode, flags, 0x10, &[1, 2]);
            let reason = terminal(&after_prefix(&packet), 4);
            assert_eq!(
                reason,
                StopReason::PredicatedWrite {
                    header: Header::new(packet[0])
                }
            );
            assert_eq!(reason.category(), StopCategory::Unsupported);
        }
    }
}

#[test]
fn reserved_header_bits_stop_as_unsupported_encoding() {
    for opcode in REGISTER_OPCODES {
        for bit in 3..8 {
            let packet = write(opcode, 1 << bit, 0x10, &[1]);
            assert_eq!(
                terminal(&after_prefix(&packet), 4),
                StopReason::UnsupportedHeaderEncoding {
                    header: Header::new(packet[0])
                }
            );
        }
    }
}

#[test]
fn high_selector_bits_stop_without_normalization() {
    for opcode in REGISTER_OPCODES {
        for offset_word in [0x0001_0000, 0x2000_0004, 0x8000_0010, 0xffff_ffff] {
            let packet = write(opcode, 0, offset_word, &[1]);
            let words = after_prefix(&packet);
            assert_eq!(
                terminal(&words, 4),
                StopReason::UnsupportedRegisterSelector {
                    header: Header::new(packet[0]),
                    offset_word
                }
            );
            // The whole framed packet was charged before the body was read.
            assert_eq!(run(&words).visited_dwords(), 4 + 3);
        }
    }
}

#[test]
fn register_runs_crossing_the_index_domain_stop_before_commit() {
    // The last case uses the largest count: 0xc002 + 0x3fff = 0x10001.
    let cases = [
        (CONTEXT, 0xffff_u16, 2_u16),
        (SH, 0xfff0, 0x11),
        (UCONFIG, 0xc002, 0x3fff),
    ];
    for (opcode, offset, values) in cases {
        let packet = write(opcode, 0, u32::from(offset), &vec![0; usize::from(values)]);
        let reason = terminal(&after_prefix(&packet), 4);
        assert_eq!(
            reason,
            StopReason::RegisterRangeOverflow {
                header: Header::new(packet[0]),
                offset,
                values,
            }
        );
        assert_eq!(reason.category(), StopCategory::Malformed);
    }
}

#[test]
fn count_zero_register_packets_stop_as_empty_writes() {
    for opcode in REGISTER_OPCODES {
        let header = type3(opcode, 0, 0);
        let words = after_prefix(&[header, 0x10]);
        let reason = terminal(&words, 4);
        assert_eq!(
            reason,
            StopReason::EmptyRegisterWrite {
                header: Header::new(header)
            }
        );
        assert_eq!(reason.category(), StopCategory::Malformed);
        assert_eq!(run(&words).visited_dwords(), 5);
    }
}

#[test]
fn every_truncation_point_keeps_the_committed_prefix() {
    let mut packets = vec![
        vec![type3(NOP, 0, 0), 0x0102_0304],
        vec![type3(NOP, 3, 0), 1, 2, 3, 4],
        indirect(0x1000, 0, 4).to_vec(),
    ];
    for opcode in REGISTER_OPCODES {
        packets.push(write(opcode, 0, 0x10, &[5]));
        packets.push(write(opcode, 0b110, 0x20, &[6, 7, 8]));
    }
    for packet in packets {
        for cut in 1..packet.len() {
            let words = after_prefix(&packet[..cut]);
            let reason = terminal(&words, 4);
            assert_eq!(
                reason,
                StopReason::Truncated {
                    header: Header::new(packet[0]),
                    packet_dwords: u16::try_from(packet.len()).unwrap(),
                    available_dwords: cut,
                }
            );
            assert_eq!(reason.category(), StopCategory::Truncated);
            assert_eq!(run(&words).visited_dwords(), 5);
        }
    }
}

#[test]
fn maximum_count_non_nop_packets_frame_ordinarily_and_truncate() {
    for opcode in REGISTER_OPCODES {
        let header = type3(opcode, 0x3fff, 0);
        let words = after_prefix(&[header, 0x10, 1, 2]);
        assert_eq!(
            terminal(&words, 4),
            StopReason::Truncated {
                header: Header::new(header),
                packet_dwords: 0x4001,
                available_dwords: 4,
            }
        );
    }
    let nop = type3(NOP, 0x3ffe, 0);
    assert_eq!(
        terminal(&after_prefix(&[nop, 0]), 4),
        StopReason::Truncated {
            header: Header::new(nop),
            packet_dwords: 0x4000,
            available_dwords: 2,
        }
    );
}

#[test]
fn stop_precedence_is_header_then_framing_then_body() {
    let cases = [
        // Reserved bits before predicate before empty count.
        (
            vec![type3(CONTEXT, 0, 0b1001), 0],
            StopReason::UnsupportedHeaderEncoding {
                header: Header::new(type3(CONTEXT, 0, 0b1001)),
            },
        ),
        (
            vec![type3(SH, 0, 0b001)],
            StopReason::PredicatedWrite {
                header: Header::new(type3(SH, 0, 0b001)),
            },
        ),
        // Header rules need no body, so truncation is not reported.
        (
            vec![type3(UCONFIG, 0, 0)],
            StopReason::EmptyRegisterWrite {
                header: Header::new(type3(UCONFIG, 0, 0)),
            },
        ),
        (
            vec![type3(0x2d, 0x3fff, 0)],
            StopReason::UnsupportedOpcode {
                header: Header::new(type3(0x2d, 0x3fff, 0)),
            },
        ),
        // Framing before body: the bad selector is never read.
        (
            vec![type3(CONTEXT, 2, 0), 0x0001_0000],
            StopReason::Truncated {
                header: Header::new(type3(CONTEXT, 2, 0)),
                packet_dwords: 4,
                available_dwords: 2,
            },
        ),
        // Selector before range.
        (
            write(SH, 0, 0x0001_ffff, &[1, 2]),
            StopReason::UnsupportedRegisterSelector {
                header: Header::new(type3(SH, 2, 0)),
                offset_word: 0x0001_ffff,
            },
        ),
    ];
    for (packet, expected) in cases {
        assert_eq!(terminal(&after_prefix(&packet), 4), expected);
        assert_eq!(expected.header(), Some(Header::new(packet[0])));
    }
}

#[test]
fn stops_at_the_first_packet_leave_nothing_committed() {
    let inspection = run(&[type3(CONTEXT, 1, 1), 0, 0, PAD]);
    assert_eq!(stopped(&inspection).0, 0);
    assert_eq!(inspection.committed_packets(), 0);
    assert_eq!(inspection.committed_dwords(), 0);
    assert_eq!(inspection.accounting().record_capacity(), 4);
}
