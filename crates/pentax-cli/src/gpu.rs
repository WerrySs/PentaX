// SPDX-License-Identifier: GPL-2.0-or-later

//! Closed original syntax fixtures: no caller images, addresses or file input.

use std::error::Error;

use pentax_gpu::{Limits, Outcome, PacketBody, RECORD_STORAGE_BYTES, RegisterSpace, StopReason};

// Padding, header-only NOP, opaque NOP, then context/SH/uconfig writes.
// These words are original synthetic data, not a captured command stream.
const COMPLETE: [u32; 18] = [
    0x8000_0000,
    0xffff_1000,
    0xc002_1000,
    0x1200_3456,
    0xc002_3f00,
    0x89ab_cdef,
    0xc002_6904,
    0x14,
    0x0011_2233,
    0x4455_6677,
    0xc001_7602,
    0x27,
    0x89ab_cdef,
    0xc003_7906,
    0x31,
    0x0bad_cafe,
    0x1234_5678,
    0xfedc_ba98,
];

// The reference's address is an opaque integer. The trailing unsupported
// type-0 word must not replace the terminal reference observation.
const TERMINAL: [u32; 6] = [0x8000_0000, 0xc002_3f07, 0x40, 1, 0x8010_0003, 0];

#[derive(Debug, PartialEq, Eq)]
struct Summary {
    packets: usize,
    dwords: usize,
    register_writes: usize,
    values: usize,
    terminal_at: usize,
    terminal_packets: usize,
    terminal_size: u32,
}

pub(super) fn run() -> Result<(), Box<dyn Error>> {
    let summary = validate(&COMPLETE, &TERMINAL)?;
    println!(
        "Original GPU syntax passed: packets={}, dwords={}, register writes={}, values={}",
        summary.packets, summary.dwords, summary.register_writes, summary.values
    );
    println!(
        "Terminal indirect reference: at={}, committed packets={}, size={}; syntax only, no rendering or execution.",
        summary.terminal_at, summary.terminal_packets, summary.terminal_size
    );
    Ok(())
}

fn require(condition: bool, failure: &'static str) -> Result<(), &'static str> {
    if condition { Ok(()) } else { Err(failure) }
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

fn validate(complete: &[u32; 18], terminal: &[u32; 6]) -> Result<Summary, &'static str> {
    let mut input = bytes(complete);
    let limits = Limits {
        max_input_bytes: 18 * 4,
        max_records: 6,
        max_record_bytes: 6 * RECORD_STORAGE_BYTES,
        max_visited_dwords: 18,
    };
    let report =
        pentax_gpu::inspect(&input, &limits).map_err(|_| "GPU syntax fixture inspection failed")?;
    input.fill(0);
    require(
        report.is_complete() && report.committed_packets() == 6 && report.committed_dwords() == 18,
        "GPU syntax fixture did not complete the expected packet sequence",
    )?;
    require(
        report.visited_dwords() == 18 && report.input() != input,
        "GPU syntax fixture ownership or work accounting failed",
    )?;

    let expected_ranges = [0..1, 1..2, 2..6, 6..10, 10..13, 13..18];
    for (packet, expected) in report.packets().zip(expected_ranges) {
        require(
            packet.dword_range() == expected
                && report.input().get(expected.start * 4..expected.end * 4) == Some(packet.bytes()),
            "GPU syntax fixture packet views do not match their owner",
        )?;
    }
    require(
        matches!(
            report.packet(0).map(|packet| packet.body()),
            Some(PacketBody::Padding)
        ) && matches!(
            report.packet(1).map(|packet| packet.body()),
            Some(PacketBody::HeaderOnlyNop)
        ),
        "GPU syntax fixture padding or special NOP framing failed",
    )?;
    let Some(PacketBody::Nop(payload)) = report.packet(2).map(|packet| packet.body()) else {
        return Err("GPU syntax fixture did not retain an opaque NOP payload");
    };
    require(
        payload.dword_range() == (3..6) && payload.bytes() == bytes(&COMPLETE[3..6]),
        "GPU syntax fixture NOP payload was changed or interpreted",
    )?;

    let mut values = 0;
    let mut register_writes = 0;
    for (packet_index, space, offset, expected, range, flags) in [
        (3, RegisterSpace::Context, 0x14, &COMPLETE[8..10], 8..10, 4),
        (4, RegisterSpace::Sh, 0x27, &COMPLETE[12..13], 12..13, 2),
        (
            5,
            RegisterSpace::Uconfig,
            0x31,
            &COMPLETE[15..18],
            15..18,
            6,
        ),
    ] {
        let packet = report
            .packet(packet_index)
            .ok_or("GPU syntax fixture write is missing")?;
        let PacketBody::RegisterWrite(write) = packet.body() else {
            return Err("GPU syntax fixture write grammar was not retained");
        };
        require(
            write.space() == space
                && write.offset() == offset
                && write.value_range() == range
                && write.values().eq(expected.iter().copied())
                && write.value(expected.len()).is_none()
                && packet.header().low_byte() == flags
                && packet.header().shader_type() == (flags & 2 != 0)
                && packet.header().filter_cam_reset() == (flags & 4 != 0)
                && !packet.header().predicate(),
            "GPU syntax fixture register values, ranges or flags differ",
        )?;
        register_writes += 1;
        values += write.len();
    }

    let mut terminal_input = bytes(terminal);
    let terminal_limits = Limits {
        max_input_bytes: 6 * 4,
        max_records: 1,
        max_record_bytes: RECORD_STORAGE_BYTES,
        max_visited_dwords: 5,
    };
    let stopped = pentax_gpu::inspect(&terminal_input, &terminal_limits)
        .map_err(|_| "GPU terminal fixture inspection failed")?;
    terminal_input.fill(0);
    let Outcome::Stopped {
        at,
        reason: StopReason::IndirectBufferReference(reference),
    } = *stopped.outcome()
    else {
        return Err("GPU terminal fixture did not stop at the indirect reference");
    };
    require(
        at == 1
            && stopped.committed_packets() == 1
            && stopped.committed_dwords() == 1
            && stopped.visited_dwords() == 5
            && matches!(
                stopped.packet(0).map(|packet| packet.body()),
                Some(PacketBody::Padding)
            )
            && reference.header().raw() == TERMINAL[1]
            && reference.address() == 0x1_0000_0040
            && reference.control_raw() == TERMINAL[4]
            && reference.size_dwords() == 3
            && reference.chain()
            && reference.upper_control_bits() == 0x8000_0000,
        "GPU terminal fixture position, ownership or reference fields differ",
    )?;
    Ok(Summary {
        packets: report.committed_packets(),
        dwords: report.committed_dwords(),
        register_writes,
        values,
        terminal_at: at,
        terminal_packets: stopped.committed_packets(),
        terminal_size: reference.size_dwords(),
    })
}

#[cfg(test)]
mod tests {
    use super::{COMPLETE, TERMINAL, validate};

    #[test]
    fn closed_fixture_rejects_changed_values_and_lost_terminal_boundary() {
        assert!(validate(&COMPLETE, &TERMINAL).is_ok());
        let mut changed_value = COMPLETE;
        changed_value[9] ^= 1;
        assert!(validate(&changed_value, &TERMINAL).is_err());

        let mut lost_boundary = TERMINAL;
        // An equal-length NOP is not the required terminal IB observation,
        // even if inspection stops for another reason afterwards.
        lost_boundary[1] = 0xc002_1000;
        assert!(validate(&COMPLETE, &lost_boundary).is_err());
    }
}
