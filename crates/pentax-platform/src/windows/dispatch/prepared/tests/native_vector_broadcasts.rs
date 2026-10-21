// SPDX-License-Identifier: GPL-2.0-only
//! Original exact-tuple broadcasts; declarations never grant host authority.
use super::*;
use pentax_codegen::{Code, CpuFeatures, CpuidFeature};

const RBX: usize = 3;
#[derive(Clone, Copy)]
struct Form {
    code: Code,
    opcode: u8,
    tuple: usize,
    width: usize,
    memory: bool,
    avx2: bool,
}
fn forms() -> Vec<Form> {
    use Code::*;
    let mut forms = Vec::new();
    for (opcode, tuple, codes) in [
        (
            0x78,
            1,
            [VEX_Vpbroadcastb_xmm_xmmm8, VEX_Vpbroadcastb_ymm_xmmm8],
        ),
        (
            0x79,
            2,
            [VEX_Vpbroadcastw_xmm_xmmm16, VEX_Vpbroadcastw_ymm_xmmm16],
        ),
        (
            0x58,
            4,
            [VEX_Vpbroadcastd_xmm_xmmm32, VEX_Vpbroadcastd_ymm_xmmm32],
        ),
        (
            0x59,
            8,
            [VEX_Vpbroadcastq_xmm_xmmm64, VEX_Vpbroadcastq_ymm_xmmm64],
        ),
    ] {
        for (width, code) in [16, 32].into_iter().zip(codes) {
            for memory in [false, true] {
                forms.push(Form {
                    code,
                    opcode,
                    tuple,
                    width,
                    memory,
                    avx2: true,
                });
            }
        }
    }
    for (code, opcode, tuple, width, memory, avx2) in [
        (VEX_Vbroadcastss_xmm_m32, 0x18, 4, 16, true, false),
        (VEX_Vbroadcastss_xmm_xmm, 0x18, 4, 16, false, true),
        (VEX_Vbroadcastss_ymm_m32, 0x18, 4, 32, true, false),
        (VEX_Vbroadcastss_ymm_xmm, 0x18, 4, 32, false, true),
        (VEX_Vbroadcastsd_ymm_m64, 0x19, 8, 32, true, false),
        (VEX_Vbroadcastsd_ymm_xmm, 0x19, 8, 32, false, true),
        (VEX_Vbroadcastf128_ymm_m128, 0x1a, 16, 32, true, false),
        (VEX_Vbroadcasti128_ymm_m128, 0x5a, 16, 32, true, true),
    ] {
        forms.push(Form {
            code,
            opcode,
            tuple,
            width,
            memory,
            avx2,
        });
    }
    assert_eq!(forms.len(), 24);
    forms
}
fn spans() -> Vec<Form> {
    [1, 2, 4, 8, 16]
        .into_iter()
        .map(|tuple| {
            forms()
                .into_iter()
                .find(|form| form.memory && form.tuple == tuple)
                .unwrap()
        })
        .collect()
}
fn policy(native: bool) -> TranslationPolicy {
    if native {
        TranslationPolicy::new(
            CpuFeatures::x64_sse2()
                .with(CpuidFeature::AVX)
                .with(CpuidFeature::AVX2),
        )
        .with_native_ymm_registers()
        .with_native_vector_broadcasts()
    } else {
        TranslationPolicy::x64_sse2()
    }
}
fn mode(form: Form) -> VectorMode {
    VectorMode::Avx {
        avx2: form.avx2,
        fma: false,
    }
}
fn available(form: Form) -> bool {
    let detected = VectorMode::detect();
    detected.features().is_some() && (!form.avx2 || detected.supports(CpuidFeature::AVX2))
}
fn encoding(form: Form, destination: u8) -> [u8; 5] {
    let high = destination >= 8;
    [
        0xc4,
        if form.memory {
            if high { 0x62 } else { 0xe2 }
        } else if high {
            0x42
        } else {
            0xc2
        },
        0x79 | if form.width == 32 { 4 } else { 0 },
        form.opcode,
        ((destination & 7) << 3) | if form.memory { 3 } else { 0xc5 },
    ]
}
fn payload(pattern: usize) -> [u8; 16] {
    match pattern {
        // Raw signaling NaNs in the low single and double lanes, plus mixed
        // high bits. Broadcasts must neither classify nor quiet these values.
        0 => [
            0x31, 0, 0xa0, 0x7f, 0x42, 0, 0xf0, 0x7f, 0x00, 0x00, 0x00, 0x80, 0x89, 0xab, 0xcd,
            0xef,
        ],
        1 => [0; 16],
        2 => [0xff; 16],
        _ => unreachable!(),
    }
}
fn controls() -> Vec<u32> {
    (0..16_u32)
        .map(|bits| 0x1f80 | ((bits & 3) << 13) | ((bits & 4) << 4) | ((bits & 8) << 12))
        .filter(|&control| mxcsr_control::valid(control))
        .collect()
}
fn input(
    mut state: NativeCpuState,
    address: u64,
    control: u32,
    bytes: &[u8; 16],
) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1122_3344_5566_7700 | index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[RBX] = address;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(11);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(17);
    }
    state.xmm[13 * 16..14 * 16].copy_from_slice(bytes);
    state
}
fn expected(
    mut state: NativeCpuState,
    form: Form,
    destination: usize,
    bytes: &[u8; 16],
) -> NativeCpuState {
    let mut repeated = [0; 32];
    for element in repeated[..form.width].chunks_exact_mut(form.tuple) {
        element.copy_from_slice(&bytes[..form.tuple]);
    }
    let at = destination * 16;
    state.xmm[at..at + 16].copy_from_slice(&repeated[..16]);
    state.ymm_upper[at..at + 16].copy_from_slice(&repeated[16..]);
    state.guest_pc = STOP;
    state.gpr[RSP] += 8;
    state
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: Mandatory control stores to exact local objects, without mutation.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]", mx=in(reg) &mut mxcsr, cw=in(reg) &mut x87, options(nostack,preserves_flags));
    }
    (mxcsr, x87)
}

