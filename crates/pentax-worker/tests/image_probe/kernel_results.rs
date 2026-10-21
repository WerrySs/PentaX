// SPDX-License-Identifier: GPL-2.0-only

//! Independent original guest branches over real library-call return words.

use super::*;

/// Only the engine's observed closed mode selects these exact counts. Each
/// fixture accounts for its own admitted register and memory promotions.
fn assert_modeled_vector_operations(fields: &HashMap<String, String>, expected: [&str; 3]) {
    let mode = fields["native_ymm_mode"].as_str();
    let index = match mode {
        "sse2" => 0,
        "avx" => 1,
        "avx2" => 2,
        other => panic!("unexpected closed native vector mode: {other}"),
    };
    assert_eq!(
        fields["vector_ops"], expected[index],
        "native_ymm_mode={mode}"
    );
}

/// Exact mediated operations and bytes after the fixture's native promotions.
fn assert_modeled_vector_counters(fields: &HashMap<String, String>, expected: [(u64, u64); 3]) {
    let mode = fields["native_ymm_mode"].as_str();
    let index = match mode {
        "sse2" => 0,
        "avx" => 1,
        "avx2" => 2,
        other => panic!("unexpected closed native vector mode: {other}"),
    };
    assert_eq!(
        fields["vector_ops"].parse::<u64>().unwrap(),
        expected[index].0,
        "mode={mode}"
    );
    assert_eq!(
        fields["vector_bytes"].parse::<u64>().unwrap(),
        expected[index].1,
        "mode={mode}"
    );
}

#[path = "font_parser.rs"]
mod font_parser;
#[path = "ime_callbacks.rs"]
mod ime_callbacks;

#[test]
fn runtime_stop_identifies_the_failed_admitted_call() {
    let registrations = pentax_kernel::Service::registrations();
    let registration = registrations
        .iter()
        .find(|entry| entry.service == pentax_kernel::Service::PthreadJoin)
        .unwrap();
    let mut p = Program::new();
    p.call(0);
    let (code, _) = p.finish(None);
    let main = image(&code, &[registration.nid]);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service");
    assert_eq!(fields["runtime_failure"], "thread-lifecycle");
    assert_eq!(fields["public_function"], registration.source_name);
    assert!(!fields.contains_key("value"));
}

#[path = "interactive.rs"]
mod interactive;
#[path = "posix.rs"]
mod posix;
#[path = "posix_self.rs"]
mod posix_self;
#[path = "private_console.rs"]
mod private_console;
#[path = "private_samples.rs"]
mod private_samples;
#[path = "usleep_wait.rs"]
mod usleep_wait;

#[path = "thread_priority.rs"]
mod thread_priority;

#[path = "system_notice.rs"]
mod system_notice;

#[path = "element_permutations.rs"]
mod element_permutations;
#[path = "immediate_permutations.rs"]
mod immediate_permutations;
#[path = "lane_extract.rs"]
mod lane_extract;
#[path = "mask_comparisons.rs"]
mod mask_comparisons;
#[path = "minimum_position.rs"]
mod minimum_position;
#[path = "multiply_add_words.rs"]
mod multiply_add_words;
#[path = "native_packed_float.rs"]
mod native_packed_float;
#[path = "native_packed_float_memory.rs"]
mod native_packed_float_memory;
#[path = "native_scalar_float.rs"]
mod native_scalar_float;
#[path = "native_scalar_fma.rs"]
mod native_scalar_fma;
#[path = "native_streaming_stores.rs"]
mod native_streaming_stores;
#[path = "native_vector_memory.rs"]
mod native_vector_memory;
#[path = "native_vector_register_bits.rs"]
mod native_vector_register_bits;
#[path = "native_ymm.rs"]
mod native_ymm;
#[path = "packed_convert.rs"]
mod packed_convert;
#[path = "qword_comparisons.rs"]
mod qword_comparisons;
#[path = "rounding.rs"]
mod rounding;
#[path = "saturating_packs.rs"]
mod saturating_packs;

