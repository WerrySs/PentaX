// SPDX-License-Identifier: GPL-2.0-only
//! Original actual callback mutations and late-fault non-replay, using only
//! public core/session authority, in one separately supervised test process.
use super::*;
use pentax_core::process::hle::{NativeEntryLayout, NativeImageLimits, with_native_image_session};
use pentax_linker::{QualifiedNid, SymbolBinding, SymbolType, inspect_sce};
use pentax_loader::open_executable;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

const CHILD: &str = "PENTAX_TEST_AGC_CALLBACK_MUTATION_CHILD";
const TEST_NAME: &str = "native_session::agc_callback_tests::actual_grow_callback_mutations_feed_current_descriptor_and_payload_once_or_preserve_fault_prefix";

fn image() -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (offset, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x100_u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&5_u32.to_le_bytes());
    for (offset, value) in [(96, 4096_u64), (104, 16384), (112, 16384)] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    // Original callback: load replacement UP from user data, publish UP/DOWN,
    // change one payload byte, and return true. This is unchanged from the
    // original core proof; execution belongs to the worker's unsafe boundary.
    let code = [
        0x48, 0x8b, 0x02, 0x48, 0x89, 0x47, 0x10, 0x48, 0x05, 0x00, 0x01, 0x00, 0x00, 0x48, 0x89,
        0x47, 0x18, 0xc6, 0x42, 0x08, 0x5c, 0xb8, 0x01, 0x00, 0x00, 0x00, 0xc3,
    ];
    bytes[0x100..0x100 + code.len()].copy_from_slice(&code);
    bytes
}

// Resolve only the exact public builder export from the current source's
// typed metadata. No assumed provider index, marker ordinal or ID is used.
// Reading its live marker does not replace core's independent site admission.
fn marker(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
) -> (usize, u64, u64) {
    let mut found = None;
    for (index, input) in proof
        .materialized_source()
        .source_plan()
        .modules()
        .iter()
        .enumerate()
    {
        let view = open_executable(input.source_bytes()).unwrap();
        let inspection = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
        let Some(metadata) = inspection.metadata() else {
            continue;
        };
        let descriptors = metadata.descriptors().decoded().unwrap();
        for symbol in metadata.link().symbols() {
            if symbol.is_undefined()
                || symbol.binding() != SymbolBinding::GLOBAL
                || symbol.symbol_type() != SymbolType::FUNC
            {
                continue;
            }
            let Some(name) = symbol.name.as_ref() else {
                continue;
            };
            let Ok(qualified) = QualifiedNid::parse(name.as_bytes()) else {
                continue;
            };
            if qualified.nid().as_bytes() != b"i1jyy49AjXU" {
                continue;
            }
            let module = descriptors
                .module_info()
                .iter()
                .find(|item| item.id() == qualified.module())
                .unwrap();
            let library = descriptors
                .export_libraries()
                .iter()
                .find(|item| item.id() == qualified.library())
                .unwrap();
            if module.name().as_bytes() != b"Graphics5"
                || library.name().as_bytes() != b"Graphics5"
                || (
                    module.version_major(),
                    module.version_minor(),
                    library.version(),
                ) != (1, 1, 1)
            {
                continue;
            }
            assert!(found.is_none());
            let address = input.load_bias().checked_add(symbol.value).unwrap();
            let mut bytes = [0; 8];
            owner
                .pause()
                .unwrap()
                .fetch_source(address, &mut bytes)
                .unwrap();
            assert_eq!(bytes[0], 0xb8);
            assert_eq!(&bytes[5..], &[0x0f, 0x05, 0xc3]);
            found = Some((
                index,
                address,
                u64::from(u32::from_le_bytes(bytes[1..5].try_into().unwrap())),
            ));
        }
    }
    found.unwrap()
}

