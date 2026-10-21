// SPDX-License-Identifier: GPL-2.0-only
//! Original pure observations; no native entry, guest reads or clock sampling.
use super::super::StartupProfile;
use super::*;
use pentax_codegen::{
    BlockLimits, CachePlacement, Decoder, DecoderOptions, TranslationPolicy,
    declare_materialized_source, plan_materialized_block,
};
use pentax_linker::{ModuleInput, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

fn original_image(code: &[u8]) -> Vec<u8> {
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
    bytes[0x200..0x200 + code.len()].copy_from_slice(code);
    bytes
}
fn identity() -> SourceIdentity {
    let bytes = original_image(&[0xc3]);
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    plan_symbolic(&inputs, &SymbolicLimits::default())
        .unwrap()
        .identity()
}
fn with_block(code: &[u8], check: impl FnOnce(SourceIdentity, &MaterializedBlockPlan<'_, '_, '_>)) {
    let bytes = original_image(code);
    let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), 0x100000)];
    let source = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let declaration = declare_materialized_source(&source, &[], &BlockLimits::default()).unwrap();
    let block = plan_materialized_block(
        &declaration,
        0,
        0x100200,
        code,
        CachePlacement {
            address: 0x1000_0000,
            capacity: 0x4000,
        },
        &TranslationPolicy::x64_sse2().with_independent_memory(),
        &BlockLimits::default(),
    )
    .unwrap();
    check(source.identity(), &block);
}
fn instruction(bytes: &[u8]) -> Instruction {
    let decoded = Decoder::with_ip(64, bytes, 0, DecoderOptions::NONE).decode();
    assert!(!decoded.is_invalid());
    decoded
}
fn sample(observer: &mut BlockSamples, epoch: SourceIdentity, module: usize, pc: u64) {
    let instructions = [instruction(&[0xc5, 0xd8, 0x59, 0xdd])];
    let pending = observer
        .prepare_records(
            Key { epoch, module, pc },
            Some(instructions[0].code()),
            instructions.iter(),
            256,
            true,
        )
        .unwrap();
    observer.complete(pending);
}

#[test]
fn recent_exact_keys_distinguish_repetition_breadth_eviction_and_source_epochs_without_exports() {
    let epoch = identity();
    let next = identity();
    assert_ne!(epoch, next);
    let mut observer = BlockSamples::default();
    observer.begin_epoch(epoch);
    for _ in 0..64 {
        sample(&mut observer, epoch, 0, 0xfedc_ba98_7654_3210);
    }
    assert_eq!(
        (
            observer.completed,
            observer.recent,
            observer.distinct,
            observer.max_repeat
        ),
        (64, 64, 1, 64)
    );
    // Same numeric PC in another module is a different exact block.
    sample(&mut observer, epoch, 1, 0xfedc_ba98_7654_3210);
    assert_eq!((observer.distinct, observer.max_repeat), (2, 63));
    for index in 0..64 {
        sample(&mut observer, epoch, 0, 0x1000 + index);
    }
    assert_eq!(
        (observer.recent, observer.distinct, observer.max_repeat),
        (64, 64, 1)
    );
    for index in 0..64 {
        sample(&mut observer, epoch, 0, 0x8000 + index % 2);
    }
    assert_eq!(
        (observer.recent, observer.distinct, observer.max_repeat),
        (64, 2, 32)
    );
    let completed = observer.completed;
    observer.begin_epoch(next);
    assert_eq!(
        (
            observer.completed,
            observer.recent,
            observer.distinct,
            observer.epoch_changes
        ),
        (completed, 0, 0, 1)
    );
    sample(&mut observer, next, 0, 0x8000);
    assert_eq!(
        (observer.recent, observer.distinct, observer.max_repeat),
        (1, 1, 1)
    );
    let debug = format!("{observer:?}");
    let fields = observer.protocol_fields();
    for forbidden in [
        "SourceIdentity",
        "fedc",
        "18364758544493064720",
        "module",
        "pc=",
        "0x8000",
    ] {
        assert!(!debug.contains(forbidden), "{debug}");
        assert!(!fields.contains(forbidden), "{fields}");
    }
    assert!(StartupProfile::sampled(true, 4093));
    assert!(!StartupProfile::sampled(true, 4096));
    assert!(!StartupProfile::sampled(false, 4093));
}

