// SPDX-License-Identifier: GPL-2.0-only
#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

struct Input {
    path: PathBuf,
    bytes: Vec<u8>,
}

#[path = "image_probe/data_files.rs"]
mod data_files;

#[path = "image_probe/directories.rs"]
mod directories;

#[path = "image_probe/cache_hints.rs"]
mod cache_hints;

#[path = "image_probe/x87_data.rs"]
mod x87_data;

#[path = "image_probe/x87_arithmetic.rs"]
mod x87_arithmetic;

#[path = "image_probe/x87_comparisons.rs"]
mod x87_comparisons;

#[path = "image_probe/x87_integer.rs"]
mod x87_integer;

#[path = "image_probe/late_binding.rs"]
mod late_binding;

#[path = "image_probe/atomic_delta.rs"]
mod atomic_delta;

#[path = "image_probe/cxa_atexit.rs"]
mod cxa_atexit;

#[path = "image_probe/json_allocator.rs"]
mod json_allocator;

#[path = "image_probe/json_value.rs"]
mod json_value;

#[path = "image_probe/json_primitives.rs"]
mod json_primitives;

#[path = "image_probe/json_initializer.rs"]
mod json_initializer;

#[path = "image_probe/json_containers.rs"]
mod json_containers;

#[path = "image_probe/json_lifecycle.rs"]
mod json_lifecycle;

#[path = "image_probe/json_failure_attribution.rs"]
mod json_failure_attribution;

#[path = "image_probe/game_intent.rs"]
mod game_intent;
#[path = "image_probe/libc_mspace.rs"]
mod libc_mspace;
#[path = "image_probe/libc_snprintf.rs"]
mod libc_snprintf;
#[path = "image_probe/np_webapi2.rs"]
mod np_webapi2;
#[path = "image_probe/trophy2.rs"]
mod trophy2;
#[path = "image_probe/uds.rs"]
mod uds;

#[path = "image_probe/dependency_flag.rs"]
mod dependency_flag;

#[path = "image_probe/kernel_results.rs"]
mod kernel_results;
impl Drop for Input {
    fn drop(&mut self) {
        assert_eq!(
            std::fs::read(&self.path).unwrap(),
            self.bytes,
            "worker modified its input"
        );
        std::fs::remove_file(&self.path).unwrap();
    }
}
fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value)
}
fn input(code: &[u8], missing: bool) -> Input {
    module_input(code, missing, None)
}
fn module_input(code: &[u8], missing: bool, initializer: Option<u64>) -> Input {
    create_input(module_bytes(code, missing, initializer))
}
fn module_bytes(code: &[u8], missing: bool, initializer: Option<u64>) -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (offset, value) in [
        (16, 3_u16),
        (18, 62),
        (52, 64),
        (54, 56),
        (
            56,
            if missing || initializer.is_some() {
                2
            } else {
                1
            },
        ),
    ] {
        put(&mut bytes, offset, &value.to_le_bytes());
    }
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &0x100_u64.to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 64, &1_u32.to_le_bytes());
    put(&mut bytes, 68, &5_u32.to_le_bytes());
    for (offset, value) in [(96, 4096_u64), (104, 16384), (112, 16384)] {
        put(&mut bytes, offset, &value.to_le_bytes());
    }
    put(&mut bytes, 0x100, code);
    if missing {
        let at = 120;
        put(&mut bytes, at, &2_u32.to_le_bytes());
        put(&mut bytes, at + 4, &4_u32.to_le_bytes());
        for (offset, value) in [
            (8, 0x200_u64),
            (16, 0x200),
            (32, 11 * 16),
            (40, 11 * 16),
            (48, 8),
        ] {
            put(&mut bytes, at + offset, &value.to_le_bytes());
        }
        let names = b"\0original-module\0original-library\0AAAAAAAAAAA#D#C\0";
        put(&mut bytes, 0x400, names);
        let qualified = names
            .windows(11)
            .position(|value| value == b"AAAAAAAAAAA")
            .unwrap();
        put(&mut bytes, 0x618, &(qualified as u32).to_le_bytes());
        bytes[0x61c] = 0x12;
        put(&mut bytes, 0x680, &0xf00_u64.to_le_bytes());
        put(&mut bytes, 0x688, &((1_u64 << 32) | 7).to_le_bytes());
        for (index, (tag, value)) in [
            (5_i64, 0x400_u64),
            (10, names.len() as u64),
            (6, 0x600),
            (39, 48),
            (11, 24),
            (0x6100_000f, 1 | (3 << 32) | (2 << 40) | (2 << 48)),
            (0x6100_0015, 17 | (7 << 32) | (3 << 48)),
            (7, 0x680),
            (8, 24),
            (9, 24),
            (0, 0),
        ]
        .into_iter()
        .enumerate()
        {
            put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
            put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
        }
    }
    if let Some(initializer) = initializer {
        let records = if missing { 12 } else { 2 };
        for (offset, value) in [
            (8, 0x200_u64),
            (16, 0x200),
            (32, records * 16),
            (40, records * 16),
            (48, 8),
        ] {
            put(&mut bytes, 120 + offset, &value.to_le_bytes());
        }
        put(&mut bytes, 120, &2_u32.to_le_bytes());
        put(&mut bytes, 124, &4_u32.to_le_bytes());
        let record = if missing { 0x2a0 } else { 0x200 };
        put(&mut bytes, record, &12_i64.to_le_bytes());
        put(&mut bytes, record + 8, &initializer.to_le_bytes());
    }
    bytes
}
fn create_input(bytes: Vec<u8>) -> Input {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    for _ in 0..16 {
        let path = std::env::temp_dir().join(format!(
            "pentax-entry-probe-{}-{}.elf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().create_new(true).write(true).open(&path) {
            Ok(mut file) => {
                file.write_all(&bytes).unwrap();
                drop(file);
                return Input { path, bytes };
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => panic!("cannot create original input: {error}"),
        }
    }
    panic!("original input name collisions exceeded bound")
}

fn parameter_bytes(code: &[u8], header: Option<u32>) -> Vec<u8> {
    let mut bytes = module_bytes(code, true, None);
    // Exact PUBLIC pinned KernelGetProcParam identity, not a game-derived NID.
    let names = b"\0libkernel\0libkernel\0959qrazPIrg#D#C\0";
    put(&mut bytes, 16, &0xfe10_u16.to_le_bytes());
    put(&mut bytes, 0x400, names);
    put(&mut bytes, 0x218, &(names.len() as u64).to_le_bytes());
    put(&mut bytes, 0x618, &21_u32.to_le_bytes());
    put(
        &mut bytes,
        0x258,
        &(1_u64 | (1 << 32) | (1 << 40) | (2 << 48)).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x268,
        &(11_u64 | (1 << 32) | (3 << 48)).to_le_bytes(),
    );
    if let Some(kind) = header {
        put(&mut bytes, 56, &3_u16.to_le_bytes());
        put(&mut bytes, 176, &kind.to_le_bytes());
        put(&mut bytes, 180, &4_u32.to_le_bytes());
        for (offset, value) in [(8, 0xe00_u64), (16, 0xe00), (32, 16), (40, 16), (48, 8)] {
            put(&mut bytes, 176 + offset, &value.to_le_bytes());
        }
        put(&mut bytes, 0xe00, &42_u64.to_le_bytes());
        put(&mut bytes, 0xe08, &77_u64.to_le_bytes());
    }
    bytes
}

fn parameter_call(read_word: bool) -> Vec<u8> {
    let mut code = vec![0x55, 0xff, 0x15];
    code.extend_from_slice(&(0xf00_i32 - 0x107).to_le_bytes());
    if read_word {
        code.extend_from_slice(&[0x48, 0x8b, 0x00]);
    }
    code.extend_from_slice(&[0x5d, 0xc3]);
    code
}

fn lookup_provider(object: bool) -> Input {
    let mut bytes = module_bytes(&[0xb8, 42, 0, 0, 0, 0xc3], true, None);
    let name = bytes
        .windows(11)
        .position(|value| value == b"AAAAAAAAAAA")
        .unwrap();
    put(&mut bytes, name, b"QDLarCRo4Bc#f#R");
    put(&mut bytes, 0x250, &0x6100_000d_i64.to_le_bytes());
    put(
        &mut bytes,
        0x258,
        &(1_u64 | (3 << 32) | (2 << 40) | (17 << 48)).to_le_bytes(),
    );
    put(&mut bytes, 0x260, &0x6100_0013_i64.to_le_bytes());
    put(
        &mut bytes,
        0x268,
        &(17_u64 | (7 << 32) | (31 << 48)).to_le_bytes(),
    );
    bytes[0x61d] = 3; // GLOBAL/PROTECTED definition in this image's own scope.
    put(&mut bytes, 0x61e, &1_u16.to_le_bytes());
    put(&mut bytes, 0x620, &0x100_u64.to_le_bytes());
    put(&mut bytes, 0x628, &6_u64.to_le_bytes());
    if object {
        bytes[0x61c] = 0x11;
        put(&mut bytes, 0x620, &0xe00_u64.to_le_bytes());
        put(&mut bytes, 0x628, &8_u64.to_le_bytes());
        put(&mut bytes, 0xe00, &42_u64.to_le_bytes());
        put(&mut bytes, 0x688, &((1_u64 << 32) | 6).to_le_bytes());
    }
    create_input(bytes)
}

enum LookupUse {
    Call,
    ReadObject,
    CheckMissing,
}
fn lookup_consumer(handle: u32, name: &[u8], usage: LookupUse) -> Input {
    let mut code = vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 32, 0xbf];
    code.extend_from_slice(&handle.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x8d, 0x35]);
    let after_name = 0x100 + code.len() as i32 + 4;
    code.extend_from_slice(&(0xd00_i32 - after_name).to_le_bytes());
    code.extend_from_slice(&[0x48, 0x8d, 0x54, 0x24, 8]);
    code.extend_from_slice(&[0x48, 0xc7, 0x44, 0x24, 8, 0x55, 0, 0, 0]);
    code.extend_from_slice(&[0xff, 0x15]);
    let after_call = 0x100 + code.len() as i32 + 4;
    code.extend_from_slice(&(0xf00_i32 - after_call).to_le_bytes());
    match usage {
        LookupUse::Call => code.extend_from_slice(&[0x48, 0x8b, 0x44, 0x24, 8, 0xff, 0xd0]),
        LookupUse::ReadObject => {
            code.extend_from_slice(&[0x48, 0x8b, 0x44, 0x24, 8, 0x48, 0x8b, 0])
        }
        LookupUse::CheckMissing => {
            // A known missing export/handle returns ESRCH AND clears the output.
            // CMP preserves the returned EAX; an uncleared output reaches UD2.
            code.extend_from_slice(&[0x48, 0x83, 0x7c, 0x24, 8, 0, 0x75, 6]);
        }
    }
    code.extend_from_slice(&[0x48, 0x83, 0xc4, 32, 0x5d, 0xc3, 0x0f, 0x0b]);
    let mut bytes = parameter_bytes(&code, None);
    put(&mut bytes, 0x400 + 21, b"LwG8g3niqwA"); // Exact public KernelDlsym NID.
    put(&mut bytes, 0xd00, name);
    create_input(bytes)
}

