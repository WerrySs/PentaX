// SPDX-License-Identifier: GPL-2.0-only

//! Closed original instruction matrix in a separately supervised test process.
//! The reference executes ordinary instructions in the real native cache; the
//! candidate models FS operands over the same owned scratch bytes. No compiler
//! intrinsics, duplicated flag formulas, host FS changes or title data are used.

use super::*;
use pentax_core::process::hle::{NativeImageError, NativeImageLimits, with_native_image_session};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MATRIX_CHILD: &str = "PENTAX_TEST_SEGMENT_MATRIX_CHILD";
const TEST_NAME: &str = "native_session::segment_tests::segment_integer_results_and_defined_flags_match_real_native_instructions";
const SCRATCH: u32 = 0x100;
const STATUS: u64 = 0x8d5;
const AF: u64 = 0x10;

#[derive(Clone, Copy, Debug)]
enum Form {
    MemoryRegister,
    RegisterMemory,
    ImmediateByte,
    ImmediateFull,
    Unary,
}

#[derive(Clone, Copy, Debug)]
enum OperandRegister {
    Rax,
    Ah,
    R8,
}

#[derive(Clone, Copy, Debug)]
struct Spec {
    mnemonic: Mnemonic,
    width: usize,
    form: Form,
    register: OperandRegister,
}
impl Spec {
    fn logical(self) -> bool {
        matches!(
            self.mnemonic,
            Mnemonic::And | Mnemonic::Or | Mnemonic::Xor | Mnemonic::Test
        )
    }
    fn memory_write(self) -> bool {
        !matches!(self.mnemonic, Mnemonic::Cmp | Mnemonic::Test)
            && !matches!(self.form, Form::RegisterMemory)
    }

    // Original fixed encodings, independent of the modeled arithmetic. The
    // only address difference is [RSI] versus FS:[disp32].
    fn code(self, segment: bool) -> Vec<u8> {
        let mut code = Vec::new();
        if segment {
            code.push(0x64);
        }
        if self.width == 2 {
            code.push(0x66);
        }
        let register_form = matches!(self.form, Form::MemoryRegister | Form::RegisterMemory);
        let rex_r = register_form && matches!(self.register, OperandRegister::R8);
        if self.width == 8 || rex_r {
            code.push(0x40 | if self.width == 8 { 8 } else { 0 } | if rex_r { 4 } else { 0 });
        }
        let group = match self.mnemonic {
            Mnemonic::Add => 0,
            Mnemonic::Or => 1,
            Mnemonic::Adc => 2,
            Mnemonic::Sbb => 3,
            Mnemonic::And => 4,
            Mnemonic::Sub => 5,
            Mnemonic::Xor => 6,
            Mnemonic::Cmp => 7,
            Mnemonic::Test | Mnemonic::Inc => 0,
            Mnemonic::Dec => 1,
            Mnemonic::Not => 2,
            Mnemonic::Neg => 3,
            _ => unreachable!(),
        };
        let byte = self.width == 1;
        let register = if matches!(self.register, OperandRegister::Ah) {
            4
        } else {
            0
        };
        let (opcode, field) = match self.form {
            Form::MemoryRegister | Form::RegisterMemory => {
                let opcode = if self.mnemonic == Mnemonic::Test {
                    0x84
                } else {
                    group * 8
                        + if matches!(self.form, Form::RegisterMemory) {
                            2
                        } else {
                            0
                        }
                };
                (opcode + u8::from(!byte), register)
            }
            Form::ImmediateByte => (0x83, group),
            Form::ImmediateFull => (
                if self.mnemonic == Mnemonic::Test {
                    if byte { 0xf6 } else { 0xf7 }
                } else if byte {
                    0x80
                } else {
                    0x81
                },
                group,
            ),
            Form::Unary => (
                if matches!(self.mnemonic, Mnemonic::Inc | Mnemonic::Dec) {
                    if byte { 0xfe } else { 0xff }
                } else if byte {
                    0xf6
                } else {
                    0xf7
                },
                group,
            ),
        };
        code.push(opcode);
        code.push((field << 3) | if segment { 4 } else { 6 });
        if segment {
            code.push(0x25);
            code.extend_from_slice(&SCRATCH.to_le_bytes());
        }
        match self.form {
            Form::ImmediateByte => code.push(0x80),
            Form::ImmediateFull => {
                let immediate = 0x8000_0081_u32.to_le_bytes();
                code.extend_from_slice(&immediate[..self.width.min(4)]);
            }
            _ => {}
        }
        code.push(0xc3);
        code
    }
}

