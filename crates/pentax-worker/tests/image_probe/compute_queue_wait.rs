// SPDX-License-Identifier: GPL-2.0-only
//! Original asynchronous ACB callers release actual waits by CPU and peer-queue stores.
//! These shader-free owned-memory effects do not claim GPU rendering or presentation.
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

impl Code {
    fn packet(&mut self, at: u32, words: &[u32], addresses: &[(usize, u32)]) {
        for (index, word) in words.iter().enumerate() {
            self.reg(0, u64::from(*word));
            self.store32(at + index as u32 * 4);
        }
        for &(index, offset) in addresses {
            self.mapped(0, offset);
            self.store32(at + index as u32 * 4);
            self.bytes.extend([0x48, 0xc1, 0xe8, 32]);
            self.store32(at + (index as u32 + 1) * 4);
        }
    }
    fn submit(&mut self, queue: u64, at: u32, words: u64) {
        self.mapped(0, at);
        self.stack(0x89, 0, 64);
        self.word(72, words);
        self.reg(7, queue);
        self.ptr(6, 64);
        self.reg(2, 0x63d2_f081_a7b4_c509);
        self.call(0);
        self.expect(0, 0);
        // This argument is outside the two-argument submit ABI and is retained
        // across the native service. R12 is consumed by every guarded readback.
        self.expect(2, 0x63d2_f081_a7b4_c509);
    }
    fn wait32(&mut self, offset: u32, expected: u32) {
        // Original bounded guest polling; a taken branch yields a real block
        // boundary for the worker's finite queue pump. No host readiness setter.
        self.reg(13, 256);
        let again = self.bytes.len();
        self.bytes.extend([0x41, 0x8b, 0x84, 0x24]);
        self.bytes.extend(offset.to_le_bytes());
        self.reg(10, u64::from(expected));
        self.bytes
            .extend([0x4c, 0x39, 0xd0, 0x0f, 0x84, 0, 0, 0, 0]);
        let done = self.bytes.len() - 4;
        self.bytes.extend([0x49, 0xff, 0xcd, 0x0f, 0x85]);
        let back = self.bytes.len();
        self.bytes.extend(0i32.to_le_bytes());
        let delta = i32::try_from(again).unwrap() - i32::try_from(back + 4).unwrap();
        self.bytes[back..back + 4].copy_from_slice(&delta.to_le_bytes());
        self.bytes.extend([0xe9, 0, 0, 0, 0]);
        self.failed.push(self.bytes.len() - 4);
        let delta = i32::try_from(self.bytes.len()).unwrap() - i32::try_from(done + 4).unwrap();
        self.bytes[done..done + 4].copy_from_slice(&delta.to_le_bytes());
    }
}
fn program(peer_release: bool) -> Vec<u8> {
    let mut p = Code::new();
    p.word(32, 0);
    p.ptr(7, 32);
    p.reg(6, 0x10000);
    p.reg(2, 0x33);
    p.reg(1, 0);
    p.call(1);
    p.expect(0, 0);
    p.stack(0x8b, 12, 32);
    for offset in [0x200u32, 0x208, 0x210, 0x218, 0x220, 0x228, 0x230, 0x238] {
        p.reg(0, GUARD);
        p.bytes.extend([0x49, 0x89, 0x84, 0x24]);
        p.bytes.extend(offset.to_le_bytes());
    }
    p.reg(0, 0);
    p.store32(0x210);
    // Same queue's first root retires a real WRITE prefix, then waits for7.
    // After resume the COPY must read the current prefix, not a stale snapshot.
    let first = [
        0xc003_3700,
        0x100,
        0,
        0,
        0x1357_2468,
        0xc005_3c00,
        0x13,
        0,
        0,
        7,
        u32::MAX,
        0,
        0xc004_4000,
        0x101,
        0,
        0,
        0,
        0,
        0xc000_4200,
        0,
    ];
    p.packet(
        0x1000,
        &first,
        &[(2, 0x204), (7, 0x210), (14, 0x204), (16, 0x224)],
    );
    p.packet(
        0x1400,
        &[0xc003_3700, 0x100, 0, 0, 0x7654_3210, 0xc000_4200, 0],
        &[(2, 0x234)],
    );
    p.packet(
        0x1800,
        &[0xc003_3700, 0x100, 0, 0, 7, 0xc000_4200, 0],
        &[(2, 0x210)],
    );
    p.submit(0x20, 0x1000, first.len() as u64);
    // Acceptance is asynchronous; wait for the real prefix, without requiring
    // it to have executed before the submit service returned. Gate remains0.
    p.wait32(0x204, 0x1357_2468);
    p.check64(0x200, 0x1357_2468_a5a5_a5a5);
    p.check64(0x220, GUARD);
    p.reg(0, 0x89ab_cdef);
    p.store32(0x204);
    if peer_release {
        p.submit(0x20, 0x1400, 7);
        // A later same-queue root must not overtake the blocked front.
        p.check64(0x230, GUARD);
        p.submit(0x21, 0x1800, 7);
    } else {
        // The CPU that submitted the blocked queue remains runnable and can
        // release it. Returning only after GPU completion deadlocks this caller.
        p.reg(0, 7);
        p.store32(0x210);
    }
    p.wait32(0x224, 0x89ab_cdef);
    if peer_release {
        p.wait32(0x234, 0x7654_3210);
    }
    p.check64(0x200, 0x89ab_cdef_a5a5_a5a5);
    p.check64(0x208, GUARD);
    p.check64(0x210, 0xa5a5_a5a5_0000_0007);
    p.check64(0x218, GUARD);
    p.check64(0x220, 0x89ab_cdef_a5a5_a5a5);
    p.check64(0x228, GUARD);
    p.check64(
        0x230,
        if peer_release {
            0x7654_3210_a5a5_a5a5
        } else {
            GUARD
        },
    );
    p.check64(0x238, GUARD);
    p.finish()
}

#[test]
fn native_compute_waits_return_to_cpu_and_preserve_prefix_fifo_and_peer_queue_progress() {
    for observed in [false, true] {
        for peer_release in [false, true] {
            let (status, fields, text) = run(&image(&program(peer_release), observed));
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42", "{text}");
            assert_eq!(fields["unavailable_functions"], "0", "{text}");
            assert_eq!(
                fields["graphics_command_buffers"],
                if peer_release { "3" } else { "1" },
                "{text}"
            );
            assert_eq!(
                fields["graphics_written_bytes"],
                if peer_release { "16" } else { "8" },
                "{text}"
            );
            for key in [
                "graphics_draw_attempts",
                "graphics_dispatch_attempts",
                "window_frames_presented",
                "window_frames_acknowledged",
            ] {
                assert_eq!(fields[key], "0", "{key}: {text}");
            }
        }
    }
    println!(
        "compute_wait_native_available=true compute_wait_native_programs=4 compute_wait_native_buffers=8 compute_wait_native_written_bytes=48"
    );
}