#[test]
fn loaded_module_symbol_lookup_returns_a_real_function_that_the_guest_then_calls() {
    for object in [false, true] {
        let main = lookup_consumer(
            2,
            b"pentax_original_return42\0",
            if object {
                LookupUse::ReadObject
            } else {
                LookupUse::Call
            },
        );
        let provider = lookup_provider(object);
        let (code, fields, text) = probe_images(&[("--image", &main), ("--image", &provider)]);
        assert_eq!(code, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], "1");
        assert_eq!(fields["unavailable_functions"], "0");
        let native: u64 = fields["native_instructions"].parse().unwrap();
        let primary: u64 = fields["primary_native_instructions"].parse().unwrap();
        // One real generated marker MOV plus the actual library function MOV.
        assert_eq!(native, primary + if object { 1 } else { 2 });
    }
}

#[test]
fn missing_module_exports_resume_with_exact_source_error_and_a_real_null_output() {
    let provider = lookup_provider(false);
    for (handle, name) in [
        (1, b"pentax_original_return42\0".as_slice()),
        (3, b"pentax_original_return42\0"),
        (2, b"original_missing_export\0"),
    ] {
        let main = lookup_consumer(handle, name, LookupUse::CheckMissing);
        let (code, fields, text) = probe_images(&[("--image", &main), ("--image", &provider)]);
        assert_eq!(code, 0, "{text}");
        assert_eq!(fields["value"], "2147614723"); // EAX=0x80020003, source ESRCH.
        assert_eq!(fields["service_stops"], "1");
    }
}

#[test]
fn process_parameter_service_returns_actual_primary_bytes_through_the_attested_kernel_marker() {
    let source = create_input(parameter_bytes(&parameter_call(true), Some(0x6100_0001)));
    let (code, fields, text) = probe(&source);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "1");
    assert_eq!(fields["primary_native_instructions"], "3");
    assert_eq!(fields["unavailable_functions"], "0");
}

#[test]
fn null_parameter_result_is_actual_primary_absence_and_never_a_provider_substitution() {
    for header in [None, Some(0x6100_0002)] {
        let source = create_input(parameter_bytes(&parameter_call(false), header));
        let (code, fields, text) = probe(&source);
        assert_eq!(code, 0, "{text}");
        assert_eq!(fields["value"], "0");
        assert_eq!(fields["service_stops"], "1");
    }
    let source = create_input(parameter_bytes(&parameter_call(false), None));
    let mut provider = parameter_bytes(&[0xc3], Some(0x6100_0001));
    put(&mut provider, 16, &3_u16.to_le_bytes());
    let provider = create_input(provider);
    let (code, fields, text) = probe_images(&[("--image", &source), ("--image", &provider)]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "0");
    assert_eq!(fields["service_stops"], "1");
}

fn probe(input: &Input) -> (i32, HashMap<String, String>, String) {
    probe_images(&[("--image", input)])
}

#[test]
fn small_source_windows_retry_a_split_instruction_before_native_execution() {
    let mut code = vec![0x90; 60];
    code.extend_from_slice(&[0x48, 0xb8]); // MOV RAX, imm64 straddles byte 64.
    code.extend_from_slice(&42_u64.to_le_bytes());
    code.push(0xc3);
    let source = input(&code, false);
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["native_instructions"], "61");
    assert_eq!(fields["block_attempts"], "2");
    assert_eq!(fields["fetched_source_bytes"], "128");
    assert!(!fields.contains_key("budget"));
}

#[test]
fn startup_execution_profile_passes_the_probe_cap_and_reports_observed_progress() {
    // Original counted loop: MOV ECX,12000; DEC ECX; JNZ; MOV EAX,42; RET.
    let source = input(
        &[
            0xb9, 0xe0, 0x2e, 0, 0, 0xff, 0xc9, 0x75, 0xfc, 0xb8, 42, 0, 0, 0, 0xc3,
        ],
        false,
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["budget"], "dispatch-blocks");
    assert_eq!(fields["block_attempts"], "10000");
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "startup".into(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["native_instructions"], "12002");
    assert_eq!(fields["block_attempts"], "12001");
    let records: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tPROGRESS\t"))
        .collect();
    assert_eq!(records.len(), 5);
    for (index, line) in records.iter().enumerate() {
        assert!(line.contains(&format!("\tblock_attempts={}\t", (index + 1) * 2048)));
        assert!(!line.contains("value="));
    }
}

