// SPDX-License-Identifier: GPL-2.0-only
//! Original full-state MIN/MAX register and exact-span memory programs.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

const RBX: usize = 3;
const ALIASES: [u8; 3] = [9, 13, 14];
const PATTERNS: usize = 12;
#[derive(Clone, Copy)]
struct Form {
    opcode: u8,
    width: usize,
    element: usize,
    scalar: bool,
}
impl Form {
    fn read(self) -> usize {
        if self.scalar {
            self.element
        } else {
            self.width
        }
    }
}
fn forms() -> Vec<Form> {
    let mut forms = Vec::new();
    for opcode in [0x5d, 0x5f] {
        for element in [4, 8] {
            for width in [16, 32] {
                forms.push(Form {
                    opcode,
                    width,
                    element,
                    scalar: false,
                });
            }
            forms.push(Form {
                opcode,
                width: 16,
                element,
                scalar: true,
            });
        }
    }
    assert_eq!(forms.len(), 12);
    forms
}
fn policy(native: bool) -> TranslationPolicy {
    let policy = TranslationPolicy::new(CpuFeatures::x64_sse2().with(CpuidFeature::AVX))
        .with_native_ymm_registers();
    if native {
        policy.with_native_float_min_max()
    } else {
        policy
    }
}
fn encoding(form: Form, destination: u8, memory: bool) -> [u8; 5] {
    [
        0xc4,
        if memory { 0x61 } else { 0x41 },
        0x10 | u8::from(form.element == 8)
            | if form.scalar {
                2
            } else if form.width == 32 {
                4
            } else {
                0
            },
        form.opcode,
        ((destination & 7) << 3) | if memory { 3 } else { 0xc6 },
    ]
}
fn controls() -> Vec<u32> {
    (0..16_u32)
        .map(|bits| 0x1f80 | ((bits & 3) << 13) | ((bits & 4) << 4) | ((bits & 8) << 12))
        .filter(|&value| mxcsr_control::valid(value))
        .collect()
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: Mandatory control stores to exact local objects; no mutation.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]",
            mx=in(reg) &mut mxcsr, cw=in(reg) &mut x87,
            options(nostack,preserves_flags));
    }
    (mxcsr, x87)
}
fn pair(double: bool, pattern: usize) -> (u64, u64) {
    let (one, two, six, exponent, quiet, sign) = if double {
        (
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x4018_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x0008_0000_0000_0000,
            1_u64 << 63,
        )
    } else {
        (
            0x3f80_0000,
            0x4000_0000,
            0x40c0_0000,
            0x7f80_0000,
            0x0040_0000,
            1_u64 << 31,
        )
    };
    match pattern % PATTERNS {
        0 => (six, two),
        1 => (0, sign),
        2 => (sign, 0),
        3 => (exponent | quiet | 0x11, two),
        4 => (one, sign | exponent | quiet | 0x22),
        5 => (one, sign | exponent | 0x33), // Raw right signaling NaN survives.
        6 => (exponent | 0x11, sign | exponent | quiet | 0x22),
        7 => (exponent | quiet | 0x11, 1), // Same-lane IE suppresses DE.
        8 => (1, one),
        9 => (one, 1),
        10 => (sign | 1, 0),
        11 => (exponent, sign | exponent),
        _ => unreachable!(),
    }
}
fn vectors(form: Form, pattern: usize) -> ([u8; 32], [u8; 32]) {
    let mut left = [0; 32];
    let mut right = [0; 32];
    for offset in (0..form.width).step_by(form.element) {
        let index = offset / form.element;
        let (a, b) = if form.scalar && index != 0 {
            let infinity = if form.element == 8 {
                0x7ff0_0000_0000_0000
            } else {
                0x7f80_0000
            };
            (
                infinity | 0x100 | index as u64,
                infinity | 0x200 | index as u64,
            )
        } else {
            pair(form.element == 8, pattern + index)
        };
        left[offset..offset + form.element].copy_from_slice(&a.to_le_bytes()[..form.element]);
        right[offset..offset + form.element].copy_from_slice(&b.to_le_bytes()[..form.element]);
    }
    (left, right)
}
fn vector(state: &NativeCpuState, register: usize) -> [u8; 32] {
    let at = register * 16;
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&state.xmm[at..at + 16]);
    bytes[16..].copy_from_slice(&state.ymm_upper[at..at + 16]);
    bytes
}
fn input(
    mut state: NativeCpuState,
    left: &[u8; 32],
    right: &[u8; 32],
    address: u64,
    control: u32,
    memory: bool,
) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.gpr[RBX] = address;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control;
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(3);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(13);
    }
    for (register, bytes) in [(13, *left), (14, if memory { [0xa5; 32] } else { *right })] {
        let at = register * 16;
        state.xmm[at..at + 16].copy_from_slice(&bytes[..16]);
        state.ymm_upper[at..at + 16].copy_from_slice(&bytes[16..]);
    }
    state
}
fn expected(
    mut state: NativeCpuState,
    form: Form,
    destination: u8,
    memory: bool,
    right: &[u8; 32],
) -> NativeCpuState {
    let left = vector(&state, 13);
    let right = if memory { *right } else { vector(&state, 14) };
    let mut output = if form.scalar { left } else { [0; 32] };
    if form.width == 16 {
        output[16..].fill(0);
    }
    let (exponent, fraction, sign) = if form.element == 8 {
        (0x7ff0_0000_0000_0000, 0x000f_ffff_ffff_ffff, 1_u64 << 63)
    } else {
        (0x7f80_0000, 0x007f_ffff, 1_u64 << 31)
    };
    for offset in (0..form.read()).step_by(form.element) {
        let raw = |bytes: &[u8; 32]| {
            let mut value = [0; 8];
            value[..form.element].copy_from_slice(&bytes[offset..offset + form.element]);
            u64::from_le_bytes(value)
        };
        let (a, b) = (raw(&left), raw(&right));
        let operation = if form.opcode == 0x5d {
            float_ops::Operation::Min
        } else {
            float_ops::Operation::Max
        };
        let (bits, control) =
            float_ops::calculate(operation, form.element == 8, a, b, state.mxcsr).unwrap();
        let nan = |value: u64| value & exponent == exponent && value & fraction != 0;
        let denormal = |value: u64| value & exponent == 0 && value & fraction != 0;
        let effective = |value| {
            if state.mxcsr & 0x40 != 0 && denormal(value) {
                value & sign
            } else {
                value
            }
        };
        let (effective_a, effective_b) = (effective(a), effective(b));
        if nan(a) || nan(b) || effective_a & !sign == 0 && effective_b & !sign == 0 {
            assert_eq!(bits, effective_b); // Right operand wins, without quieting.
        }
        let invalid = nan(a) || nan(b);
        let denormal = !invalid && state.mxcsr & 0x40 == 0 && (denormal(a) || denormal(b));
        assert_eq!(
            control & 3,
            (state.mxcsr & 3) | u32::from(invalid) | (u32::from(denormal) << 1)
        );
        assert_eq!(control & 0x38, state.mxcsr & 0x38); // No arithmetic OE/UE/PE.
        if (a, b) == pair(form.element == 8, 0) {
            assert_eq!(bits, if form.opcode == 0x5d { b } else { a });
        }
        output[offset..offset + form.element].copy_from_slice(&bits.to_le_bytes()[..form.element]);
        state.mxcsr = control;
    }
    let at = destination as usize * 16;
    state.xmm[at..at + 16].copy_from_slice(&output[..16]);
    state.ymm_upper[at..at + 16].copy_from_slice(&output[16..]);
    state.gpr[RSP] += 8;
    state.guest_pc = STOP;
    state
}