#[test]
fn all_broadcast_shapes_match_software_full_state_and_exact_tuple_literals() {
    if VectorMode::detect().features().is_none() {
        println!(
            "native_vector_broadcasts_available=false native_vector_broadcasts_codes=0 native_vector_broadcasts_shapes=0 native_vector_broadcasts_cases=0 native_vector_broadcasts_mode=sse2"
        );
        return;
    }
    let controls = controls();
    let mut cases = 0_u64;
    let mut shapes = 0_u64;
    let mut codes = Vec::new();
    for form in forms().into_iter().filter(|form| available(*form)) {
        shapes += 1;
        if !codes.contains(&form.code) {
            codes.push(form.code);
        }
        for destination in [0_u8, 13, 15] {
            let mut code = encoding(form, destination).to_vec();
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, state| {
                let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let addresses = if form.memory {
                    vec![page + 64, page + 65, page + PAGE - form.tuple as u64]
                } else {
                    vec![page + 64]
                };
                let initial = input(state, addresses[0], controls[0], &payload(0));
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    initial,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                let native = plan_materialized_block(
                    source,
                    0,
                    state.guest_pc,
                    &code,
                    engine.placement(),
                    &policy(true),
                    &Default::default(),
                )
                .unwrap();
                let software = plan_materialized_block(
                    source,
                    0,
                    state.guest_pc,
                    &code,
                    engine.placement(),
                    &policy(false),
                    &Default::default(),
                )
                .unwrap();
                let ret = plan_materialized_block(
                    source,
                    0,
                    state.guest_pc + 5,
                    &[0xc3],
                    engine.placement(),
                    &policy(false),
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(native.instructions()[0].instruction().code(), form.code);
                assert!(native.instructions()[0].emitted_len() > 0);
                assert_eq!(native.emits_vector_broadcast_memory(), form.memory);
                assert!(!native.emits_non_temporal_store());
                assert!(!software.emits_vector_broadcast_memory());
                assert_eq!(software.instructions()[0].emitted_len(), 0);
                let mut calls = 0;
                for address in addresses {
                    for pattern in 0..3 {
                        let bytes = payload(pattern);
                        if form.memory {
                            arena
                                .protect(page, PAGE, NativePermissions::READ_WRITE)
                                .unwrap();
                            arena
                                .pause()
                                .unwrap()
                                .write(address, &bytes[..form.tuple])
                                .unwrap();
                            arena.protect(page, PAGE, NativePermissions::READ).unwrap();
                        }
                        for &control in &controls {
                            let control = control
                                | if pattern == 0 {
                                    0
                                } else if pattern == 1 {
                                    0x25
                                } else {
                                    0x3f
                                };
                            let current = input(state, address, control, &bytes);
                            let wanted = expected(current, form, usize::from(destination), &bytes);
                            for use_native in [false, true] {
                                if calls != 0 {
                                    engine
                                        .begin_call(&mut arena.pause().unwrap(), current)
                                        .unwrap();
                                } else {
                                    assert_eq!(*engine.state(), current);
                                }
                                engine.machine.vector_mode = if use_native {
                                    mode(form)
                                } else {
                                    VectorMode::Sse2
                                };
                                let before = engine.usage();
                                let host = host_controls();
                                // SAFETY: Original source, live exact read span and
                                // real admitted host mode are independently checked.
                                let stop = unsafe {
                                    engine.step_checked(
                                        &mut arena.pause().unwrap(),
                                        if use_native { &native } else { &software },
                                    )
                                }
                                .unwrap();
                                if use_native {
                                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                                } else {
                                    assert_eq!(
                                        stop,
                                        NativeBlockStop::Resume {
                                            guest_pc: state.guest_pc + 5
                                        }
                                    );
                                    // SAFETY: Same exact owned original RET after software completion.
                                    let stop = unsafe {
                                        engine.step_checked(&mut arena.pause().unwrap(), &ret)
                                    }
                                    .unwrap();
                                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                                }
                                assert_eq!(
                                    *engine.state(),
                                    wanted,
                                    "code={:?} memory={} destination={destination} pattern={pattern} control={control:x}",
                                    form.code,
                                    form.memory
                                );
                                assert_eq!(host_controls(), host);
                                let after = engine.usage();
                                assert_eq!(
                                    after.blocks - before.blocks,
                                    if use_native { 1 } else { 2 }
                                );
                                assert_eq!(
                                    after.native_entries - before.native_entries,
                                    u64::from(use_native)
                                );
                                assert_eq!(
                                    after.native_instructions - before.native_instructions,
                                    u64::from(use_native)
                                );
                                assert_eq!(
                                    after.vector_operations - before.vector_operations,
                                    u64::from(!use_native)
                                );
                                assert_eq!(
                                    after.vector_bytes - before.vector_bytes,
                                    if form.memory && !use_native {
                                        form.tuple as u64
                                    } else {
                                        0
                                    }
                                );
                                assert_eq!(
                                    after.memory_bytes_checked - before.memory_bytes_checked,
                                    if form.memory && use_native {
                                        form.tuple as u64
                                    } else {
                                        0
                                    }
                                );
                                calls += 1;
                            }
                            cases += 1;
                        }
                        if form.memory {
                            let mut live = [0; 16];
                            arena
                                .pause()
                                .unwrap()
                                .read(address, &mut live[..form.tuple])
                                .unwrap();
                            assert_eq!(live[..form.tuple], bytes[..form.tuple]);
                        }
                    }
                }
                assert_eq!(engine.usage().native_entries, calls / 2);
            });
        }
    }
    let avx2 = VectorMode::detect().supports(CpuidFeature::AVX2);
    assert_eq!(shapes, if avx2 { 24 } else { 4 });
    assert_eq!(codes.len(), if avx2 { 16 } else { 4 });
    assert_eq!(cases, if avx2 { 450 } else { 108 } * controls.len() as u64);
    let mode = if avx2 { "avx2" } else { "avx" };
    println!(
        "native_vector_broadcasts_available=true native_vector_broadcasts_codes={} native_vector_broadcasts_shapes={shapes} native_vector_broadcasts_cases={cases} native_vector_broadcasts_mode={mode}",
        codes.len()
    );
}