#[test]
fn plan_cache_hits_still_fetch_source_and_execute_current_memory_operands() {
    let code = [
        0x48, 0x83, 0xec, 0x20, // SUB RSP,32
        0x48, 0xc7, 0x04, 0x24, 0, 0, 0, 0, // MOV qword[RSP],0
        0xb9, 0xe8, 3, 0, 0, // MOV ECX,1000
        0x48, 0x83, 0x04, 0x24, 1, // ADD qword[RSP],1
        0xff, 0xc9, 0x75, 0xf7, // DEC ECX; JNZ to ADD
        0x48, 0x8b, 0x04, 0x24, // MOV RAX,[RSP]
        0x48, 0x83, 0xc4, 0x20, 0xc3,
    ];
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "1000");
    assert!(fields["plan_cache_hits"].parse::<u64>().unwrap() > 900);
    assert!(fields["plan_cache_misses"].parse::<u64>().unwrap() < 32);
    assert!(fields["cache_reuses"].parse::<u64>().unwrap() > 900);
    assert!(fields["block_attempts"].parse::<u64>().unwrap() < 1100);
    assert!(fields["cache_publications"].parse::<u64>().unwrap() < 32);
    assert_eq!(fields["native_entries"], fields["native_body_blocks"]);
    assert_eq!(
        fields["native_entries"].parse::<u64>().unwrap(),
        fields["cache_publications"].parse::<u64>().unwrap()
            + fields["cache_reuses"].parse::<u64>().unwrap()
    );
    assert_eq!(
        fields["fetched_source_bytes"].parse::<u64>().unwrap(),
        fields["block_attempts"].parse::<u64>().unwrap() * 64
    );
}

#[test]
fn native_cache_exit_only_return_completes_without_hardware_entry() {
    let (status, fields, text) = probe(&input(&[0xc3], false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "0");
    assert_eq!(fields["block_attempts"], "1");
    assert_eq!(fields["native_instructions"], "0");
    assert_eq!(fields["native_entries"], "0");
    assert_eq!(fields["cache_publications"], "0");
    assert_eq!(fields["exit_only_blocks"], "1");
}

#[test]
fn independent_memory_batch_executes_aliasing_stores_and_loads_in_order() {
    let code = [
        0x48, 0x83, 0xec, 32, // SUB RSP,32; split before using updated RSP.
        0x48, 0xc7, 0x04, 0x24, 9, 0, 0, 0, // MOV qword[RSP],9.
        0x48, 0x8b, 0x04, 0x24, // MOV RAX,[RSP].
        0x48, 0x83, 0xc0, 33, // ADD RAX,33.
        0x48, 0x89, 0x44, 0x24, 8, // MOV [RSP+8],RAX.
        0x48, 0x8b, 0x44, 0x24, 8, // MOV RAX,[RSP+8].
        0x48, 0x83, 0xc4, 32, 0xc3,
    ];
    let (status, fields, text) = probe(&input(&code, false));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["native_instructions"], "7");
    assert_eq!(fields["block_attempts"], "2");
}

#[test]
fn execution_profiles_reject_unknown_or_repeated_options_before_intake() {
    for tail in [
        vec!["--execution-profile", "unbounded"],
        vec![
            "--execution-profile",
            "startup",
            "--execution-profile",
            "probe",
        ],
    ] {
        let mut args = vec!["--image".into(), "original-nonexistent.elf".into()];
        args.extend(tail.into_iter().map(OsString::from));
        let (status, fields, text) = probe_arguments(&args);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "image-arguments");
        assert_eq!(fields["native_instructions"], "0");
    }
}

#[test]
fn extended_startup_runs_beyond_one_hundred_thousand_blocks_with_bounded_progress() {
    let mut code = vec![0xb9];
    code.extend(140_100_u32.to_le_bytes());
    code.extend([0xff, 0xc9, 0x75, 0xfc, 0xb8, 42, 0, 0, 0, 0xc3]);
    let source = input(&code, false);
    for profile in ["extended-startup", "long-startup"] {
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            source.path.as_os_str().to_owned(),
            "--execution-profile".into(),
            profile.into(),
        ]);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["block_attempts"], "140101");
        let records: Vec<_> = text
            .lines()
            .filter(|line| line.starts_with("PENTAX_WORKER\t1\tPROGRESS\t"))
            .collect();
        assert!(records.is_empty()); // Below the extended progress interval.
    }
}

fn probe_images(images: &[(&str, &Input)]) -> (i32, HashMap<String, String>, String) {
    let arguments: Vec<OsString> = images
        .iter()
        .flat_map(|(flag, input)| [OsString::from(flag), input.path.as_os_str().to_owned()])
        .collect();
    probe_arguments(&arguments)
}

fn probe_arguments(arguments: &[OsString]) -> (i32, HashMap<String, String>, String) {
    // Reuse these actual-file cases against a verified extracted package.
    let worker = std::env::var_os("PENTAX_TEST_WORKER")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_pentax-worker").into());
    let mut command = Command::new(worker);
    command.arg("probe-entry");
    command.args(arguments);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("image worker exceeded deadline")
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.stderr.is_empty());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        !text.contains("original-module")
            && !text.contains("AAAAAAAAAAA")
            && !text.contains(".elf")
            && !text.contains("/app0/")
    );
    let results: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tRESULT\t"))
        .collect();
    assert_eq!(results.len(), 1, "{text}");
    let fields = results[0]
        .split('\t')
        .skip(4)
        .map(|field| field.split_once('=').unwrap())
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    (output.status.code().unwrap(), fields, text)
}

fn retain_consumer(flags: u32, path: &[u8]) -> Input {
    retain_consumer_result(flags, path, 0)
}

fn retain_consumer_result(flags: u32, path: &[u8], first_result: u8) -> Input {
    let mut code = vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 32];
    let mut branches = vec![];
    for output in [true, false] {
        code.extend([0x48, 0x8d, 0x3d]);
        code.extend_from_slice(&(0xd00_i32 - (0x100 + code.len() as i32 + 4)).to_le_bytes());
        // Existing-program source branch does not dereference args/argp.
        code.extend([0xbe, 17, 0, 0, 0, 0xba, 0xad, 0xde, 0, 0, 0xb9]);
        code.extend_from_slice(&flags.to_le_bytes());
        code.extend([0x45, 0x31, 0xc0]);
        if output {
            code.extend([
                0x4c, 0x8d, 0x4c, 0x24, 8, 0xc7, 0x44, 0x24, 8, 0x55, 0, 0, 0,
            ]);
        } else {
            code.extend([0x45, 0x31, 0xc9]);
        }
        code.extend([0xff, 0x15]);
        code.extend_from_slice(&(0xf00_i32 - (0x100 + code.len() as i32 + 4)).to_le_bytes());
        code.extend([0x83, 0xf8, 2, 0x75, 0]);
        branches.push(code.len() - 1);
        if output {
            code.extend([0x83, 0x7c, 0x24, 8, first_result, 0x75, 0]);
            branches.push(code.len() - 1);
        }
    }
    // The initializer increments this real TCB field once. A repeated call
    // would change the value; successful retaining must not run it again.
    code.extend([
        0x64, 0x48, 0x8b, 4, 0x25, 0x28, 0, 0, 0, 0x83, 0xf8, 1, 0x75, 0,
    ]);
    branches.push(code.len() - 1);
    code.extend([0x83, 0xc0, 41, 0x48, 0x83, 0xc4, 32, 0x5d, 0xc3]);
    let failure = code.len();
    code.extend([0x0f, 0x0b]);
    for branch in branches {
        code[branch] = u8::try_from(failure - branch - 1).unwrap();
    }
    assert!(code.len() < 256);
    let mut bytes = parameter_bytes(&code, None);
    put(&mut bytes, 0x415, b"wzvqT4UqKX8"); // Exact public LoadStartModule registration.
    put(&mut bytes, 0xd00, path);
    create_input(bytes)
}

