// SPDX-License-Identifier: GPL-2.0-only
//! Original register FMA3 programs; actual AVX/FMA and checked-software paths.
use super::*;
use pentax_codegen::{CpuFeatures, CpuidFeature};

const ALIASES: [[u8; 3]; 3] = [[9, 13, 14], [13, 13, 14], [14, 13, 14]];
const PATTERNS: usize = 12;

#[derive(Clone, Copy)]
struct Form {
    order: u16,
    operation: u8, // ADD, SUB, NMADD, NMSUB, ADDSUB, SUBADD.
    width: usize,
    element: usize,
    scalar: bool,
    opcode: u8,
}
impl Form {
    fn signs(self, lane: usize) -> (bool, bool) {
        match self.operation {
            0 => (false, false),
            1 => (false, true),
            2 => (true, false),
            3 => (true, true),
            4 => (false, lane.is_multiple_of(2)),
            5 => (false, !lane.is_multiple_of(2)),
            _ => unreachable!(),
        }
    }
    fn operands(self, prior: u64, second: u64, third: u64) -> [u64; 3] {
        match self.order {
            132 => [prior, third, second],
            213 => [second, prior, third],
            231 => [second, third, prior],
            _ => unreachable!(),
        }
    }
}
fn forms() -> Vec<Form> {
    let mut forms = Vec::new();
    for (order, high) in [(132, 0x90), (213, 0xa0), (231, 0xb0)] {
        for (operation, low) in [(0, 8), (1, 10), (2, 12), (3, 14), (4, 6), (5, 7)] {
            for element in [4, 8] {
                for width in [16, 32] {
                    forms.push(Form {
                        order,
                        operation,
                        width,
                        element,
                        scalar: false,
                        opcode: high | low,
                    });
                }
                if operation < 4 {
                    forms.push(Form {
                        order,
                        operation,
                        width: 16,
                        element,
                        scalar: true,
                        opcode: high | (low + 1),
                    });
                }
            }
        }
    }
    assert_eq!(forms.len(), 96);
    forms
}
fn policy(native: bool) -> TranslationPolicy {
    let base = TranslationPolicy::new(
        CpuFeatures::x64_sse2()
            .with(CpuidFeature::AVX)
            .with(CpuidFeature::FMA),
    )
    .with_native_ymm_registers();
    if native {
        base.with_native_fma_registers()
    } else {
        base
    }
}
fn encoding(form: Form, registers: [u8; 3]) -> [u8; 5] {
    let [destination, second, third] = registers;
    [
        0xc4,
        2 | ((!destination & 8) << 4) | 0x40 | ((!third & 8) << 2),
        (u8::from(form.element == 8) << 7)
            | ((!second & 15) << 3)
            | (u8::from(form.width == 32) << 2)
            | 1,
        form.opcode,
        0xc0 | ((destination & 7) << 3) | (third & 7),
    ]
}
fn host_controls() -> (u32, u16) {
    let mut mxcsr = 0_u32;
    let mut x87 = 0_u16;
    // SAFETY: Mandatory control stores to exact local objects; no mutation.
    unsafe {
        std::arch::asm!("stmxcsr [{mx}]", "fnstcw [{cw}]",
            mx = in(reg) &mut mxcsr, cw = in(reg) &mut x87,
            options(nostack, preserves_flags));
    }
    (mxcsr, x87)
}
fn controls() -> Vec<u32> {
    (0..16_u32)
        .map(|bits| 0x1f80 | ((bits & 3) << 13) | ((bits & 4) << 4) | ((bits & 8) << 12))
        .filter(|&control| mxcsr_control::valid(control))
        .collect()
}
fn values(double: bool, pattern: usize) -> [u64; 3] {
    let (one, two, three, four, normal, infinity, sign, quiet) = if double {
        (
            0x3ff0_0000_0000_0000,
            0x4000_0000_0000_0000,
            0x4008_0000_0000_0000,
            0x4010_0000_0000_0000,
            0x0010_0000_0000_0000,
            0x7ff0_0000_0000_0000,
            0x8000_0000_0000_0000,
            0x0008_0000_0000_0000,
        )
    } else {
        (
            0x3f80_0000,
            0x4000_0000,
            0x4040_0000,
            0x4080_0000,
            0x0080_0000,
            0x7f80_0000,
            0x8000_0000,
            0x0040_0000,
        )
    };
    // Physical old DEST, SRC2, SRC3. Orders permute these only after capture.
    match pattern % PATTERNS {
        0 => [two, three, four],
        1 => [one + 1, one - 2, sign | one], // Exact fused cancellation.
        2 => [
            sign | infinity | quiet | 0x22,
            infinity | 0x11,
            infinity | quiet | 0x33,
        ],
        3 => [one, sign | infinity | quiet | 0x22, infinity | 0x33],
        4 => [one, two, infinity | 0x33],
        5 => [infinity, 0, infinity | quiet | 0x55],
        6 => [infinity, 0, 0],
        7 => [1, one, 0],
        8 => [normal + 1, one - 2, 0], // Precision/unbounded rounding normal.
        9 => [normal, one - 1, 0],     // Tiny before final subnormal rounding.
        10 => [infinity - 1, two, 0],
        11 => [sign, one, sign],
        _ => unreachable!(),
    }
}
fn physical_values(form: Form, pattern: usize) -> [u64; 3] {
    let values = values(form.element == 8, pattern);
    if pattern.is_multiple_of(PATTERNS) {
        return values;
    }
    // Reuse the directed 213 triple in every algebraic order, so denormal,
    // overflow, precision-boundary and NaN cases cover all three permutations.
    // Pattern zero deliberately keeps identical physical 2/3/4 operands for
    // the independent per-order 11/10/14 and alternating-sign literal oracles.
    match form.order {
        132 => [values[1], values[2], values[0]],
        213 => values,
        231 => [values[2], values[1], values[0]],
        _ => unreachable!(),
    }
}
fn vector(state: &NativeCpuState, register: u8) -> [u8; 32] {
    let at = register as usize * 16;
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&state.xmm[at..at + 16]);
    bytes[16..].copy_from_slice(&state.ymm_upper[at..at + 16]);
    bytes
}
fn set_vector(state: &mut NativeCpuState, register: u8, bytes: &[u8; 32]) {
    let at = register as usize * 16;
    state.xmm[at..at + 16].copy_from_slice(&bytes[..16]);
    state.ymm_upper[at..at + 16].copy_from_slice(&bytes[16..]);
}
fn lane(bytes: &[u8; 32], offset: usize, size: usize) -> u64 {
    let mut raw = [0; 8];
    raw[..size].copy_from_slice(&bytes[offset..offset + size]);
    u64::from_le_bytes(raw)
}
fn input(
    mut state: NativeCpuState,
    form: Form,
    registers: [u8; 3],
    pattern: usize,
    control: u32,
) -> NativeCpuState {
    for (index, value) in state.gpr.iter_mut().enumerate() {
        if index != RSP {
            *value = 0x1020_3040_5060_7080 ^ index as u64;
        }
    }
    state.gpr[RAX] = 42;
    state.rflags = 0x202 | 0x8d5 | 0x400;
    state.mxcsr = control | if pattern == 11 { 0x24 } else { 0 };
    for (index, byte) in state.xmm.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(3);
    }
    for (index, byte) in state.ymm_upper.iter_mut().enumerate() {
        *byte = !(index as u8).wrapping_mul(13);
    }
    // Last write establishes old DEST when an operand aliases it. Scalar
    // inactive lanes contain raw signaling NaNs and must never be evaluated.
    for operand in [1, 2, 0] {
        let mut bytes = vector(&state, registers[operand]);
        for offset in (0..form.width).step_by(form.element) {
            let index = offset / form.element;
            let bits = if form.scalar && index != 0 {
                (if form.element == 8 {
                    0x7ff0_0000_0000_0000
                } else {
                    0x7f80_0000
                }) | 0x100
                    | ((operand as u64) << 4)
                    | index as u64
            } else {
                physical_values(form, pattern + index)[operand]
            };
            bytes[offset..offset + form.element]
                .copy_from_slice(&bits.to_le_bytes()[..form.element]);
        }
        set_vector(&mut state, registers[operand], &bytes);
    }
    state
}
fn apply(mut state: NativeCpuState, form: Form, registers: [u8; 3]) -> NativeCpuState {
    let [prior, second, third] = registers.map(|register| vector(&state, register));
    let mut output = if form.scalar { prior } else { [0; 32] };
    if form.width == 16 {
        output[16..].fill(0);
    }
    let width = if form.scalar {
        form.element
    } else {
        form.width
    };
    let double = form.element == 8;
    let (infinity, fraction, quiet, sign) = if double {
        (
            0x7ff0_0000_0000_0000,
            0x000f_ffff_ffff_ffff,
            0x0008_0000_0000_0000,
            1_u64 << 63,
        )
    } else {
        (0x7f80_0000, 0x007f_ffff, 0x0040_0000, 1_u64 << 31)
    };
    for (index, offset) in (0..width).step_by(form.element).enumerate() {
        let physical = [
            lane(&prior, offset, form.element),
            lane(&second, offset, form.element),
            lane(&third, offset, form.element),
        ];
        let [x, y, z] = form.operands(physical[0], physical[1], physical[2]);
        let (negative_product, negative_addend) = form.signs(index);
        let (bits, control) = fused_float::calculate(
            double,
            x,
            y,
            z,
            negative_product,
            negative_addend,
            state.mxcsr,
        )
        .unwrap();
        let nan = |value: u64| value & infinity == infinity && value & fraction != 0;
        // Independent literal NaN priority: first algebraic source wins;
        // arithmetic negation does not change its sign or selected payload.
        if let Some(value) = [x, y, z].into_iter().find(|&value| nan(value)) {
            assert_eq!(bits, value | quiet);
            let signaling = [x, y, z]
                .into_iter()
                .any(|value| nan(value) && value & quiet == 0);
            assert_eq!(control & 1, (state.mxcsr & 1) | u32::from(signaling));
        }
        if physical == values(double, 0) {
            let (sum, difference) = match (double, form.order) {
                (false, 132) => (0x4130_0000, 0x40a0_0000), // 2*4 +/- 3.
                (false, 213) => (0x4120_0000, 0x4000_0000), // 3*2 +/- 4.
                (false, 231) => (0x4160_0000, 0x4120_0000), // 3*4 +/- 2.
                (true, 132) => (0x4026_0000_0000_0000, 0x4014_0000_0000_0000),
                (true, 213) => (0x4024_0000_0000_0000, 0x4000_0000_0000_0000),
                (true, 231) => (0x402c_0000_0000_0000, 0x4024_0000_0000_0000),
                _ => unreachable!(),
            };
            let expected = if negative_product == negative_addend {
                sum
            } else {
                difference
            } | if negative_product { sign } else { 0 };
            assert_eq!(bits, expected); // Includes both alternating-lane families.
        }
        if form.operation == 0 && physical == physical_values(form, 1) {
            assert_eq!(
                bits,
                if double {
                    0xb970_0000_0000_0000
                } else {
                    0xa880_0000
                }
            );
        }
        output[offset..offset + form.element].copy_from_slice(&bits.to_le_bytes()[..form.element]);
        state.mxcsr = control;
    }
    set_vector(&mut state, registers[0], &output);
    state
}
fn returned(mut state: NativeCpuState) -> NativeCpuState {
    state.gpr[RSP] += 8;
    state.guest_pc = STOP;
    state
}

