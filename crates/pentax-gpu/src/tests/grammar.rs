// SPDX-License-Identifier: GPL-2.0-or-later

use super::{CONTEXT, NOP, PAD, REGISTER_OPCODES, SH, UCONFIG, bytes, indirect, run, type3, write};
use crate::{Header, PacketBody, RegisterSpace, RegisterWrite};

fn register(inspection: &crate::Inspection, index: usize) -> RegisterWrite<'_> {
    match inspection.packet(index).unwrap().body() {
        PacketBody::RegisterWrite(write) => write,
        other => panic!("expected a register write, got {other:?}"),
    }
}

#[test]
fn header_fields_are_extracted_separately() {
    let header = Header::new(0xc123_76ae);
    assert_eq!(header.packet_type(), 3);
    assert_eq!(header.count(), 0x0123);
    assert_eq!(header.opcode(), 0x76);
    assert_eq!(header.low_byte(), 0xae);
    assert!(!header.predicate());
    assert!(header.shader_type());
    assert!(header.filter_cam_reset());
    assert_eq!(header.reserved_bits(), 0xa8);
    assert_eq!(header.raw(), 0xc123_76ae);
    assert_eq!(Header::new(PAD).packet_type(), 2);
    assert_eq!(Header::new(type3(NOP, 0x3fff, 0)).count(), 0x3fff);
}

#[test]
fn empty_input_completes_without_records_or_storage() {
    let inspection = run(&[]);
    assert!(inspection.is_complete());
    assert_eq!(inspection.packets().len(), 0);
    assert_eq!(
        (
            inspection.committed_dwords(),
            inspection.visited_dwords(),
            inspection.input_dwords()
        ),
        (0, 0, 0)
    );
    assert_eq!(inspection.accounting().owned_capacity_bytes(), 0);
}

#[test]
fn exact_type2_padding_is_one_dword_per_packet() {
    let inspection = run(&[PAD, PAD, PAD]);
    assert!(inspection.is_complete());
    let ranges: Vec<_> = inspection.packets().map(|p| p.dword_range()).collect();
    assert_eq!(ranges, [0..1, 1..2, 2..3]);
    assert!(
        inspection
            .packets()
            .all(|p| p.body() == PacketBody::Padding && p.header().raw() == PAD)
    );
    assert_eq!(inspection.committed_dwords(), 3);
}

#[test]
fn ordinary_nop_payload_is_opaque_and_owner_backed() {
    // Payload words chosen to resemble other packets and marker-like values;
    // none of them may be interpreted.
    let resembling = [
        type3(CONTEXT, 1, 0),
        0x0000_0010,
        type3(0x3f, 2, 0),
        0x4e4f_5021,
        type3(NOP, 0x3fff, 0),
    ];
    let mut words = vec![type3(NOP, 0, 0), 0xfeed_f00d, type3(NOP, 4, 0)];
    words.extend(resembling);
    words.push(PAD);
    let inspection = run(&words);
    assert!(inspection.is_complete());
    assert_eq!(inspection.committed_packets(), 3);

    let PacketBody::Nop(short) = inspection.packet(0).unwrap().body() else {
        panic!("two-dword NOP");
    };
    assert_eq!(short.dword_range(), 1..2);
    assert_eq!(short.bytes(), 0xfeed_f00d_u32.to_le_bytes());

    let long = inspection.packet(1).unwrap();
    assert_eq!((long.at(), long.dwords()), (2, 6));
    let PacketBody::Nop(payload) = long.body() else {
        panic!("six-dword NOP");
    };
    assert_eq!(payload.dword_range(), 3..8);
    assert_eq!(payload.bytes(), bytes(&resembling));
    assert_eq!(inspection.packet(2).unwrap().at(), 8);
}

#[test]
fn largest_ordinary_nop_frames_count_plus_two() {
    let mut words = vec![type3(NOP, 0x3ffe, 0)];
    words.resize(0x4000, 0x1234_5678);
    let inspection = run(&words);
    assert!(inspection.is_complete());
    assert_eq!(inspection.committed_packets(), 1);
    let PacketBody::Nop(payload) = inspection.packet(0).unwrap().body() else {
        panic!("ordinary NOP");
    };
    assert_eq!(payload.dword_range(), 1..0x4000);
}