fn retain_preload(initializer: bool) -> Input {
    module_input(
        &[
            0x64, 0x48, 0x8b, 4, 0x25, 0x28, 0, 0, 0, 0x83, 0xc0, 1, 0x64, 0x48, 0x89, 4, 0x25,
            0x28, 0, 0, 0, 0xb8, 7, 0, 0, 0, 0xc3,
        ],
        false,
        initializer.then_some(0x100),
    )
}

fn probe_named_preload(main: &Input, library: &Input) -> (i32, HashMap<String, String>, String) {
    probe_arguments(&[
        "--image".into(),
        main.path.as_os_str().to_owned(),
        "--preload-libc-as".into(),
        format!(
            "/app0/sce_module/pentax-original.sprx={}",
            library.path.display()
        )
        .into(),
    ])
}

fn probe_dynamic(
    main: &Input,
    modules: &[(&str, &Input)],
) -> (i32, HashMap<String, String>, String) {
    let mut args = vec!["--image".into(), main.path.as_os_str().to_owned()];
    for (path, module) in modules {
        args.push("--module-file".into());
        args.push(format!("{path}={}", module.path.display()).into());
    }
    probe_arguments(&args)
}

#[test]
fn requested_new_file_runs_its_actual_initializer_and_resumes_the_same_caller_and_thread() {
    let main = retain_consumer_result(0, b"/app0/sce_module/pentax-original.sprx\0", 7);
    let library = retain_preload(true);
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "2");
    assert_eq!(fields["phase"], "main-entry");
}

#[test]
fn runtime_memory_profile_keeps_dynamic_code_within_relative_cache_reach() {
    let main = retain_consumer_result(0, b"/app0/sce_module/pentax-original.sprx\0", 7);
    let library = retain_preload(true);
    let (code, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.as_os_str().to_owned(),
        "--memory-profile".into(),
        "runtime".into(),
        "--module-file".into(),
        format!(
            "/app0/sce_module/pentax-original.sprx={}",
            library.path.display()
        )
        .into(),
    ]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["phase"], "main-entry");
}

#[test]
fn runtime_memory_profile_rejects_unknown_or_repeated_options_before_intake() {
    for suffix in [
        vec!["--memory-profile", "unknown"],
        vec!["--memory-profile", "runtime", "--memory-profile", "compact"],
    ] {
        let mut arguments = vec!["--image".into(), "does-not-exist.elf".into()];
        arguments.extend(suffix.into_iter().map(OsString::from));
        let (code, fields, text) = probe_arguments(&arguments);
        assert_ne!(code, 0, "{text}");
        assert_eq!(fields["code"], "image-arguments");
    }
}

fn rip(code: &mut Vec<u8>, base: i32, opcode: &[u8], target: i32) {
    code.extend_from_slice(opcode);
    code.extend_from_slice(&(target - (base + code.len() as i32 + 4)).to_le_bytes());
}
fn checked(code: &mut Vec<u8>, branches: &mut Vec<usize>) {
    code.extend([0x0f, 0x85, 0, 0, 0, 0]);
    branches.push(code.len() - 4);
}
fn finish_checks(code: &mut Vec<u8>, branches: Vec<usize>) {
    let failure = code.len();
    code.extend([0x0f, 0x0b]);
    for branch in branches {
        let relative = (failure - branch - 4) as i32;
        code[branch..branch + 4].copy_from_slice(&relative.to_le_bytes());
    }
}

fn dynamic_relocated_library(object: bool) -> Input {
    let original = lookup_provider(object);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x4008, 0);
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    put(&mut bytes, 176, &1_u32.to_le_bytes());
    put(&mut bytes, 180, &6_u32.to_le_bytes());
    for (offset, value) in [
        (8, 0x4000_u64),
        (16, 0x4000),
        (32, 8),
        (40, 0x4000),
        (48, 0x4000),
    ] {
        put(&mut bytes, 176 + offset, &value.to_le_bytes());
    }
    for offset in [152, 160] {
        put(&mut bytes, offset, &(12_u64 * 16).to_le_bytes());
    }
    put(&mut bytes, 0x2a0, &12_i64.to_le_bytes());
    put(&mut bytes, 0x2a8, &0x100_u64.to_le_bytes());
    put(&mut bytes, 0x680, &0x4000_u64.to_le_bytes());
    put(&mut bytes, 0x688, &8_u64.to_le_bytes());
    put(&mut bytes, 0x690, &0x4010_u64.to_le_bytes());
    let mut init = vec![0x83, 0xff, 17];
    let mut branches = Vec::new();
    checked(&mut init, &mut branches);
    init.extend([0x81, 0xfe, 0xad, 0xde, 0, 0]);
    checked(&mut init, &mut branches);
    init.extend([0x48, 0x85, 0xd2]);
    checked(&mut init, &mut branches);
    init.extend([0x64, 0x48, 0x8b, 4, 0x25, 0x28, 0, 0, 0, 0x83, 0xf8, 13]);
    checked(&mut init, &mut branches);
    init.extend([0x83, 0xc0, 1, 0x64, 0x48, 0x89, 4, 0x25, 0x28, 0, 0, 0]);
    rip(&mut init, 0x100, &[0x48, 0x8b, 5], 0x4000);
    init.extend([0x48, 0x83, 0x38, 0]);
    checked(&mut init, &mut branches);
    init.extend([0x48, 0xc7, 0, 42, 0, 0, 0, 0xb8, 7, 0, 0, 0, 0xc3]);
    finish_checks(&mut init, branches);
    assert!(init.len() < 256);
    put(&mut bytes, 0x100, &init);
    let mut function = Vec::new();
    rip(&mut function, 0x300, &[0x48, 0x8b, 5], 0x4000);
    function.extend([0x48, 0x8b, 0, 0xc3]);
    put(&mut bytes, 0x300, &function);
    put(
        &mut bytes,
        0x620,
        &(if object { 0x4010_u64 } else { 0x300 }).to_le_bytes(),
    );
    put(
        &mut bytes,
        0x628,
        &(if object { 8 } else { function.len() as u64 }).to_le_bytes(),
    );
    create_input(bytes)
}

