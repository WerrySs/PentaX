// SPDX-License-Identifier: GPL-2.0-only
//! Original pure session snapshots. No guest execution, file I/O or payload.

use super::*;
use crate::native_session::startup_profile::StartupProfile;
use pentax_codegen::{Decoder, DecoderOptions};
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn image() -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 1)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(20, 1_u32), (64, 1), (68, 5)] {
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

fn with_sources(check: impl FnOnce(&SymbolicLinkPlan<'_>, &SymbolicLinkPlan<'_>)) {
    let first = image();
    let second = image();
    let inputs = [
        ModuleInput::new(open_executable(&first).unwrap(), 0x100000),
        ModuleInput::new(open_executable(&second).unwrap(), 0x200000),
    ];
    let source = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let next = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    assert_ne!(source.identity(), next.identity());
    check(&source, &next);
}

fn sample(
    profile: &mut StartupProfile,
    source: &SymbolicLinkPlan<'_>,
    module: usize,
    pc: u64,
    is_main: bool,
) {
    let instruction = Decoder::with_ip(64, &[0x90], pc, DecoderOptions::NONE).decode();
    let instructions = [instruction];
    let pending = profile
        .blocks
        .prepare_records(
            super::super::Key {
                epoch: source.identity(),
                module,
                pc,
            },
            Some(instruction.code()),
            instructions.iter(),
            256,
            is_main,
        )
        .unwrap();
    profile.complete_block_sample(pending);
}

type Row = (u32, u32, u64, u64);
fn rows(bytes: &[u8]) -> (u32, Vec<Row>) {
    assert_eq!(&bytes[..8], b"PXSAMP01");
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    assert!(count <= 64 && flags <= 3);
    assert_eq!(bytes.len(), 16 + count * 24);
    assert!(bytes.len() <= 1552);
    let rows = bytes[16..]
        .chunks_exact(24)
        .map(|row| {
            (
                u32::from_le_bytes(row[..4].try_into().unwrap()),
                u32::from_le_bytes(row[4..8].try_into().unwrap()),
                u64::from_le_bytes(row[8..16].try_into().unwrap()),
                u64::from_le_bytes(row[16..24].try_into().unwrap()),
            )
        })
        .collect();
    (flags, rows)
}

#[test]
fn private_recent_rows_deduplicate_actual_roles_only_after_success_and_redact_data() {
    with_sources(|source, _| {
        let mut profile = StartupProfile::default();
        profile.begin_sample_epoch(source.identity());
        let ordinary = profile.final_sample_fields();
        profile.capture_private_samples(source);
        assert_eq!(profile.final_sample_fields(), ordinary); // Opt-out has no observer effect.
        assert!(profile.private_samples_bytes().is_none());
        profile.enable_private_samples();
        let work = profile.blocks.work;
        profile.enable_private_samples();
        assert_eq!(profile.blocks.work, work);
        for _ in 0..2 {
            sample(&mut profile, source, 0, 0x1002d8, true);
        }
        for _ in 0..3 {
            sample(&mut profile, source, 0, 0x1002d8, false);
        }
        sample(&mut profile, source, 1, 0x200240, true);
        // An admitted observation for a failed dispatch never reaches the ring.
        let instruction = Decoder::with_ip(64, &[0x90], 0x100280, DecoderOptions::NONE).decode();
        let instructions = [instruction];
        let pending = profile
            .blocks
            .prepare_records(
                super::super::Key {
                    epoch: source.identity(),
                    module: 0,
                    pc: 0x100280,
                },
                Some(instruction.code()),
                instructions.iter(),
                256,
                false,
            )
            .unwrap();
        let _ = pending;
        profile.capture_private_samples(source);
        let (flags, mut captured) = rows(profile.private_samples_bytes().unwrap());
        captured.sort_unstable();
        assert_eq!(flags, 0);
        assert_eq!(
            captured,
            [(0, 0, 0x2d8, 2), (0, 1, 0x2d8, 3), (1, 0, 0x240, 1)]
        );
        assert_eq!(
            (
                profile.blocks.completed,
                profile.blocks.recent,
                profile.blocks.distinct,
                profile.blocks.max_repeat
            ),
            (6, 6, 2, 5)
        );
        assert_eq!(
            (profile.blocks.main_context, profile.blocks.child_context),
            (3, 3)
        );
        for text in [
            format!("{profile:?}"),
            profile.protocol_fields(),
            profile.final_sample_fields(),
        ] {
            for forbidden in [
                "PXSAMP01",
                "SourceIdentity",
                "private_samples",
                "1002d8",
                "1049304",
                "module=",
                "offset=",
            ] {
                assert!(!text.contains(forbidden), "{text}");
            }
        }
    });
}

#[test]
fn private_capture_caps_rows_handles_eviction_and_requires_current_source_epoch() {
    with_sources(|source, next| {
        let mut profile = StartupProfile::default();
        profile.enable_private_samples();
        profile.begin_sample_epoch(source.identity());
        for offset in 0..64 {
            sample(&mut profile, source, 0, 0x100200 + offset, offset % 2 == 0);
        }
        profile.capture_private_samples(source);
        let (_, captured) = rows(profile.private_samples_bytes().unwrap());
        assert_eq!(captured.len(), 64);
        assert_eq!(profile.private_samples_bytes().unwrap().len(), 1552);
        assert!(captured.iter().all(|row| row.3 == 1));
        sample(&mut profile, source, 0, 0x100240, true);
        assert!(profile.private_samples_bytes().is_none());
        profile.capture_private_samples(source);
        let (_, captured) = rows(profile.private_samples_bytes().unwrap());
        assert_eq!(captured.len(), 64);
        assert!(!captured.iter().any(|row| row.2 == 0x200));
        assert!(captured.contains(&(0, 0, 0x240, 1)));
        profile.begin_sample_epoch(next.identity());
        assert!(profile.private_samples_bytes().is_none());
        sample(&mut profile, next, 0, 0x100240, false);
        profile.capture_private_samples(next);
        assert_eq!(
            rows(profile.private_samples_bytes().unwrap()),
            (0, vec![(0, 1, 0x240, 1)])
        );
        profile.capture_private_samples(source); // Same numeric addresses cannot bind another epoch.
        assert!(profile.private_samples_bytes().is_none());
    });
}

#[test]
fn private_capture_rejects_bad_geometry_and_work_refusal_never_disables_the_observer() {
    with_sources(|source, next| {
        for case in 0..5 {
            let mut profile = StartupProfile::default();
            profile.enable_private_samples();
            profile.begin_sample_epoch(source.identity());
            sample(&mut profile, source, 0, 0x100200, true);
            let entry = profile.blocks.entries[0].as_mut().unwrap();
            match case {
                0 => entry.key.pc = 0xffff,   // Checked subtraction, not wrapped offset.
                1 => entry.key.module = 2,    // No such module in this exact source.
                2 => entry.key.pc = 0x101100, // Mapped BSS is not initialized RX input.
                3 => entry.key.epoch = next.identity(),
                4 => entry.key.pc = u64::MAX,
                _ => unreachable!(),
            }
            profile.capture_private_samples(source);
            assert!(profile.private_samples_bytes().is_none());
            assert!(!profile.blocks.disabled && !profile.blocks.overflow);
            assert_eq!(profile.blocks.completed, 1);
        }
        for phase in 0..3 {
            let mut profile = StartupProfile::default();
            if phase == 0 {
                profile.blocks.maximum_work = 0;
            }
            profile.enable_private_samples();
            if phase != 0 {
                profile.begin_sample_epoch(source.identity());
                sample(&mut profile, source, 0, 0x100200, true);
                profile.blocks.maximum_work = profile.blocks.work + if phase == 1 { 0 } else { 3 };
                profile.capture_private_samples(source);
            }
            assert!(profile.private_samples_bytes().is_none());
            assert!(!profile.blocks.disabled && !profile.blocks.overflow);
            assert!(profile.blocks.work <= profile.blocks.maximum_work);
            // The optional artifact cannot poison later ordinary observations.
            profile.blocks.maximum_work = super::super::MAX_WORK;
            profile.begin_sample_epoch(source.identity());
            sample(&mut profile, source, 0, 0x100240, false);
            assert!(!profile.blocks.disabled);
        }
        let mut profile = StartupProfile::default();
        profile.enable_private_samples();
        profile.begin_sample_epoch(source.identity());
        sample(&mut profile, source, 0, 0x100200, true);
        let instruction = Decoder::with_ip(64, &[0x90], 0x100280, DecoderOptions::NONE).decode();
        let instructions = [instruction];
        assert!(
            profile
                .blocks
                .prepare_records(
                    super::super::Key {
                        epoch: source.identity(),
                        module: 0,
                        pc: 0x100280,
                    },
                    None,
                    instructions.iter(),
                    0,
                    false
                )
                .is_none()
        );
        assert!(profile.blocks.disabled);
        profile.capture_private_samples(source);
        assert_eq!(
            rows(profile.private_samples_bytes().unwrap()),
            (1, vec![(0, 0, 0x200, 1)])
        );
        assert!(profile.blocks.disabled);
    });
}
