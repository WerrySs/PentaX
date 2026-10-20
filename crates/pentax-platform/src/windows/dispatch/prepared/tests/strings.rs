// SPDX-License-Identifier: GPL-2.0-only

//! Modeled elements compared with actual hardware on original owned arrays.
//! No native guest/cache body or untrusted operand is executed by these tests.

use super::*;
use std::arch::asm;

const STATUS_DF: u64 = 0xcd5;

// The assembly saves/restores host flags, including DF, within one block. All
// operands point inside the original 8KiB test array for every admitted count.
fn hardware(
    op: u8,
    size: usize,
    repeat: u8,
    data: &mut [u8; 8192],
    state: &NativeCpuState,
    bias: u64,
) -> (u64, u64, u64, u64, u64) {
    let base = data.as_mut_ptr() as u64;
    let mut src = base + state.gpr[RSI] - bias;
    let mut dst = base + state.gpr[RDI] - bias;
    let mut count = state.gpr[RCX];
    let mut acc = state.gpr[RAX];
    let flags: u64;
    macro_rules! run {
        ($body:expr) => {{
            // SAFETY: This closed reference operates solely on the admitted
            // original array; bounded indices/counts are constructed below.
            // Flags/DF are restored before returning to Rust. No stack pointer
            // output or raw guest address is installed in the host CPU.
            unsafe { asm!(
                "pushfq", "push {initial}", "popfq", $body,
                "pushfq", "pop {observed}", "popfq",
                initial = in(reg) state.rflags,
                observed = lateout(reg) flags,
                inout("rsi") src, inout("rdi") dst, inout("rcx") count,
                inout("rax") acc,
            ); }
        }};
    }
    macro_rules! width {
        ($suffix:literal) => {
            match (op, repeat) {
                (0, 0) => run!(concat!("movs", $suffix)),
                (0, 0xf3) => run!(concat!("rep movs", $suffix)),
                (1, 0) => run!(concat!("stos", $suffix)),
                (1, 0xf3) => run!(concat!("rep stos", $suffix)),
                (2, 0) => run!(concat!("lods", $suffix)),
                (2, 0xf3) => run!(concat!("rep lods", $suffix)),
                (3, 0) => run!(concat!("cmps", $suffix)),
                (3, 0xf3) => run!(concat!("repe cmps", $suffix)),
                (3, 0xf2) => run!(concat!("repne cmps", $suffix)),
                (4, 0) => run!(concat!("scas", $suffix)),
                (4, 0xf3) => run!(concat!("repe scas", $suffix)),
                (4, 0xf2) => run!(concat!("repne scas", $suffix)),
                _ => panic!("invalid closed reference variant"),
            }
        };
    }
    match size {
        1 => width!("b"),
        2 => width!("w"),
        4 => width!("d"),
        8 => width!("q"),
        _ => unreachable!(),
    }
    (src - base, dst - base, count, acc, flags & STATUS_DF)
}

fn encoding(op: u8, size: usize, repeat: u8) -> Vec<u8> {
    let mut code = Vec::new();
    if repeat != 0 {
        code.push(repeat);
    }
    match size {
        2 => code.push(0x66),
        8 => code.push(0x48),
        _ => {}
    }
    code.push([0xa4, 0xaa, 0xac, 0xa6, 0xae][op as usize] + u8::from(size != 1));
    code.push(0xc3);
    code
}