#[path = "directory_abi.rs"]
mod directory_abi;

#[path = "mutex_wait.rs"]
mod mutex_wait;
#[path = "pthread_attr.rs"]
mod pthread_attr;
#[path = "pthread_condition.rs"]
mod pthread_condition;
#[path = "pthread_mutex.rs"]
mod pthread_mutex;
#[path = "pthread_rwlock.rs"]
mod pthread_rwlock;
#[path = "pthread_semaphore.rs"]
mod pthread_semaphore;

#[path = "cpu_identification.rs"]
mod cpu_identification;
#[path = "pthread_affinity.rs"]
mod pthread_affinity;
#[path = "pthread_create.rs"]
mod pthread_create;
#[path = "pthread_keys.rs"]
mod pthread_keys;
#[path = "thread_exit.rs"]
mod thread_exit;
#[path = "time_counter.rs"]
mod time_counter;
#[path = "timestamp_counter.rs"]
mod timestamp_counter;

#[path = "acm_context.rs"]
mod acm_context;
#[path = "ampr.rs"]
mod ampr;
#[path = "audio_out2.rs"]
mod audio_out2;
#[path = "audio_out2_lifecycle.rs"]
mod audio_out2_lifecycle;
#[path = "calendar.rs"]
mod calendar;
#[path = "content_init.rs"]
mod content_init;
#[path = "dialog.rs"]
mod dialog;
#[path = "entitlement.rs"]
mod entitlement;
#[path = "fiber.rs"]
mod fiber;
#[path = "font_native.rs"]
mod font_native;
#[path = "handle_capacity.rs"]
mod handle_capacity;
#[path = "hardware_mode.rs"]
mod hardware_mode;
#[path = "http2_startup.rs"]
mod http2_startup;
#[path = "ime_native.rs"]
mod ime_native;
#[path = "lea.rs"]
mod lea;
#[path = "network_startup.rs"]
mod network_startup;
#[path = "np.rs"]
mod np;
#[path = "output.rs"]
mod output;
#[path = "pad.rs"]
mod pad;
#[path = "playgo.rs"]
mod playgo;
#[path = "raw_input.rs"]
mod raw_input;
#[path = "rudp.rs"]
mod rudp;
#[path = "save.rs"]
mod save;
#[path = "share.rs"]
mod share;
#[path = "share_media.rs"]
mod share_media;
#[path = "sysmodule.rs"]
mod sysmodule;
#[path = "system.rs"]
mod system;
#[path = "system_events.rs"]
mod system_events;
#[path = "user.rs"]
mod user;
#[path = "video.rs"]
mod video;

#[path = "agc_fusion.rs"]
mod agc_fusion;
#[path = "agc_interpolants.rs"]
mod agc_interpolants;
#[path = "agc_shaders.rs"]
mod agc_shaders;
#[path = "atomic_instructions.rs"]
mod atomic_instructions;
#[path = "bit_counts.rs"]
mod bit_counts;
#[path = "bit_strings.rs"]
mod bit_strings;
#[path = "bmi_instructions.rs"]
mod bmi_instructions;
#[path = "debug_stops.rs"]
mod debug_stops;
#[path = "event_flag_wait.rs"]
mod event_flag_wait;
#[path = "event_queue.rs"]
mod event_queue;
#[path = "fma_instructions.rs"]
mod fma_instructions;
#[path = "graphics_events.rs"]
mod graphics_events;
#[path = "graphics_submit.rs"]
mod graphics_submit;
#[path = "heap_trace.rs"]
mod heap_trace;
#[path = "kernel_objects.rs"]
mod kernel_objects;
#[path = "kernel_semaphore_wait.rs"]
mod kernel_semaphore_wait;
#[path = "memory_ranges.rs"]
mod memory_ranges;
#[path = "minmax_vectors.rs"]
mod minmax_vectors;
#[path = "named_direct.rs"]
mod named_direct;
#[path = "physical_memory.rs"]
mod physical_memory;
#[path = "precision_vectors.rs"]
mod precision_vectors;
#[path = "pthread_once.rs"]
mod pthread_once;
#[path = "string_instructions.rs"]
mod string_instructions;
#[path = "vector_instructions.rs"]
mod vector_instructions;
#[path = "vector_masks.rs"]
mod vector_masks;
#[path = "virtual_names.rs"]
mod virtual_names;