#[test]
fn all_fma_register_forms_match_software_full_state_controls_nan_order_and_aliases() {
    if !VectorMode::detect().supports(CpuidFeature::FMA) {
        println!(
            "native_fma_registers_available=false native_fma_registers_cases=0 native_fma_registers_mode=software"
        );
        return;
    }
    let controls = controls();
    assert!(!controls.is_empty());
    let mut cases = 0_u64;
    let mut codes = Vec::new();
    for form in forms() {
        let code: Vec<_> = ALIASES
            .into_iter()
            .flat_map(|registers| encoding(form, registers).into_iter().chain([0xc3]))
            .collect();
        with_source(Some(&code), |source, arena, stack, state| {
            let initial = input(state, form, ALIASES[0], 0, controls[0]);
            let mut engine = NativeBlockStepper::new(
                source,
                &mut arena.pause().unwrap(),
                initial,
                stack,
                &Default::default(),
            )
            .unwrap();
            // Actual AVX/FMA is established above; AVX2 is deliberately absent.
            engine.machine.vector_mode = VectorMode::Avx {
                avx2: false,
                fma: true,
            };
            let mut calls = 0_u64;
            for (alias, registers) in ALIASES.into_iter().enumerate() {
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
                assert_eq!(software.instructions()[0].emitted_len(), 0);
                if alias == 0 {
                    let decoded = native.instructions()[0].instruction().code();
                    assert!(!codes.contains(&decoded));
                    codes.push(decoded);
                    let modeled = fused_float::form(decoded).unwrap();
                    assert_eq!(
                        (
                            modeled.order,
                            modeled.vector_bytes,
                            modeled.element_bytes,
                            modeled.scalar
                        ),
                        (form.order, form.width, form.element, form.scalar)
                    );
                }
                for pattern in 0..PATTERNS {
                    for &control in &controls {
                        let mut current = input(state, form, registers, pattern, control);
                        current.guest_pc = pc;
                        let expected = returned(apply(current, form, registers));
                        for use_native in [true, false] {
                            if calls != 0 {
                                engine
                                    .begin_call(&mut arena.pause().unwrap(), current)
                                    .unwrap();
                            }
                            let before = engine.usage();
                            let host = host_controls();
                            // SAFETY: Original exact same-owner sources/stack,
                            // masked controls and independently admitted AVX/FMA.
                            unsafe {
                                if use_native {
                                    let stop = engine
                                        .step_checked(&mut arena.pause().unwrap(), &native)
                                        .unwrap();
                                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                                } else {
                                    let stop = engine
                                        .step_checked(&mut arena.pause().unwrap(), &software)
                                        .unwrap();
                                    assert_eq!(stop, NativeBlockStop::Resume { guest_pc: pc + 5 });
                                    let stop = engine
                                        .step_checked(&mut arena.pause().unwrap(), &ret)
                                        .unwrap();
                                    assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                                }
                            }
                            assert_eq!(
                                *engine.state(),
                                expected,
                                "order={} op={} width={} element={} scalar={} alias={alias} pattern={pattern} control={control:04x}",
                                form.order,
                                form.operation,
                                form.width,
                                form.element,
                                form.scalar
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
                            assert_eq!(after.vector_bytes, before.vector_bytes);
                            assert_eq!(
                                after.blocks - before.blocks,
                                if use_native { 1 } else { 2 }
                            );
                            calls += 1;
                        }
                        cases += 1;
                    }
                }
            }
            assert_eq!(engine.usage().native_entries, calls / 2);
            assert_eq!(engine.usage().native_instructions, calls / 2);
            assert_eq!(engine.usage().vector_operations, calls / 2);
        });
    }
    assert_eq!(codes.len(), 96);
    assert_eq!(
        cases,
        96 * ALIASES.len() as u64 * PATTERNS as u64 * controls.len() as u64
    );
    println!(
        "native_fma_registers_available=true native_fma_registers_cases={cases} native_fma_registers_forms=96 native_fma_registers_mode=avx-fma"
    );
}

#[test]
fn fma_register_profile_refuses_modes_controls_source_quotas_and_later_bad_memory() {
    for form in forms()
        .into_iter()
        .filter(|form| form.order == 231 && form.element == 4 && form.width == 32)
    {
        for case in 0..10 {
            if case >= 6 && !VectorMode::detect().supports(CpuidFeature::FMA) {
                continue;
            }
            let mut code = encoding(form, ALIASES[0]).to_vec();
            if case == 8 {
                code.extend([0x8b, 0x00]);
            } // Native MOV EAX,[RAX].
            code.push(0xc3);
            with_source(Some(&code), |source, arena, stack, state| {
                let mut current = input(state, form, ALIASES[0], 2, 0x1f80);
                current.gpr[RAX] = source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE;
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
                assert!(plan.instructions()[0].emitted_len() > 0);
                if case == 8 {
                    assert!(plan.instructions()[1].emitted_len() > 0);
                }
                match case {
                    0 => engine.machine.vector_mode = VectorMode::Sse2,
                    1 => {
                        engine.machine.vector_mode = VectorMode::Avx {
                            avx2: true,
                            fma: false,
                        }
                    }
                    2 => engine.machine.state.mxcsr &= !0x80,
                    3 => engine.machine.state.mxcsr |= 1 << 16,
                    4 => engine.machine.limits.max_work_units = 0,
                    5 => engine.machine.usage.native_instructions = u64::MAX,
                    6 => arena
                        .protect(
                            current.guest_pc & !(PAGE - 1),
                            PAGE,
                            NativePermissions::READ_WRITE,
                        )
                        .unwrap(),
                    7 => {
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
                let result = if case == 9 {
                    let mut other =
                        NativeAddressSpace::reserve_any(Default::default(), 65_536).unwrap();
                    // SAFETY: Deliberate wrong-owner refusal before entry.
                    unsafe { engine.step_checked(&mut other.pause().unwrap(), &plan) }
                } else {
                    // SAFETY: Original controlled invalid profile/source/access.
                    unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) }
                };
                match case {
                    0..=3 => assert_eq!(result, Err(NativeDispatchError::InvalidState)),
                    4 | 5 => assert!(matches!(result, Err(NativeDispatchError::Budget(_)))),
                    6 => assert_eq!(result, Err(NativeDispatchError::WritableSource)),
                    7 => assert_eq!(result, Err(NativeDispatchError::SourceMismatch)),
                    8 => assert!(matches!(result, Err(NativeDispatchError::Memory(_)))),
                    9 => assert_eq!(result, Err(NativeDispatchError::WrongOwner)),
                    _ => unreachable!(),
                }
                assert_eq!(*engine.state(), before);
                assert_eq!(host_controls(), host);
                assert_eq!(engine.usage().native_entries, 0);
                assert_eq!(engine.usage().cache_publications, 0);
                assert_eq!(engine.usage().vector_operations, 0);
            });
        }
    }
}

#[test]
fn both_publishers_and_feature_cache_require_the_independent_fma_register_profile() {
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
        for (requested, mode, feature) in [
            (
                TranslationPolicy::x64_sse2().with_native_fma_registers(),
                VectorMode::Avx {
                    avx2: false,
                    fma: true,
                },
                "native YMM context",
            ),
            (policy(true), VectorMode::Sse2, "native YMM context"),
            (
                policy(true),
                VectorMode::Avx {
                    avx2: true,
                    fma: false,
                },
                "native FMA register profile",
            ),
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
            assert!(
                matches!(super::super::super::publish_block_with_mode(&mut engine.machine.cache, arena, &diagnostic, state.guest_pc, mode), Err(PlatformError::UnsupportedHostFeature { feature: actual }) if actual == feature)
            );
        }
        assert_eq!(*engine.state(), state);
        assert_eq!(engine.usage().native_entries, 0);
        assert_eq!(engine.usage().cache_publications, 0);
    });
    for form in forms() {
        let mut code = encoding(form, ALIASES[0]).to_vec();
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
            for fma in [true, false, true, false] {
                for avx2 in [false, true] {
                    let mode = VectorMode::Avx { avx2, fma };
                    assert_eq!(mode.supports_admission(plan.native_admission()), fma);
                    assert_eq!(mode.features().unwrap().contains(CpuidFeature::FMA), fma);
                }
            }
            assert!(!VectorMode::Sse2.supports_admission(plan.native_admission()));
        });
    }
}