fn image(reference: &[u8], segment: &[u8]) -> Vec<u8> {
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
    bytes[0x100..0x100 + reference.len()].copy_from_slice(reference);
    bytes[0x200..0x200 + segment.len()].copy_from_slice(segment);
    bytes
}

#[test]
fn segment_integer_results_and_defined_flags_match_real_native_instructions() {
    if std::env::var_os(MATRIX_CHILD).as_deref() != Some(std::ffi::OsStr::new("1")) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(MATRIX_CHILD, "1")
            .creation_flags(0x0800_0000)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("original segment hardware matrix exceeded its thirty-second deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "hardware matrix failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        print!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    pentax_platform::configure_worker_fault_policy().unwrap();
    let mut pairs = 0;
    let mut encodings = 0;
    for width in [1, 2, 4, 8] {
        for mnemonic in [
            Mnemonic::Add,
            Mnemonic::Adc,
            Mnemonic::Sub,
            Mnemonic::Sbb,
            Mnemonic::And,
            Mnemonic::Or,
            Mnemonic::Xor,
            Mnemonic::Cmp,
            Mnemonic::Test,
            Mnemonic::Inc,
            Mnemonic::Dec,
            Mnemonic::Neg,
            Mnemonic::Not,
        ] {
            let unary = matches!(
                mnemonic,
                Mnemonic::Inc | Mnemonic::Dec | Mnemonic::Neg | Mnemonic::Not
            );
            let forms: &[Form] = if unary {
                &[Form::Unary]
            } else if mnemonic == Mnemonic::Test {
                &[Form::MemoryRegister, Form::ImmediateFull]
            } else if width == 1 {
                &[
                    Form::MemoryRegister,
                    Form::RegisterMemory,
                    Form::ImmediateFull,
                ]
            } else {
                &[
                    Form::MemoryRegister,
                    Form::RegisterMemory,
                    Form::ImmediateFull,
                    Form::ImmediateByte,
                ]
            };
            for &form in forms {
                pairs += compare(Spec {
                    mnemonic,
                    width,
                    form,
                    register: OperandRegister::Rax,
                });
                encodings += 1;
            }
            if !unary {
                for register in [OperandRegister::Ah, OperandRegister::R8] {
                    if width != 1 && matches!(register, OperandRegister::Ah) {
                        continue;
                    }
                    for form in [Form::MemoryRegister, Form::RegisterMemory] {
                        if mnemonic == Mnemonic::Test && matches!(form, Form::RegisterMemory) {
                            continue;
                        }
                        pairs += compare(Spec {
                            mnemonic,
                            width,
                            form,
                            register,
                        });
                        encodings += 1;
                    }
                }
            }
        }
    }
    println!(
        "Original FS integer hardware matrix: {encodings} encoding pairs, {pairs} state pairs, {} bounded runner calls",
        pairs * 2
    );
}

fn compare(spec: Spec) -> usize {
    let bytes = image(&spec.code(false), &spec.code(true));
    let limits = NativeImageLimits {
        stack_bytes: 16384,
        kernel_window_bytes: 65536,
        ..Default::default()
    };
    let mut pairs = 0;
    with_native_image_session(&[&bytes], &limits, |session, layout| {
        let proof = session.prepare_native_source(&Default::default())?;
        let entry = layout.entry;
        for segment in [false, true] {
            let planned = plan_materialized_block(
                proof.materialized_source(), 0,
                entry.entry + if segment { 0x100 } else { 0 },
                &spec.code(segment),
                pentax_codegen::CachePlacement { address: entry.entry + 16 * 1024 * 1024, capacity: 4096 },
                &TranslationPolicy::x64_sse2().with_isolated_memory(), &BlockLimits::default(),
            )?;
            let instruction = planned.instructions()[0].instruction();
            assert_eq!(instruction.mnemonic(), spec.mnemonic, "{spec:?}");
            assert_eq!(instruction.memory_size().size(), spec.width, "{spec:?}");
            assert_eq!(instruction.segment_prefix(), if segment { pentax_codegen::Register::FS } else { pentax_codegen::Register::None });
        }
        let stack = NativeStackFrame {
            stack: entry.stack, guards: Some(entry.guards),
            return_slot: entry.return_slot, return_marker: entry.return_marker,
        };
        let scratch = session.tls().instances()[0].thread_pointer + u64::from(SCRATCH);
        let mask = u64::MAX >> (64 - spec.width * 8);
        let sign = 1_u64 << (spec.width * 8 - 1);
        for (memory, register) in [(0, 0), (0, 1), (mask, 1), (mask, mask), (sign - 1, 1),
            (sign, 1), (sign, mask), (0xf, 1), (0x10, 1), (sign, sign)] {
            for flags in [0x202 | (STATUS & !1), 0x202 | 0x400 | 1] {
                let mut initial = NativeCpuState::new(entry.entry, entry.return_slot);
                initial.gpr[5] = entry.frame_pointer;
                initial.gpr[6] = scratch;
                initial.rflags = flags;
                initial.xmm.fill(0xa5);
                let slot = if matches!(spec.register, OperandRegister::R8) { 8 } else { 0 };
                let shift = if matches!(spec.register, OperandRegister::Ah) { 8 } else { 0 };
                initial.gpr[slot] = (0xbace_d00d_abcd_efab & !(mask << shift)) | ((register & mask) << shift);
                let original_memory = (0xa5a5_a5a5_a5a5_a5a5 & !mask) | (memory & mask);
                session.pause()?.write(scratch, &original_memory.to_le_bytes())?;
                let reference = run_prepared(session, &proof, initial, stack, &Default::default(),
                    &mut NativeSessionProgress::default())?;
                let mut reference_memory = [0; 8];
                session.pause()?.read(scratch, &mut reference_memory)?;
                session.pause()?.write(scratch, &original_memory.to_le_bytes())?;
                initial.guest_pc += 0x100;
                let modeled = run_prepared(session, &proof, initial, stack, &Default::default(),
                    &mut NativeSessionProgress::default())?;
                let mut modeled_memory = [0; 8];
                session.pause()?.read(scratch, &mut modeled_memory)?;
                let defined = if spec.logical() { !AF } else { u64::MAX };
                assert_eq!(modeled.state.gpr, reference.state.gpr, "registers {spec:?}, memory={memory:#x}, register={register:#x}, flags={flags:#x}");
                assert_eq!(modeled.state.rflags & defined, reference.state.rflags & defined,
                    "flags {spec:?}, memory={memory:#x}, register={register:#x}, flags={flags:#x}");
                if spec.logical() { assert_eq!(modeled.state.rflags & AF, flags & AF); }
                assert_eq!(modeled_memory, reference_memory, "memory {spec:?}");
                assert_eq!(modeled.state.xmm, reference.state.xmm);
                assert_eq!(modeled.state.mxcsr, reference.state.mxcsr);
                assert_eq!(modeled.state.guest_pc, entry.return_marker);
                assert_eq!(modeled.state.fs_base, reference.state.fs_base);
                assert_eq!(reference.dispatch.native_instructions, 1);
                assert_eq!(reference.primary_native_instructions, 1);
                assert_eq!(reference.dispatch.segment_operations, 0);
                assert_eq!(modeled.dispatch.native_instructions, 0);
                assert_eq!(modeled.dispatch.segment_operations, 1);
                assert_eq!(modeled.dispatch.segment_bytes, spec.width as u64 * (1 + u64::from(spec.memory_write())));
                pairs += 1;
            }
        }
        Ok::<_, NativeSessionError>(())
    }).unwrap_or_else(|error| match error {
        NativeImageError::Callback(error) => panic!("matrix execution {spec:?}: {error:?}"),
        error => panic!("matrix admission {spec:?}: {error:?}"),
    });
    pairs
}
