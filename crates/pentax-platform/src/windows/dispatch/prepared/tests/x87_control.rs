// SPDX-License-Identifier: GPL-2.0-only
use super::*;

const WAIT: &[u8] = &[0x9b];
const LOAD: &[u8] = &[0xd9, 0x28];
const CONTROL: &[u8] = &[0xd9, 0x38];
const STATUS: &[u8] = &[0xdd, 0x38];
const AX: &[u8] = &[0xdf, 0xe0];
const CLEAR: &[u8] = &[0xdb, 0xe2];
const INIT: &[u8] = &[0xdb, 0xe3];

fn program(bytes: &[u8]) -> Vec<u8> {
    let mut result = bytes.to_vec();
    result.push(0xc3);
    result
}

fn seed(state: &mut NativeCpuState) {
    state.rflags |= 0x8d5;
    state.mxcsr |= 0x25;
    state.xmm.fill(0x56);
    state.ymm_upper.fill(0xa7);
    state.x87.status_word = 0x553f; // Masked sticky exceptions, TOP and conditions.
    state.x87.tag_word = 0x12f3;
    state.x87.instruction_opcode = 0x0456;
    state.x87.instruction_pointer = 0x1234;
    state.x87.data_pointer = 0x5678;
    for (index, register) in state.x87.registers.iter_mut().enumerate() {
        register.fill(0x13 + index as u8);
    }
}

#[test]
fn x87_control_transfers_preserve_register_bits_and_exact_memory_widths() {
    for operation in [WAIT, LOAD, CONTROL, STATUS, AX, CLEAR, INIT] {
        with_source(
            Some(&program(operation)),
            |source, arena, stack, mut state| {
                seed(&mut state);
                let address = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - 2;
                arena
                    .pause()
                    .unwrap()
                    .write(address - 6, &[0xa5; 8])
                    .unwrap();
                arena
                    .pause()
                    .unwrap()
                    .write(address, &0xffff_u16.to_le_bytes())
                    .unwrap();
                state.gpr[RAX] = address;
                let mut expected = state;
                match operation {
                    LOAD => expected.x87.control_word = 0x1f7f,
                    AX => expected.gpr[RAX] = (address & !0xffff) | 0x553f,
                    CLEAR => expected.x87.status_word = 0x5500,
                    INIT => {
                        expected.x87 = NativeX87State::new();
                        expected.x87.registers = state.x87.registers;
                    }
                    _ => {}
                }
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
                engine
                    .resolve(&mut arena.pause().unwrap(), &planned)
                    .unwrap();
                expected.guest_pc = planned.exit().next_guest_pc();
                assert_eq!(engine.machine.state, expected, "{operation:02x?}");
                let mut actual = [0; 8];
                arena
                    .pause()
                    .unwrap()
                    .read(address - 6, &mut actual)
                    .unwrap();
                assert_eq!(&actual[..6], &[0xa5; 6]);
                let last = match operation {
                    CONTROL => 0x037f_u16,
                    STATUS => 0x553f,
                    _ => 0xffff,
                };
                assert_eq!(&actual[6..], &last.to_le_bytes());
                assert_eq!(engine.machine.usage.vector_operations, 1);
                assert_eq!(
                    engine.machine.usage.vector_bytes,
                    if [LOAD, CONTROL, STATUS].contains(&operation) {
                        2
                    } else {
                        0
                    }
                );
            },
        );
    }
}