#[test]
fn checked_and_manual_broadcast_memory_refusals_preserve_state_before_entry() {
    for form in spans().into_iter().filter(|form| available(*form)) {
        for manual in [false, true] {
            for case in 0..13 {
                if case == 12 && !form.avx2 {
                    continue;
                }
                let mut code = encoding(form, 15).to_vec();
                if case == 7 {
                    code.extend([0x8b, 0x00]);
                }
                code.push(0xc3);
                with_source(Some(&code), |source, arena, stack, state| {
                    let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let end = page + PAGE;
                    if case == 1 {
                        arena
                            .map_at(end, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    let address = match case {
                        0 => end - form.tuple as u64 + 1,
                        1 => end - form.tuple as u64 / 2,
                        _ => page + 65,
                    };
                    if case != 0 {
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &payload(0)[..form.tuple])
                            .unwrap();
                    }
                    let mut current = input(state, address, 0x1f80, &payload(0));
                    current.gpr[RAX] = end + PAGE;
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        current,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    engine.machine.vector_mode = mode(form);
                    let plan = plan_materialized_block(
                        source,
                        0,
                        state.guest_pc,
                        &code,
                        engine.placement(),
                        &policy(true).with_independent_memory(),
                        &Default::default(),
                    )
                    .unwrap();
                    assert!(plan.emits_vector_broadcast_memory());
                    if case == 7 {
                        assert!(plan.instructions()[1].emitted_len() > 0);
                    }
                    match case {
                        1 => arena
                            .protect(end, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        2 => arena
                            .protect(page, PAGE, NativePermissions::new(false, true, false))
                            .unwrap(),
                        3 => engine.machine.vector_mode = VectorMode::Sse2,
                        4 => engine.machine.state.mxcsr &= !0x80,
                        5 => engine.machine.state.rflags |= 1 << 18,
                        6 => engine.machine.limits.max_work_units = 0,
                        8 => arena
                            .protect(
                                state.guest_pc & !(PAGE - 1),
                                PAGE,
                                NativePermissions::READ_WRITE,
                            )
                            .unwrap(),
                        9 => engine.machine.usage.memory_bytes_checked = u64::MAX,
                        11 => {
                            let page = state.guest_pc & !(PAGE - 1);
                            arena
                                .protect(page, PAGE, NativePermissions::READ_WRITE)
                                .unwrap();
                            arena
                                .pause()
                                .unwrap()
                                .write(state.guest_pc, &[0x90])
                                .unwrap();
                            arena
                                .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                                .unwrap();
                        }
                        12 => {
                            engine.machine.vector_mode = VectorMode::Avx {
                                avx2: false,
                                fma: false,
                            }
                        }
                        _ => {}
                    }
                    let current = *engine.state();
                    let before = engine.usage();
                    let slots = engine.machine.published_slots.clone();
                    let mut foreign = (case == 10).then(|| {
                        NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap()
                    });
                    let memory = if let Some(foreign) = &mut foreign {
                        foreign
                    } else {
                        &mut *arena
                    };
                    // SAFETY: Every setup is a deliberate pre-entry refusal,
                    // including a changing broadcast before a denied native MOV.
                    let result = unsafe {
                        if manual {
                            engine.step(&mut memory.pause().unwrap(), &plan)
                        } else {
                            engine.step_checked(&mut memory.pause().unwrap(), &plan)
                        }
                    };
                    if case == 10 {
                        assert_eq!(result, Err(NativeDispatchError::WrongOwner));
                    } else {
                        assert!(
                            result.is_err(),
                            "tuple={} manual={manual} case={case}",
                            form.tuple
                        );
                    }
                    assert_eq!(*engine.state(), current);
                    assert_eq!(engine.usage().native_entries, before.native_entries);
                    assert_eq!(
                        engine.usage().native_instructions,
                        before.native_instructions
                    );
                    assert_eq!(engine.usage().cache_publications, before.cache_publications);
                    assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
                    assert!(engine.machine.published_slots == slots);
                });
            }
        }
    }
}

#[test]
fn cached_broadcasts_use_current_tuple_address_contents_and_entire_live_read_span() {
    for form in spans().into_iter().filter(|form| available(*form)) {
        let mut code = encoding(form, 13).to_vec();
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let end = page + PAGE;
            arena
                .map_at(end, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            let first = input(state, page + 65, 0xdfbf, &payload(0));
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                first,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = mode(form);
            let plan = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &policy(true),
                &Default::default(),
            )
            .unwrap();
            for (call, (address, pattern)) in [(page + 65, 0), (page + 97, 1), (page + 97, 2)]
                .into_iter()
                .enumerate()
            {
                arena
                    .pause()
                    .unwrap()
                    .write(address, &payload(pattern)[..form.tuple])
                    .unwrap();
                let current = input(state, address, 0xdfbf, &payload(0));
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                // SAFETY: Same source-bound plan, fresh address and bytes under
                // the current pause; the cache grants no operand permissions.
                let stop =
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap();
                assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                assert_eq!(
                    *engine.state(),
                    expected(current, form, 13, &payload(pattern))
                );
            }
            let cross = end - form.tuple as u64 / 2;
            arena
                .pause()
                .unwrap()
                .write(cross, &payload(0)[..form.tuple])
                .unwrap();
            let current = input(state, cross, 0xdfbf, &payload(2));
            engine
                .begin_call(&mut arena.pause().unwrap(), current)
                .unwrap();
            arena
                .protect(end, PAGE, NativePermissions::new(false, false, false))
                .unwrap();
            let before = engine.usage();
            let slots = engine.machine.published_slots.clone();
            // SAFETY: A cached read whose final page has lost permission must
            // refuse before any destination/upper-state or cache-event effect.
            let result = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) };
            assert!(result.is_err());
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, before.native_entries);
            assert_eq!(engine.usage().cache_publications, before.cache_publications);
            assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
            assert!(engine.machine.published_slots == slots);
            arena.protect(end, PAGE, NativePermissions::READ).unwrap();
            // SAFETY: Restore the actual whole span and retry the never-entered
            // operation. No fresh plan or cached permission authority is used.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap();
            assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
            assert_eq!(*engine.state(), expected(current, form, 13, &payload(0)));
            assert_eq!(engine.usage().cache_publications, 1);
            assert_eq!(engine.usage().cache_reuses, 3);
            assert_eq!(engine.usage().memory_bytes_checked, 5 * form.tuple as u64);
        });
    }
}