#[test]
fn all_minmax_register_and_memory_forms_match_full_state_nan_zero_controls_and_aliases() {
    if VectorMode::detect().features().is_none() {
        println!(
            "native_float_min_max_available=false native_float_min_max_cases=0 native_float_min_max_mode=software"
        );
        return;
    }
    let controls = controls();
    assert!(!controls.is_empty());
    let mut cases = 0_u64;
    let mut codes = Vec::new();
    for form in forms() {
        for memory in [false, true] {
            let code: Vec<_> = ALIASES
                .into_iter()
                .flat_map(|destination| {
                    encoding(form, destination, memory)
                        .into_iter()
                        .chain([0xc3])
                })
                .collect();
            with_source(Some(&code), |source, arena, stack, state| {
                let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                let addresses = [data + 64, data + 65, data + PAGE - form.read() as u64];
                let (left, right) = vectors(form, 0);
                let initial = input(state, &left, &right, addresses[0], controls[0], memory);
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    initial,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                engine.machine.vector_mode = VectorMode::Avx {
                    avx2: false,
                    fma: false,
                };
                let mut calls = 0_u64;
                for (alias, destination) in ALIASES.into_iter().enumerate() {
                    let pc = state.guest_pc + (alias * 6) as u64;
                    let bytes = &code[alias * 6..alias * 6 + 6];
                    let native = plan_materialized_block(
                        source,
                        0,
                        pc,
                        bytes,
                        engine.placement(),
                        &policy(true),
                        &Default::default(),
                    )
                    .unwrap();
                    let software = plan_materialized_block(
                        source,
                        0,
                        pc,
                        bytes,
                        engine.placement(),
                        &policy(false),
                        &Default::default(),
                    )
                    .unwrap();
                    let ret = plan_materialized_block(
                        source,
                        0,
                        pc + 5,
                        &bytes[5..],
                        engine.placement(),
                        &policy(false),
                        &Default::default(),
                    )
                    .unwrap();
                    assert!(native.instructions()[0].emitted_len() > 0);
                    assert_eq!(native.emits_float_min_max_memory(), memory);
                    assert_eq!(software.instructions()[0].emitted_len(), 0);
                    if !memory && alias == 0 {
                        let code = native.instructions()[0].instruction().code();
                        assert!(!codes.contains(&code));
                        codes.push(code);
                    }
                    for &address in &addresses[..if memory { 3 } else { 1 }] {
                        for pattern in 0..PATTERNS {
                            let (left, right) = vectors(form, pattern);
                            if memory {
                                arena
                                    .pause()
                                    .unwrap()
                                    .write(address, &right[..form.read()])
                                    .unwrap();
                            }
                            for &control in &controls {
                                let mut current = input(
                                    state,
                                    &left,
                                    &right,
                                    address,
                                    control | if pattern == 11 { 0x24 } else { 0 },
                                    memory,
                                );
                                current.guest_pc = pc;
                                let wanted = expected(current, form, destination, memory, &right);
                                for use_native in [true, false] {
                                    if calls != 0 {
                                        engine
                                            .begin_call(&mut arena.pause().unwrap(), current)
                                            .unwrap();
                                    }
                                    let before = engine.usage();
                                    let host = host_controls();
                                    // SAFETY: Original same-owner full spans,
                                    // actual AVX mode, masked controls and stack.
                                    unsafe {
                                        if use_native {
                                            let stop = engine
                                                .step_checked(&mut arena.pause().unwrap(), &native)
                                                .unwrap();
                                            assert_eq!(
                                                stop,
                                                NativeBlockStop::Returned { value: 42 }
                                            );
                                        } else {
                                            let stop = engine
                                                .step_checked(
                                                    &mut arena.pause().unwrap(),
                                                    &software,
                                                )
                                                .unwrap();
                                            assert_eq!(
                                                stop,
                                                NativeBlockStop::Resume { guest_pc: pc + 5 }
                                            );
                                            let stop = engine
                                                .step_checked(&mut arena.pause().unwrap(), &ret)
                                                .unwrap();
                                            assert_eq!(
                                                stop,
                                                NativeBlockStop::Returned { value: 42 }
                                            );
                                        }
                                    }
                                    assert_eq!(
                                        *engine.state(),
                                        wanted,
                                        "opcode={} scalar={} width={} element={} memory={memory} alias={alias} pattern={pattern} control={control:04x}",
                                        form.opcode,
                                        form.scalar,
                                        form.width,
                                        form.element
                                    );
                                    assert_eq!(host_controls(), host);
                                    let after = engine.usage();
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
                                        if memory && !use_native {
                                            form.read() as u64
                                        } else {
                                            0
                                        }
                                    );
                                    assert_eq!(
                                        after.memory_bytes_checked - before.memory_bytes_checked,
                                        if memory && use_native {
                                            form.read() as u64
                                        } else {
                                            0
                                        }
                                    );
                                    assert_eq!(
                                        after.blocks - before.blocks,
                                        if use_native { 1 } else { 2 }
                                    );
                                    calls += 1;
                                }
                                cases += 1;
                            }
                            if memory {
                                let mut live = [0; 32];
                                arena
                                    .pause()
                                    .unwrap()
                                    .read(address, &mut live[..form.read()])
                                    .unwrap();
                                assert_eq!(live[..form.read()], right[..form.read()]);
                            }
                        }
                    }
                }
                assert_eq!(engine.usage().native_entries, calls / 2);
                assert_eq!(engine.usage().native_instructions, calls / 2);
                assert_eq!(engine.usage().vector_operations, calls / 2);
            });
        }
    }
    assert_eq!(codes.len(), 12);
    assert_eq!(cases, 12 * 3 * PATTERNS as u64 * controls.len() as u64 * 4);
    println!(
        "native_float_min_max_available=true native_float_min_max_codes=12 native_float_min_max_shapes=24 native_float_min_max_cases={cases} native_float_min_max_mode=avx"
    );
}