#[test]
fn x87_waiting_and_nonwaiting_forms_distinguish_pending_exceptions() {
    for flag in 0..6 {
        for operation in [WAIT, LOAD, CONTROL, STATUS, AX, CLEAR, INIT] {
            with_source(
                Some(&program(operation)),
                |source, arena, stack, mut state| {
                    seed(&mut state);
                    let address = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    arena
                        .pause()
                        .unwrap()
                        .write(address, &0x037f_u16.to_le_bytes())
                        .unwrap();
                    state.gpr[RAX] = address;
                    state.x87.control_word &= !(1 << flag);
                    state.x87.status_word = 0x8080 | (1 << flag);
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
                    let result = engine.resolve(&mut arena.pause().unwrap(), &planned);
                    if [WAIT, LOAD].contains(&operation) {
                        assert_eq!(result, Err(NativeDispatchError::UnsupportedX87Exception));
                        assert_eq!(engine.machine.state, state);
                        assert_eq!(engine.machine.usage.vector_operations, 0);
                    } else {
                        result.unwrap(); // No-wait stores, clear and init must remain usable.
                        if operation == CLEAR || operation == INIT {
                            assert_eq!(engine.machine.state.x87.status_word, 0);
                        }
                    }
                },
            );
        }
    }
    // FLDCW can unmask an already sticky exception; the next WAIT must see it.
    with_source(Some(&program(LOAD)), |source, arena, stack, mut state| {
        let address = source.source_plan().modules()[0].load_bias() + DATA_OFF;
        arena
            .pause()
            .unwrap()
            .write(address, &0x037e_u16.to_le_bytes())
            .unwrap();
        state.gpr[RAX] = address;
        state.x87.status_word = 0x3801;
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
        engine
            .resolve(&mut arena.pause().unwrap(), &planned)
            .unwrap();
        assert_eq!(engine.machine.state.x87.control_word, 0x037e);
        assert_eq!(engine.machine.state.x87.status_word, 0xb881);
    });
}

#[test]
fn x87_failed_memory_transfers_publish_neither_cpu_nor_output() {
    for operation in [LOAD, CONTROL, STATUS] {
        for case in 0..4 {
            with_source(
                Some(&program(operation)),
                |source, arena, stack, mut state| {
                    seed(&mut state);
                    let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let address = page + PAGE - 2;
                    arena.pause().unwrap().write(address, &[0xa5; 2]).unwrap();
                    state.gpr[RAX] = match case {
                        0 => address + 1,
                        3 => u64::MAX,
                        _ => address,
                    };
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
                    if case == 1 {
                        arena
                            .protect(
                                page,
                                PAGE,
                                NativePermissions::new(operation != LOAD, false, false),
                            )
                            .unwrap();
                    }
                    let used = if case == 2 { u64::MAX - 1 } else { 0 };
                    engine.machine.usage.vector_bytes = used;
                    engine.machine.phase = Phase::Terminal;
                    assert!(
                        engine
                            .resolve(&mut arena.pause().unwrap(), &planned)
                            .is_err()
                    );
                    assert_eq!(engine.machine.state, state);
                    assert_eq!(engine.machine.usage.vector_operations, 0);
                    assert_eq!(engine.machine.usage.vector_bytes, used);
                    if case == 1 {
                        arena
                            .protect(page, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    let mut actual = [0; 2];
                    arena.pause().unwrap().read(address, &mut actual).unwrap();
                    assert_eq!(actual, [0xa5; 2]);
                },
            );
        }
    }
}

#[test]
fn x87_control_word_normalization_matches_original_closed_host_reference() {
    if !std::is_x86_feature_detected!("fxsr") {
        return;
    }
    #[repr(align(16))]
    struct Area([u8; 512]);
    for input in [0x037f_u16, 0xffff, 0x003f, 0x127f, 0x1f7f] {
        let mut saved = Area([0; 512]);
        let mut output = 0_u16;
        // SAFETY: FXSR checked, exact local operands and aligned 512-byte save
        // area. Every input masks exceptions. Clear before loading and restore
        // the complete original host state within one closed assembly block.
        unsafe {
            std::arch::asm!(
            "fxsave64 [{saved}]", "fnclex", "fldcw [{input}]",
            "fnstcw [{output}]", "fxrstor64 [{saved}]",
            saved=in(reg) saved.0.as_mut_ptr(), input=in(reg) &input,
            output=in(reg) &mut output, options(nostack, preserves_flags));
        }
        assert_eq!(x87_control::control_word(input), output);
    }
}
