// SPDX-License-Identifier: GPL-2.0-only

//! Real native context alternation in a separately supervised test process.

use super::*;
use pentax_core::process::hle::{NativeImageLimits, with_native_image_session};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativePermissions, NativeRange,
};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

const CHILD: &str = "PENTAX_TEST_THREAD_EXCHANGE_CHILD";
const TEST_NAME: &str =
    "native_session::thread_tests::two_guest_contexts_execute_and_return_with_one_cache_and_budget";
const PAGE: u64 = 16384;

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
    for (offset, value) in [(96, 4096_u64), (104, PAGE), (112, PAGE)] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    // MOV RAX,RSI; ADD RAX,1; PADDQ XMM0,XMM1; JMP next; MOV [RDI],RAX; RET.
    let code = [
        0x48, 0x89, 0xf0, 0x48, 0x83, 0xc0, 1, 0x66, 0x0f, 0xd4, 0xc1, 0xeb, 0, 0x48, 0x89, 0x07,
        0xc3,
    ];
    bytes[0x100..0x100 + code.len()].copy_from_slice(&code);
    bytes
}

fn step<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'plan, 'image>,
    engine: &mut NativeBlockStepper<'_, 'plan, 'image>,
) -> Result<NativeBlockStop, NativeSessionError> {
    let pc = engine.state().guest_pc;
    let mut paused = session.pause()?;
    let mut bytes = [0; 32];
    paused.fetch_source(pc, &mut bytes)?;
    let block = plan_materialized_block(
        proof.materialized_source(),
        0,
        pc,
        &bytes,
        engine.placement_for(pc),
        &TranslationPolicy::x64_sse2().with_independent_memory(),
        &BlockLimits::default(),
    )?;
    // SAFETY: Exact core-prepared owner/source with no deferred TLS patches.
    // The original code uses GPR/SSE arithmetic and one checked write to a live
    // owned stack. Both independent stacks and their return slots are admitted
    // by this executor. There are no segment operands or host pointers.
    Ok(unsafe { engine.step_checked(&mut paused, &block) }?)
}

#[test]
fn two_guest_contexts_execute_and_return_with_one_cache_and_budget() {
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
                panic!("native thread exchange exceeded its deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "native thread exchange failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    pentax_platform::configure_worker_fault_policy().unwrap();
    let bytes = image();
    let limits = NativeImageLimits {
        stack_bytes: PAGE,
        kernel_window_bytes: 8 * PAGE,
        ..Default::default()
    };
    with_native_image_session(&[&bytes], &limits, |session, layout| {
        let proof = session.prepare_native_source(&BlockLimits::default())?;
        let entry = layout.entry;
        let stack = NativeStackFrame {
            stack: entry.stack,
            guards: Some(entry.guards),
            return_slot: entry.return_slot,
            return_marker: entry.return_marker,
        };
        let address = session.plan().limits().kernel.virtual_base + PAGE;
        let other_stack = NativeStackFrame {
            stack: NativeRange {
                address,
                size: PAGE,
            },
            guards: Some([
                NativeRange {
                    address: address - PAGE,
                    size: PAGE,
                },
                NativeRange {
                    address: address + PAGE,
                    size: PAGE,
                },
            ]),
            return_slot: address + PAGE - 8,
            return_marker: entry.return_marker,
        };
        session.pause()?.apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::MapZeroed {
                    address,
                    size: PAGE,
                    permissions: NativePermissions::READ_WRITE,
                },
                writes: &[NativeMemoryWrite {
                    address: other_stack.return_slot,
                    bytes: &entry.return_marker.to_le_bytes(),
                }],
            },
            &Default::default(),
        )?;
        let mut main = NativeCpuState::new(entry.entry, entry.return_slot);
        main.gpr[6] = 41;
        main.gpr[7] = entry.stack.address + 0x100;
        main.xmm.fill(0x11);
        main.ymm_upper.fill(0x23);
        let mut other = NativeCpuState::new(entry.entry, other_stack.return_slot);
        other.gpr[6] = 99;
        other.gpr[7] = address + 0x100;
        other.xmm.fill(0x33);
        other.ymm_upper.fill(0x45);
        other.mxcsr = 0x3f80;
        other.x87.control_word = 0x0f7f;
        other.x87.status_word = 0x4500;
        other.x87.registers.fill([0x85; 10]);
        let dispatch_limits = NativeDispatchLimits {
            max_thread_contexts: 2,
            max_blocks: 20,
            cache_slots: 4,
            ..Default::default()
        };
        let mut engine = NativeBlockStepper::new(
            proof.materialized_source(),
            &mut session.pause()?,
            main,
            stack,
            &dispatch_limits,
        )?;
        let mut parked = engine.prepare_thread(&mut session.pause()?, other, other_stack)?;
        let mut returned = [false; 2];
        let mut snapshots = [main, other];
        let mut observed = [None, None];
        for _ in 0..20 {
            let index = engine.thread_index();
            assert!(!returned[index]);
            assert_eq!(*engine.state(), snapshots[index]);
            let result = step(session, &proof, &mut engine)?;
            snapshots[index] = *engine.state();
            assert_eq!(
                engine.state().ymm_upper,
                if index == 0 {
                    main.ymm_upper
                } else {
                    other.ymm_upper
                }
            );
            assert_eq!(
                engine.state().mxcsr,
                if index == 0 { main.mxcsr } else { other.mxcsr }
            );
            assert_eq!(
                engine.state().x87,
                if index == 0 { main.x87 } else { other.x87 }
            );
            match result {
                NativeBlockStop::Resume { .. } => {}
                NativeBlockStop::Returned { value } => {
                    assert_eq!(value, if index == 0 { 42 } else { 100 });
                    returned[index] = true;
                    let observation = engine.observed_return().expect("actual return");
                    observed[index] =
                        Some((observation.executor_id(), observation.call_sequence()));
                }
                _ => panic!("unexpected original thread stop"),
            }
            if returned == [true, true] {
                break;
            }
            if !returned[parked.index()] {
                engine.exchange_thread(&mut session.pause()?, &mut parked)?;
            }
        }
        assert_eq!(returned, [true, true]);
        assert!(engine.usage().native_instructions >= 8);
        assert!(engine.usage().blocks <= 20);
        assert!(engine.usage().cache_reuses > 0);
        assert_eq!(observed[0].unwrap().0, observed[1].unwrap().0);
        assert_ne!(observed[0].unwrap().1, observed[1].unwrap().1);
        for (index, (output, value)) in [(main.gpr[7], 42_u64), (other.gpr[7], 100)]
            .into_iter()
            .enumerate()
        {
            let mut actual = [0; 8];
            session.pause()?.read(output, &mut actual)?;
            assert_eq!(u64::from_le_bytes(actual), value);
            let expected = if index == 0 {
                0x2222_2222_2222_2222_u64
            } else {
                0x6666_6666_6666_6666
            };
            assert_eq!(&snapshots[index].xmm[..8], expected.to_le_bytes());
        }
        Ok::<_, NativeSessionError>(())
    })
    .unwrap();
}