fn dynamic_lookup_main(object: bool) -> Input {
    let base = 0x800;
    let mut code = vec![
        0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 32, 0xb8, 13, 0, 0, 0, 0x64, 0x48, 0x89, 4, 0x25,
        0x28, 0, 0, 0, 0x48, 0xc7, 0x44, 0x24, 16, 37, 0, 0, 0,
    ];
    let mut branches = Vec::new();
    for result in [7, 0] {
        rip(&mut code, base, &[0x48, 0x8d, 0x3d], 0xd00);
        code.extend([
            0xbe, 17, 0, 0, 0, 0xba, 0xad, 0xde, 0, 0, 0x31, 0xc9, 0x45, 0x31, 0xc0, 0x4c, 0x8d,
            0x4c, 0x24, 8, 0xc7, 0x44, 0x24, 8, 0x55, 0, 0, 0,
        ]);
        rip(&mut code, base, &[0xff, 0x15], 0xf00);
        code.extend([0x83, 0xf8, 2]);
        checked(&mut code, &mut branches);
        code.extend([0x83, 0x7c, 0x24, 8, result]);
        checked(&mut code, &mut branches);
    }
    code.extend([0x64, 0x48, 0x8b, 4, 0x25, 0x28, 0, 0, 0, 0x83, 0xf8, 14]);
    checked(&mut code, &mut branches);
    code.extend([0xbf, 2, 0, 0, 0]);
    rip(&mut code, base, &[0x48, 0x8d, 0x35], 0xd40);
    code.extend([0x48, 0x8d, 0x54, 0x24, 8]);
    rip(&mut code, base, &[0xff, 0x15], 0xf08);
    code.extend([0x85, 0xc0]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x8b, 0x44, 0x24, 8]);
    code.extend_from_slice(if object {
        &[0x48, 0x8b, 0]
    } else {
        &[0xff, 0xd0]
    });
    code.extend([0x83, 0xf8, 42]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x83, 0x7c, 0x24, 16, 37]);
    checked(&mut code, &mut branches);
    code.extend([0x48, 0x83, 0xc4, 32, 0x5d, 0xc3]);
    finish_checks(&mut code, branches);
    let mut bytes = parameter_bytes(&[], None);
    put(&mut bytes, 24, &(base as u64).to_le_bytes());
    put(&mut bytes, base as usize, &code);
    let names = b"\0libkernel\0libkernel\0wzvqT4UqKX8#D#C\0LwG8g3niqwA#D#C\0";
    put(&mut bytes, 0x400, names);
    put(&mut bytes, 0x218, &(names.len() as u64).to_le_bytes());
    put(&mut bytes, 0x238, &72_u64.to_le_bytes());
    put(&mut bytes, 0x630, &37_u32.to_le_bytes());
    bytes[0x634] = 0x12;
    put(&mut bytes, 0x698, &0xf08_u64.to_le_bytes());
    put(&mut bytes, 0x6a0, &((2_u64 << 32) | 7).to_le_bytes());
    put(&mut bytes, 0x288, &48_u64.to_le_bytes());
    put(
        &mut bytes,
        0xd00,
        b"/app0/sce_module/pentax-original.sprx\0",
    );
    put(&mut bytes, 0xd40, b"pentax_original_return42\0");
    create_input(bytes)
}

#[test]
fn new_module_relocates_initializes_bss_preserves_tls_and_exposes_real_exports() {
    for object in [false, true] {
        let main = dynamic_lookup_main(object);
        let library = dynamic_relocated_library(object);
        let (code, fields, text) = probe_dynamic(
            &main,
            &[("/app0/sce_module/pentax-original.sprx", &library)],
        );
        assert_eq!(code, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["initializer_calls"], "1");
        assert_eq!(fields["service_stops"], "3");
        assert_eq!(fields["phase"], "main-entry");
    }
}

fn add_tls_header(bytes: &mut [u8], at: usize, source: u64, initialized: u64, memory: u64) {
    put(bytes, at, &7_u32.to_le_bytes());
    put(bytes, at + 4, &4_u32.to_le_bytes());
    for (field, value) in [
        (8, source),
        (16, source),
        (32, initialized),
        (40, memory),
        (48, 16),
    ] {
        put(bytes, at + field, &value.to_le_bytes());
    }
}

fn new_tls_library() -> Input {
    let original = dynamic_relocated_library(false);
    let mut bytes = original.bytes.clone();
    bytes.resize(0x4068, 0);
    put(&mut bytes, 56, &4_u16.to_le_bytes());
    put(&mut bytes, 208, &0x68_u64.to_le_bytes());
    add_tls_header(&mut bytes, 232, 0x4040, 8, 32);
    put(&mut bytes, 0x2a8, &0x800_u64.to_le_bytes());
    put(&mut bytes, 0x288, &72_u64.to_le_bytes());
    put(&mut bytes, 0x698, &0x4040_u64.to_le_bytes());
    put(&mut bytes, 0x6a0, &8_u64.to_le_bytes());
    put(&mut bytes, 0x6a8, &0x4010_u64.to_le_bytes());
    put(&mut bytes, 0x6b0, &0x4060_u64.to_le_bytes());
    put(&mut bytes, 0x6b8, &((2_u64 << 32) | 18).to_le_bytes());
    // An original TLS definition in the same image's exact declared scope.
    let names_size = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap());
    put(
        &mut bytes,
        0x400 + names_size as usize,
        b"AAAAAAAAAAA#f#R\0",
    );
    put(&mut bytes, 0x218, &(names_size + 16).to_le_bytes());
    put(&mut bytes, 0x238, &72_u64.to_le_bytes());
    put(&mut bytes, 0x630, &(names_size as u32).to_le_bytes());
    bytes[0x634] = 0x16;
    bytes[0x635] = 3;
    put(&mut bytes, 0x636, &1_u16.to_le_bytes());
    put(&mut bytes, 0x640, &8_u64.to_le_bytes());
    let mut init = vec![0x64, 0x48, 0x8b, 4, 0x25];
    init.extend_from_slice(&(-32_i32).to_le_bytes());
    init.extend([0x83, 0xf8, 37]);
    let mut branches = Vec::new();
    checked(&mut init, &mut branches);
    init.extend([0xb8, 42, 0, 0, 0, 0x64, 0x48, 0x89, 4, 0x25]);
    init.extend_from_slice(&(-32_i32).to_le_bytes());
    rip(&mut init, 0x800, &[0x48, 0x8b, 0x0d], 0x4060);
    init.extend([0x64, 0x48, 0x8b, 1, 0x48, 0x83, 0x38, 0]);
    checked(&mut init, &mut branches);
    init.extend([
        0x48, 0xc7, 0, 42, 0, 0, 0, 0x64, 0x48, 0x8b, 0x41, 8, 0x48, 0x85, 0xc0,
    ]);
    checked(&mut init, &mut branches);
    init.extend([
        0x64, 0x48, 0xc7, 0x41, 8, 11, 0, 0, 0, 0xb8, 14, 0, 0, 0, 0x64, 0x48, 0x89, 4, 0x25, 0x28,
        0, 0, 0, 0xb8, 7, 0, 0, 0, 0xc3,
    ]);
    finish_checks(&mut init, branches);
    put(&mut bytes, 0x800, &init);
    let mut function = Vec::new();
    rip(&mut function, 0x300, &[0x48, 0x8b, 0x0d], 0x4060);
    function.extend([0x64, 0x48, 0x8b, 0x41, 8, 0x83, 0xf8, 11]);
    let mut branches = Vec::new();
    checked(&mut function, &mut branches);
    function.extend([0x64, 0x48, 0x8b, 4, 0x25]);
    function.extend_from_slice(&(-32_i32).to_le_bytes());
    function.extend([0x83, 0xf8, 42]);
    checked(&mut function, &mut branches);
    function.extend([0x64, 0x48, 0x8b, 1, 0x48, 0x8b, 0, 0xc3]);
    finish_checks(&mut function, branches);
    put(&mut bytes, 0x300, &function);
    put(&mut bytes, 0x628, &(function.len() as u64).to_le_bytes());
    create_input(bytes)
}

#[test]
fn newly_loaded_tls_uses_relocated_template_and_tpoff_without_resetting_existing_thread_storage() {
    let main = dynamic_lookup_main(false);
    let mut bytes = main.bytes.clone();
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    add_tls_header(&mut bytes, 176, 0xe00, 8, 32);
    put(&mut bytes, 0xe00, &37_u64.to_le_bytes());
    let main = create_input(bytes);
    let library = new_tls_library();
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "3");
}

