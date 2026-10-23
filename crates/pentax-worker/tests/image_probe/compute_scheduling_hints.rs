// SPDX-License-Identifier: GPL-2.0-only
//! Original shader-free ACB callers consume closed scheduling-hint admission.
//! The WRITE_DATA readback is real owned memory; no shader, GPU or image is substituted.
use super::*;
const ENTRY: i32 = 0x1000;
const GUARD: u64 = 0xa5a5_a5a5_a5a5_a5a5;
struct Code {
    bytes: Vec<u8>,
    failed: Vec<usize>,
}
impl Code {
    fn new() -> Self {
        Self {
            bytes: vec![0x55, 0x48, 0x81, 0xec, 0, 8, 0, 0],
            failed: Vec::new(),
        }
    }
    fn reg(&mut self, r: u8, value: u64) {
        self.bytes.extend([0x48 | u8::from(r >= 8), 0xb8 + (r & 7)]);
        self.bytes.extend(value.to_le_bytes());
    }
    fn stack(&mut self, op: u8, r: u8, at: u32) {
        self.bytes.extend([
            0x48 | if r >= 8 { 4 } else { 0 },
            op,
            0x84 | ((r & 7) << 3),
            0x24,
        ]);
        self.bytes.extend(at.to_le_bytes());
    }
    fn ptr(&mut self, r: u8, at: u32) {
        self.stack(0x8d, r, at);
    }
    fn word(&mut self, at: u32, value: u64) {
        self.reg(0, value);
        self.stack(0x89, 0, at);
    }
    fn call(&mut self, slot: i32) {
        rip(&mut self.bytes, ENTRY, &[0xff, 0x15], 0xf00 + slot * 8);
    }
    fn expect(&mut self, r: u8, value: u64) {
        self.reg(10, value);
        self.bytes.extend([
            0x4c | u8::from(r >= 8),
            0x39,
            0xd0 | (r & 7),
            0x0f,
            0x85,
            0,
            0,
            0,
            0,
        ]);
        self.failed.push(self.bytes.len() - 4);
    }
    fn mapped(&mut self, r: u8, offset: u32) {
        self.bytes
            .extend([0x4c | u8::from(r >= 8), 0x89, 0xe0 | (r & 7)]);
        self.bytes
            .extend([0x48 | u8::from(r >= 8), 0x81, 0xc0 | (r & 7)]);
        self.bytes.extend(offset.to_le_bytes());
    }
    fn store32(&mut self, offset: u32) {
        self.bytes.extend([0x41, 0x89, 0x84, 0x24]);
        self.bytes.extend(offset.to_le_bytes());
    }
    fn check64(&mut self, offset: u32, value: u64) {
        self.bytes.extend([0x49, 0x8b, 0x84, 0x24]);
        self.bytes.extend(offset.to_le_bytes());
        self.expect(0, value);
    }
    fn finish(mut self) -> Vec<u8> {
        self.reg(0, 42);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 8, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        self.bytes
    }
}
fn image(code: &[u8], observed: bool) -> Input {
    let pair = if observed {
        ("libSceAgcDriver", "libSceAgcDriver")
    } else {
        ("Graphics5Driver", "Graphics5Driver")
    };
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x10000, 0);
    bytes[0x400..0x1000].fill(0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x10000u64.to_le_bytes());
    put(&mut bytes, 104, &0x10000u64.to_le_bytes());
    assert!(code.len() < 0xe000);
    put(&mut bytes, ENTRY as usize, code);
    let mut names = vec![0];
    let mut offsets = Vec::new();
    for (m, l) in [pair, ("libkernel", "libkernel")] {
        let mo = names.len() as u64;
        names.extend(m.as_bytes());
        names.push(0);
        let lo = names.len() as u64;
        names.extend(l.as_bytes());
        names.push(0);
        offsets.push((mo, lo));
    }
    for (index, (nid, group)) in [("gSRnr79F8tQ", 0), ("IWIBBdTHit4", 1)]
        .into_iter()
        .enumerate()
    {
        let at = 0x600 + (index + 1) * 24;
        put(&mut bytes, at, &(names.len() as u32).to_le_bytes());
        bytes[at + 4] = 0x12;
        names.extend(
            format!(
                "{nid}#{}#{}\0",
                char::from(b'D' + group * 2),
                char::from(b'C' + group * 2)
            )
            .as_bytes(),
        );
        put(
            &mut bytes,
            0x800 + index * 24,
            &(0xf00u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x808 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(names.len() < 0x200);
    put(&mut bytes, 0x400, &names);
    let mut dynamic = vec![
        (5i64, 0x400u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, 72),
        (11, 24),
    ];
    for (index, (m, l)) in offsets.into_iter().enumerate() {
        let (major, minor, version) = (1u64, 1u64, 1u64);
        let id = 2 + index as u64 * 2;
        dynamic.push((0x6100_000f, m | (minor << 32) | (major << 40) | (id << 48)));
        dynamic.push((0x6100_0015, l | (version << 32) | ((id + 1) << 48)));
    }
    dynamic.extend([(7, 0x800), (8, 48), (9, 24), (0, 0)]);
    for at in [152, 160] {
        put(&mut bytes, at, &((dynamic.len() * 16) as u64).to_le_bytes());
    }
    for (i, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + i * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + i * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}
fn run(source: &Input) -> (i32, HashMap<String, String>, String) {
    probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "extended-startup".into(),
        "--present-window".into(),
        "no".into(),
    ])
}
fn program(bad_register: Option<u32>) -> Vec<u8> {
    let mut p = Code::new();
    p.word(32, 0);
    p.ptr(7, 32);
    p.reg(6, 0x10000);
    p.reg(2, 0x33);
    p.reg(1, 0);
    p.call(1);
    p.expect(0, 0);
    p.stack(0x8b, 12, 32);
    for offset in [0x200u32, 0x208] {
        p.reg(0, GUARD);
        p.bytes.extend([0x49, 0x89, 0x84, 0x24]);
        p.bytes.extend(offset.to_le_bytes());
    }
    let mut words = vec![
        0xc001_7600,
        0x215,
        0x83f0_0107,
        0xc001_7602,
        0x22a,
        0xcaf3_0021,
    ];
    if let Some(register) = bad_register {
        words.extend([0xc001_7602, register, 0x7e01_f00d]);
    }
    // Address words are patched from the real allocation below. A refused SH
    // write must stop before this WRITE_DATA is published or Drain is reached.
    let write = words.len() as u32 * 4;
    words.extend([0xc003_3700, 0x100, 0, 0, 0x1256_789a, 0xc000_4200, 0]);
    for (index, word) in words.iter().enumerate() {
        p.reg(0, u64::from(*word));
        p.store32(index as u32 * 4);
    }
    p.mapped(0, 0x204);
    p.store32(write + 8);
    p.bytes.extend([0x48, 0xc1, 0xe8, 32]);
    p.store32(write + 12);
    p.mapped(0, 0);
    p.stack(0x89, 0, 64);
    p.word(72, words.len() as u64);
    p.reg(7, 0x20);
    p.ptr(6, 64);
    p.reg(2, u64::MAX);
    p.reg(1, u64::MAX);
    p.reg(8, u64::MAX);
    p.reg(9, u64::MAX);
    p.call(0);
    p.expect(0, 0);
    p.check64(0x200, 0x1256_789a_a5a5_a5a5);
    p.check64(0x208, GUARD);
    p.finish()
}

#[test]
fn native_compute_scheduling_hints_reach_real_acb_effects_and_refuse_other_sh_registers() {
    for observed in [false, true] {
        let (status, fields, text) = run(&image(&program(None), observed));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["graphics_command_buffers"], "1", "{text}");
        assert_eq!(fields["graphics_written_bytes"], "4", "{text}");
        for key in [
            "graphics_draw_attempts",
            "graphics_dispatch_attempts",
            "window_frames_presented",
            "window_frames_acknowledged",
        ] {
            assert_eq!(fields[key], "0", "{key}: {text}");
        }
    }
    for bad in [0x218, 0x22b] {
        let (status, fields, text) = run(&image(&program(Some(bad)), true));
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service", "{text}");
        assert_eq!(fields["runtime_failure"], "compute-unsupported", "{text}");
        assert!(!fields.contains_key("value"), "{text}");
        // Buffer admission precedes command preparation and its SH refusal.
        assert_eq!(fields["graphics_command_buffers"], "1", "{text}");
        for key in [
            "graphics_written_bytes",
            "graphics_dispatch_attempts",
            "window_frames_presented",
        ] {
            assert_eq!(fields[key], "0", "{key}: {text}");
        }
    }
    println!(
        "compute_hint_native_available=true compute_hint_native_programs=2 compute_hint_native_command_buffers=2 compute_hint_native_written_bytes=8 compute_hint_native_refusals=2"
    );
}