#[test]
fn coalesced_fma_bodies_and_excluded_memory_inputs_keep_exact_native_boundaries() {
    if !VectorMode::detect().supports(CpuidFeature::FMA) {
        return;
    }
    let forms = forms();
    let first = *forms
        .iter()
        .find(|form| {
            form.order == 231 && form.operation == 0 && form.element == 4 && form.width == 32
        })
        .unwrap();
    let second = *forms
        .iter()
        .find(|form| {
            form.order == 132 && form.operation == 4 && form.element == 4 && form.width == 32
        })
        .unwrap();
    let mut code = encoding(first, ALIASES[0]).to_vec();
    code.extend(encoding(second, ALIASES[0]));
    code.push(0xc3);
    for native in [false, true] {
        with_source(Some(&code), |source, arena, stack, state| {
            let current = input(state, first, ALIASES[0], 0, 0x1f80);
            let expected = returned(apply(apply(current, first, ALIASES[0]), second, ALIASES[0]));
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
                fma: true,
            };
            let selected = policy(native);
            let first_plan = plan_materialized_block(
                source,
                0,
                state.guest_pc,
                &code,
                engine.placement(),
                &selected,
                &Default::default(),
            )
            .unwrap();
            let second_plan = plan_materialized_block(
                source,
                0,
                state.guest_pc + 5,
                &code[5..],
                engine.placement(),
                &selected,
                &Default::default(),
            )
            .unwrap();
            let ret = plan_materialized_block(
                source,
                0,
                state.guest_pc + 10,
                &code[10..],
                engine.placement(),
                &selected,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                first_plan.native_admission().emitted_instructions(),
                if native { 2 } else { 0 }
            );
            for call in 0..64 {
                if call != 0 {
                    engine
                        .begin_call(&mut arena.pause().unwrap(), current)
                        .unwrap();
                }
                // SAFETY: Same bounded original program, actual AVX/FMA and
                // current full source/stack/control checks in both profiles.
                unsafe {
                    let stop = engine
                        .step_checked(&mut arena.pause().unwrap(), &first_plan)
                        .unwrap();
                    if native {
                        assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                    } else {
                        assert_eq!(
                            stop,
                            NativeBlockStop::Resume {
                                guest_pc: state.guest_pc + 5
                            }
                        );
                        let stop = engine
                            .step_checked(&mut arena.pause().unwrap(), &second_plan)
                            .unwrap();
                        assert_eq!(
                            stop,
                            NativeBlockStop::Resume {
                                guest_pc: state.guest_pc + 10
                            }
                        );
                        let stop = engine
                            .step_checked(&mut arena.pause().unwrap(), &ret)
                            .unwrap();
                        assert_eq!(stop, NativeBlockStop::Returned { value: 42 });
                    }
                }
                assert_eq!(*engine.state(), expected);
            }
            assert_eq!(engine.usage().blocks, if native { 64 } else { 192 });
            assert_eq!(engine.usage().native_entries, if native { 64 } else { 0 });
            assert_eq!(
                engine.usage().native_instructions,
                if native { 128 } else { 0 }
            );
            assert_eq!(
                engine.usage().vector_operations,
                if native { 0 } else { 128 }
            );
        });
    }
    for (scalar, width, element) in [(true, 16, 4), (true, 16, 8), (false, 16, 4), (false, 32, 4)] {
        let form = *forms
            .iter()
            .find(|form| {
                form.order == 231
                    && form.operation == 0
                    && form.scalar == scalar
                    && form.width == width
                    && form.element == element
            })
            .unwrap();
        let mut bytes = encoding(form, [9, 13, 0]);
        bytes[4] &= 0x3f; // Exact memory source [RAX], never native under this flag.
        let mut code = bytes.to_vec();
        code.push(0xc3);
        with_source(Some(&code), |source, arena, stack, state| {
            let mut current = input(state, form, ALIASES[0], 2, 0x1f80);
            let read = if scalar { element } else { width };
            current.gpr[RAX] =
                source.source_plan().modules()[0].load_bias() + DATA_OFF + PAGE - read as u64 + 1;
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
            assert_eq!(plan.instructions()[0].emitted_len(), 0);
            assert_eq!(
                plan.instructions()[0].instruction().memory_size().size(),
                read
            );
            // SAFETY: Original excluded memory form crosses the exact live
            // readable span by one byte; checked software must refuse atomically.
            let result = unsafe { engine.step_checked(&mut arena.pause().unwrap(), &plan) };
            assert!(matches!(result, Err(NativeDispatchError::Memory(_))));
            assert_eq!(*engine.state(), current);
            assert_eq!(engine.usage().native_entries, 0);
            assert_eq!(engine.usage().cache_publications, 0);
            assert_eq!(engine.usage().vector_operations, 0);
        });
    }
}
