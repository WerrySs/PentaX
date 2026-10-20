// SPDX-License-Identifier: GPL-2.0-only
//! Original NetCtl callbacks through the production prepared-session consumer.
//! Native execution runs only in a separately supervised test child.

use super::*;
use pentax_core::process::hle::{NativeImageLimits, with_native_image_session};
use pentax_kernel::Service;
use pentax_linker::{QualifiedNid, inspect_sce};
use pentax_loader::open_executable;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

const CHILD: &str = "PENTAX_TEST_NETCTL_CALLBACK_CHILD";
const TEST_NAME: &str = "native_session::netctl_tests::netctl_callbacks_execute_in_slot_order_once_and_term_clears_pending_slots";

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
    // Preserve RBX and the SysV stack alignment. Call the exact generated
    // CheckCallback marker in [RDI] twice; neither numeric callback return may
    // become the CheckCallback result or cause the second check to replay it.
    let main = [0x53, 0x48, 0x89, 0xfb, 0xff, 0x13, 0xff, 0x13, 0x5b, 0xc3];
    bytes[0x100..0x100 + main.len()].copy_from_slice(&main);
    for (offset, result) in [(0x200, 0xdead_u32), (0x300, 0xbeef)] {
        let mut code = vec![
            0x48, 0x89, 0x3e, // MOV [RSI],RDI: event
            0x48, 0x89, 0x56, 8, // MOV [RSI+8],RDX: source's zero argument
            0x48, 0x8b, 0x46, 16, // MOV RAX,[RSI+16]: shared sequence address
            0x48, 0x8b, 0x08, // MOV RCX,[RAX]
            0x48, 0x83, 0xc1, 1, // ADD RCX,1
            0x48, 0x89, 0x08, // MOV [RAX],RCX
            0x48, 0x89, 0x4e, 24, // MOV [RSI+24],RCX: callback order
            0x48, 0x89, 0x76, 32,   // MOV [RSI+32],RSI: exact original arg
            0xb8, // MOV EAX,deliberately nonzero void result
        ];
        code.extend_from_slice(&result.to_le_bytes());
        code.push(0xc3);
        bytes[offset..offset + code.len()].copy_from_slice(&code);
    }
    bytes
}

// Read exact normal-spelling exports from the factory's retained source, then
// fetch their live marker bytes. The production core still independently
// attests the private ID, PC, scope, owner and source; this helper grants none
// of that authority and hardcodes no marker ordinal or service ID.
fn marker(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    service: Service,
) -> (u64, u64) {
    let module = &proof.materialized_source().source_plan().modules()
        [session.plan().kernel_provider_index()];
    let view = open_executable(module.source_bytes()).unwrap();
    let inspection = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let metadata = inspection.metadata().unwrap();
    let descriptors = metadata.descriptors().decoded().unwrap();
    let registration = Service::registrations()
        .iter()
        .find(|registration| {
            registration.service == service
                && registration.module == "NetCtl"
                && registration.library == "NetCtl"
        })
        .unwrap();
    let mut exports = metadata.link().symbols().iter().filter_map(|symbol| {
        let qualified = QualifiedNid::parse(symbol.name.as_ref()?.as_bytes()).ok()?;
        if qualified.nid().as_bytes() != registration.nid.as_bytes() {
            return None;
        }
        let module = descriptors
            .module_info()
            .iter()
            .find(|m| m.id() == qualified.module())?;
        let library = descriptors
            .export_libraries()
            .iter()
            .find(|l| l.id() == qualified.library())?;
        (module.name().as_bytes() == b"NetCtl" && library.name().as_bytes() == b"NetCtl")
            .then_some(symbol.value)
    });
    let address = module.load_bias() + exports.next().unwrap();
    assert!(exports.next().is_none());
    let mut code = [0; 8];
    session
        .pause()
        .unwrap()
        .fetch_source(address, &mut code)
        .unwrap();
    assert_eq!(code[0], 0xb8);
    assert_eq!(&code[5..], &[0x0f, 0x05, 0xc3]);
    (
        address,
        u32::from_le_bytes(code[1..5].try_into().unwrap()).into(),
    )
}
fn scalar(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    service: Service,
    args: [u64; 6],
) {
    let (entry, id) = marker(session, proof, service);
    let request = NativeServiceRequest {
        owner_id: session.owner_id(),
        syscall_pc: entry + 5,
        next_pc: entry + 7,
        service_id: id,
        args,
        vector_args: None,
        stack_pointer: None,
    };
    assert!(matches!(
        session.service_action(proof, request),
        Ok(NativeServiceAction::Returned(0))
    ));
}
fn word(session: &mut PreparedNativeKernelStartup<'_, '_>, address: u64) -> u64 {
    let mut value = [0; 8];
    session.pause().unwrap().read(address, &mut value).unwrap();
    u64::from_le_bytes(value)
}