// Exercise the previously unported FS comparisons directly inside a real new
// module initializer, without replacing the older MOV-based regression case.
fn new_tls_integer_library() -> Input {
    let original = new_tls_library();
    let mut bytes = original.bytes.clone();
    let mut init = vec![0x64, 0x48, 0x83, 0x3c, 0x25];
    init.extend_from_slice(&(-32_i32).to_le_bytes());
    init.push(37);
    let mut branches = Vec::new();
    checked(&mut init, &mut branches);
    init.extend([0x64, 0x48, 0x83, 0x04, 0x25]);
    init.extend_from_slice(&(-32_i32).to_le_bytes());
    init.push(5); // Existing thread TLS changes from 37 to 42 in place.
    rip(&mut init, 0x800, &[0x48, 0x8b, 0x0d], 0x4060);
    init.extend([0x64, 0x48, 0x83, 0x79, 8, 0]);
    checked(&mut init, &mut branches);
    init.extend([0x64, 0x48, 0xf7, 0x41, 8, 0xff, 0xff, 0xff, 0xff]);
    checked(&mut init, &mut branches);
    init.extend([0x64, 0x48, 0x83, 0x41, 8, 11]);
    init.extend([0x64, 0x48, 0x83, 0x79, 8, 11]);
    checked(&mut init, &mut branches);
    init.extend([0x64, 0x48, 0x8b, 1, 0x48, 0xc7, 0, 42, 0, 0, 0]);
    init.extend([0x64, 0x48, 0xc7, 4, 0x25, 0x28, 0, 0, 0, 14, 0, 0, 0]);
    init.extend([0xb8, 7, 0, 0, 0, 0xc3]);
    finish_checks(&mut init, branches);
    put(&mut bytes, 0x800, &init);
    let mut function = Vec::new();
    rip(&mut function, 0x300, &[0x48, 0x8b, 0x0d], 0x4060);
    function.extend([0x64, 0x48, 0x83, 0x79, 8, 11]);
    let mut branches = Vec::new();
    checked(&mut function, &mut branches);
    function.extend([0x64, 0x48, 0x83, 0x3c, 0x25]);
    function.extend_from_slice(&(-32_i32).to_le_bytes());
    function.push(42);
    checked(&mut function, &mut branches);
    function.extend([0x64, 0x48, 0x8b, 1, 0x48, 0x8b, 0, 0xc3]);
    finish_checks(&mut function, branches);
    put(&mut bytes, 0x300, &function);
    put(&mut bytes, 0x628, &(function.len() as u64).to_le_bytes());
    create_input(bytes)
}

#[test]
fn new_module_fs_integer_operations_drive_real_tls_initialization_branches_and_later_calls() {
    let main = tls_main();
    let library = new_tls_integer_library();
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "3");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
    assert_eq!(fields["segment_ops"], "13", "{text}");
}

fn tls_descriptor_library() -> Input {
    tls_descriptor_library_with_fault(None)
}
enum TlsDescriptorFault {
    Module(u64),
    Offset(u64),
    Null,
}
fn tls_descriptor_library_with_fault(fault: Option<TlsDescriptorFault>) -> Input {
    let original = new_tls_library();
    let mut bytes = original.bytes.clone();
    bytes.resize(0x4080, 0);
    put(&mut bytes, 208, &0x80_u64.to_le_bytes());
    let names_size = u64::from_le_bytes(bytes[0x218..0x220].try_into().unwrap());
    put(
        &mut bytes,
        0x400 + names_size as usize,
        b"libkernel\0vNe1w4diLCs#D#C\0",
    );
    put(&mut bytes, 0x218, &(names_size + 26).to_le_bytes());
    put(&mut bytes, 0x238, &96_u64.to_le_bytes());
    put(&mut bytes, 0x640, &32_u64.to_le_bytes());
    put(&mut bytes, 0x648, &((names_size + 10) as u32).to_le_bytes());
    bytes[0x64c] = 0x12;
    for (at, tag, value) in [
        (
            0x2b0,
            0x6100_000f_i64,
            names_size | (1 << 32) | (1 << 40) | (2 << 48),
        ),
        (0x2c0, 0x6100_0015, names_size | (1 << 32) | (3 << 48)),
        (0x2d0, 0, 0),
    ] {
        put(&mut bytes, at, &tag.to_le_bytes());
        put(&mut bytes, at + 8, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(14_u64 * 16).to_le_bytes());
    }
    for (at, target, info, addend) in [
        (0x6c8, 0x4070_u64, (2_u64 << 32) | 16, 0_u64),
        (0x6e0, 0x4078, (2 << 32) | 17, 8),
        (0x6f8, 0x4020, (3 << 32) | 7, 0),
    ] {
        put(&mut bytes, at, &target.to_le_bytes());
        put(&mut bytes, at + 8, &info.to_le_bytes());
        put(&mut bytes, at + 16, &addend.to_le_bytes());
    }
    put(&mut bytes, 0x288, &144_u64.to_le_bytes());
    let mut function = vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 16];
    match fault {
        Some(TlsDescriptorFault::Module(value)) | Some(TlsDescriptorFault::Offset(value)) => {
            function.extend([0x48, 0xb8]);
            function.extend_from_slice(&value.to_le_bytes());
            let target = if matches!(fault, Some(TlsDescriptorFault::Module(_))) {
                0x4070
            } else {
                0x4078
            };
            rip(&mut function, 0x300, &[0x48, 0x89, 5], target);
        }
        _ => {}
    }
    rip(&mut function, 0x300, &[0x48, 0x8d, 0x3d], 0x4070);
    if matches!(fault, Some(TlsDescriptorFault::Null)) {
        function.extend([0x31, 0xff]);
    }
    rip(&mut function, 0x300, &[0xff, 0x15], 0x4020);
    function.extend([0x48, 0x8b, 0, 0x83, 0xf8, 11]);
    let mut branches = Vec::new();
    checked(&mut function, &mut branches);
    function.extend([0x64, 0x48, 0x8b, 4, 0x25]);
    function.extend_from_slice(&(-32_i32).to_le_bytes());
    function.extend([0x83, 0xf8, 42]);
    checked(&mut function, &mut branches);
    rip(&mut function, 0x300, &[0x48, 0x8b, 0x0d], 0x4060);
    function.extend([
        0x64, 0x48, 0x8b, 1, 0x48, 0x8b, 0, 0x48, 0x83, 0xc4, 16, 0x5d, 0xc3,
    ]);
    finish_checks(&mut function, branches);
    put(&mut bytes, 0x300, &function);
    put(&mut bytes, 0x628, &(function.len() as u64).to_le_bytes());
    create_input(bytes)
}

#[test]
fn new_tls_descriptor_resolves_through_actual_kernel_marker_with_opaque_runtime_key_and_offset() {
    let main = dynamic_lookup_main(false);
    let mut bytes = main.bytes.clone();
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    add_tls_header(&mut bytes, 176, 0xe00, 8, 32);
    put(&mut bytes, 0xe00, &37_u64.to_le_bytes());
    let main = create_input(bytes);
    let library = tls_descriptor_library();
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "4");
}

fn tls_main() -> Input {
    let main = dynamic_lookup_main(false);
    let mut bytes = main.bytes.clone();
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    add_tls_header(&mut bytes, 176, 0xe00, 8, 32);
    put(&mut bytes, 0xe00, &37_u64.to_le_bytes());
    create_input(bytes)
}

#[test]
fn untrusted_tls_descriptors_never_become_host_pointers_or_guessed_success() {
    let main = tls_main();
    for fault in [
        TlsDescriptorFault::Module(0),
        TlsDescriptorFault::Module(u64::MAX),
        TlsDescriptorFault::Offset(32),
        TlsDescriptorFault::Offset(u64::MAX),
        TlsDescriptorFault::Null,
    ] {
        let library = tls_descriptor_library_with_fault(Some(fault));
        let (code, fields, text) = probe_dynamic(
            &main,
            &[("/app0/sce_module/pentax-original.sprx", &library)],
        );
        assert_eq!(code, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["initializer_calls"], "1", "{text}");
        assert_eq!(fields["service_stops"], "4", "{text}");
        assert!(!fields.contains_key("value"));
    }
}