fn stopped<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    request: NativeServiceRequest,
    layout: NativeEntryLayout,
    module: usize,
) -> (NativeBlockStepper<'view, 'plan, 'image>, PreSyscallToken) {
    let stack = NativeStackFrame {
        stack: layout.stack,
        guards: None,
        return_slot: request.stack_pointer.unwrap(),
        return_marker: layout.return_marker,
    };
    owner
        .pause()
        .unwrap()
        .write(stack.return_slot, &stack.return_marker.to_le_bytes())
        .unwrap();
    let mut state = NativeCpuState::new(request.syscall_pc, stack.return_slot);
    // This original fixture has only its initial thread and never switches it.
    assert_eq!(owner.tls().instances().len(), 1);
    if owner.tls().thread_control_policy()
        != pentax_core::process::tls::ThreadControlBlockPolicy::None
    {
        state.fs_base = Some(owner.tls().instances()[0].thread_pointer);
    }
    state.gpr[0] = request.service_id;
    for (register, value) in [7, 6, 2, 1, 8, 9].into_iter().zip(request.args) {
        state.gpr[register] = value;
    }
    let mut engine = NativeBlockStepper::new(
        proof.materialized_source(),
        &mut owner.pause().unwrap(),
        state,
        stack,
        &Default::default(),
    )
    .unwrap();
    let block = plan_materialized_block(
        proof.materialized_source(),
        module,
        request.syscall_pc,
        &[15, 5],
        engine.placement_for(request.syscall_pc),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut owner.pause().unwrap(), &block)
        .unwrap()
    else {
        panic!("expected syscall")
    };
    (engine, token)
}

