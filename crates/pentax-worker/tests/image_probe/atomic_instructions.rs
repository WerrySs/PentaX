// SPDX-License-Identifier: GPL-2.0-only

//! Original guests branch over actual hardware RMW effects and preserved flags.
use super::*;

#[test]
fn native_memory_fences_preserve_flags_and_memory() {
    let mut p = Program::new();
    p.set(8, 42);
    p.code
        .extend([0xf9, 0x0f, 0xae, 0xf0, 0x0f, 0xae, 0xe8, 0x0f, 0xae, 0xf8]);
    p.load(0, 8);
    p.code.extend([0x48, 0x83, 0xd0, 0]); // ADC observes carry across all three fences.
    p.expect(43);
    success(p);
}

fn mask(size: usize) -> u64 {
    u64::MAX >> (64 - size * 8)
}
fn cell(p: &mut Program, value: u64) {
    p.reg(0, value);
    p.save(8);
    p.ptr(7, 8);
}
fn flags(p: &mut Program) {
    p.code
        .extend([0x41, 0x0f, 0x94, 0xc0, 0x41, 0x0f, 0x92, 0xc1]);
}
fn expected_flags(p: &mut Program, zf: u64, cf: u64) {
    p.code.extend([0x41, 0x0f, 0xb6, 0xc0]);
    p.expect(zf);
    p.code.extend([0x41, 0x0f, 0xb6, 0xc1]);
    p.expect(cf);
}
fn operation(p: &mut Program, size: usize, locked: bool, opcode: &[u8]) {
    if locked {
        p.code.push(0xf0);
    }
    if size == 2 {
        p.code.push(0x66);
    }
    if size == 8 {
        p.code.push(0x48);
    }
    p.code.extend(opcode);
}
fn success(p: Program) {
    let (code, _) = p.finish(None);
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "0");
}

#[test]
fn native_locked_alu_all_widths_preserves_neighbor_bytes_flags_and_immediate_semantics() {
    for size in [1, 2, 4, 8] {
        let wide = u8::from(size != 1);
        let m = mask(size);
        for (opcode, start, source, cf, zf) in [
            (vec![wide, 0x0f], 33, 9, 0, 0),
            (vec![0x10 + wide, 0x0f], 32, 9, 0, 0),
            (vec![0x28 + wide, 0x0f], 51, 9, 0, 0),
            (vec![0x18 + wide, 0x0f], 52, 9, 0, 0),
            (vec![0x20 + wide, 0x0f], 63, 42, 0, 0),
            (vec![0x08 + wide, 0x0f], 40, 2, 0, 0),
            (vec![0x30 + wide, 0x0f], 40, 2, 0, 0),
            (vec![if size == 1 { 0xfe } else { 0xff }, 0x07], 41, 0, 1, 0),
            (vec![if size == 1 { 0xfe } else { 0xff }, 0x0f], 43, 0, 1, 0),
            (
                vec![if size == 1 { 0xf6 } else { 0xf7 }, 0x1f],
                0_u64.wrapping_sub(42),
                0,
                1,
                0,
            ),
            (
                vec![if size == 1 { 0xf6 } else { 0xf7 }, 0x17],
                !42,
                0,
                1,
                1,
            ),
            (
                vec![if size == 1 { 0x80 } else { 0x83 }, 0x07, 0xff],
                43,
                0,
                1,
                0,
            ),
        ] {
            let mut p = Program::new();
            let initial = (0x1122_3344_5566_7788 & !m) | (start & m);
            cell(&mut p, initial);
            p.reg(1, source);
            p.code.extend([0x45, 0x31, 0xc0, 0xf9]); // ZF=1,CF=1 before the RMW.
            operation(&mut p, size, true, &opcode);
            flags(&mut p);
            expected_flags(&mut p, zf, cf);
            p.word(8, (initial & !m) | 42);
            success(p);
        }
    }
}