fn span_forms() -> Vec<Form> {
    [4, 8, 16, 32]
        .into_iter()
        .map(|size| {
            forms()
                .into_iter()
                .find(|form| form.opcode == 0x5d && form.read() == size)
                .unwrap()
        })
        .collect()
}

#[test]
fn minmax_memory_manual_and_checked_refusals_preserve_full_state_before_publication() {
    if VectorMode::detect().features().is_none() {
        return;
    }
    for form in span_forms() {
        for manual in [false, true] {
            for case in 0..12 {
                let mut code = encoding(form, 9, true).to_vec();
                if case == 7 {
                    code.extend([0x8b, 0x00]);
                }
                code.push(0xc3);
                with_source(Some(&code), |source, arena, stack, state| {
                    let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
                    let end = data + PAGE;
                    let size = form.read() as u64;
                    if case == 1 {
                        arena
                            .map_at(end, PAGE, NativePermissions::READ_WRITE)
                            .unwrap();
                    }
                    let address = if case == 0 {
                        end - size + 1
                    } else if case == 1 {
                        end - size / 2
                    } else {
                        end - size
                    };
                    let (left, right) = vectors(form, 5);
                    if case != 0 {
                        arena
                            .pause()
                            .unwrap()
                            .write(address, &right[..form.read()])
                            .unwrap();
                    }
                    let mut current = input(state, &left, &right, address, 0x1f80, true);
                    current.gpr[RAX] = end + PAGE;
                    let mut engine = NativeBlockStepper::new(
                        source,
                        &mut arena.pause().unwrap(),
                        current,
                        stack,
                        &Default::default(),
                    )
                    .unwrap();
                    let plan = plan_materialized_block(
                        source,
                        0,
                        current.guest_pc,
                        &code,
                        engine.placement(),
                        &policy(true).with_independent_memory(),
                        &Default::default(),
                    )
                    .unwrap();
                    assert!(plan.emits_float_min_max_memory());
                    assert!(!plan.emits_non_temporal_store());
                    assert!(plan.instructions()[0].emitted_len() > 0);
                    if case == 7 {
                        assert!(plan.instructions()[1].emitted_len() > 0);
                    }
                    match case {
                        1 => arena
                            .protect(end, PAGE, NativePermissions::new(false, false, false))
                            .unwrap(),
                        2 => arena
                            .protect(data, PAGE, NativePermissions::new(false, true, false))
                            .unwrap(),
                        3 => engine.machine.vector_mode = VectorMode::Sse2,
                        4 => engine.machine.state.mxcsr &= !0x80,
                        5 => engine.machine.state.rflags |= 1 << 18,
                        6 => engine.machine.limits.max_work_units = 0,
                        8 => arena
                            .protect(
                                current.guest_pc & !(PAGE - 1),
                                PAGE,
                                NativePermissions::READ_WRITE,
                            )
                            .unwrap(),
                        9 => engine.machine.usage.memory_bytes_checked = u64::MAX,
                        11 => {
                            let page = current.guest_pc & !(PAGE - 1);
                            arena
                                .protect(page, PAGE, NativePermissions::READ_WRITE)
                                .unwrap();
                            arena
                                .pause()
                                .unwrap()
                                .write(current.guest_pc, &[0x90])
                                .unwrap();
                            arena
                                .protect(page, PAGE, NativePermissions::READ_EXECUTE)
                                .unwrap();
                        }
                        _ => {}
                    }
                    let before = *engine.state();
                    let host = host_controls();
                    let result = if case == 10 {
                        let mut other =
                            NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                        // SAFETY: Deliberate wrong-owner refusal before entry.
                        unsafe {
                            if manual {
                                engine.step(&mut other.pause().unwrap(), &plan)
                            } else {
                                engine.step_checked(&mut other.pause().unwrap(), &plan)
                            }
                        }
                    } else {
                        // SAFETY: Original controlled invalid span/state/quota,
                        // checked before any native instruction can execute.
                        unsafe {
                            if manual {
                                engine.step(&mut arena.pause().unwrap(), &plan)
                            } else {
                                engine.step_checked(&mut arena.pause().unwrap(), &plan)
                            }
                        }
                    };
                    match case {
                        0..=2 | 7 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                        3..=5 => assert_eq!(result, Err(NativeDispatchError::InvalidState)),
                        6 | 9 => assert!(matches!(result, Err(NativeDispatchError::Budget(_)))),
                        8 => assert_eq!(result, Err(NativeDispatchError::WritableSource)),
                        10 => assert_eq!(result, Err(NativeDispatchError::WrongOwner)),
                        11 => assert_eq!(result, Err(NativeDispatchError::SourceMismatch)),
                        _ => unreachable!(),
                    }
                    assert_eq!(*engine.state(), before);
                    assert_eq!(host_controls(), host);
                    assert_eq!(engine.usage().native_entries, 0);
                    assert_eq!(engine.usage().native_instructions, 0);
                    assert_eq!(engine.usage().cache_publications, 0);
                });
            }
        }
    }
}