#[test]
fn all_twelve_fp_forms_count_planned_register_and_memory_occurrences_only_after_completion() {
    let epoch = identity();
    let mut observer = BlockSamples::default();
    observer.begin_epoch(epoch);
    let mut instructions = Vec::new();
    for opcode in [0x58, 0x5c, 0x59] {
        for double in [false, true] {
            for wide in [false, true] {
                for memory in [false, true] {
                    instructions.push(instruction(&[
                        0xc5,
                        0xd8 | u8::from(double) | if wide { 4 } else { 0 },
                        opcode,
                        if memory { 0x18 } else { 0xdd },
                    ]));
                }
            }
        }
    }
    // Legacy ADDPS, scalar VADDSS, EVEX VADDPS and FMA are not candidate aliases.
    for bytes in [
        &[0x0f, 0x58, 0xdd][..],
        &[0xc5, 0xda, 0x58, 0xdd][..],
        &[0x62, 0xf1, 0x5c, 0x08, 0x58, 0xdd][..],
        &[0xc4, 0xe2, 0x59, 0x98, 0xdd][..],
    ] {
        let decoded = instruction(bytes);
        assert!(candidate(&decoded).is_none());
        instructions.push(decoded);
    }
    let key = Key {
        epoch,
        module: 0,
        pc: 0x1234,
    };
    let pending = observer
        .prepare_records(key, Some(FORMS[0]), instructions.iter(), 256, false)
        .unwrap();
    assert_eq!((observer.completed, observer.records), (0, 0));
    assert_eq!(observer.counts, [[0; 2]; 12]);
    assert_eq!(
        (
            observer.exit_codes_retained,
            observer.main_context,
            observer.child_context
        ),
        (0, 0, 0)
    );
    // A failed dispatch would discard this receipt instead of calling complete.
    let _ = pending;
    let pending = observer
        .prepare_records(key, Some(FORMS[0]), instructions.iter(), 256, true)
        .unwrap();
    observer.complete(pending);
    assert_eq!((observer.completed, observer.records), (1, 28));
    assert_eq!(observer.counts, [[1; 2]; 12]);
    assert_eq!((observer.main_context, observer.child_context), (1, 0));
    let mut malformed = instructions[0];
    malformed.set_op0_register(Register::YMM3);
    assert!(candidate(&malformed).is_none());
    let mut prefixed = instructions[0];
    prefixed.set_segment_prefix(Register::FS);
    assert!(candidate(&prefixed).is_none());
}