#[test]
fn extended_startup_crosses_the_kernel_stop_cap_without_changing_default_startup() {
    let mut p = Program::new();
    p.reg(12, 10_001);
    let loop_start = p.code.len();
    p.call(0); // getpid: observe its real return on every iteration.
    p.expect(1);
    p.code.extend([0x49, 0xff, 0xcc, 0x0f, 0x85]); // DEC R12; JNZ loop.
    let offset = loop_start as i32 - (p.code.len() as i32 + 4);
    p.code.extend(offset.to_le_bytes());
    let (code, _) = p.finish(None);
    let source = image(&code, &["HoLVWNanBBc"]);
    for profile in ["startup", "extended-startup"] {
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            source.path.as_os_str().to_owned(),
            "--execution-profile".into(),
            profile.into(),
        ]);
        assert_eq!(fields["service_stops"], "10001", "{text}");
        if profile == "startup" {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["budget"], "kernel-stops", "{text}");
        } else {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42", "{text}");
        }
    }
}

const PATH: &str = "/app0/assets/pentax-results.bin";
const MISSING: i32 = 0xd80;
const NAME: i32 = 0xd00;
const I32_INVALID: u64 = 0x8002_0016;
const I32_BUSY: u64 = 0x8002_0010;

#[path = "align_right.rs"]
mod align_right;
#[path = "libc_memory_builtins.rs"]
mod libc_memory_builtins;

struct Program {
    code: Vec<u8>,
    branches: Vec<usize>,
    calls: u64,
}
impl Program {
    fn new() -> Self {
        Self {
            code: vec![0x55, 0x48, 0x81, 0xec, 0, 1, 0, 0],
            branches: Vec::new(),
            calls: 0,
        }
    }
    fn reg(&mut self, reg: u8, value: u64) {
        self.code
            .extend([0x48 | u8::from(reg >= 8), 0xb8 + (reg & 7)]);
        self.code.extend_from_slice(&value.to_le_bytes());
    }
    fn stack(&mut self, opcode: u8, reg: u8, offset: u8) {
        self.code.extend([
            0x48 | if reg >= 8 { 4 } else { 0 },
            opcode,
            0x44 | ((reg & 7) << 3),
            0x24,
            offset,
        ]);
    }
    fn ptr(&mut self, reg: u8, offset: u8) {
        self.stack(0x8d, reg, offset);
    }
    fn load(&mut self, reg: u8, offset: u8) {
        self.stack(0x8b, reg, offset);
    }
    fn save(&mut self, offset: u8) {
        self.code.extend([0x48, 0x89, 0x44, 0x24, offset]);
    }
    fn set(&mut self, offset: u8, value: u8) {
        self.code
            .extend([0x48, 0xc7, 0x44, 0x24, offset, value, 0, 0, 0]);
    }
    fn path(&mut self, reg: u8, target: i32) {
        rip(
            &mut self.code,
            0x800,
            &[
                0x48 | if reg >= 8 { 4 } else { 0 },
                0x8d,
                ((reg & 7) << 3) | 5,
            ],
            target,
        );
    }
    fn call(&mut self, slot: i32) {
        rip(&mut self.code, 0x800, &[0xff, 0x15], 0xf00 + slot * 8);
        self.calls += 1;
    }
    fn expect(&mut self, value: u64) {
        self.reg(10, value);
        self.code.extend([0x4c, 0x39, 0xd0]);
        checked(&mut self.code, &mut self.branches);
    }
    fn word(&mut self, offset: u8, value: u64) {
        self.load(0, offset);
        self.expect(value);
    }
    fn finish(mut self, slot: Option<u8>) -> (Vec<u8>, u64) {
        if let Some(slot) = slot {
            self.load(0, slot);
        } else {
            self.reg(0, 42);
        }
        self.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.code, self.branches);
        assert!(self.code.len() < 0x500);
        (self.code, self.calls)
    }
}