#[test]
fn actual_grow_callback_mutations_feed_current_descriptor_and_payload_once_or_preserve_fault_prefix()
 {
    if std::env::var_os(CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD, "1")
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("original AGC callback mutation test exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "original AGC callback mutation test failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let marker =
            "agc_command_callback_mutation_available=true agc_command_callback_mutation_cases=2";
        assert_eq!(
            String::from_utf8_lossy(&output.stdout)
                .matches(marker)
                .count(),
            1
        );
        // Forward only the positive proof marker, not the child libtest
        // summary, so the outer runner counts this test function once.
        println!("{marker}");
        return;
    }
    pentax_platform::configure_worker_fault_policy().unwrap();
    let primary = image();
    for bad_payload in [false, true] {
        with_native_image_session(&[&primary], &NativeImageLimits::default(), |owner, layout| {
            let proof = owner.prepare_native_source(&BlockLimits::default()).unwrap();
            let (module, entry, id) = marker(owner, &proof);
            let base = layout.entry.stack.address;
            let (buf, user, up) = (base + 0x100, base + 0x200, base + 0x300);
            let mut descriptor = [0; 56];
            descriptor[16..24].copy_from_slice(&(up + 0x100).to_le_bytes());
            descriptor[24..32].copy_from_slice(&(up + 0x100).to_le_bytes());
            descriptor[32..40].copy_from_slice(&layout.entry.entry.to_le_bytes());
            descriptor[40..48].copy_from_slice(&user.to_le_bytes());
            descriptor[48..52].copy_from_slice(&2u32.to_le_bytes());
            owner.pause().unwrap().write(buf, &descriptor).unwrap();
            owner.pause().unwrap().write(user, &up.to_le_bytes()).unwrap();
            owner.pause().unwrap().write(user + 8, &[0x4a; 8]).unwrap();
            owner.pause().unwrap().write(up, &[0xa5; 32]).unwrap();
            let parent_rsp = layout.entry.return_slot - 64;
            owner.pause().unwrap().write(parent_rsp + 8, &[0; 16]).unwrap();
            let raw = NativeServiceRequest {
                owner_id: owner.owner_id(), syscall_pc: entry + 5, next_pc: entry + 7, service_id: id,
                args: [buf, 1, 0, 0, if bad_payload { 1 } else { user + 8 }, 2],
                vector_args: None, stack_pointer: Some(parent_rsp),
            };
            let (mut engine, token) = stopped(owner, &proof, raw, layout.entry, module);
            let NativeServiceAction::AgcCommandGrow(start) = owner.service_action(&proof, raw).unwrap() else { panic!("expected growth") };
            assert_eq!(start.args(), [buf, 8, user]);
            let nested = engine.begin_syscall_call(&mut owner.pause().unwrap(), token, start.entry(), start.args()).unwrap();
            owner.observe_agc_command_buffer_call(&proof, start, &nested).unwrap();
            assert!(owner.observe_agc_command_buffer_call(&proof, start, &nested).is_err());
            let mut finished = false;
            for _ in 0..8 {
                let pc = engine.state().guest_pc;
                let mut paused = owner.pause().unwrap();
                let mut bytes = [0; 64];
                paused.fetch_source(pc, &mut bytes).unwrap();
                let block = plan_materialized_block(proof.materialized_source(), 0, pc, &bytes,
                    engine.placement_for(pc), &TranslationPolicy::x64_sse2(), &BlockLimits::default()).unwrap();
                // SAFETY: only the original immutable callback and its actual
                // exclusively paused core owner are used. The checked executor
                // validates the real nested stack and all current operands.
                let stop = unsafe { engine.step_checked(&mut paused, &block) }.unwrap();
                match stop {
                    NativeBlockStop::Resume { .. } => {},
                    NativeBlockStop::Returned { .. } => { finished = true; break; },
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert!(finished);
            assert!(engine.usage().native_entries > 0);
            let returned = engine.observed_return().unwrap();
            assert_eq!(returned.value(), 1);
            // The actual guest callback already changed these live bytes.
            let mut grown = [0; 16]; owner.pause().unwrap().read(buf + 16, &mut grown).unwrap();
            assert_eq!(u64::from_le_bytes(grown[..8].try_into().unwrap()), up);
            assert_eq!(u64::from_le_bytes(grown[8..].try_into().unwrap()), up + 0x100);
            let mut payload = [0; 8]; owner.pause().unwrap().read(user + 8, &mut payload).unwrap();
            assert_eq!(payload, [0x5c, 0x4a, 0x4a, 0x4a, 0x4a, 0x4a, 0x4a, 0x4a]);
            let result = owner.complete_agc_command_buffer_call(&proof, start, &returned, nested, &engine);
            if bad_payload {
                assert!(matches!(result, Err(KernelStartupError::Graphics(pentax_core::gpu::driver::Failure::Memory))));
                assert!(matches!(owner.service_action(&proof, raw), Err(KernelStartupError::Graphics(pentax_core::gpu::driver::Failure::Poisoned))));
            } else { assert!(matches!(result, Ok(NativeServiceAction::Returned(value)) if value == up)); }
            let mut actual = [0; 24]; owner.pause().unwrap().read(up, &mut actual).unwrap();
            assert_eq!(u32::from_le_bytes(actual[..4].try_into().unwrap()), 0xc004_3700);
            assert_eq!(&actual[16..], if bad_payload { &[0xa5; 8] } else { &payload });
            let mut cursor = [0; 8]; owner.pause().unwrap().read(buf + 16, &mut cursor).unwrap();
            assert_eq!(u64::from_le_bytes(cursor), up + 24);
            assert_eq!(owner.graphics_usage().calls, 1);
            let usage = engine.usage();
            let replay_reason = if bad_payload { "AGC callback return replay" } else { "AGC callback ticket" };
            assert!(matches!(owner.complete_agc_command_buffer_call(&proof, start, &returned, nested, &engine),
                Err(KernelStartupError::InvalidProvider(reason)) if reason == replay_reason));
            // A rejected replay changes neither prefix nor native credits.
            assert_eq!(engine.usage(), usage);
            let mut replay = [0; 24]; owner.pause().unwrap().read(up, &mut replay).unwrap();
            assert_eq!(replay, actual);
            engine.complete_syscall_call(&mut owner.pause().unwrap(), nested, 0).unwrap();
            let resumed_usage = engine.usage();
            assert!(owner.complete_agc_command_buffer_call(&proof, start, &returned, nested, &engine).is_err());
            assert_eq!(engine.usage(), resumed_usage);
            Ok::<_, ()>(())
        }).unwrap();
    }
    println!("agc_command_callback_mutation_available=true agc_command_callback_mutation_cases=2");
}
