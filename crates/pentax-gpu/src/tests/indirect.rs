// SPDX-License-Identifier: GPL-2.0-or-later

use super::{CONTEXT, INDIRECT, PAD, indirect, run, terminal, type3, write};
use crate::{Header, IndirectBuffer, StopCategory, StopReason};

fn reference(reason: StopReason) -> IndirectBuffer {
    match reason {
        StopReason::IndirectBufferReference(reference) => reference,
        other => panic!("expected an indirect-buffer reference, got {other}"),
    }
}

#[test]
fn references_are_terminal_observations_and_never_followed() {
    // Address zero names the first byte of this very input, and the packets
    // after the reference are admissible; neither may be inspected.
    let prefix = [vec![PAD], write(CONTEXT, 0, 4, &[9])].concat();
    let tail = [write(CONTEXT, 0, 8, &[1]), vec![PAD]].concat();
    let words = [prefix.clone(), indirect(0, 0, 2).to_vec(), tail].concat();

    let reason = terminal(&words, prefix.len());
    assert_eq!(reason.category(), StopCategory::IndirectReference);
    let observed = reference(reason);
    assert_eq!(observed.header(), Header::new(type3(INDIRECT, 2, 0)));
    assert_eq!((observed.address(), observed.size_dwords()), (0, 2));

    let inspection = run(&words);
    assert_eq!(inspection.committed_packets(), 2);
    assert_eq!(inspection.committed_dwords(), 4);
    // Prefix plus the four reference dwords; nothing after or elsewhere.
    assert_eq!(inspection.visited_dwords(), 8);
}

#[test]
fn sizes_chain_and_high_addresses_are_kept_raw() {
    let addresses = [
        (0x0000_0000, 0x0000_0100),
        (0x0000_0001, 0x0000_0000),
        (0xffff_ffff, 0xffff_fffc),
    ];
    for size in [0, 1, 0xf_ffff] {
        for chain in [false, true] {
            for (high, low) in addresses {
                let control = size | (u32::from(chain) << 20);
                let observed = reference(terminal(&indirect(low, high, control), 0));
                assert_eq!(observed.size_dwords(), size);
                assert_eq!(observed.chain(), chain);
                assert_eq!(observed.address(), (u64::from(high) << 32) | u64::from(low));
                assert_eq!(
                    (
                        observed.address_low_raw(),
                        observed.address_high_raw(),
                        observed.control_raw()
                    ),
                    (low, high, control)
                );
                assert_eq!(
                    (observed.alignment_bits(), observed.upper_control_bits()),
                    (0, 0)
                );
            }
        }
    }
}

#[test]
fn upper_control_bits_and_header_flags_stay_opaque() {
    for bit in 21..32 {
        let control = (1 << bit) | 5;
        let words = [
            write(CONTEXT, 0, 1, &[2]),
            indirect(0x40, 0, control).to_vec(),
        ]
        .concat();
        let observed = reference(terminal(&words, 3));
        assert_eq!(observed.upper_control_bits(), 1 << bit);
        assert_eq!((observed.size_dwords(), observed.chain()), (5, false));
    }
    // Header flags do not change the terminal decision; they are retained.
    for flags in [0b001, 0b110, 0xf8] {
        let header = type3(INDIRECT, 2, flags);
        let observed = reference(terminal(&[header, 0x80, 0, 1], 0));
        assert_eq!(observed.header(), Header::new(header));
    }
}

#[test]
fn misaligned_addresses_stop_as_malformed_without_following() {
    for low in [1, 2, 3, 0x1000_0003] {
        let words = [vec![PAD], indirect(low, 0, 1).to_vec(), vec![PAD]].concat();
        let reason = terminal(&words, 1);
        assert_eq!(reason.category(), StopCategory::Malformed);
        let StopReason::MisalignedIndirectBuffer(observed) = reason else {
            panic!("expected a misaligned reference, got {reason}");
        };
        assert_eq!(observed.alignment_bits(), low & 3);
        assert_eq!(observed.address_low_raw(), low);
        assert_eq!(run(&words).visited_dwords(), 5);
    }
}

#[test]
fn other_indirect_lengths_stop_without_reading_fields() {
    for count in [0, 1, 3, 4, 0x3fff] {
        let header = type3(INDIRECT, count, 0);
        let words = [vec![PAD, header], vec![0x1000; 5]].concat();
        let reason = terminal(&words, 1);
        assert_eq!(
            reason,
            StopReason::UnsupportedIndirectBufferLength {
                header: Header::new(header)
            }
        );
        assert_eq!(reason.category(), StopCategory::Unsupported);
        assert_eq!(run(&words).visited_dwords(), 2);
    }
}