#[test]
fn broadcast_rip_sib_and_gpr_dependency_splits_read_actual_observed_addresses() {
    for form in spans().into_iter().filter(|form| available(*form)) {
        for addressing in 0..3 {
            let instruction = encoding(form, 15);
            let mut code = instruction[..4].to_vec();
            match addressing {
                0 => {
                    code.push(0x3d); // RIP-relative source to high destination15.
                    code.extend(
                        i32::try_from((DATA_OFF + 128) as i64 - (ENTRY + 9) as i64)
                            .unwrap()
                            .to_le_bytes(),
                    );
                }
                1 => code.extend([0x7c, 0x8b, 16]), // [RBX+RCX*4+16].
                _ => {
                    code = vec![0x48, 0x89, 0xcb];
                    code.extend(instruction);
                }
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, state| {
                let target = source.source_plan().modules()[0].load_bias() + DATA_OFF + 128;
                arena
                    .pause()
                    .unwrap()
                    .write(target, &payload(0)[..form.tuple])
                    .unwrap();
                let old_base = match addressing {
                    1 => target - 28,
                    2 => target + PAGE,
                    _ => target,
                };
                let mut current = input(state, old_base, 0x1fbf, &payload(2));
                current.gpr[RCX] = if addressing == 1 { 3 } else { target };
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    current,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                engine.machine.vector_mode = mode(form);
                let selected = policy(true).with_independent_memory();
                let first = plan_materialized_block(
                    source,
                    0,
                    current.guest_pc,
                    &code,
                    engine.placement(),
                    &selected,
                    &Default::default(),
                )
                .unwrap();
                if addressing == 0 {
                    assert_eq!(first.instructions()[0].rip_relative_target(), Some(target));
                }
                if addressing == 2 {
                    assert_eq!(first.instructions().len(), 1);
                    assert!(!first.emits_vector_broadcast_memory());
                    // SAFETY: The dependent address is unavailable until this
                    // actual GPR-only block has completed and been observed.
                    let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) }
                        .unwrap();
                    current.gpr[RBX] = target;
                    current.guest_pc += 3;
                    assert_eq!(
                        stop,
                        NativeBlockStop::Resume {
                            guest_pc: current.guest_pc
                        }
                    );
                    assert_eq!(*engine.state(), current);
                    let second = plan_materialized_block(
                        source,
                        0,
                        current.guest_pc,
                        &code[3..],
                        engine.placement(),
                        &selected,
                        &Default::default(),
                    )
                    .unwrap();
                    assert!(second.emits_vector_broadcast_memory());
                    // SAFETY: Observe the new base before checking the exact tuple.
                    let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &second) }
                        .unwrap();
                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                } else {
                    // SAFETY: Original RIP/SIB forms use the independently checked tuple.
                    let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) }
                        .unwrap();
                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                }
                assert_eq!(*engine.state(), expected(current, form, 15, &payload(0)));
                assert_eq!(
                    engine.usage().native_entries,
                    if addressing == 2 { 2 } else { 1 }
                );
                assert_eq!(engine.usage().memory_access_checks, 1);
                assert_eq!(engine.usage().memory_bytes_checked, form.tuple as u64);
            });
        }
    }
}