#[test]
fn header_only_nop_is_framed_before_count_arithmetic() {
    let mut words = vec![type3(NOP, 0x3fff, 0)];
    words.extend(write(CONTEXT, 0, 0x20, &[0xaabb_ccdd]));
    let inspection = run(&words);
    assert!(inspection.is_complete());
    let nop = inspection.packet(0).unwrap();
    assert_eq!((nop.body(), nop.dwords()), (PacketBody::HeaderOnlyNop, 1));
    // Ordinary framing would have demanded 0x4001 dwords.
    let next = inspection.packet(1).unwrap();
    assert_eq!(next.at(), 1);
    let write = register(&inspection, 1);
    assert_eq!((write.offset(), write.value(0)), (0x20, Some(0xaabb_ccdd)));
    assert_eq!(inspection.committed_dwords(), 4);
}

#[test]
fn every_register_grammar_keeps_values_ranges_and_raw_header() {
    let spaces = [
        RegisterSpace::Context,
        RegisterSpace::Sh,
        RegisterSpace::Uconfig,
    ];
    for (opcode, space) in REGISTER_OPCODES.into_iter().zip(spaces) {
        assert_eq!(space.opcode(), opcode);
        let values = [0x0000_0001, 0xffff_ffff, 0x8000_0000];
        let words = [
            write(opcode, 0, 0x0102, &[0x5a5a_a5a5]),
            write(opcode, 0, 0x0300, &values),
        ]
        .concat();
        let inspection = run(&words);
        assert!(inspection.is_complete());
        assert_eq!(inspection.committed_dwords(), words.len());

        let one = register(&inspection, 0);
        assert_eq!((one.space(), one.offset(), one.len()), (space, 0x0102, 1));
        assert_eq!(one.value_range(), 2..3);
        assert_eq!(one.relative_indices(), 0x102..0x103);
        assert_eq!(one.values().collect::<Vec<_>>(), [0x5a5a_a5a5]);

        let many = register(&inspection, 1);
        let packet = inspection.packet(1).unwrap();
        assert_eq!(packet.header().raw(), words[3]);
        assert_eq!(packet.dword_range(), 3..8);
        assert_eq!(many.value_range(), 5..8);
        assert_eq!(many.relative_indices(), 0x300..0x303);
        assert_eq!(many.values().collect::<Vec<_>>(), values);
        assert_eq!(many.value(3), None);
        assert_eq!(many.value_bytes(), &inspection.input()[20..32]);
        assert!(!many.is_empty());
    }
}

#[test]
fn shader_type_and_filter_flags_survive_decoding() {
    for opcode in REGISTER_OPCODES {
        for flags in [0b010, 0b100, 0b110] {
            let inspection = run(&write(opcode, flags, 4, &[9]));
            assert!(inspection.is_complete(), "{inspection:?}");
            let header = inspection.packet(0).unwrap().header();
            assert_eq!(header.shader_type(), flags & 0b010 != 0);
            assert_eq!(header.filter_cam_reset(), flags & 0b100 != 0);
            assert!(!header.predicate());
        }
    }
}

#[test]
fn register_runs_may_end_exactly_at_the_index_domain_end() {
    let words = [
        write(SH, 0, 0xffff, &[1]),
        write(UCONFIG, 0, 0xfff0, &[0; 0x10]),
        write(CONTEXT, 0, 0, &[7]),
    ]
    .concat();
    let inspection = run(&words);
    assert!(inspection.is_complete());
    assert_eq!(
        register(&inspection, 0).relative_indices(),
        0xffff..0x1_0000
    );
    assert_eq!(
        register(&inspection, 1).relative_indices(),
        0xfff0..0x1_0000
    );
    assert_eq!(register(&inspection, 2).relative_indices(), 0..1);
}

#[test]
fn inspection_owns_its_copy_and_repeats_deterministically() {
    let mut source = bytes(&[write(CONTEXT, 0, 1, &[2]), indirect(0, 0, 1).to_vec()].concat());
    let first = crate::inspect(&source, &crate::Limits::default()).unwrap();
    let second = crate::inspect(&source, &crate::Limits::default()).unwrap();
    assert_eq!(first, second);
    source.fill(0xff);
    assert_eq!(register(&first, 0).value(0), Some(2));
    assert_eq!(first, second);
    assert_ne!(first.input(), source.as_slice());
}
