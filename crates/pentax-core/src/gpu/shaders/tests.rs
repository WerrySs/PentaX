// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn put(memory: &mut PausedNativeMemory<'_>, at: u64, value: u64) {
    memory.write(at, &value.to_le_bytes()).unwrap();
}
fn word(memory: &PausedNativeMemory<'_>, at: u64) -> u64 {
    let mut b = [0; 8];
    memory.read(at, &mut b).unwrap();
    u64::from_le_bytes(b)
}
fn data(memory: &PausedNativeMemory<'_>, at: u64, n: usize) -> Vec<u8> {
    let mut b = vec![0; n];
    memory.read(at, &mut b).unwrap();
    b
}
fn owner() -> (NativeAddressSpace, u64) {
    let mut memory = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
    let base = memory.reserved_ranges().next().unwrap().address;
    memory
        .map_at(
            base,
            0x8000,
            NativePermissions::READ_WRITE.with_gpu(true, true),
        )
        .unwrap();
    (memory, base)
}
fn seed(
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    header: u64,
    kind: u8,
    extent: usize,
) -> [u64; 6] {
    let user = base + 0x300;
    let regs = base + 0x600;
    let mut h = [0xa5; 96];
    h[..4].copy_from_slice(&0x3433_3231_u32.to_le_bytes());
    h[4..8].copy_from_slice(&0x18_u32.to_le_bytes());
    for (field, target) in [
        (8, user),
        (16, 0),
        (24, base + 0x280),
        (32, regs),
        (40, base + 0x700),
        (48, base + 0x800),
        (56, base + 0x900),
    ] {
        let encoded = if target == 0 {
            0
        } else {
            target.wrapping_sub(header + field)
        };
        h[field as usize..field as usize + 8].copy_from_slice(&encoded.to_le_bytes());
    }
    h[64..68].copy_from_slice(&96_u32.to_le_bytes());
    h[68..72].copy_from_slice(&128_u32.to_le_bytes());
    h[80..84].copy_from_slice(&2_u32.to_le_bytes());
    h[84..86].copy_from_slice(&3_u16.to_le_bytes());
    h[90] = kind;
    h[91] = 2;
    h[92] = 2;
    memory.write(header, &h[..extent]).unwrap();
    let mut u = [0x5a; 56];
    for i in 0..5 {
        u[i * 8..i * 8 + 8]
            .copy_from_slice(&(base + 0xa00 + i as u64 * 32 - (user + i as u64 * 8)).to_le_bytes());
    }
    memory.write(user, &u).unwrap();
    let low = program_register(kind).unwrap_or(0x20c);
    put(memory, regs, (2_u64 << 32) | u64::from(low));
    put(
        memory,
        regs + 8,
        (0xa5a5_a500_u64 << 32) | u64::from(low + 1),
    );
    put(memory, base + 0x200, 0x5a5a_5a5a_5a5a_5a5a);
    [base + 0x200, header, base + 0x4000, 0, 0, 0]
}

#[test]
fn create_shader_models_every_source_stage_relative_pointer_and_retained_mapping() {
    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    for kind in [0, 1, 2, 3, 4, 5, 6, 7, 8, 255] {
        let args = seed(&mut memory, base, base + 0x400, kind, 96);
        let mut state = State::default();
        let mut work = 0;
        assert_eq!(
            state.create(&mut memory, args, 8, &mut work, u64::MAX),
            Ok(0)
        );
        assert_eq!(word(&memory, args[0]), args[1]);
        assert_eq!(word(&memory, args[1] + 8), base + 0x300);
        assert_eq!(word(&memory, args[1] + 16), args[2]);
        for (field, target) in [
            (24, base + 0x280),
            (32, base + 0x600),
            (40, base + 0x700),
            (48, base + 0x800),
            (56, base + 0x900),
        ] {
            assert_eq!(word(&memory, args[1] + field), target);
        }
        for i in 0..5 {
            assert_eq!(word(&memory, base + 0x300 + i * 8), base + 0xa00 + i * 32);
        }
        assert_eq!(data(&memory, args[1] + 93, 3), [0xa5; 3]);
        if program_register(kind).is_some() {
            let patched = args[2] + 512;
            assert_eq!(
                word(&memory, base + 0x600) >> 32,
                (patched >> 8) & 0xffff_ffff
            );
            assert_eq!(
                word(&memory, base + 0x608) >> 32,
                0xa5a5_a500 | ((patched >> 40) & 255)
            );
        } else {
            assert_eq!(word(&memory, base + 0x600) >> 32, 2);
        }
        let mapped = state.mapping(memory.owner_id(), args[2]).unwrap();
        assert_eq!(mapped.kind, kind);
        assert_eq!(mapped.code_bytes, 128);
        assert_eq!(mapped.scratch_dwords, 3);
        assert_eq!(mapped.user_data, base + 0x300);
        assert_eq!(mapped.input_semantics, base + 0x800);
        assert_eq!(mapped.input_count, 2);
        assert_eq!(state.entries.len(), 1);
        assert_eq!(state.generation, 2);
    }
}

