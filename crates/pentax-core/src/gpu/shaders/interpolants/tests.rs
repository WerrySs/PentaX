// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::gpu::driver::{Driver, Function, Limits};
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};
const PAGE: u64 = 16384;
const GS: u64 = 0x100;
const PS: u64 = 0x200;
const GS_SEM: u64 = 0x400;
const PS_SEM: u64 = 0x800;
const OUTPUT: u64 = 0x1000;

fn backing() -> (NativeAddressSpace, u64) {
    let mut owner = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    owner
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    (owner, base)
}
fn word(memory: &PausedNativeMemory<'_>, address: u64) -> u32 {
    let mut bytes = [0; 4];
    memory.read(address, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
fn bytes(memory: &PausedNativeMemory<'_>, base: u64) -> Vec<u8> {
    let mut bytes = vec![0; PAGE as usize];
    memory.read(base, &mut bytes).unwrap();
    bytes
}
fn setup(memory: &mut PausedNativeMemory<'_>, base: u64) -> [u64; 6] {
    memory
        .write(base + GS + 56, &(base + GS_SEM).to_le_bytes())
        .unwrap();
    memory.write(base + GS + 86, &2_u16.to_le_bytes()).unwrap();
    memory
        .write(base + PS + 48, &(base + PS_SEM).to_le_bytes())
        .unwrap();
    memory.write(base + PS + 80, &3_u32.to_le_bytes()).unwrap();
    for (i, value) in [0x0020_1f37_u32, 0x0030_0737].into_iter().enumerate() {
        memory
            .write(base + GS_SEM + i as u64 * 4, &value.to_le_bytes())
            .unwrap();
    }
    for (i, value) in [0x6020_0137_u32, 0x3000_0099, 0x3140_0037]
        .into_iter()
        .enumerate()
    {
        memory
            .write(base + PS_SEM + i as u64 * 4, &value.to_le_bytes())
            .unwrap();
    }
    memory.write(base + OUTPUT - 8, &[0xa5; 280]).unwrap();
    [base + OUTPUT, base + GS, base + PS, 0, 0, 0]
}

#[test]
fn interpolant_scalar_flags_match_pinned_cpp_oracles_for_all_modes_and_defaults() {
    // Expected digests come from the unmodified pinned C++ helper/functions,
    // compiled independently with synthetic descriptors, not from Rust output.
    for (mapping2, expected) in [
        (false, 0xe694_da7c_6736_5325_u64),
        (true, 0x6ac1_1e27_f8da_cb25),
    ] {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut count = 0;
        for mode in 0..4_u32 {
            for default in 0..4 {
                for high in 0..4 {
                    for flat in 0..2 {
                        for custom in 0..2 {
                            for linear in 0..2 {
                                for present in 0..2 {
                                    for gs_mode in 0..4 {
                                        for mapping in [0, 1, 7, 15, 16, 31, 32, 255] {
                                            let source = 0x0e0f_aa37
                                                | (mode << 20)
                                                | (flat << 22)
                                                | (linear << 23)
                                                | (custom << 24)
                                                | (default << 28)
                                                | (high << 30);
                                            let mask = 0x37 | (mapping << 8) | (gs_mode << 20);
                                            let output = (present != 0).then_some(mask);
                                            let value = if mapping2 {
                                                second(source, output)
                                            } else {
                                                original(source, output)
                                            };
                                            hash = (hash ^ u64::from(value))
                                                .wrapping_mul(0x100000001b3);
                                            count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(count, 32768);
        assert_eq!(hash, expected);
    }
}

#[test]
fn interpolants_keep_first_match_f16_profiles_missing_defaults_and_identity_tail() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    for (function, first) in [
        (Function::AgcCreateInterpolantMapping, 0x0228_023f),
        (Function::AgcCreateInterpolantMapping2, 0x0228_011f),
    ] {
        let args = setup(&mut memory, base);
        let before_gs = word(&memory, base + GS_SEM);
        let mut driver = Driver::new(Default::default());
        assert_eq!(driver.submit(&mut memory, function, args), Ok(0));
        for (i, value) in [first, 0x320, 0x73f].into_iter().enumerate() {
            assert_eq!(
                word(&memory, base + OUTPUT + i as u64 * 8),
                0x191 + i as u32
            );
            assert_eq!(word(&memory, base + OUTPUT + i as u64 * 8 + 4), value);
        }
        for index in 3..32_u32 {
            assert_eq!(
                word(&memory, base + OUTPUT + u64::from(index) * 8),
                0x191 + index
            );
            assert_eq!(
                word(&memory, base + OUTPUT + u64::from(index) * 8 + 4),
                index
            );
        }
        assert_eq!(word(&memory, base + OUTPUT - 4), 0xa5a5_a5a5);
        assert_eq!(word(&memory, base + OUTPUT + 256), 0xa5a5_a5a5);
        assert_eq!(word(&memory, base + GS_SEM), before_gs);
        assert_eq!(driver.usage().dispatches, 0);
        // Null/empty PS ignores GS and its own unused pointer entirely.
        for pixel in [0, base + PS] {
            memory.write(base + PS + 80, &0_u32.to_le_bytes()).unwrap();
            memory
                .write(base + PS + 48, &u64::MAX.to_le_bytes())
                .unwrap();
            assert_eq!(
                driver.submit(
                    &mut memory,
                    function,
                    [base + OUTPUT, u64::MAX, pixel, 0, 0, 0]
                ),
                Ok(0)
            );
            for index in 0..32_u32 {
                assert_eq!(
                    word(&memory, base + OUTPUT + u64::from(index) * 8 + 4),
                    index
                );
            }
        }
    }
}

#[test]
fn interpolants_observe_staged_aliases_and_refuse_faults_or_work_before_publication() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    for (function, aliased_second) in [
        (Function::AgcCreateInterpolantMapping, 0x0218_0020),
        (Function::AgcCreateInterpolantMapping2, 0x0208_0020),
    ] {
        let mut args = setup(&mut memory, base);
        memory.write(base + PS + 80, &2_u32.to_le_bytes()).unwrap();
        args[0] = base + PS_SEM;
        let mut driver = Driver::new(Default::default());
        assert_eq!(driver.submit(&mut memory, function, args), Ok(0));
        assert_eq!(word(&memory, base + PS_SEM + 12), aliased_second);
        // A rewritten source count is rechecked on the next iteration.
        args = setup(&mut memory, base);
        args[0] = base + PS + 80;
        let before = bytes(&memory, base);
        assert_eq!(
            driver.submit(&mut memory, function, args),
            Err(Failure::Limit)
        );
        assert_eq!(bytes(&memory, base), before);
        args = setup(&mut memory, base);
        memory
            .write(base + PS + 48, &(base + PAGE - 4).to_le_bytes())
            .unwrap();
        memory
            .write(base + PAGE - 4, &0x37_u32.to_le_bytes())
            .unwrap();
        let before = bytes(&memory, base);
        assert_eq!(
            driver.submit(&mut memory, function, args),
            Err(Failure::Memory)
        );
        assert_eq!(bytes(&memory, base), before);
        args = setup(&mut memory, base);
        args[0] = base + PAGE - 255;
        let before = bytes(&memory, base);
        assert_eq!(
            driver.submit(&mut memory, function, args),
            Err(Failure::Memory)
        );
        assert_eq!(bytes(&memory, base), before);
        let mut limited = Driver::new(Limits {
            max_preparation_work: 1,
            ..Default::default()
        });
        assert_eq!(
            limited.submit(&mut memory, function, args),
            Err(Failure::Limit)
        );
        assert_eq!(bytes(&memory, base), before);
        assert_eq!(
            driver.submit(&mut memory, function, [0; 6]),
            Err(Failure::Shader(ShaderFailure::NullArgument))
        );
    }
}

#[test]
fn interpolants_preserve_source_counts_above_32_without_clamping_or_unbounded_staging() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let args = setup(&mut memory, base);
    memory.write(base + GS + 86, &0_u16.to_le_bytes()).unwrap();
    memory.write(base + GS + 56, &0_u64.to_le_bytes()).unwrap();
    memory.write(base + PS_SEM, &[0; 256 * 4]).unwrap();
    for function in [
        Function::AgcCreateInterpolantMapping,
        Function::AgcCreateInterpolantMapping2,
    ] {
        let mut driver = Driver::new(Default::default());
        for count in [33_u32, 256] {
            memory.write(base + PS + 80, &count.to_le_bytes()).unwrap();
            assert_eq!(driver.submit(&mut memory, function, args), Ok(0));
            assert_eq!(
                word(&memory, base + OUTPUT + u64::from(count - 1) * 8 + 4),
                0x20
            );
        }
        memory
            .write(base + PS + 80, &257_u32.to_le_bytes())
            .unwrap();
        let before = bytes(&memory, base);
        assert_eq!(
            driver.submit(&mut memory, function, args),
            Err(Failure::Limit)
        );
        assert_eq!(bytes(&memory, base), before);
    }
}