#[test]
fn netctl_callbacks_execute_in_slot_order_once_and_term_clears_pending_slots() {
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
                panic!("original NetCtl callback test exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "original NetCtl callback test failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    pentax_platform::configure_worker_fault_policy().unwrap();
    let bytes = image();
    with_native_image_session(
        &[&bytes],
        &NativeImageLimits::default(),
        |session, layout| {
            let proof = session.prepare_native_source(&BlockLimits::default())?;
            let scratch = layout.entry.stack.address;
            let sequence = scratch + 0x80;
            let function_cell = scratch + 0x88;
            let ids = scratch + 0x90;
            let records = [scratch + 0x100, scratch + 0x140, scratch + 0x180];
            let base = proof.materialized_source().source_plan().modules()[0].load_bias();
            let callbacks = [base + 0x200, base + 0x300];
            scalar(session, &proof, Service::NetCtlInit, [0; 6]);
            for (index, &record) in records.iter().enumerate() {
                session
                    .pause()?
                    .write(record + 16, &sequence.to_le_bytes())?;
                if index < 2 {
                    scalar(
                        session,
                        &proof,
                        Service::NetCtlRegisterCallback,
                        [callbacks[index], record, ids + index as u64 * 4, 0, 0, 0],
                    );
                }
            }
            let check = marker(session, &proof, Service::NetCtlCheckCallback).0;
            session
                .pause()?
                .write(function_cell, &check.to_le_bytes())?;
            let stack = NativeStackFrame {
                stack: layout.entry.stack,
                guards: Some(layout.entry.guards),
                return_slot: layout.entry.return_slot,
                return_marker: layout.entry.return_marker,
            };
            let mut initial = NativeCpuState::new(layout.entry.entry, layout.entry.return_slot);
            initial.gpr[7] = function_cell;
            let limits = NativeSessionLimits::default();
            let mut progress = NativeSessionProgress::default();
            let report = run_prepared(session, &proof, initial, stack, &limits, &mut progress)?;
            assert_eq!(report.return_value, 0);
            assert_eq!(report.service_stops, 2);
            assert_eq!(progress.initializer_calls, 0);
            assert_eq!(progress.phase.code(), "main-entry");
            assert!(report.dispatch.native_instructions > 0);
            assert_eq!(word(session, sequence), 2);
            for (index, &record) in records[..2].iter().enumerate() {
                assert_eq!(word(session, record), 1);
                assert_eq!(word(session, record + 8), 0);
                assert_eq!(word(session, record + 24), index as u64 + 1);
                assert_eq!(word(session, record + 32), record);
            }
            // This third registration has never received the event. Term must
            // remove it, not merely suppress callbacks that already ran.
            scalar(
                session,
                &proof,
                Service::NetCtlRegisterCallback,
                [callbacks[0], records[2], ids + 8, 0, 0, 0],
            );
            scalar(session, &proof, Service::NetCtlTerm, [0; 6]);
            scalar(session, &proof, Service::NetCtlInit, [0; 6]);
            let report = run_prepared(session, &proof, initial, stack, &limits, &mut progress)?;
            assert_eq!(report.return_value, 0);
            assert_eq!(report.service_stops, 2);
            assert_eq!(progress.initializer_calls, 0);
            assert_eq!(word(session, sequence), 2);
            assert_eq!(word(session, records[2]), 0);
            assert_eq!(word(session, records[2] + 24), 0);
            Ok::<_, NativeSessionError>(())
        },
    )
    .unwrap();
}
