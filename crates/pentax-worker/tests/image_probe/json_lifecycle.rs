// SPDX-License-Identifier: GPL-2.0-only
//! Original native default-engine initialization and source void lifecycle.
use super::json_primitives::Program;
use super::*;

fn call(program: &mut Program, calls: &mut u64, nid: &'static [u8; 11]) {
    program.call_nid(nid);
    *calls += 1;
}
fn opaque(program: &mut Program, self_word: u64, parameter: u64) {
    program.code.extend([0x48, 0xbf]);
    program.code.extend(self_word.to_le_bytes());
    program.argument(parameter);
    program.code.extend([0x48, 0xba]);
    program.code.extend(u64::MAX.to_le_bytes());
}

#[test]
fn native_json_lifecycle_keeps_owned_defaults_and_values_in_both_exact_scopes() {
    for (module, library) in [
        (b"Json2".as_slice(), b"Json2".as_slice()),
        (b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
    ] {
        let mut program = Program::new();
        let mut calls = 0;
        program.store(112, 0x91a2_b3c4_d5e6_f708);
        for (self_word, parameter) in [(u64::MAX, 1 << 63), (0, 0)] {
            opaque(&mut program, self_word, parameter);
            call(&mut program, &mut calls, b"Cxwy7wHq4J0");
            program.result(0);
        }
        opaque(&mut program, 0, u64::MAX);
        call(&mut program, &mut calls, b"ONT8As5R1ug");
        program.code.extend([0x48, 0x89, 0x44, 0x24, 64]);
        program.object(112);
        program.argument(u64::MAX);
        call(&mut program, &mut calls, b"Cxwy7wHq4J0");
        program.result(0);
        program.word(112, 0x91a2_b3c4_d5e6_f708);
        opaque(&mut program, 0, 0);
        call(&mut program, &mut calls, b"ONT8As5R1ug");
        program.code.extend([0x48, 0x3b, 0x44, 0x24, 64]);
        program.checked();

        program.object(80);
        call(&mut program, &mut calls, b"JP-PtKMiI1E");
        program.pointer(80);
        program.object(0);
        program.argument(17);
        call(&mut program, &mut calls, b"0lLK8+kDqmE");
        program.pointer(0);
        program.object(80);
        program.source(0);
        call(&mut program, &mut calls, b"zQtLRTqceMY");
        program.pointer(80);
        program.object(32);
        program.source(80);
        call(&mut program, &mut calls, b"iZeYfOxtMRg");
        program.pointer(32);
        program.object(80);
        call(&mut program, &mut calls, b"HJ8GpRT1aiw");
        program.word(80, 0);
        for function in [b"PR5k1penBLM", b"RujUxbr3haM"] {
            opaque(&mut program, u64::MAX, u64::MAX);
            call(&mut program, &mut calls, function);
            program.object(32);
            call(&mut program, &mut calls, b"RBw+4NukeGQ");
            program.result(1);
            program.object(32);
            program.argument(0);
            call(&mut program, &mut calls, b"XlWbvieLj2M");
            program.code.extend([0x48, 0x89, 0xc7]);
            call(&mut program, &mut calls, b"DIxvoy7Ngvk");
            program.code.extend([0x48, 0x8b, 0x00]);
            program.result(17);
        }
        opaque(&mut program, u64::MAX, 0);
        call(&mut program, &mut calls, b"00oCq0RwSAY");
        program.result(0);
        opaque(&mut program, 0, u64::MAX);
        call(&mut program, &mut calls, b"Cxwy7wHq4J0");
        program.result(0);
        opaque(&mut program, 0, 0);
        call(&mut program, &mut calls, b"ONT8As5R1ug");
        program.code.extend([0x48, 0x3b, 0x44, 0x24, 64]);
        program.checked();
        program.word(112, 0x91a2_b3c4_d5e6_f708);
        let image = program.finish(module, library);
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        // Defaults + standalone Array + its grown table + deep-copied Value.
        // Destructor quarantine remains counted; repeats add no mappings.
        assert_eq!(fields["kernel_handles_live"], "4", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "4", "{text}");
    }
}

#[test]
fn native_json_lifecycle_registers_opaque_callbacks_without_fetching_or_guest_mapping() {
    for (module, library) in [
        (b"Json2".as_slice(), b"Json2".as_slice()),
        (b"libSceJson".as_slice(), b"libSceJson2".as_slice()),
    ] {
        let mut program = Program::new();
        let mut calls = 0;
        for offset in [0, 8, 16, 24, 32, 40, 48, 56] {
            program.store(offset, 0x91a2_b3c4_d5e6_f708);
        }
        for (callback, context) in [
            (u64::MAX, 0x9234_5678_9abc_def0),
            (1_u64 << 63, 0),
            (0, u64::MAX),
        ] {
            opaque(&mut program, u64::MAX, callback);
            program.code.extend([0x48, 0xba]);
            program.code.extend(context.to_le_bytes());
            call(&mut program, &mut calls, b"00oCq0RwSAY");
            program.result(0);
            for function in [b"PR5k1penBLM", b"RujUxbr3haM"] {
                opaque(&mut program, u64::MAX, u64::MAX);
                call(&mut program, &mut calls, function);
            }
            for offset in [0, 8, 16, 24, 32, 40, 48, 56] {
                program.word(offset, 0x91a2_b3c4_d5e6_f708);
            }
        }
        let image = program.finish(module, library);
        let (status, fields, text) = probe(&image);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["kernel_handles_live"], "0", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
    }
}