#[test]
fn cached_minmax_memory_rechecks_current_address_bytes_and_second_page_permissions() {
    if VectorMode::detect().features().is_none() {
        return;
    }
    for form in span_forms() {
        let mut code = encoding(form, 9, true).to_vec();
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            arena
                .map_at(data + PAGE, PAGE, NativePermissions::READ_WRITE)
                .unwrap();
            let addresses = [data + 64, data + 128, data + PAGE - form.read() as u64 / 2];
            let (left, first) = vectors(form, 0);
            let (_, second) = vectors(form, 4);
            let (_, changed) = vectors(form, 5);
            for (address, right) in [
                (addresses[0], first),
                (addresses[1], second),
                (addresses[2], first),
            ] {
                arena
                    .pause()
                    .unwrap()
                    .write(address, &right[..form.read()])
                    .unwrap();
            }
            arena
                .protect(data, 2 * PAGE, NativePermissions::READ)
                .unwrap();
            let initial = input(state, &left, &first, addresses[0], 0x1f80, true);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            engine.machine.vector_mode = VectorMode::Avx {
                avx2: false,
                fma: false,
            };
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
            for (call, (address, right)) in [
                (addresses[0], first),
                (addresses[1], second),
                (addresses[0], changed),
                (addresses[2], first),
            ]
            .into_iter()
            .enumerate()
            {
                if call == 2 {
                    arena
                        .protect(data, PAGE, NativePermissions::READ_WRITE)
                        .unwrap();
                    arena
                        .pause()
                        .unwrap()
                        .write(address, &right[..form.read()])
                        .unwrap();
                    arena.protect(data, PAGE, NativePermissions::READ).unwrap();
                }
                let current = input(state, &left, &right, address, 0x1f80, true);
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                // SAFETY: Manual stepping must perform the new profile's live
                // full-span check even though the sealed native bytes are reused.
                let stop = unsafe { engine.step(&mut arena.pause().unwrap(), &plan) }.unwrap();
                assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                assert_eq!(*engine.state(), expected(current, form, 9, true, &right));
            }
            assert_eq!(engine.usage().native_entries, 4);
            assert_eq!(engine.usage().cache_publications, 1);
            assert_eq!(engine.usage().cache_reuses, 3);
            let current = input(state, &left, &first, addresses[2], 0x1f80, true);
            engine
                .begin_call(&mut arena.pause().unwrap(), current)
                .unwrap();
            arena
                .protect(
                    data + PAGE,
                    PAGE,
                    NativePermissions::new(false, false, false),
                )
                .unwrap();
            // SAFETY: Reuse cannot override revoked second-page rights.
            let refused = unsafe { engine.step(&mut arena.pause().unwrap(), &plan) };
            assert!(matches!(refused, Err(NativeDispatchError::Memory(_))));
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, 4);
            assert_eq!(engine.usage().cache_reuses, 3);
            arena
                .protect(data + PAGE, PAGE, NativePermissions::READ)
                .unwrap();
            // SAFETY: Same pending call, with actual full-span rights restored.
            let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }.unwrap();
            assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
            assert_eq!(*engine.state(), expected(current, form, 9, true, &first));
            assert_eq!(engine.usage().native_entries, 5);
            assert_eq!(engine.usage().cache_reuses, 4);
        });
    }
}