#[test]
fn optional_work_record_and_counter_limits_disable_without_publication_and_final_fields_are_bounded()
 {
    let epoch = identity();
    let instructions = [instruction(&[0xc5, 0xd8, 0x59, 0xdd])];
    let key = Key {
        epoch,
        module: 0,
        pc: 0x1234,
    };
    for case in 0..9 {
        let mut observer = BlockSamples::default();
        observer.begin_epoch(epoch);
        match case {
            0 => observer.maximum_work = observer.work + FIXED_WORK + RECORD_WORK - 1,
            1 => {}
            2 => observer.completed = u64::MAX,
            3 => observer.counts[8][0] = u64::MAX,
            4 => {
                observer.exits[0] = Some((FORMS[8], u64::MAX));
                observer.exit_codes_retained = 1;
            }
            5 => {
                for (slot, code) in observer.exits.iter_mut().zip(Code::values()) {
                    *slot = Some((code, 1));
                }
                observer.exit_codes_retained = 64;
                observer.exit_overflow = u64::MAX;
            }
            6 => observer.exit_no_instruction = u64::MAX,
            7 => observer.main_context = u64::MAX,
            8 => observer.child_context = u64::MAX,
            _ => unreachable!(),
        }
        let completed = observer.completed;
        let counts = observer.counts;
        let exits = observer.exits;
        let contexts = (observer.main_context, observer.child_context);
        let planned = if case == 6 {
            None
        } else if case == 5 {
            Code::values().nth(64)
        } else {
            Some(FORMS[8])
        };
        assert!(
            observer
                .prepare_records(
                    key,
                    planned,
                    instructions.iter(),
                    if case == 1 { 0 } else { 256 },
                    case != 8,
                )
                .is_none()
        );
        assert!(observer.disabled);
        assert_eq!(observer.overflow, case >= 2);
        assert_eq!((observer.completed, observer.recent), (completed, 0));
        assert_eq!(observer.counts, counts);
        assert_eq!(observer.exits, exits);
        assert_eq!((observer.main_context, observer.child_context), contexts);
        assert!(
            observer
                .prepare_records(key, None, instructions.iter(), 256, true)
                .is_none()
        );
    }
    let longest = Code::values()
        .max_by_key(|code| format!("{code:?}").len())
        .unwrap();
    let observer = BlockSamples {
        planned: Some(longest),
        counts: [[u64::MAX; 2]; 12],
        completed: u64::MAX,
        records: u64::MAX,
        work: u64::MAX,
        maximum_work: u64::MAX,
        epoch_changes: u64::MAX,
        recent: 64,
        distinct: 64,
        max_repeat: 64,
        // Repeating the longest catalogue label in all slots conservatively
        // bounds serialization even though live histogram Codes are unique.
        exits: [Some((longest, u64::MAX)); EXIT_CAPACITY],
        exit_codes_retained: 64,
        exit_overflow: u64::MAX,
        exit_no_instruction: u64::MAX,
        main_context: u64::MAX,
        child_context: u64::MAX,
        self_target_branches: u64::MAX,
        self_target_native_register_blocks: u64::MAX,
        self_target_native_register_instructions: u64::MAX,
        disabled: true,
        overflow: true,
        ..Default::default()
    };
    let fields = observer.protocol_fields();
    assert!(fields.len() <= 3072, "{}", fields.len());
    assert_eq!(fields.matches("sample_fp_forms=").count(), 1);
    assert!(!fields.contains('\n'));
    // New observer fields are final-only; old progress text remains unchanged.
    let progress = crate::native_session::NativeSessionProgress::default();
    assert!(!progress.progress_fields().contains("sample_completed="));
    assert!(!progress.progress_fields().contains("scheduling_threads="));
    assert!(!progress.progress_fields().contains("sample_exit_top="));
    assert!(progress.protocol_fields().contains("sample_completed=0"));
    assert!(progress.protocol_fields().contains("scheduling_threads=0"));
    for name in [
        "sample_self_target_branches",
        "sample_self_target_native_register_blocks",
        "sample_self_target_native_register_instructions",
    ] {
        assert!(!progress.progress_fields().contains(name));
        assert!(progress.protocol_fields().contains(&format!("{name}=0")));
        assert_eq!(fields.matches(&format!("{name}=")).count(), 1);
    }
    for name in ["graphics_function_top", "graphics_function_last"] {
        assert!(!progress.progress_fields().contains(name));
        assert!(progress.protocol_fields().contains(&format!("{name}=none")));
    }
    for name in [
        "libc_builtin_attempted_memcpy",
        "libc_builtin_attempted_memmove",
        "libc_builtin_attempted_memset",
        "libc_builtin_completed_memcpy",
        "libc_builtin_completed_memmove",
        "libc_builtin_completed_memset",
        "libc_builtin_bound_entries",
        "graphics_shader_create_ok",
        "graphics_shader_create_rejected",
        "graphics_shader_create_failed",
        "graphics_shader_fuse_size_ok",
        "graphics_shader_fuse_size_rejected",
        "graphics_shader_fuse_size_failed",
        "graphics_shader_fuse_ok",
        "graphics_shader_fuse_rejected",
        "graphics_shader_fuse_failed",
        "scheduling_pending_equeue_waits",
        "video_vblank_subscriptions",
        "video_registered_buffers",
        "video_open_ports",
        "graphics_calls",
        "graphics_command_buffers",
        "graphics_draw_attempts",
        "graphics_dispatch_attempts",
        "graphics_written_bytes",
        "video_flip_reserved",
        "video_flip_recording_with_frame",
        "video_flip_ready_with_frame",
        "video_flip_presenting",
        "video_flip_acknowledged",
    ] {
        assert!(!progress.progress_fields().contains(name));
        assert!(progress.protocol_fields().contains(&format!("{name}=0")));
        assert_eq!(
            progress
                .protocol_fields()
                .matches(&format!("{name}="))
                .count(),
            1
        );
    }
    assert!(
        progress
            .protocol_fields()
            .contains("sample_exit_capacity=64")
    );
}