fn image(code: &[u8], nids: &[&str]) -> Input {
    scoped_system_image(code, nids, "libkernel", "libkernel", 1)
}

fn scoped_system_image(
    code: &[u8],
    nids: &[&str],
    module: &str,
    library: &str,
    version: u64,
) -> Input {
    assert!(nids.len() <= 8);
    let mut bytes = module_bytes(&[], true, None);
    bytes[0x600..0x800].fill(0);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    put(&mut bytes, 0x800, code);
    put(&mut bytes, NAME as usize, format!("{PATH}\0").as_bytes());
    put(
        &mut bytes,
        MISSING as usize,
        b"/app0/assets/missing-original.bin\0",
    );
    let mut names = format!("\0{module}\0{library}\0").into_bytes();
    for (index, nid) in nids.iter().enumerate() {
        let symbol = 0x600 + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12;
        names.extend_from_slice(format!("{nid}#D#C\0").as_bytes());
        put(
            &mut bytes,
            0x700 + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x708 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    put(&mut bytes, 0x400, &names);
    for (index, (tag, value)) in [
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, (nids.len() as u64 + 1) * 24),
        (11, 24),
        (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
        (
            0x6100_0015,
            (module.len() as u64 + 2) | (version << 32) | (3 << 48),
        ),
        (7, 0x700),
        (8, nids.len() as u64 * 24),
        (9, 24),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}

#[test]
fn guest_handles_real_file_errno_in_both_register_widths_then_reads_and_closes_data() {
    // Slots: open, read, pread, lseek, close, fstat, stat, reachability.
    let mut p = Program::new();
    p.path(7, MISSING);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(0);
    p.expect(0x8002_0002);
    p.path(7, NAME);
    p.reg(6, 1);
    p.call(0);
    p.expect(0x8002_001e);
    p.path(7, NAME);
    p.reg(6, 0);
    p.call(0);
    p.expect(3);
    p.save(8);
    p.set(32, 85);
    p.load(7, 8);
    p.reg(6, 0);
    p.reg(2, 8);
    p.call(1);
    p.expect(0xffff_ffff_8002_000e);
    p.reg(7, 0);
    p.ptr(6, 32);
    p.call(1);
    p.expect(0xffff_ffff_8002_0001);
    p.load(7, 8);
    p.ptr(6, 32);
    p.reg(1, u64::MAX);
    p.call(2);
    p.expect(0xffff_ffff_8002_0016);
    p.reg(7, 0);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(3);
    p.expect(0xffff_ffff_8002_0009);
    p.reg(7, 0);
    p.ptr(6, 64);
    p.call(5);
    p.expect(0x8002_0001);
    p.reg(7, 0);
    p.call(4);
    p.expect(0x8002_0001);
    p.set(64, 85);
    p.path(7, MISSING);
    p.ptr(6, 64);
    p.call(6);
    p.expect(0x8002_0002);
    p.word(64, 85);
    p.path(7, MISSING);
    p.call(7);
    p.expect(0x8002_0002);
    p.word(32, 85);
    p.load(7, 8);
    p.ptr(6, 32);
    p.reg(2, 8);
    p.call(1);
    p.expect(8);
    p.word(32, 42);
    p.load(7, 8);
    p.call(4);
    p.expect(0);
    p.load(7, 8);
    p.call(4);
    p.expect(0x8002_0009);
    let (code, calls) = p.finish(Some(32));
    let main = image(
        &code,
        &[
            "1G3lF1Gg1k8",
            "Cg4srZ6TKbU",
            "+r3rMFwItV4",
            "oib76F-12fk",
            "UK2Tl2DWUns",
            "kBwCPsYX-m4",
            "eV9wAD2riIA",
            "uWyW3v98sU4",
        ],
    );
    let data = create_input(42_u64.to_le_bytes().to_vec());
    let mut binding = OsString::from(format!("{PATH}="));
    binding.push(&data.path);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-file".into(),
        binding,
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn guest_sync_errno_keeps_real_output_and_state_and_memory_clock_errors_resume_with_source_codes() {
    let registrations = pentax_kernel::Service::registrations();
    let services = [
        pentax_kernel::Service::CreateSema,
        pentax_kernel::Service::PollSema,
        pentax_kernel::Service::SignalSema,
        pentax_kernel::Service::CreateEventFlag,
        pentax_kernel::Service::PollEventFlag,
        pentax_kernel::Service::SetEventFlag,
        pentax_kernel::Service::MapFlexible,
        pentax_kernel::Service::ClockGettime,
    ];
    let nids: Vec<_> = services
        .iter()
        .map(|service| {
            registrations
                .iter()
                .find(|entry| entry.service == *service)
                .unwrap()
                .nid
        })
        .collect();
    let mut p = Program::new();
    p.ptr(7, 8);
    p.path(6, NAME);
    p.reg(2, 1);
    p.reg(1, 0);
    p.reg(8, 2);
    p.reg(9, 0);
    p.call(0);
    p.expect(0);
    p.load(7, 8);
    p.reg(6, 1);
    p.call(1);
    p.expect(I32_BUSY);
    p.load(7, 8);
    p.reg(6, 1);
    p.call(2);
    p.expect(0);
    p.load(7, 8);
    p.reg(6, 1);
    p.call(1);
    p.expect(0);
    p.ptr(7, 16);
    p.path(6, NAME);
    p.reg(2, 0x11);
    p.reg(1, 0x10);
    p.reg(8, 0);
    p.call(3);
    p.expect(0);
    p.set(24, 85);
    p.load(7, 16);
    p.reg(6, 0x20);
    p.reg(2, 1);
    p.ptr(1, 24);
    p.call(4);
    p.expect(I32_BUSY);
    p.word(24, 0x10);
    p.load(7, 16);
    p.reg(6, 0x20);
    p.call(5);
    p.expect(0);
    p.load(7, 16);
    p.reg(6, 0x20);
    p.reg(2, 0x21);
    p.ptr(1, 24);
    p.call(4);
    p.expect(0);
    p.word(24, 0x30);
    p.load(7, 16);
    p.reg(6, 0x20);
    p.reg(2, 1);
    p.ptr(1, 24);
    p.call(4);
    p.expect(I32_BUSY);
    p.word(24, 0x10);
    p.set(40, 85);
    p.ptr(7, 40);
    p.reg(6, 0);
    p.reg(2, 3);
    p.reg(1, 0);
    p.call(6);
    p.expect(I32_INVALID);
    p.word(40, 85);
    p.set(64, 85);
    p.reg(7, 3);
    p.ptr(6, 64);
    p.call(7);
    p.expect(I32_INVALID);
    p.word(64, 85);
    let (code, calls) = p.finish(None);
    let main = image(&code, &nids);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn known_missing_function_reports_only_its_scoped_public_reference_name() {
    let mut p = Program::new();
    p.call(0);
    let (code, _) = p.finish(None);
    assert!(
        !pentax_kernel::Service::registrations()
            .iter()
            .any(|entry| entry.nid == "zE-wXIZjLoM")
    );
    let source = image(&code, &["zE-wXIZjLoM"]); // Pinned unimplemented debug export.
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unavailable-function");
    assert_eq!(
        fields["public_function"],
        "LibKernel::KernelDebugRaiseExceptionOnReleaseMode"
    );
    assert_eq!(fields["reference_match"], "exact");
    assert!(!fields.contains_key("reference_function"));
    assert!(!fields.contains_key("value"));
    assert!(!text.contains("zE-wXIZjLoM"));
    for scope in ["descriptor-version", "different-scope"] {
        let mut changed = source.bytes.clone();
        if scope == "descriptor-version" {
            put(
                &mut changed,
                0x208 + 5 * 16,
                &(1_u64 | (2 << 32) | (1 << 40) | (2 << 48)).to_le_bytes(),
            );
        } else {
            put(&mut changed, 0x401, b"libother_");
        }
        let changed = create_input(changed);
        let (status, fields, text) = probe(&changed);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "unavailable-function");
        assert_eq!(fields["reference_match"], scope);
        assert_eq!(
            fields["reference_function"],
            "LibKernel::KernelDebugRaiseExceptionOnReleaseMode"
        );
        assert!(!fields.contains_key("public_function"));
        assert!(!fields.contains_key("value"));
        assert!(!text.contains("zE-wXIZjLoM"));
        assert!(!text.contains("libother_"));
    }
}

#[test]
fn public_library_reference_is_a_stop_without_exact_binding_or_unknown_identity_output() {
    let mut p = Program::new();
    p.call(0);
    let (code, _) = p.finish(None);
    // This identity is a published SharpEmu declaration, never a guest dump.
    let nid = "09tk+kIA1Ns";
    for (library, category, reference) in [
        (
            "libSceHttp2",
            "library-reference",
            Some("sceHttp2SetMinSslVersion"),
        ),
        ("original-unknown-library", "unlisted-reference", None),
    ] {
        let source = scoped_system_image(&code, &[nid], "libSceHttp2", library, 1);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "unavailable-function", "{text}");
        assert_eq!(fields["reference_match"], category, "{text}");
        assert_eq!(
            fields.get("reference_function").map(String::as_str),
            reference
        );
        assert!(!fields.contains_key("public_function"), "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        assert!(!text.contains(nid));
        assert!(!text.contains("original-unknown-library"));
    }
}

#[test]
fn reference_library_category_keeps_unknown_functions_unimplemented() {
    let mut p = Program::new();
    p.call(0);
    let (code, _) = p.finish(None);
    // Original synthetic identities, not identifiers taken from any input.
    let nid = "AAAAAAAAAAA";
    for (library, category) in [
        ("libSceHttp2", Some("Http2")),
        ("original-unknown-library", None),
    ] {
        let source = scoped_system_image(&code, &[nid], "original-module", library, 1);
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "unavailable-function", "{text}");
        assert_eq!(fields["reference_match"], "unlisted-reference", "{text}");
        assert_eq!(
            fields.get("reference_library").map(String::as_str),
            category
        );
        assert!(!fields.contains_key("public_function"), "{text}");
        assert!(!fields.contains_key("reference_function"), "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        assert!(!text.contains(nid));
        assert!(!text.contains("original-module"));
        assert!(!text.contains("original-unknown-library"));
    }
}

#[test]
fn reference_public_name_hash_is_only_an_unscoped_candidate() {
    let mut p = Program::new();
    p.call(0);
    let (code, _) = p.finish(None);
    // Independently verified public corpus name/hash, with original scopes.
    let nid = "+KDWny9Y-6k";
    let source = scoped_system_image(
        &code,
        &[nid],
        "original-module",
        "original-unknown-library",
        1,
    );
    let (status, fields, text) = probe(&source);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unavailable-function", "{text}");
    assert_eq!(
        fields["reference_match"], "public-name-hash-reference",
        "{text}"
    );
    assert_eq!(
        fields["reference_function"], "sceContentExportTerm",
        "{text}"
    );
    assert!(!fields.contains_key("reference_library"), "{text}");
    assert!(!fields.contains_key("public_function"), "{text}");
    assert!(!fields.contains_key("value"), "{text}");
    assert!(!text.contains(nid));
    assert!(!text.contains("original-module"));
    assert!(!text.contains("original-unknown-library"));
}

#[test]
fn native_library_unavailable_semantics_remain_stops_without_a_register_result() {
    let mut p = Program::new();
    p.reg(7, 14);
    p.ptr(6, 32);
    p.call(0); // Unported CPU-time clock domain.
    let (code, _) = p.finish(None);
    let nid = pentax_kernel::Service::registrations()
        .iter()
        .find(|entry| entry.service == pentax_kernel::Service::ClockGettime)
        .unwrap()
        .nid;
    let main = image(&code, &[nid]);
    let (status, fields, text) = probe(&main);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "runtime-service");
    assert_eq!(fields["service_stops"], "1");
    assert!(!fields.contains_key("value"));
}

#[test]
fn native_thread_lifecycle_registration_resumes_without_calling_cleanup_and_refuses_replacement() {
    for replace in [false, true] {
        let mut p = Program::new();
        p.reg(7, 0);
        p.call(0);
        p.path(7, 0xc00);
        p.call(0);
        p.path(7, 0xc00);
        p.call(1);
        p.path(7, 0xc00);
        p.call(2);
        if replace {
            p.reg(7, 0);
            p.call(0);
        }
        let (code, calls) = p.finish(None);
        let original = image(&code, &["rNhWz+lvOMU", "pB-yGZ2nQ9o", "WhCc1w3EhSI"]);
        let mut bytes = original.bytes.clone();
        put(&mut bytes, 0xc00, &[0x0f, 0x0b]); // Calling cleanup during registration must stop.
        let source = create_input(bytes);
        let (exit, fields, text) = probe(&source);
        assert_eq!(fields["unavailable_functions"], "0");
        assert_eq!(fields["service_stops"], calls.to_string());
        if replace {
            assert_eq!(exit, 2, "{text}");
            assert_eq!(fields["code"], "runtime-service");
            assert!(!fields.contains_key("value"));
        } else {
            assert_eq!(exit, 0, "{text}");
            assert_eq!(fields["value"], "42");
        }
    }
}

#[test]
fn native_application_heap_registration_accepts_a_real_table_and_stops_on_invalid_backing() {
    for invalid in [false, true] {
        let mut p = Program::new();
        for offset in (16..96).step_by(8) {
            p.set(offset, 0);
        }
        p.path(0, 0xc00);
        for offset in [16, 24, 64] {
            p.save(offset);
        }
        p.ptr(7, 16);
        p.call(0);
        p.reg(7, 0);
        p.call(0);
        if invalid {
            p.reg(7, u64::MAX - 79);
        } else {
            p.ptr(7, 16);
        }
        p.call(0);
        let (code, calls) = p.finish(None);
        let original = image(&code, &["p5EcQeEeJAE"]);
        let mut bytes = original.bytes.clone();
        put(&mut bytes, 0xc00, &[0x0f, 0x0b]); // Registration must not invoke allocators.
        let source = create_input(bytes);
        let (exit, fields, text) = probe(&source);
        assert_eq!(fields["unavailable_functions"], "0");
        assert_eq!(fields["service_stops"], calls.to_string());
        if invalid {
            assert_eq!(exit, 2, "{text}");
            assert_eq!(fields["code"], "runtime-service");
            assert!(!fields.contains_key("value"));
        } else {
            assert_eq!(exit, 0, "{text}");
            assert_eq!(fields["value"], "42");
        }
    }
}

#[path = "gpu_window.rs"]
mod gpu_window;
#[path = "present_window.rs"]
mod present_window;