#[test]
fn minmax_rip_sib_and_register_dependency_splits_use_current_guest_addresses() {
    if VectorMode::detect().features().is_none() {
        return;
    }
    for form in span_forms() {
        for addressing in 0..3 {
            let instruction = encoding(form, 9, true);
            let mut code = instruction[..4].to_vec();
            if addressing == 0 {
                code.push(0x0d); // RIP-relative source, high destination XMM9.
                let displacement =
                    i32::try_from((DATA_OFF + 128) as i64 - (ENTRY + 9) as i64).unwrap();
                code.extend(displacement.to_le_bytes());
            } else if addressing == 1 {
                code.extend([0x4c, 0x8b, 16]); // [RBX+RCX*4+16].
            } else {
                code = vec![0x48, 0x89, 0xcb]; // MOV RBX,RCX must split before the dependent read.
                code.extend(instruction);
            }
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, state| {
                let target = source.source_plan().modules()[0].load_bias() + DATA_OFF + 128;
                let (left, right) = vectors(form, 5);
                arena
                    .pause()
                    .unwrap()
                    .write(target, &right[..form.read()])
                    .unwrap();
                let old_base = match addressing {
                    1 => target - 28,
                    2 => target + PAGE,
                    _ => target,
                };
                let mut current = input(state, &left, &right, old_base, 0x1f80, true);
                current.gpr[RCX] = if addressing == 1 { 3 } else { target };
                let mut engine = NativeBlockStepper::new(
                    source,
                    &mut arena.pause().unwrap(),
                    current,
                    stack,
                    &Default::default(),
                )
                .unwrap();
                engine.machine.vector_mode = VectorMode::Avx {
                    avx2: false,
                    fma: false,
                };
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
                    assert!(!first.emits_float_min_max_memory());
                    // SAFETY: The first body only copies a GPR; no memory read.
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
                    assert!(second.emits_float_min_max_memory());
                    // SAFETY: Freshly observed address and live exact read span.
                    let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &second) }
                        .unwrap();
                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                } else {
                    // SAFETY: Original RIP/SIB source has the same proven owned target.
                    let stop = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &first) }
                        .unwrap();
                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                }
                assert_eq!(*engine.state(), expected(current, form, 9, true, &right));
                assert_eq!(
                    engine.usage().native_entries,
                    if addressing == 2 { 2 } else { 1 }
                );
                assert_eq!(engine.usage().memory_access_checks, 1);
                assert_eq!(engine.usage().memory_bytes_checked, form.read() as u64);
            });
        }
    }
}