#[test]
fn modeled_strings_match_hardware_all_widths_directions_counts_and_repeat_conditions() {
    let mut cases = 0;
    for op in 0..5 {
        for size in [1, 2, 4, 8] {
            for repeat in [0, 0xf3, 0xf2] {
                if repeat == 0xf2 && op < 3 {
                    continue;
                }
                let code = encoding(op, size, repeat);
                for backward in [false, true] {
                    for count in [0_u64, 1, 9, 257] {
                        with_source(Some(&code), |source, arena, stack, mut state| {
                            let bias = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                            let n = if repeat == 0 { 1 } else { count as usize };
                            let mut data = [0_u8; 8192];
                            for (index, byte) in data.iter_mut().enumerate() {
                                *byte = (index % 251) as u8;
                            }
                            let acc = 0x8877_6655_4433_2211_u64;
                            let start = if backward {
                                n.saturating_sub(1) * size
                            } else {
                                0
                            };
                            for i in 0..n {
                                let offset = if backward { start - i * size } else { i * size };
                                let mut value = acc.to_le_bytes();
                                if (repeat == 0xf2) != (i + 1 == n) {
                                    value[0] ^= 1;
                                }
                                if op == 3 {
                                    data[1024 + offset..1024 + offset + size]
                                        .copy_from_slice(&acc.to_le_bytes()[..size]);
                                }
                                if op >= 3 {
                                    data[4096 + offset..4096 + offset + size]
                                        .copy_from_slice(&value[..size]);
                                }
                            }
                            state.gpr[RSI] = bias + 1024 + start as u64;
                            state.gpr[RDI] = bias + 4096 + start as u64;
                            state.gpr[RCX] = if repeat == 0 { 0x1234 } else { count };
                            state.gpr[RAX] = acc;
                            state.rflags = 0x202 | 0x8d5 | if backward { 0x400 } else { 0 };
                            arena.pause().unwrap().write(bias, &data).unwrap();
                            let expected = hardware(op, size, repeat, &mut data, &state, bias);
                            let mut engine = NativeBlockStepper::new(
                                source,
                                &mut arena.pause().unwrap(),
                                state,
                                stack,
                                &Default::default(),
                            )
                            .unwrap();
                            let planned = block(
                                source,
                                &arena.pause().unwrap(),
                                state.guest_pc,
                                engine.placement(),
                            );
                            loop {
                                engine.machine.phase = Phase::Terminal;
                                let result = engine
                                    .emulate_string(&mut arena.pause().unwrap(), &planned)
                                    .unwrap();
                                if result
                                    == (NativeBlockStop::Resume {
                                        guest_pc: planned.exit().next_guest_pc(),
                                    })
                                {
                                    break;
                                }
                                assert_eq!(
                                    result,
                                    NativeBlockStop::Resume {
                                        guest_pc: state.guest_pc
                                    }
                                );
                            }
                            let actual = engine.state();
                            assert_eq!(
                                (
                                    actual.gpr[RSI] - bias,
                                    actual.gpr[RDI] - bias,
                                    actual.gpr[RCX],
                                    actual.gpr[RAX],
                                    actual.rflags & STATUS_DF
                                ),
                                expected,
                                "op={op} size={size} repeat={repeat} df={backward} count={count}"
                            );
                            let mut bytes = [0; 8192];
                            arena.pause().unwrap().read(bias, &mut bytes).unwrap();
                            assert_eq!(bytes, data);
                            let elements = if repeat == 0 { 1 } else { count - expected.2 };
                            assert_eq!(engine.usage().string_elements, elements);
                            assert_eq!(engine.usage().native_instructions, 0);
                            assert!(engine.machine.string_repeat.is_none());
                            cases += 1;
                        });
                    }
                }
            }
        }
    }
    assert_eq!(cases, 384);
}

#[test]
fn modeled_rep_moves_observe_destructive_overlap_in_both_directions() {
    for size in [1, 2, 4, 8] {
        for backward in [false, true] {
            let code = encoding(0, size, 0xf3);
            with_source(Some(&code), |source, arena, stack, mut state| {
                let bias = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let mut data = [0_u8; 8192];
                for (i, b) in data.iter_mut().enumerate() {
                    *b = (i % 251) as u8;
                }
                state.gpr[RSI] = bias + 3072;
                state.gpr[RDI] = if backward {
                    state.gpr[RSI] - 1
                } else {
                    state.gpr[RSI] + 1
                };
                state.gpr[RCX] = 257;
                state.rflags |= if backward { 0x400 } else { 0 };
                arena.pause().unwrap().write(bias, &data).unwrap();
                let expected = hardware(0, size, 0xf3, &mut data, &state, bias);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement(),
                );
                while engine.state().guest_pc == state.guest_pc {
                    engine.machine.phase = Phase::Terminal;
                    engine
                        .emulate_string(&mut arena.pause().unwrap(), &planned)
                        .unwrap();
                }
                let mut actual = [0; 8192];
                arena.pause().unwrap().read(bias, &mut actual).unwrap();
                assert_eq!(actual, data);
                assert_eq!(engine.state().gpr[RSI] - bias, expected.0);
                assert_eq!(engine.state().gpr[RDI] - bias, expected.1);
            });
        }
    }
}

#[test]
fn modeled_zero_count_string_ignores_invalid_indices_unconfigured_source_and_accumulator() {
    with_source(
        Some(&[0x64, 0xf3, 0x48, 0xa5, 0xc3]),
        |source, arena, stack, mut state| {
            state.gpr[RSI] = u64::MAX;
            state.gpr[RDI] = u64::MAX;
            state.gpr[RAX] = u64::MAX;
            state.gpr[RCX] = 0;
            state.rflags |= STATUS_DF;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            engine
                .emulate_string(&mut arena.pause().unwrap(), &planned)
                .unwrap();
            let mut expected = state;
            expected.guest_pc = planned.exit().next_guest_pc();
            assert_eq!(*engine.state(), expected);
            assert_eq!(engine.usage().string_elements, 0);
            assert_eq!(engine.usage().string_bytes, 0);
        },
    );
}