#[test]
fn new_tls_descriptor_is_available_inside_its_actual_initializer_and_is_not_reinitialized_after_return()
 {
    let main = tls_main();
    let library = tls_descriptor_library();
    let mut bytes = library.bytes.clone();
    put(&mut bytes, 0x2a8, &0xa00_u64.to_le_bytes());
    let mut init = vec![0x55, 0x48, 0x89, 0xe5, 0x48, 0x83, 0xec, 16];
    rip(&mut init, 0xa00, &[0x48, 0x8d, 0x3d], 0x4070);
    rip(&mut init, 0xa00, &[0xff, 0x15], 0x4020);
    init.extend([0x48, 0x8b, 0, 0x48, 0x85, 0xc0]);
    let mut branches = Vec::new();
    checked(&mut init, &mut branches);
    rip(&mut init, 0xa00, &[0xe8], 0x800);
    init.extend([0x48, 0x83, 0xc4, 16, 0x5d, 0xc3]);
    finish_checks(&mut init, branches);
    put(&mut bytes, 0xa00, &init);
    let library = create_input(bytes);
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "5");
}

fn nested_loading_library() -> Input {
    let parent = retain_consumer_result(0, b"/app0/sce_module/pentax-child.sprx\0", 7);
    let mut bytes = parent.bytes.clone();
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 152, &(12_u64 * 16).to_le_bytes());
    put(&mut bytes, 160, &(12_u64 * 16).to_le_bytes());
    put(&mut bytes, 0x2a0, &12_i64.to_le_bytes());
    put(&mut bytes, 0x2a8, &0x100_u64.to_le_bytes());
    let mut replaced = 0;
    for at in 0x100..0x1fd {
        if bytes[at..at + 3] == [0x83, 0xf8, 2] {
            bytes[at + 2] = 3;
            replaced += 1;
        }
    }
    assert_eq!(replaced, 2);
    create_input(bytes)
}

#[test]
fn nested_new_file_initializers_return_in_order_without_restarting_either_module() {
    let main = retain_consumer_result(0, b"/app0/sce_module/pentax-original.sprx\0", 42);
    let parent = nested_loading_library();
    let child = retain_preload(true);
    let (code, fields, text) = probe_dynamic(
        &main,
        &[
            ("/app0/sce_module/pentax-original.sprx", &parent),
            ("/app0/sce_module/pentax-child.sprx", &child),
        ],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "2");
    assert_eq!(fields["service_stops"], "4");
    assert_eq!(fields["phase"], "main-entry");
}

#[test]
fn preload_completion_remains_bound_to_its_actual_return_after_a_nested_source_append() {
    let main = retain_consumer(0, b"/app0/sce_module/pentax-original.sprx\0");
    let parent = nested_loading_library();
    let child = retain_preload(true);
    let (code, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.as_os_str().to_owned(),
        "--module-file".into(),
        format!(
            "/app0/sce_module/pentax-child.sprx={}",
            child.path.display()
        )
        .into(),
        "--preload-libc-as".into(),
        format!(
            "/app0/sce_module/pentax-original.sprx={}",
            parent.path.display()
        )
        .into(),
    ]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "2");
    assert_eq!(fields["service_stops"], "4");
    assert_eq!(fields["phase"], "main-entry");
}

#[test]
fn new_module_failures_never_resume_the_caller_or_claim_initializer_completion() {
    let main = retain_consumer_result(0, b"/app0/sce_module/pentax-original.sprx\0", 7);
    for (library, category, phase) in [
        (
            module_input(&[0x0f, 0x0b], false, Some(0x100)),
            "unsupported-instruction",
            "initializer",
        ),
        (
            module_input(&[0xb8, 0xff, 0xff, 0xff, 0xff, 0xc3], false, Some(0x100)),
            "runtime-service",
            "initializer",
        ),
        (retain_preload(false), "runtime-service", "main-entry"),
        (
            module_input(&[0xc3], false, Some(0x3000)),
            "runtime-service",
            "main-entry",
        ),
    ] {
        let (code, fields, text) = probe_dynamic(
            &main,
            &[("/app0/sce_module/pentax-original.sprx", &library)],
        );
        assert_eq!(code, 2, "{text}");
        assert_eq!(fields["code"], category, "{text}");
        assert_eq!(fields["phase"], phase, "{text}");
        assert_eq!(fields["service_stops"], "1", "{text}");
        assert_eq!(fields["initializer_calls"], "0", "{text}");
        assert!(!fields.contains_key("value"));
    }
}

#[test]
fn unused_dynamic_capabilities_remain_unread_and_do_not_shift_the_preload_index() {
    let unused = create_input(b"original invalid executable, deliberately never selected".to_vec());
    let main = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let library = retain_preload(true);
    let args = [
        "--image".into(),
        main.path.as_os_str().to_owned(),
        "--module-file".into(),
        format!("/app0/unused.sprx={}", unused.path.display()).into(),
        "--preload-libc".into(),
        library.path.as_os_str().to_owned(),
    ];
    let (code, fields, text) = probe_arguments(&args);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "0");
}

#[test]
fn retained_initialized_module_loads_return_the_actual_handle_without_restarting_the_initializer() {
    let main = retain_consumer(0, b"/app0/sce_module/pentax-original.sprx\0");
    let library = retain_preload(true);
    let (code, fields, text) = probe_named_preload(&main, &library);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "2");
    assert_eq!(fields["unavailable_functions"], "0");
}

#[test]
fn retained_load_requires_completed_initialization_an_exact_binding_and_supported_flags() {
    for (flags, path, initialized) in [
        (
            0,
            b"/app0/sce_module/pentax-original.sprx\0".as_slice(),
            false,
        ),
        (0, b"/app0/sce_module/original-unbound.sprx\0", true),
        (1, b"/app0/sce_module/pentax-original.sprx\0", true),
    ] {
        let main = retain_consumer(flags, path);
        let library = retain_preload(initialized);
        let (code, fields, text) = probe_named_preload(&main, &library);
        assert_eq!(code, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service");
        assert_eq!(
            fields["initializer_calls"],
            if initialized { "1" } else { "0" }
        );
        assert_eq!(fields["service_stops"], "1");
        assert!(!fields.contains_key("value"));
    }
}

#[test]
fn supplied_original_image_returns_through_the_real_prepared_entry() {
    let source = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let (code, fields, text) = probe(&source);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["code"], "outer-return");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["primary_native_instructions"], "1");
    assert_eq!(fields["unavailable_functions"], "0");
}
#[test]
fn native_primary_progress_survives_an_explicit_unsupported_instruction_stop() {
    let source = input(&[0xb8, 42, 0, 0, 0, 0x0f, 0x0b], false);
    let (code, fields, text) = probe(&source);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction");
    assert_eq!(fields["primary_native_instructions"], "1");
    assert_eq!(fields["instruction"], "Ud2");
    assert!(!fields.contains_key("value"));
}
#[test]
fn unavailable_function_and_exit_callback_never_resume_with_a_fake_return() {
    let mut code = vec![0x55, 0xb8, 7, 0, 0, 0, 0xff, 0x15];
    code.extend_from_slice(&(0xf00_i32 - 0x10c).to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x0b]);
    let source = input(&code, true);
    let (code, fields, text) = probe(&source);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "unavailable-function");
    assert_eq!(fields["unavailable_functions"], "1");
    assert_eq!(fields["primary_native_instructions"], "2");
    assert_eq!(fields["service_stops"], "1");
    assert!(!fields.contains_key("value"));
    // Tail-transfer preserves the incoming SysV stack alignment and executes
    // no primary native body before the provider's own marker instruction.
    let callback = input(&[0xff, 0xe6, 0x0f, 0x0b], false);
    let (code, fields, text) = probe(&callback);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "module-exit-unavailable");
    assert_eq!(fields["primary_native_instructions"], "0");
    assert_eq!(fields["native_instructions"], "1");
}

