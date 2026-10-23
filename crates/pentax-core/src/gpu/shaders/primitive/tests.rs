// SPDX-License-Identifier: GPL-2.0-only
//! Original literal primitive pairs, selective access, alias order and refusal.
use super::*;
use crate::gpu::driver::{Driver, Function, Limits};
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};
fn backing() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, 0x10000, NativePermissions::READ_WRITE)
        .unwrap();
    (owner, base)
}
fn words(memory: &PausedNativeMemory<'_>, address: u64, count: usize) -> Vec<u32> {
    let mut bytes = vec![0; count * 4];
    memory.read(address, &mut bytes).unwrap();
    bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn setup(
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    gs_stages: u32,
    hs_stages: u32,
) -> [u64; 6] {
    for (shader, specials, kind) in [
        (base + 0x100, base + 0x400, 2),
        (base + 0x200, base + 0x500, 3),
    ] {
        memory.write(shader + 40, &specials.to_le_bytes()).unwrap();
        memory.write(shader + 90, &[kind]).unwrap();
    }
    for (specials, stages, output, ge, user) in [
        (base + 0x400, gs_stages, 0x1122, 0x4567, 0x89ab),
        (base + 0x500, hs_stages, 0x3344, 0x7654, 0xabcd),
    ] {
        let pairs = [
            0x25bu32, ge, 0x2d5, stages, 0, 0, 0, 0, 0x29b, output, 0x262, user,
        ];
        memory
            .write(
                specials,
                &pairs
                    .into_iter()
                    .flat_map(u32::to_le_bytes)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }
    memory.write(base + 0x1000 - 8, &[0xa5; 64]).unwrap();
    [base + 0x1000, base + 0x1010, 0, base + 0x100, 4, 0]
}
#[test]
fn primitive_helper_literal_mapping_and_original_gs_choice_precede_optional_hs_stage_merge() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    for (primitive, expected) in [
        (0, 2),
        (1, 0),
        (2, 1),
        (3, 1),
        (4, 2),
        (5, 2),
        (6, 2),
        (7, 3),
        (9, 2),
        (10, 1),
        (11, 1),
        (12, 2),
        (13, 2),
        (17, 4),
        (18, 1),
        (19, 2),
        (20, 2),
        (21, 2),
        (u32::MAX, 2),
    ] {
        let mut args = setup(&mut memory, base, 0x8040_0000, 0);
        args[4] = 0xffff_ffff_0000_0000 | u64::from(primitive);
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            driver.submit(&mut memory, Function::AgcCreatePrimState, args),
            Ok(0)
        );
        assert_eq!(
            words(&memory, args[0], 4),
            [0x2d5, 0x8040_0000, 0x29b, expected]
        );
        assert_eq!(
            words(&memory, args[1], 6),
            [0x25b, 0x4567, 0x262, 0x89ab, 0x242, primitive]
        );
        assert_eq!(words(&memory, args[0] - 8, 2), [0xa5a5_a5a5; 2]);
        assert_eq!(words(&memory, args[1] + 24, 2), [0xa5a5_a5a5; 2]);
    }
    for (gs, hs, expected) in [
        (0, 0, 0x3344),
        (0x20, 0, 0x1122),
        (0, 0x20, 2),
        (0x20, 0x20, 0x1122),
    ] {
        let mut args = setup(&mut memory, base, 0x8040_0000 | gs, 0x4000_0000 | hs);
        args[2] = base + 0x200;
        let mut driver = Driver::new(Default::default());
        assert_eq!(
            driver.submit(&mut memory, Function::AgcCreatePrimState, args),
            Ok(0)
        );
        assert_eq!(
            words(&memory, args[0], 4),
            [0x2d5, 0xc040_0000 | gs | hs, 0x29b, expected]
        );
        assert_eq!(
            words(&memory, args[1], 6),
            [0x25b, 0x4567, 0x262, 0xabcd, 0x242, 4]
        );
    }
}
#[test]
fn primitive_helper_only_reads_required_branch_fields_and_defined_empty_output_skips_shaders() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcCreatePrimState,
            [0, 0, u64::MAX, u64::MAX, u64::MAX, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcCreatePrimState,
            [base, 0, u64::MAX, 0, 4, 0]
        ),
        Err(Failure::Shader(ShaderFailure::NullArgument))
    );
    let mut args = setup(&mut memory, base, 0, 0);
    args[1] = 0;
    memory.write(base + 0x400, &0u32.to_le_bytes()).unwrap();
    memory.write(base + 0x428, &0u32.to_le_bytes()).unwrap();
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Ok(0)
    );
    let mut args = setup(&mut memory, base, 0, 0);
    args[0] = 0;
    memory.write(base + 0x408, &0u32.to_le_bytes()).unwrap();
    memory.write(base + 0x420, &0u32.to_le_bytes()).unwrap();
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Ok(0)
    );
}
#[test]
fn primitive_helper_same_address_aliases_observe_current_staged_pairs_not_presnapshots() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut args = setup(&mut memory, base, 0x0040_0020, 0);
    args[0] = base + 0x420;
    args[1] = 0;
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Ok(0)
    );
    assert_eq!(
        words(&memory, args[0], 4),
        [0x2d5, 0x0040_0020, 0x2d5, 0x0040_0020]
    );
    let mut args = setup(&mut memory, base, 0x0040_0000, 0x80);
    args[0] = base + 0x508;
    args[1] = 0;
    args[2] = base + 0x200;
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Ok(0)
    );
    assert_eq!(
        words(&memory, args[0], 4),
        [0x2d5, 0x0040_0000, 0x29b, 0x3344]
    );
    let mut args = setup(&mut memory, base, 0, 0);
    args[0] = 0;
    args[1] = base + 0x428;
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Ok(0)
    );
    assert_eq!(
        words(&memory, args[1], 6),
        [0x25b, 0x4567, 0x25b, 0x4567, 0x242, 4]
    );
    // CX0 aliases the Shader.specials pointer field itself. The next explicit
    // gs->specials expression must read the new shadow bytes, not a captured
    // old pointer. They name unmapped memory, so the checked helper refuses
    // atomically rather than returning success through the original specials.
    let mut args = setup(&mut memory, base, 0x20, 0);
    args[0] = args[3] + 40;
    args[1] = 0;
    let before = words(&memory, args[3], 24);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Err(Failure::Memory)
    );
    assert_eq!(words(&memory, args[3], 24), before);
}
#[test]
fn primitive_helper_late_invalid_tag_readonly_output_and_work_refusals_publish_no_staged_prefix() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let args = setup(&mut memory, base, 0, 0);
    memory.write(base + 0x428, &0u32.to_le_bytes()).unwrap();
    let before = words(&memory, args[0] - 8, 16);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Err(Failure::Shader(ShaderFailure::Registers))
    );
    assert_eq!(words(&memory, args[0] - 8, 16), before);
    let mut args = setup(&mut memory, base, 0, 0);
    args[1] = base + 0xc000;
    memory.write(args[1], &[0x96; 24]).unwrap();
    let before = words(&memory, args[0] - 8, 16);
    owner
        .protect(base + 0xc000, 0x4000, NativePermissions::READ)
        .unwrap();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Err(Failure::Memory)
    );
    assert_eq!(words(&memory, args[0] - 8, 16), before);
    assert_eq!(words(&memory, args[1], 6), [0x9696_9696; 6]);
    let mut driver = Driver::new(Limits {
        max_shader_call_work: Some(1),
        ..Default::default()
    });
    assert!(matches!(
        driver.submit(&mut memory, Function::AgcCreatePrimState, args),
        Err(Failure::Limit)
    ));
    assert_eq!(words(&memory, args[0] - 8, 16), before);
    // Distinct virtual views of the source GSout pair alias the first output.
    // The established Editor policy refuses these instead of inventing a
    // shadow read from a different virtual address with shared physical bytes.
    memory
        .apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: 0x10000,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    for address in [base + 0x10000, base + 0x20000] {
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapPhysical {
                        address,
                        size: 0x10000,
                        physical: 0,
                        permissions: NativePermissions::READ_WRITE,
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    memory
        .write(base + 0x100 + 40, &(base + 0x20000).to_le_bytes())
        .unwrap();
    memory
        .write(
            base + 0x20000,
            &[0x25bu32, 0, 0x2d5, 0x20, 0, 0, 0, 0, 0x29b, 7, 0x262, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        )
        .unwrap();
    let before = words(&memory, base + 0x20000, 12);
    let mut driver = Driver::new(Default::default());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::AgcCreatePrimState,
            [base + 0x10020, 0, 0, base + 0x100, 4, 0]
        ),
        Err(Failure::Shader(ShaderFailure::PhysicalAlias))
    );
    assert_eq!(words(&memory, base + 0x20000, 12), before);
}