#[test]
fn minmax_publishers_require_full_context_and_diagnostic_memory_checks_live_spans_and_ac() {
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
                TranslationPolicy::x64_sse2().with_native_float_min_max(),
                VectorMode::Avx {
                    avx2: false,
                    fma: false,
                },
            ),
            (policy(true), VectorMode::Sse2),
        ] {
            engine.machine.vector_mode = mode;
            let plan = plan_materialized_block(
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
                engine.publish(&arena.pause().unwrap(), &plan),
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
        assert_eq!(*engine.state(), state);
        assert_eq!(engine.usage().cache_publications, 0);
    });
    // This pure diagnostic preflight never enters AVX and is meaningful even
    // when the host cannot execute the optional native profile.
    for form in span_forms() {
        let mut code = encoding(form, 9, true).to_vec();
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
            let plan = pentax_codegen::plan_block(
                source.source_plan(),
                0,
                state.guest_pc,
                engine.placement(),
                &policy(true),
                &Default::default(),
            )
            .unwrap();
            assert!(plan.emits_float_min_max_memory());
            let data = source.source_plan().modules()[0].load_bias() + DATA_OFF;
            let _guard = DispatchGuard::acquire().unwrap();
            // SAFETY: Hold the actual process-wide guard; no native call is in flight.
            unsafe {
                (*CONTEXT.0.get()).gpr = state.gpr;
                (*CONTEXT.0.get()).gpr[RBX] = data + PAGE - form.read() as u64 + 1;
                (*CONTEXT.0.get()).rflags = 0x202 | (1 << 18);
            }
            assert_eq!(
                super::super::super::preflight_diagnostic_vector_memory(arena, &plan),
                Err(PlatformError::NativeDispatchFailed {
                    stage: "native-vector flags"
                })
            );
            // SAFETY: Same exclusive guard and artificial diagnostic context.
            unsafe {
                (*CONTEXT.0.get()).rflags = 0x202;
            }
            assert!(super::super::super::preflight_diagnostic_vector_memory(arena, &plan).is_err());
            // SAFETY: Move the whole span back into the owned readable page.
            unsafe {
                (*CONTEXT.0.get()).gpr[RBX] = data + PAGE - form.read() as u64;
            }
            assert_eq!(
                super::super::super::preflight_diagnostic_vector_memory(arena, &plan),
                Ok(())
            );
            assert_eq!(engine.usage().native_entries, 0);
        });
    }
}