#[test]
fn completed_self_target_samples_require_real_emitted_memory_free_rsp_preserving_bodies() {
    // Literal original byte programs end in a Jcc back to their exact start,
    // except the final nonself/control cases. No code is executed by this test.
    for (code, self_target, native_instructions) in [
        (&[0x83, 0xc0, 1, 0x83, 0xf8, 2, 0x75, 0xf8][..], true, 2),
        (&[0x8b, 0x00, 0x85, 0xc0, 0x75, 0xfa][..], true, 0), // memory poll
        (&[0x48, 0x87, 0xe0, 0x75, 0xfb][..], true, 0),       // XCHG RAX,RSP
        (&[0x83, 0xc4, 8, 0x75, 0xfb][..], true, 0),          // ESP alias write
        (&[0x50, 0x75, 0xfd][..], true, 0),                   // implicit stack write
        (&[0x58, 0x75, 0xfd][..], true, 0),                   // implicit stack read
        (&[0xc9, 0x75, 0xfd][..], true, 0),                   // LEAVE implicit memory/RSP
        (&[0xf7, 0xf1, 0x75, 0xfc][..], true, 0),             // DIV ECX
        (&[0xf7, 0xf9, 0x75, 0xfc][..], true, 0),             // IDIV ECX
        (&[0x0f, 0xae, 0xe8, 0x75, 0xfb][..], true, 0),       // LFENCE
        (&[0x48, 0x8d, 0x00, 0x75, 0xfb][..], true, 0),       // conservative LEA exclusion
        (&[0x75, 0xfe][..], true, 0),                         // no native body
        (&[0x83, 0xc0, 1, 0x75, 0][..], false, 0),            // nonself target
        (&[0xe2, 0xfe][..], false, 0),                        // LOOP is not a direct Jcc
        (&[0xeb, 0xfe][..], false, 0),                        // unconditional jump
        (&[0x0f, 0x05][..], false, 0),                        // service boundary
        (&[0xc5, 0xf2, 0x5e, 0xc2, 0x75, 0xf8][..], false, 0), // mediated arithmetic
    ] {
        with_block(code, |epoch, block| {
            let mut observer = BlockSamples::default();
            observer.begin_epoch(epoch);
            let pending = observer.prepare(epoch, block, 256, true).unwrap();
            assert_eq!(
                (
                    observer.self_target_branches,
                    observer.self_target_native_register_blocks,
                    observer.self_target_native_register_instructions
                ),
                (0, 0, 0)
            );
            observer.complete(pending);
            assert_eq!(
                (
                    observer.self_target_branches,
                    observer.self_target_native_register_blocks,
                    observer.self_target_native_register_instructions
                ),
                (
                    u64::from(self_target),
                    u64::from(native_instructions != 0),
                    native_instructions
                ),
                "{code:x?}"
            );
            assert_eq!(observer.completed, 1);
        });
    }
    // Missing retained information cannot be promoted to eligibility even for
    // a harmless-looking decoded instruction. This is not a fabricated plan.
    let mov = instruction(&[0x89, 0xc0]);
    assert!(!native_register_record(&SampleRecord {
        instruction: &mov,
        emitted: true,
        memory: None,
    }));
    assert!(!native_register_record(&SampleRecord {
        instruction: &mov,
        emitted: false,
        memory: Some((false, MemoryRegisterDependencies::default())),
    }));
}

