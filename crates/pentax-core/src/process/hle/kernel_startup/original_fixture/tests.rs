// SPDX-License-Identifier: GPL-2.0-only

//! Data-only preparation/decoding/error tests. No CPU runner is invoked, and no
//! successful execution report is fabricated by these host callbacks.

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
use super::image::ENTRY;
use super::*;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
use pentax_codegen::{
    BlockLimits, CachePlacement, Code, ControlTarget, ExitKind, Register, TranslationPolicy,
    plan_materialized_block,
};
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

#[derive(Debug, PartialEq, Eq)]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
enum Stop {
    Prepared,
}

#[test]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn original_source_prepares_exact_bindings_tls_and_stack_then_decodes_without_execution() {
    let result = run_original_prepared_fixture(|session, layout| {
        let base = layout.entry - ENTRY as u64;
        assert_eq!(session.memory().module_count(), 3);
        assert_eq!(session.memory().applied_relocations(), 8);
        // Three original image pages, two libc pages, two TLS blocks,
        // two thread headers and the entry stack, plus the kernel extent.
        assert_eq!(
            session.memory().mapped_bytes(),
            10 * PAGE_SIZE + KERNEL_MAPPED_BYTES
        );
        assert_eq!(session.plan().kernel_bindings(), 3);
        assert_eq!(session.plan().hle_startup_plan().provider_bindings(), 1);
        assert_eq!(session.plan().hle_startup_plan().tls_bindings(), 3);
        assert_eq!(session.native_service_requests(), 0);
        assert_eq!(session.kernel_usage().handles_live, 0);
        assert_eq!(layout.return_slot % 16, 8);
        assert!(layout.return_slot - 128 > layout.stack.address);
        assert_eq!(
            word::<Stop>(session, layout.return_slot).unwrap(),
            RETURN_MARKER
        );
        let symbols = tls_symbols::<Stop>(session).unwrap();
        assert_ne!(symbols[0], symbols[1]);
        assert_eq!(word::<Stop>(session, symbols[0]).unwrap(), 37);
        assert_eq!(word::<Stop>(session, symbols[1]).unwrap(), 37);
        assert_eq!(word::<Stop>(session, base + OUTPUT + 0x80).unwrap(), 0);
        let thread = session.prepare_main_thread().unwrap();
        assert_eq!(thread.thread_pointer(), base + TPS[0]);
        assert_eq!(thread.owner_id(), session.owner_id());
        assert!(std::ptr::eq(
            thread.source_plan(),
            session.plan().link_plan()
        ));
        for instance in session.tls().instances() {
            let header = instance.thread_control_block.unwrap();
            assert_eq!(header.address, instance.thread_pointer);
            assert_eq!(header.size, 64);
            assert_eq!(
                instance.guard_pages.unwrap()[1],
                instance.thread_pointer + PAGE_SIZE
            );
        }
        assert_eq!(word::<Stop>(session, base + TPS[0]).unwrap(), base + TPS[0]);
        {
            let paused = session.pause().unwrap();
            for guard in layout.guards {
                paused
                    .preflight_reserved_unmapped(guard.address, guard.size)
                    .unwrap();
            }
            assert!(matches!(
                paused.preflight_write(base + 0x4000, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.preflight_fetch_source(layout.stack.address, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.preflight_read(base + PREFIX, 1),
                Err(NativeMemoryError::Unmapped(_))
            ));
        }
        let proof = session
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        assert_eq!(proof.owner_id(), session.owner_id());
        for write in proof.materialized_source().declared_writes() {
            assert_eq!(word::<Stop>(session, write.address).unwrap(), write.value);
        }
        let source = image::code(base).unwrap();
        let mut live = vec![0; source.len()];
        session
            .pause()
            .unwrap()
            .fetch_source(layout.entry, &mut live)
            .unwrap();
        assert_eq!(source, live);
        let cache = if base >= 2 * ARENA {
            base - ARENA
        } else {
            base + 2 * ARENA
        };
        let mut offset = 0;
        let mut calls = 0;
        let mut decoded = 0;
        let mut segments = 0;
        let mut absolute = Vec::new();
        let mut original_rcx = false;
        for _ in 0..8 {
            let block = plan_materialized_block(
                proof.materialized_source(),
                0,
                layout.entry + offset as u64,
                &live[offset..],
                CachePlacement {
                    address: cache,
                    capacity: 4096,
                },
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            )
            .unwrap();
            decoded += block.instructions().len();
            for record in block.instructions() {
                let instruction = record.instruction();
                if instruction.code() == Code::Mov_r64_imm64 {
                    absolute.push(instruction.immediate64());
                }
                if instruction.op0_register() == Register::ECX
                    && instruction.code() == Code::Mov_r32_imm32
                {
                    assert_eq!(instruction.immediate32(), 17);
                    original_rcx = true;
                }
                if let Some(address) = record.rip_relative_target() {
                    assert!(
                        [
                            base + 0x4000,
                            base + 0x4008,
                            base + 0x4010,
                            base + 0x4018,
                            base + OUTPUT + 8,
                            base + OUTPUT + 0x80,
                            base + OUTPUT + 0x90,
                            base + OUTPUT + 0x98,
                            base + OUTPUT + 0xa0,
                            base + OUTPUT + 0xa8,
                            base + OUTPUT + 0xb0,
                            base + OUTPUT + 0xb8,
                            base + OUTPUT + 0xc0,
                            base + TLS_RVA + 16
                        ]
                        .contains(&address)
                    );
                }
            }
            match block.exit().kind() {
                ExitKind::Call {
                    target: ControlTarget::Memory(pointer),
                    ..
                } => {
                    assert_eq!(
                        pointer.ip_relative_address(),
                        Some(base + 0x4000 + calls * 8)
                    );
                    calls += 1;
                    offset = (block.exit().next_guest_pc() - layout.entry) as usize;
                }
                ExitKind::Return { adjust: 0 } => {
                    assert_eq!(
                        block.exit().next_guest_pc(),
                        layout.entry + live.len() as u64
                    );
                    break;
                }
                ExitKind::Mediated {
                    reason: pentax_codegen::UnsupportedReason::SegmentPrefix,
                } => {
                    let instruction = block.exit().instruction().unwrap();
                    assert_eq!(instruction.segment_prefix(), Register::FS);
                    assert_eq!(instruction.mnemonic(), pentax_codegen::Mnemonic::Mov);
                    segments += 1;
                    offset = (block.exit().next_guest_pc() - layout.entry) as usize;
                }
                other => {
                    panic!("original consumer must decode only calls and outer RET: {other:?}")
                }
            }
        }
        assert_eq!(calls, 4);
        assert_eq!(decoded, 40);
        assert_eq!(segments, 3);
        assert!(original_rcx);
        assert_eq!(
            absolute,
            [base + OUTPUT + 8, base + image::NAME, base + OUTPUT + 0x80]
        );
        // Propagate an explicit host stop: the test has only prepared/decoded.
        Err(Stop::Prepared)
    });
    assert!(matches!(
        result,
        Err(OriginalPreparedError::Callback(Stop::Prepared))
    ));
}

#[test]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn main_thread_admission_refuses_changed_or_executable_header_pages() {
    let result = run_original_prepared_fixture(|session, layout| {
        let pointer = layout.entry - ENTRY as u64 + TPS[0];
        session
            .pause()
            .unwrap()
            .write(pointer + 0x28, &1_u64.to_le_bytes())
            .unwrap();
        assert!(matches!(
            session.prepare_main_thread(),
            Err(KernelStartupError::InvalidProvider(
                "initial thread header bytes"
            ))
        ));
        session
            .pause()
            .unwrap()
            .write(pointer + 0x28, &0_u64.to_le_bytes())
            .unwrap();
        let thread = session.prepare_main_thread().unwrap();
        assert_eq!(thread.thread_pointer(), pointer);
        // The opaque history borrows only the retained plan, not this pause.
        session
            .pause()
            .unwrap()
            .write(pointer, &0_u64.to_le_bytes())
            .unwrap();
        assert_eq!(thread.owner_id(), session.owner_id());
        assert!(session.prepare_main_thread().is_err());
        session
            .pause()
            .unwrap()
            .write(pointer, &pointer.to_le_bytes())
            .unwrap();
        session
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::Protect {
                        address: pointer,
                        size: PAGE_SIZE,
                        permissions: NativePermissions::new(true, true, true),
                    },
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
        assert!(matches!(
            session.prepare_main_thread(),
            Err(KernelStartupError::InvalidProvider(
                "thread header must be NX"
            ))
        ));
        Err(Stop::Prepared)
    });
    assert!(matches!(
        result,
        Err(OriginalPreparedError::Callback(Stop::Prepared))
    ));
}

#[test]
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn a_constant_callback_cannot_publish_a_report_without_actual_service_effects() {
    let result = run_original_prepared_fixture(|_, _| Ok::<_, Stop>(42));
    assert!(matches!(
        result,
        Err(OriginalPreparedError::Fixture("observed service sequence"))
    ));
    let wrong = run_original_prepared_fixture(|_, _| Ok::<_, Stop>(7));
    assert!(matches!(
        wrong,
        Err(OriginalPreparedError::Fixture("callback result"))
    ));
}

#[test]
fn original_factory_refuses_overflow_before_address_encoding() {
    assert!(matches!(
        original_image(u64::MAX),
        Err(HleError::InvalidProvider("original address range"))
    ));
    let valid = original_image(0x10_0000).unwrap();
    let view = open_executable(&valid).unwrap();
    assert_eq!(view.metadata().entry, ENTRY as u64);
    assert_eq!(view.metadata().program_headers.len(), 6);
}