#[test]
fn guest_program_error_keeps_source_header_and_mapping_effects_but_not_destination() {
    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    for bad_high in [false, true] {
        let mut args = seed(&mut memory, base, base + 0x400, 0, 96);
        if bad_high {
            memory
                .write(base + 0x608, &0x555_u32.to_le_bytes())
                .unwrap();
        } else {
            memory
                .write(base + 0x600, &0x555_u32.to_le_bytes())
                .unwrap();
        }
        args[0] = u64::MAX; // Source never accesses dst after this guest error.
        let mut state = State::default();
        let mut work = 0;
        assert_eq!(
            state.create(&mut memory, args, 8, &mut work, u64::MAX),
            Ok(INVALID_PROGRAM)
        );
        assert_eq!(word(&memory, args[1] + 16), args[2]);
        assert_eq!(word(&memory, args[1] + 8), base + 0x300);
        assert!(state.mapping(memory.owner_id(), args[2]).is_some());
        assert_eq!(word(&memory, base + 0x200), 0x5a5a_5a5a_5a5a_5a5a);
        assert_eq!(word(&memory, base + 0x600) >> 32, 2);
    }
}

#[test]
fn shader_admission_memory_and_source_aborts_leave_outputs_and_mapping_unchanged() {
    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    for case in 0..8 {
        let mut args = seed(&mut memory, base, base + 0x400, 0, 96);
        let mut state = State::default();
        let mut work = 0;
        let expected = match case {
            0 => {
                memory.write(args[1], &0_u32.to_le_bytes()).unwrap();
                Failure::Shader(ShaderFailure::Header)
            }
            1 => {
                memory.write(args[1] + 4, &0_u32.to_le_bytes()).unwrap();
                Failure::Shader(ShaderFailure::Version)
            }
            2 => {
                args[2] += 1;
                Failure::Shader(ShaderFailure::CodeAddress)
            }
            3 => {
                args[0] = base + 0x10000;
                Failure::Memory
            }
            4 => Failure::Limit,
            5 => Failure::Limit,
            6 => {
                put(
                    &mut memory,
                    args[1] + 32,
                    (base + 0x10000).wrapping_sub(args[1] + 32),
                );
                Failure::Memory
            }
            _ => {
                state.generation = u64::MAX;
                Failure::Limit
            }
        };
        let before = data(&memory, base, 0x8000);
        assert_eq!(
            state.create(
                &mut memory,
                args,
                if case == 5 { 0 } else { 8 },
                &mut work,
                if case == 4 { 0 } else { u64::MAX }
            ),
            Err(expected)
        );
        assert!(before == data(&memory, base, 0x8000));
        assert!(state.entries.is_empty());
    }
}

#[test]
fn shader_same_address_aliases_follow_source_order_and_exact_field_boundaries() {
    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    let mut args = seed(&mut memory, base, base + 0x400, 0, 96);
    args[0] = args[1] + 16;
    let mut state = State::default();
    let mut work = 0;
    assert_eq!(
        state.create(&mut memory, args, 8, &mut work, u64::MAX),
        Ok(0)
    );
    assert_eq!(word(&memory, args[1] + 16), args[1]);
    assert_eq!(
        state.mapping(memory.owner_id(), args[2]).unwrap().code,
        args[2]
    );
    let args = seed(&mut memory, base, base + 0x400, 4, 96);
    put(&mut memory, args[1] + 8, 16); // userdata begins at the cx pointer field.
    let mut state = State::default();
    let mut work = 0;
    assert_eq!(
        state.create(&mut memory, args, 8, &mut work, u64::MAX),
        Ok(0)
    );
    assert_eq!(word(&memory, args[1] + 24), base + 0x280 + args[1] + 24);
    assert_eq!(
        state
            .mapping(memory.owner_id(), args[2])
            .unwrap()
            .input_semantics,
        base + 0x800 + args[1] + 48
    );
    space.unmap(base + 0x4000, 0x4000).unwrap();
    let mut memory = space.pause().unwrap();
    let args = seed(&mut memory, base, base + 0x4000 - 93, 0, 93);
    let mut state = State::default();
    let mut work = 0;
    assert_eq!(
        state.create(&mut memory, args, 8, &mut work, u64::MAX),
        Ok(0)
    );
}

#[test]
fn shader_mapping_replacement_generation_and_owner_identity_are_retained() {
    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    let mut state = State::default();
    let mut work = 0;
    for kind in [0, 1] {
        let args = seed(&mut memory, base, base + 0x400, kind, 96);
        assert_eq!(
            state.create(&mut memory, args, 1, &mut work, u64::MAX),
            Ok(0)
        );
        assert_eq!(state.entries.len(), 1);
        assert_eq!(
            state.mapping(memory.owner_id(), args[2]).unwrap().kind,
            kind
        );
    }
    assert_eq!(state.generation, 3);
    let (mut other, other_base) = owner();
    let mut other_memory = other.pause().unwrap();
    let args = seed(&mut other_memory, other_base, other_base + 0x400, 0, 96);
    assert!(
        state
            .mapping(other_memory.owner_id(), base + 0x4000)
            .is_none()
    );
    assert_eq!(
        state.create(&mut other_memory, args, 1, &mut work, u64::MAX),
        Err(Failure::Memory)
    );
}