#[test]
fn explicit_libc_initializer_returns_before_main_and_preserves_its_actual_thread_write() {
    // Check the observed three zero arguments, write a real TCB field, and
    // return a nonzero int. Kyty Execute does not replace it with success or
    // condition main entry on that value.
    let mut init = vec![];
    for register in [0xff, 0xf6, 0xd2] {
        init.extend_from_slice(&[0x48, 0x85, register, 0x75, 0]);
    }
    init.extend_from_slice(&[
        0xb8, 13, 0, 0, 0, 0x64, 0x48, 0x89, 0x04, 0x25, 0x28, 0, 0, 0, 0xb8, 7, 0, 0, 0, 0xc3,
        0x0f, 0x0b,
    ]);
    for branch in [3, 8, 13] {
        init[branch + 1] = (init.len() - 2 - branch - 2) as u8;
    }
    let library = module_input(&init, false, Some(0x100));
    let main = input(
        &[
            0x83, 0xf8, 7, 0x75, 13, 0x64, 0x48, 0x8b, 0x04, 0x25, 0x28, 0, 0, 0, 0x83, 0xc0, 29,
            0xc3, 0x0f, 0x0b,
        ],
        false,
    );
    let (code, fields, text) = probe_images(&[("--image", &main), ("--preload-libc", &library)]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["phase"], "main-entry");
    assert_eq!(fields["primary_native_instructions"], "2");
    assert_eq!(fields["segment_ops"], "2");
    // Mere presence in the link set never sweeps all modules' constructors.
    let main = input(
        &[
            0x64, 0x48, 0x8b, 0x04, 0x25, 0x28, 0, 0, 0, 0x83, 0xc0, 29, 0xc3,
        ],
        false,
    );
    let (code, fields, text) = probe_images(&[("--image", &main), ("--image", &library)]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "29");
    assert_eq!(fields["initializer_calls"], "0");
}

#[test]
fn initializer_stop_prevents_main_entry_and_preserves_honest_partial_progress() {
    let main = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let library = module_input(&[0xb8, 13, 0, 0, 0, 0x0f, 0x0b], false, Some(0x100));
    let (code, fields, text) = probe_images(&[("--image", &main), ("--preload-libc", &library)]);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction");
    assert_eq!(fields["phase"], "initializer");
    assert_eq!(fields["initializer_calls"], "0");
    assert_eq!(fields["primary_native_instructions"], "0");
    assert_eq!(fields["native_instructions"], "1");
    assert!(!fields.contains_key("value"));
    let mut init = vec![0x55, 0xb8, 7, 0, 0, 0, 0xff, 0x15];
    init.extend_from_slice(&(0xf00_i32 - 0x10c).to_le_bytes());
    init.extend_from_slice(&[0x0f, 0x0b]);
    let library = module_input(&init, true, Some(0x100));
    let (code, fields, text) = probe_images(&[("--image", &main), ("--preload-libc", &library)]);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "unavailable-function");
    assert_eq!(fields["phase"], "initializer");
    assert_eq!(fields["initializer_calls"], "0");
    assert_eq!(fields["primary_native_instructions"], "0");
    assert_eq!(fields["service_stops"], "1");
    assert!(!fields.contains_key("value"));
}

#[test]
fn absent_initializer_is_not_counted_and_invalid_preload_roles_fail_before_execution() {
    let main = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let library = input(&[0x0f, 0x0b], false);
    let (code, fields, text) = probe_images(&[("--image", &main), ("--preload-libc", &library)]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "0");
    let invalid = module_input(&[0xc3], false, Some(0x1200));
    let (code, fields, text) = probe_images(&[("--image", &main), ("--preload-libc", &invalid)]);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "initializer-target");
    assert_eq!(fields["phase"], "admission");
    assert_eq!(fields["native_instructions"], "0");
    for images in [
        vec![("--preload-libc", &library)],
        vec![
            ("--image", &main),
            ("--preload-libc", &library),
            ("--preload-libc", &library),
        ],
    ] {
        let (code, fields, text) = probe_images(&images);
        assert_eq!(code, 2, "{text}");
        assert_eq!(fields["code"], "image-arguments");
        assert_eq!(fields["phase"], "admission");
        assert_eq!(fields["native_instructions"], "0");
    }
}

#[test]
fn local_module_dtpmod_descriptor_resolves_real_loaded_tls_through_kernel() {
    let main = tls_main();
    let original = tls_descriptor_library();
    let mut bytes = original.bytes.clone();
    // Original fixture's descriptor module word uses symbol zero (local dynamic).
    put(&mut bytes, 0x6d0, &16_u64.to_le_bytes());
    let library = create_input(bytes);
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "4");
}

#[test]
fn local_dtpmod_got_writes_do_not_initialize_overlapping_tls_zero_tail() {
    let main = tls_main();
    let original = tls_descriptor_library();
    let mut bytes = original.bytes.clone();
    put(&mut bytes, 0x6d0, &16_u64.to_le_bytes());
    // Extend this original module's TLS tail over its ordinary relocated GOT.
    put(&mut bytes, 232 + 40, &96_u64.to_le_bytes());
    // Check that the DTPMOD GOT word at image 0x4070 did not leak into TLS +48.
    let mut init = Vec::new();
    rip(&mut init, 0x900, &[0x48, 0x8b, 0x0d], 0x4060);
    init.extend([0x64, 0x48, 0x8b, 0x41, 0x30, 0x48, 0x85, 0xc0]);
    let mut branches = Vec::new();
    checked(&mut init, &mut branches);
    rip(&mut init, 0x900, &[0xe9], 0x800);
    finish_checks(&mut init, branches);
    put(&mut bytes, 0x900, &init);
    put(&mut bytes, 0x2a8, &0x900_u64.to_le_bytes());
    let library = create_input(bytes);
    let (code, fields, text) = probe_dynamic(
        &main,
        &[("/app0/sce_module/pentax-original.sprx", &library)],
    );
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], "4");
}

#[test]
fn execute_only_primary_and_initializer_run_without_data_read_permission() {
    let mut main = module_bytes(&[0x48, 0x83, 0xc0, 29, 0xc3], false, None);
    let mut library = module_bytes(&[0xb8, 13, 0, 0, 0, 0xc3], false, Some(0x100));
    for bytes in [&mut main, &mut library] {
        put(bytes, 68, &1_u32.to_le_bytes()); // PF_X, without PF_R or PF_W.
    }
    let main = create_input(main);
    let library = create_input(library);
    let (code, fields, text) = probe_images(&[("--image", &main), ("--preload-libc", &library)]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["primary_native_instructions"], "1");
}

#[test]
fn execute_only_code_cannot_read_itself_as_guest_data() {
    // LEA points to its own initialized code; MOV tries an ordinary data read.
    let mut bytes = module_bytes(
        &[0x48, 0x8d, 0x05, 0xf9, 0xff, 0xff, 0xff, 0x8b, 0x00, 0xc3],
        false,
        None,
    );
    put(&mut bytes, 68, &1_u32.to_le_bytes());
    let source = create_input(bytes);
    let (code, fields, text) = probe(&source);
    assert_eq!(code, 2, "{text}");
    assert_eq!(fields["code"], "native-dispatch");
    assert_eq!(fields["phase"], "main-entry");
    assert!(!fields.contains_key("value"));
}

#[path = "image_probe/x87_conditional_moves.rs"]
mod x87_conditional_moves;