#[test]
fn broadcast_publishers_and_diagnostic_preflight_require_actual_authority_and_live_tuples() {
    let code = [0xb8, 42, 0, 0, 0, 0xc3];
    with_source(Some(&code), |source, arena, stack, state| {
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            state,
            stack,
            &Default::default(),
        )
        .unwrap();
        for (requested, mode) in [
            (
                TranslationPolicy::x64_sse2().with_native_vector_broadcasts(),
                VectorMode::Avx {
                    avx2: true,
                    fma: false,
                },
            ),
            (policy(true), VectorMode::Sse2),
        ] {
            engine.machine.vector_mode = mode;
            let prepared = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &requested,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                engine.publish(&arena.pause().unwrap(), &prepared),
                Err(NativeDispatchError::InvalidState)
            );
            let diagnostic = pentax_codegen::plan_block(
                source.source_plan(),
                0,
                state.guest_pc,
                engine.placement(),
                &requested,
                &Default::default(),
            )
            .unwrap();
            assert!(matches!(
                super::super::super::publish_block_with_mode(
                    &mut engine.machine.cache,
                    arena,
                    &diagnostic,
                    state.guest_pc,
                    mode
                ),
                Err(PlatformError::UnsupportedHostFeature {
                    feature: "native YMM context"
                })
            ));
        }
        assert_eq!(engine.usage().cache_publications, 0);
        assert_eq!(*engine.state(), state);
    });
    // No hardware entry is needed for these actual-context diagnostic checks;
    // declared optional host features never authorize a memory read.
    for form in spans() {
        let mut code = encoding(form, 15).to_vec();
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                state,
                stack,
                &Default::default(),
            )
            .unwrap();
            let diagnostic = pentax_codegen::plan_block(
                source.source_plan(),
                0,
                state.guest_pc,
                engine.placement(),
                &policy(true),
                &Default::default(),
            )
            .unwrap();
            assert!(diagnostic.emits_vector_broadcast_memory());
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let _guard = DispatchGuard::acquire().unwrap();
            // SAFETY: Hold the real process-wide guard; no assembly is running.
            unsafe {
                (*CONTEXT.0.get()).gpr = state.gpr;
                (*CONTEXT.0.get()).gpr[RBX] = data + PAGE - form.tuple as u64 + 1;
                (*CONTEXT.0.get()).rflags = 0x202 | (1 << 18);
            }
            assert_eq!(
                super::super::super::preflight_diagnostic_vector_memory(arena, &diagnostic),
                Err(PlatformError::NativeDispatchFailed {
                    stage: "native-vector flags"
                })
            );
            // SAFETY: Same exclusive artificial context, still before any entry.
            unsafe {
                (*CONTEXT.0.get()).rflags = 0x202;
            }
            assert!(
                super::super::super::preflight_diagnostic_vector_memory(arena, &diagnostic)
                    .is_err()
            );
            // SAFETY: The exact tuple now ends on the last readable byte.
            unsafe {
                (*CONTEXT.0.get()).gpr[RBX] = data + PAGE - form.tuple as u64;
            }
            assert_eq!(
                super::super::super::preflight_diagnostic_vector_memory(arena, &diagnostic),
                Ok(())
            );
            assert_eq!(engine.usage().native_entries, 0);
        });
    }
}