#[test]
fn modeled_string_element_write_failure_and_budget_do_not_partially_update_memory_or_registers() {
    for budget in [false, true] {
        with_source(
            Some(&[0xf3, 0x66, 0xa5, 0xc3]),
            |source, arena, stack, mut state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                state.gpr[RSI] = data;
                state.gpr[RDI] = data + PAGE - 1;
                state.gpr[RCX] = 3;
                arena
                    .pause()
                    .unwrap()
                    .write(data + PAGE - 1, &[0xa5])
                    .unwrap();
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    state,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let planned = block(
                    source,
                    &arena.pause().unwrap(),
                    state.guest_pc,
                    engine.placement(),
                );
                engine.machine.phase = Phase::Terminal;
                if budget {
                    engine.machine.limits.max_work_units = engine.machine.usage.work_units;
                }
                let result = engine.emulate_string(&mut arena.pause().unwrap(), &planned);
                assert!(if budget {
                    matches!(result, Err(NativeDispatchError::Budget(_)))
                } else {
                    matches!(result, Err(NativeDispatchError::Memory(_)))
                });
                assert_eq!(*engine.state(), state);
                assert_eq!(engine.usage().string_elements, 0);
                let mut byte = [0];
                arena
                    .pause()
                    .unwrap()
                    .read(data + PAGE - 1, &mut byte)
                    .unwrap();
                assert_eq!(byte, [0xa5]);
            },
        );
    }
}

#[test]
fn modeled_repeat_compare_fault_restores_original_flags_across_a_quantum_and_keeps_completed_indices()
 {
    with_source(
        Some(&[0xf3, 0xa6, 0xc3]),
        |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            arena
                .pause()
                .unwrap()
                .write(data, &vec![0x55; PAGE as usize])
                .unwrap();
            state.gpr[RSI] = data;
            state.gpr[RDI] = data + PAGE - 257;
            state.gpr[RCX] = 258;
            state.rflags = 0x202 | 0x8d5;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            assert_eq!(
                engine
                    .emulate_string(&mut arena.pause().unwrap(), &planned)
                    .unwrap(),
                NativeBlockStop::Resume {
                    guest_pc: state.guest_pc
                }
            );
            assert_eq!(engine.state().gpr[RCX], 2);
            assert_ne!(engine.state().rflags, state.rflags);
            // Detach/reattach is the real worker's per-block path; original flags
            // must survive along with the unfinished instruction and cumulative use.
            let mut slot = Some(engine.into_machine());
            let mut engine =
                NativeBlockStepper::from_machine(source, &mut arena.pause().unwrap(), &mut slot)
                    .unwrap();
            engine.machine.phase = Phase::Terminal;
            assert!(matches!(
                engine.emulate_string(&mut arena.pause().unwrap(), &planned),
                Err(NativeDispatchError::Memory(_))
            ));
            assert_eq!(engine.state().gpr[RCX], 1);
            assert_eq!(engine.state().gpr[RSI], data + 257);
            assert_eq!(engine.state().gpr[RDI], data + PAGE);
            assert_eq!(engine.state().rflags, state.rflags);
            assert_eq!(engine.usage().string_elements, 257);
            assert_eq!(engine.usage().string_bytes, 514);
            assert_eq!(engine.state().guest_pc, state.guest_pc);
        },
    );
}

#[test]
fn modeled_string_source_segment_and_unsupported_variants_remain_explicit() {
    for (code, valid) in [
        (vec![0x64, 0xf3, 0xac, 0xc3], true),
        (vec![0xf2, 0xa4, 0xc3], false),
        (vec![0x67, 0xf3, 0xa4, 0xc3], false),
    ] {
        with_source(Some(&code), |source, arena, stack, mut state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            state.fs_base = Some(data);
            state.gpr[RSI] = 3;
            state.gpr[RCX] = 2;
            state.gpr[RAX] = u64::MAX;
            arena
                .pause()
                .unwrap()
                .write(data + 3, &[0x11, 0x2a])
                .unwrap();
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let planned = block(
                source,
                &arena.pause().unwrap(),
                state.guest_pc,
                engine.placement(),
            );
            engine.machine.phase = Phase::Terminal;
            let result = engine.emulate_string(&mut arena.pause().unwrap(), &planned);
            if valid {
                assert!(result.is_ok());
                assert_eq!(engine.state().gpr[RAX], 0xffff_ffff_ffff_ff2a);
                assert_eq!(engine.state().gpr[RSI], 5);
                assert_eq!(engine.state().gpr[RCX], 0);
                assert_eq!(engine.state().rflags, state.rflags);
            } else {
                assert_eq!(
                    result,
                    Err(NativeDispatchError::UnsupportedStringInstruction)
                );
                assert_eq!(*engine.state(), state);
            }
        });
    }
}