#[test]
fn shader_physical_aliases_are_explicit_refusals_and_read_only_outputs_do_not_publish() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    let limits = NativeTransactionLimits::default();
    let mut memory = space.pause().unwrap();
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: 0x4000,
                },
                writes: &[],
            },
            &limits,
        )
        .unwrap();
    for address in [base, base + 0x10000] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x4000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE,
                    },
                    writes: &[],
                },
                &limits,
            )
            .unwrap();
    }
    let args = seed(&mut memory, base, base + 0x400, 4, 96);
    put(
        &mut memory,
        args[1] + 8,
        (base + 0x10000 + 0x418).wrapping_sub(args[1] + 8),
    );
    let before = data(&memory, base, 0x4000);
    let mut state = State::default();
    let mut work = 0;
    assert_eq!(
        state.create(&mut memory, args, 8, &mut work, u64::MAX),
        Err(Failure::Shader(ShaderFailure::PhysicalAlias))
    );
    assert!(before == data(&memory, base, 0x4000));
    assert!(state.entries.is_empty());

    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    let mut args = seed(&mut memory, base, base + 0x400, 0, 96);
    args[0] = base + 0x4000;
    let before = data(&memory, base, 0x4000);
    space
        .protect(base + 0x4000, 0x4000, NativePermissions::READ)
        .unwrap();
    let mut memory = space.pause().unwrap();
    let mut state = State::default();
    let mut work = 0;
    assert_eq!(
        state.create(&mut memory, args, 8, &mut work, u64::MAX),
        Err(Failure::Memory)
    );
    assert!(before == data(&memory, base, 0x4000));
    assert!(state.entries.is_empty());
}

#[test]
fn registered_shader_extent_is_consumed_by_actual_compute_preparation_and_gpu_writeback() {
    use crate::gpu::driver::{Driver, Function};
    let (mut space, base) = owner();
    let mut memory = space.pause().unwrap();
    for declared in [4_u32, 20] {
        let args = seed(&mut memory, base, base + 0x400, 0, 96);
        put(&mut memory, base + 0x600, 0x20c); // Relative program offset zero.
        put(&mut memory, args[1] + 8, 0);
        put(&mut memory, args[1] + 48, 0);
        memory.write(args[1] + 80, &[0; 6]).unwrap();
        memory.write(args[1] + 68, &declared.to_le_bytes()).unwrap();
        let shader = [
            (0x25_u32 << 25) | (1 << 17) | 4,
            (0x1a << 25) | (2 << 17) | 130,
            0xe070_1000,
            (128 << 24) | (1 << 8) | 2,
            0xbf81_0000,
        ];
        let bytes: Vec<u8> = shader.into_iter().flat_map(u32::to_le_bytes).collect();
        memory.write(args[2], &bytes).unwrap();
        let output = base + 0x2000;
        memory.write(output, &[0xa5; 48]).unwrap();
        let mut commands = Vec::new();
        let mut set = |first: u32, values: &[u32]| {
            commands.push(0xc000_7602 | (values.len() as u32) << 16);
            commands.push(first);
            commands.extend(values);
        };
        set(0x207, &[8, 1, 1]);
        set(0x20c, &[(args[2] >> 8) as u32, (args[2] >> 40) as u32]);
        set(0x212, &[0x2c_0000, 10]);
        set(0x228, &[0]);
        set(0x240, &[output as u32, (output >> 32) as u32, 48, 0, 17]);
        commands.extend([0xc003_1502, 1, 1, 1, 0x8005]);
        let packet = base + 0x1200;
        let stream = base + 0x1000;
        memory
            .write(
                stream,
                &commands
                    .iter()
                    .copied()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        put(&mut memory, packet, stream);
        put(&mut memory, packet + 8, commands.len() as u64);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            driver.submit(&mut memory, Function::AgcCreateShader, args),
            Ok(0)
        );
        let result = driver.submit(&mut memory, Function::Acb, [0x20, packet, 0, 0, 0, 0]);
        if declared == 4 {
            assert!(matches!(result, Err(Failure::Preparation(_))));
            assert_eq!(data(&memory, output, 48), [0xa5; 48]);
        } else {
            assert_eq!(result, Ok(0));
            let actual = data(&memory, output, 48);
            for i in 0..8 {
                assert_eq!(
                    u32::from_le_bytes(actual[i * 4..i * 4 + 4].try_into().unwrap()),
                    17 + i as u32
                );
            }
            assert_eq!(&actual[32..], &[0xa5; 16]);
            assert_eq!(driver.usage().written_bytes, 48);
        }
    }
}
