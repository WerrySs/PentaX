// SPDX-License-Identifier: GPL-2.0-only
//! Original CPU/OS facts and optional real context round trips.

use super::*;
use std::cell::Cell;

#[test]
fn vector_mode_requires_cpu_os_and_xcr0_and_marshals_every_guest_half() {
    const REQUIRED: u32 = (1 << 26) | (1 << 27) | (1 << 28);
    for maximum in [0, 1, 7] {
        for missing in [0, 1 << 26, 1 << 27, 1 << 28] {
            for xcr0 in [0, 2, 4, 6, u64::MAX] {
                for avx2 in [false, true] {
                    let xgets = Cell::new(0);
                    let leaf7 = Cell::new(0);
                    let mode = VectorMode::detect_with(|leaf, subleaf| {
                        assert_eq!(subleaf, 0);
                        match leaf {
                            0 => [maximum, 0, 0, 0],
                            1 => { assert!(maximum >= 1); [0, 0, REQUIRED & !missing, 0] },
                            7 => { leaf7.set(leaf7.get() + 1); [0, if avx2 { 1 << 5 } else { 0 }, 0, 0] },
                            _ => panic!("unexpected fixed CPU capability leaf"),
                        }
                    }, || { xgets.set(xgets.get() + 1); xcr0 });
                    let cpu = maximum >= 1 && missing == 0;
                    let admitted = cpu && xcr0 & 6 == 6;
                    assert_eq!(xgets.get(), u32::from(cpu));
                    assert_eq!(leaf7.get(), u32::from(admitted && maximum >= 7));
                    assert_eq!(mode.supports(CpuidFeature::AVX), admitted);
                    assert_eq!(mode.supports(CpuidFeature::AVX2), admitted && maximum >= 7 && avx2);
                    assert_eq!(mode.features().is_some(), admitted);
                    if admitted {
                        assert_ne!(mode.trailer(), VectorMode::Sse2.trailer());
                        assert_ne!(mode.entry() as *const (), VectorMode::Sse2.entry() as *const ());
                    }
                }
            }
        }
    }
    let low = std::array::from_fn(|index| (index as u8).wrapping_mul(17).wrapping_add(7));
    let upper = std::array::from_fn(|index| (index as u8).wrapping_mul(31).wrapping_add(0x81));
    for mode in [VectorMode::Sse2, VectorMode::Avx { avx2: false }] {
        let mut context = GuestContext::zeroed();
        mode.load(&mut context, &low, &upper);
        let mut observed_low = [0; 256];
        let mut observed_upper = upper;
        mode.save(&context, &mut observed_low, &mut observed_upper);
        assert_eq!(observed_low, low);
        assert_eq!(observed_upper, upper);
        if mode.features().is_some() {
            for register in 0..16 {
                assert_eq!(&context.ymm[register * 32..register * 32 + 16], &low[register * 16..register * 16 + 16]);
                assert_eq!(&context.ymm[register * 32 + 16..register * 32 + 32], &upper[register * 16..register * 16 + 16]);
            }
        }
    }
}

#[test]
fn admitted_ymm_bridge_roundtrips_all_guest_halves_and_host_nonvolatile_control_state() {
    let mode = VectorMode::detect();
    if mode.features().is_none() {
        println!("native_ymm_context_available=false native_ymm_context_cases=0");
        return;
    }
    super::super::context_probe::verify_context_roundtrip_mode(mode).unwrap();
    println!("native_ymm_context_available=true native_ymm_context_cases=48");
}
