// SPDX-License-Identifier: GPL-2.0-only
//! Original metadata-only observations. No native execution or guest reads.

use super::*;
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn image(flags: u32) -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 1)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(20, 1_u32), (64, 1), (68, flags)] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (24, 0x200_u64),
        (32, 64),
        (96, 4096),
        (104, 0x4000),
        (112, 0x4000),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x200] = 0xc3;
    bytes
}

fn with_sources(flags: u32, check: impl FnOnce(&SymbolicLinkPlan<'_>, &SymbolicLinkPlan<'_>)) {
    let first = image(flags);
    let second = image(5);
    let inputs = [
        ModuleInput::new(open_executable(&first).unwrap(), 0x100000),
        ModuleInput::new(open_executable(&second).unwrap(), 0x200000),
    ];
    let source = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let next = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    check(&source, &next);
}

fn word(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}

#[test]
fn private_thread_states_encode_actual_roles_registers_and_current_epoch_only() {
    with_sources(5, |source, next| {
        let mut main = NativeCpuState::new(0x1002d8, 0x300000);
        let mut child = NativeCpuState::new(0x200240, 0x400000);
        for (index, value) in main.gpr.iter_mut().enumerate() {
            *value = 0x1122_3344_5500_0000 | index as u64;
        }
        for (index, value) in child.gpr.iter_mut().enumerate() {
            *value = 0x8877_6655_4400_0000 | index as u64;
        }
        main.rflags = 0x247;
        child.rflags = 0x202;
        let before = (main, child);
        let states = [Some(&main), Some(&child)];
        let mut capture = PrivateThreadStates::default();
        capture.begin_epoch(source.identity());
        capture.capture(source, states);
        assert!(capture.bytes().is_none());
        assert_eq!(capture.work, 0);
        capture.enable();
        capture.begin_epoch(source.identity());
        capture.capture(source, states);
        let bytes = capture.bytes().unwrap();
        assert_eq!(bytes.len(), 336);
        assert_eq!(&bytes[..8], b"PXCTX001");
        assert_eq!(&bytes[8..12], &2_u32.to_le_bytes());
        assert_eq!(&bytes[12..16], &[0; 4]);
        for (role, (module, offset, state)) in [(0_u32, 0x2d8_u64, &main), (1, 0x240, &child)]
            .into_iter()
            .enumerate()
        {
            let at = HEADER + ROW * role;
            assert_eq!(&bytes[at..at + 4], &module.to_le_bytes());
            assert_eq!(&bytes[at + 4..at + 8], &(role as u32).to_le_bytes());
            assert_eq!(word(bytes, at + 8), offset);
            for (index, expected) in state.gpr.iter().enumerate() {
                assert_eq!(word(bytes, at + 16 + 8 * index), *expected);
            }
            assert_eq!(word(bytes, at + 144), state.rflags);
            assert_eq!(word(bytes, at + 152), 0);
        }
        assert_eq!((main, child), before);
        assert_eq!(
            format!("{capture:?}"),
            "PrivateThreadStates { payload: redacted }"
        );
        capture.begin_epoch(next.identity());
        assert!(capture.bytes().is_none());
        let work = capture.work;
        capture.capture(source, states);
        assert!(capture.bytes().is_none());
        assert_eq!(capture.work, work);
        capture.capture(next, [None, Some(&child)]);
        let bytes = capture.bytes().unwrap();
        assert_eq!(bytes.len(), 176);
        assert_eq!(&bytes[20..24], &1_u32.to_le_bytes());
    });
}

#[test]
fn private_thread_states_refuse_bss_writable_or_unmapped_pc_without_guessing() {
    for flags in [1, 5, 7] {
        with_sources(flags, |source, _| {
            let mut capture = PrivateThreadStates::default();
            capture.enable();
            capture.begin_epoch(source.identity());
            let mut cpu = NativeCpuState::new(0x100fff, 0x300000);
            capture.capture(source, [Some(&cpu), None]);
            assert_eq!(capture.bytes().is_some(), flags == 5);
            for pc in [0x101000, 0x103fff, 0x104000, 0xfffff, u64::MAX] {
                cpu.guest_pc = pc;
                capture.capture(source, [Some(&cpu), None]);
                assert!(capture.bytes().is_none());
            }
        });
    }
}

#[test]
fn private_thread_states_work_refusal_is_artifact_only_and_never_public_payload() {
    with_sources(5, |source, _| {
        let cpu = NativeCpuState::new(0x100200, 0x300000);
        let before = cpu;
        let mut progress = super::super::NativeSessionProgress::default();
        let public = progress.protocol_fields();
        let ordinary = progress.progress_fields();
        progress.private_thread_states.enable();
        progress
            .private_thread_states
            .begin_epoch(source.identity());
        progress
            .private_thread_states
            .capture(source, [Some(&cpu), None]);
        assert!(progress.private_thread_states_bytes().is_some());
        assert_eq!(progress.protocol_fields(), public);
        assert_eq!(progress.progress_fields(), ordinary);
        assert!(!format!("{progress:?}").contains("PXCTX001"));
        progress.private_thread_states.work = MAX_WORK;
        progress
            .private_thread_states
            .capture(source, [Some(&cpu), None]);
        assert!(progress.private_thread_states_bytes().is_none());
        assert_eq!(progress.private_thread_states.work, MAX_WORK);
        assert_eq!(cpu, before);
        assert_eq!(progress.protocol_fields(), public);
        assert_eq!(progress.progress_fields(), ordinary);
    });
}