#[test]
fn broadcast_affine_traces_check_both_live_tuples_and_the_new_profile_authority() {
    for form in forms().into_iter().filter(|form| {
        form.memory
            && matches!(
                form.code,
                Code::VEX_Vbroadcastss_xmm_m32 | Code::VEX_Vpbroadcastq_ymm_xmmm64
            )
            && available(*form)
    }) {
        let prefix = encoding(form, 15);
        let mut code = prefix[..4].to_vec();
        code.extend([0x3c, 0x3b]); // Actual tuple at [RBX+RDI].
        code.extend([0x48, 0x83, 0xc7, form.tuple as u8, 0x48, 0x39, 0xcf]);
        let back = i8::try_from(-(code.len() as i64 + 2)).unwrap();
        code.extend([0x75, back as u8, 0xc3]);
        with_source(Some(&code), |source, arena, stack, state| {
            let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let first = page + 65;
            arena
                .pause()
                .unwrap()
                .write(first, &payload(0)[..form.tuple])
                .unwrap();
            arena
                .pause()
                .unwrap()
                .write(first + form.tuple as u64, &payload(1)[..form.tuple])
                .unwrap();
            let mut current = input(state, first, 0x1fbf, &payload(2));
            current.gpr[RDI] = 0;
            current.gpr[RCX] = 2 * form.tuple as u64;
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                current,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = mode(form);
            let selected = policy(true)
                .with_independent_memory()
                .with_affine_loop_trace();
            let limits = BlockLimits::default();
            let mut body = plan_materialized_block(
                source,
                0,
                current.guest_pc,
                &code,
                engine.placement(),
                &selected,
                &limits,
            )
            .unwrap();
            let reserved = body.affine_loop_trace_work_bound(&limits).unwrap();
            assert!(
                body.prepare_affine_loop_trace(&limits, reserved, limits.max_allocation_bytes)
                    <= reserved
            );
            let trace = body.affine_loop_trace().unwrap();
            assert!(body.emits_vector_broadcast_memory());
            assert_eq!(body.native_admission().emitted_instructions(), 3);
            let ret_pc = current.guest_pc + code.len() as u64 - 1;
            let ret = plan_materialized_block(
                source,
                0,
                ret_pc,
                &[0xc3],
                engine.placement(),
                &selected,
                &limits,
            )
            .unwrap();
            let before = engine.usage();
            // SAFETY: Original finite two-iteration tuple loop; both current
            // reads and source/stack/owner/mode are checked under one pause.
            let stop = unsafe {
                engine.try_step_affine_loop_checked(&mut arena.pause().unwrap(), &trace, 2)
            }
            .unwrap()
            .unwrap();
            assert_eq!(stop, NativeBlockStop::Resume { guest_pc: ret_pc });
            assert_eq!(engine.usage().native_entries - before.native_entries, 1);
            assert_eq!(
                engine.usage().native_instructions - before.native_instructions,
                6
            );
            assert_eq!(
                engine.usage().affine_loop_entries - before.affine_loop_entries,
                1
            );
            assert_eq!(
                engine.usage().affine_loop_iterations - before.affine_loop_iterations,
                2
            );
            assert_eq!(
                engine.usage().memory_bytes_checked - before.memory_bytes_checked,
                2 * form.tuple as u64
            );
            // SAFETY: The same original owned RET follows the captured trace.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &ret) }.unwrap();
            assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
            let mut wanted = expected(current, form, 15, &payload(1));
            wanted.gpr[RDI] = 2 * form.tuple as u64;
            wanted.rflags = 0x202 | 0x400 | 0x44; // Equal CMP, preserving IF/DF.
            assert_eq!(*engine.state(), wanted);

            current.gpr[RBX] = page + PAGE - form.tuple as u64;
            current.gpr[RCX] = form.tuple as u64;
            arena
                .pause()
                .unwrap()
                .write(current.gpr[RBX], &payload(0)[..form.tuple])
                .unwrap();
            engine
                .begin_call(&mut arena.pause().unwrap(), current)
                .unwrap();
            let before = engine.usage();
            let slots = engine.machine.published_slots.clone();
            // SAFETY: Even though the first iteration would leave the loop,
            // the speculative second tuple is unmapped and must force fallback.
            let stop = unsafe {
                engine.try_step_affine_loop_checked(&mut arena.pause().unwrap(), &trace, 2)
            }
            .unwrap();
            assert_eq!(stop, None);
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, before.native_entries);
            assert_eq!(engine.usage().cache_publications, before.cache_publications);
            assert_eq!(engine.usage().cache_reuses, before.cache_reuses);
            assert!(engine.machine.published_slots == slots);
            engine.machine.vector_mode = if form.avx2 {
                VectorMode::Avx {
                    avx2: false,
                    fma: false,
                }
            } else {
                VectorMode::Sse2
            };
            let before = engine.usage();
            // SAFETY: Deliberately remove actual AVX/AVX2 authority; an opaque
            // retained template grants no right to bypass current host admission.
            let stop = unsafe {
                engine.try_step_affine_loop_checked(&mut arena.pause().unwrap(), &trace, 2)
            }
            .unwrap();
            assert_eq!(stop, None);
            assert_eq!(*engine.state(), current);
            assert_eq!(
                engine.usage().memory_access_checks,
                before.memory_access_checks
            );
            assert!(engine.machine.published_slots == slots);
            engine.machine.vector_mode = mode(form);
            // SAFETY: The unchanged one-iteration path requires only its real
            // first tuple, which remains fully readable despite trace refusal.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &body) }.unwrap();
            assert_eq!(stop, NativeBlockStop::Resume { guest_pc: ret_pc });
        });
    }
    // A requested-but-unused profile still needs full-YMM authority in the
    // trace path, independently of the emitted instruction feature union.
    let code = [
        0x8b, 0x04, 0x3b, 0x48, 0x83, 0xc7, 4, 0x48, 0x39, 0xcf, 0x75, 0xf4, 0xc3,
    ];
    with_source(Some(&code), |source, arena, stack, state| {
        let page = source.source_plan().modules()[0].load_bias() + DATA_OFF;
        let mut current = input(state, page, 0x1f80, &payload(0));
        current.gpr[RDI] = 0;
        current.gpr[RCX] = 8;
        let mut engine = NativeBlockStepper::new(
            source,
            &mut arena.pause().unwrap(),
            current,
            stack,
            &Default::default(),
        )
        .unwrap();
        let selected = TranslationPolicy::x64_sse2()
            .with_native_vector_broadcasts()
            .with_independent_memory()
            .with_affine_loop_trace();
        let limits = BlockLimits::default();
        let mut body = plan_materialized_block(
            source,
            0,
            state.guest_pc,
            &code,
            engine.placement(),
            &selected,
            &limits,
        )
        .unwrap();
        let reserved = body.affine_loop_trace_work_bound(&limits).unwrap();
        body.prepare_affine_loop_trace(&limits, reserved, limits.max_allocation_bytes);
        let trace = body.affine_loop_trace().unwrap();
        assert!(!body.emits_vector_broadcast_memory());
        let before = engine.usage();
        let slots = engine.machine.published_slots.clone();
        // SAFETY: This original GPR body does not waive the requested optional
        // profile's independent full-YMM requirement. No hardware may execute.
        let stop =
            unsafe { engine.try_step_affine_loop_checked(&mut arena.pause().unwrap(), &trace, 2) }
                .unwrap();
        assert_eq!(stop, None);
        assert_eq!(*engine.state(), current);
        assert_eq!(engine.usage().native_entries, before.native_entries);
        assert_eq!(
            engine.usage().memory_access_checks,
            before.memory_access_checks
        );
        assert!(engine.machine.published_slots == slots);
    });
}