#[test]
fn self_target_sample_discard_work_and_counter_refusals_publish_no_eligibility() {
    with_block(
        &[0x83, 0xc0, 1, 0x83, 0xf8, 2, 0x75, 0xf8],
        |epoch, block| {
            let mut observer = BlockSamples::default();
            observer.begin_epoch(epoch);
            let pending = observer.prepare(epoch, block, 256, false).unwrap();
            let _ = pending; // Actual dispatch failure discards this staged receipt.
            assert_eq!(
                (
                    observer.completed,
                    observer.self_target_branches,
                    observer.self_target_native_register_blocks,
                    observer.self_target_native_register_instructions
                ),
                (0, 0, 0, 0)
            );
            let pending = observer.prepare(epoch, block, 256, false).unwrap();
            observer.complete(pending);
            assert_eq!(
                (
                    observer.completed,
                    observer.self_target_branches,
                    observer.self_target_native_register_blocks,
                    observer.self_target_native_register_instructions
                ),
                (1, 1, 1, 2)
            );
            observer.begin_epoch(identity());
            assert_eq!(
                (
                    observer.self_target_branches,
                    observer.self_target_native_register_blocks,
                    observer.self_target_native_register_instructions
                ),
                (1, 1, 2)
            );

            for case in 0..5 {
                let mut observer = BlockSamples::default();
                observer.begin_epoch(epoch);
                match case {
                    0 => observer.maximum_work = observer.work + FIXED_WORK + 3 * RECORD_WORK - 1,
                    1 => {}
                    2 => observer.self_target_branches = u64::MAX,
                    3 => observer.self_target_native_register_blocks = u64::MAX,
                    4 => observer.self_target_native_register_instructions = u64::MAX - 1,
                    _ => unreachable!(),
                }
                let before = (
                    observer.self_target_branches,
                    observer.self_target_native_register_blocks,
                    observer.self_target_native_register_instructions,
                );
                assert!(
                    observer
                        .prepare(epoch, block, if case == 1 { 0 } else { 256 }, true)
                        .is_none()
                );
                assert!(observer.disabled);
                assert_eq!(observer.overflow, case >= 2);
                assert_eq!(observer.completed, 0);
                assert_eq!(observer.counts, [[0; 2]; 12]);
                assert_eq!(observer.exit_codes_retained, 0);
                assert_eq!(
                    (
                        observer.self_target_branches,
                        observer.self_target_native_register_blocks,
                        observer.self_target_native_register_instructions
                    ),
                    before
                );
            }
        },
    );
}

#[test]
fn exit_histogram_retains_first_codes_counts_observation_overflow_and_commits_contexts_only_on_success()
 {
    let epoch = identity();
    let next = identity();
    let codes: Vec<_> = Code::values().take(65).collect();
    assert_eq!(codes.len(), 65);
    let mut observer = BlockSamples::default();
    observer.begin_epoch(epoch);
    let commit = |observer: &mut BlockSamples, planned: Option<Code>, is_main: bool| {
        let pending = observer
            .prepare_records(
                Key {
                    epoch,
                    module: 0,
                    pc: 0x5000,
                },
                planned,
                [].iter(),
                0,
                is_main,
            )
            .unwrap();
        observer.complete(pending);
    };
    for (index, code) in codes[..64].iter().enumerate() {
        commit(&mut observer, Some(*code), index % 2 == 0);
    }
    assert_eq!(
        (observer.exit_codes_retained, observer.exit_overflow),
        (64, 0)
    );
    for (index, repeats) in [(0, 3), (1, 2), (2, 1)] {
        for _ in 0..repeats {
            commit(&mut observer, Some(codes[index]), true);
        }
    }
    for _ in 0..3 {
        commit(&mut observer, Some(codes[64]), false);
    }
    commit(&mut observer, None, false);
    assert_eq!(
        (
            observer.completed,
            observer.main_context,
            observer.child_context
        ),
        (74, 38, 36)
    );
    assert_eq!(
        (observer.exit_overflow, observer.exit_no_instruction),
        (3, 1)
    );
    let retained = observer
        .exits
        .iter()
        .flatten()
        .map(|(_, count)| count)
        .sum::<u64>();
    assert_eq!(
        retained + observer.exit_overflow + observer.exit_no_instruction,
        observer.completed
    );
    let fields = observer.protocol_fields();
    let top = format!(
        "sample_exit_top={:?}:4,{:?}:3,{:?}:2,{:?}:1",
        codes[0], codes[1], codes[2], codes[3]
    );
    assert!(fields.ends_with(&top), "{fields}");
    let before = observer.exits;
    let pending = observer
        .prepare_records(
            Key {
                epoch,
                module: 0,
                pc: 0x5000,
            },
            Some(codes[0]),
            [].iter(),
            0,
            true,
        )
        .unwrap();
    let _ = pending; // Actual hook discards this update on a failed dispatch.
    assert_eq!(observer.exits, before);
    assert_eq!(
        (
            observer.completed,
            observer.main_context,
            observer.child_context
        ),
        (74, 38, 36)
    );
    observer.begin_epoch(next);
    assert_eq!(observer.exits, before); // Public Code histogram is cumulative.
    assert_eq!(
        (observer.recent, observer.distinct, observer.epoch_changes),
        (0, 0, 1)
    );
}
