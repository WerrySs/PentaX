// SPDX-License-Identifier: GPL-2.0-only

//! Original native calls to the exact stateless Initializer constructor.
use super::json_primitives::Program;
use super::*;

fn ignored_arguments(program: &mut Program) {
    for (prefix, opcode, value) in [
        (0x48, 0xbe, 0xffff_ffff_ffff_fff1_u64),
        (0x48, 0xba, 0x9234_5678_9abc_def2),
        (0x48, 0xb9, 0xa345_6789_abcd_ef03),
        (0x49, 0xb8, 0xb456_789a_bcde_f014),
        (0x49, 0xb9, 0xc567_89ab_cdef_0125),
    ] {
        program.code.extend([prefix, opcode]);
        program.code.extend(value.to_le_bytes());
    }
}

#[test]
fn native_json_initializer_constructor_preserves_opaque_self_and_guest_bytes_in_exact_scopes() {
    for (module, library) in [
        (b"Json2".as_slice(), b"Json2".as_slice()),
        (b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
    ] {
        let mut program = Program::new();
        for offset in [0, 8, 16, 24, 32, 40, 48, 56] {
            program.store(offset, 0x91a2_b3c4_d5e6_f708);
        }
        for opaque_self in [0, u64::MAX, 0x8123_4567_89ab_cdef] {
            program.code.extend([0x48, 0xbf]);
            program.code.extend(opaque_self.to_le_bytes());
            ignored_arguments(&mut program);
            program.call_nid(b"cK6bYHf-Q5E");
            program.result(opaque_self);
        }
        program.object(17);
        ignored_arguments(&mut program);
        program.call_nid(b"cK6bYHf-Q5E");
        program.pointer(17);
        for offset in [0, 8, 16, 24, 32, 40, 48, 56] {
            program.word(offset, 0x91a2_b3c4_d5e6_f708);
        }
        let image = program.finish(module, library);
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["service_stops"], "4", "{text}");
    }
}