#[test]
fn native_exchange_add_and_compare_exchange_keep_full_width_register_outputs_and_real_flags() {
    for size in [1, 2, 4, 8] {
        let m = mask(size);
        let wide = u8::from(size != 1);
        let old = (0x1122_3344_5566_0000 & !m) | 33;
        for exchange in [false, true] {
            let mut p = Program::new();
            cell(&mut p, old);
            let source = (0xaabb_ccdd_eeff_0000 & !m) | 9;
            p.reg(1, source);
            p.code.extend([0x45, 0x31, 0xc0, 0xf9]);
            let opcode = if exchange {
                vec![0x86 + wide, 0x0f]
            } else {
                vec![0x0f, 0xc0 + wide, 0x0f]
            };
            operation(&mut p, size, !exchange, &opcode);
            flags(&mut p);
            p.stack(0x89, 1, 24);
            expected_flags(&mut p, u64::from(exchange), u64::from(exchange));
            p.word(8, (old & !m) | if exchange { 9 } else { 42 });
            p.word(
                24,
                if size >= 4 {
                    old & m
                } else {
                    (source & !m) | (old & m)
                },
            );
            success(p);
        }
        for matched in [false, true] {
            let mut p = Program::new();
            cell(&mut p, old);
            p.reg(1, 42);
            let accumulator = if size >= 4 {
                if matched { 33 } else { 34 }
            } else {
                0xaabb_ccdd_eeff_0000 | if matched { 33 } else { 34 }
            };
            p.reg(0, accumulator);
            operation(&mut p, size, true, &[0x0f, 0xb0 + wide, 0x0f]);
            flags(&mut p);
            p.save(24);
            expected_flags(&mut p, u64::from(matched), 0);
            p.word(8, if matched { (old & !m) | 42 } else { old });
            p.word(
                24,
                if matched {
                    accumulator
                } else if size >= 4 {
                    old & m
                } else {
                    (accumulator & !m) | (old & m)
                },
            );
            success(p);
        }
    }
}

#[test]
fn native_locked_cmpxchg8b_updates_the_real_register_pair_on_both_paths() {
    for matched in [false, true] {
        let mut p = Program::new();
        cell(&mut p, 0x1122_3344_5566_7788);
        p.reg(0, if matched { 0x5566_7788 } else { 0 });
        p.reg(2, 0x1122_3344);
        p.reg(3, 42);
        p.reg(1, 0x99aa_bbcc);
        operation(&mut p, 4, true, &[0x0f, 0xc7, 0x0f]);
        flags(&mut p);
        p.save(24);
        p.stack(0x89, 2, 32);
        p.code.extend([0x41, 0x0f, 0xb6, 0xc0]);
        p.expect(u64::from(matched));
        p.word(
            8,
            if matched {
                0x99aa_bbcc_0000_002a
            } else {
                0x1122_3344_5566_7788
            },
        );
        p.word(24, 0x5566_7788);
        p.word(32, 0x1122_3344);
        success(p);
    }
}

#[test]
fn native_atomic_refusals_do_not_execute_unaligned_foreign_or_read_only_operands() {
    for (instruction, offset, category) in [
        (
            vec![0xf0, 0x48, 0x01, 0x0f],
            9,
            "unsupported-memory-footprint",
        ),
        (vec![0x48, 0x87, 0x0f], 9, "unsupported-memory-footprint"),
        (
            vec![0xf2, 0xf0, 0x48, 0x01, 0x0f],
            8,
            "unsupported-instruction",
        ),
        (
            vec![0xf0, 0x48, 0x0f, 0xab, 0x0f],
            8,
            "unsupported-instruction",
        ),
    ] {
        let mut p = Program::new();
        cell(&mut p, 33);
        p.ptr(7, offset);
        p.reg(1, 9);
        p.code.extend(instruction);
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_ne!(status, 0, "{text}");
        assert_eq!(fields["code"], category);
        if category == "unsupported-memory-footprint" {
            assert_eq!(fields["runtime_failure"], "footprint-atomic-alignment");
            assert_eq!(fields["dispatch_body_ran"], "0");
            assert!(matches!(
                fields["planned_form"].as_str(),
                "Add_rm64_r64" | "Xchg_rm64_r64"
            ));
            assert_eq!(fields["planned_memory"], "destination");
        }
        assert!(!fields.contains_key("value"));
    }
    for readonly in [false, true] {
        let mut p = Program::new();
        p.reg(0, 0);
        p.reg(1, 42);
        if readonly {
            let next = 0x100 + p.code.len() + 7;
            p.code.extend([0x48, 0x8d, 0x3d]);
            p.code.extend((0x100_i32 - next as i32).to_le_bytes());
        } else {
            p.reg(7, u64::MAX - 7);
        }
        p.code.extend([0xf0, 0x48, 0x0f, 0xb1, 0x0f]);
        let (code, _) = p.finish(None);
        let (status, fields, text) = probe(&input(&code, false));
        assert_ne!(status, 0, "{text}");
        assert_eq!(
            fields["code"],
            if readonly {
                "native-dispatch"
            } else {
                "unsupported-memory-footprint"
            }
        );
        assert!(!fields.contains_key("value"));
    }
}
