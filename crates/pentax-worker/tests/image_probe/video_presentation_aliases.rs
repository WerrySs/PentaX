// SPDX-License-Identifier: GPL-2.0-only
//! Original imported VideoOut callers; a captured CPU frame is not a Presented ack.
use super::*;

const ENTRY: i32 = 0x1000;
const DATA: i32 = 0x14000;
const GUARD: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const OPEN: i32 = 0;
const MAP: i32 = 1;
const REGISTER: i32 = 2;
const FLIP: i32 = 3;
const CLOSE: i32 = 4;
const PENDING: i32 = 5;
const ADD_EVENT: i32 = 6;
const VBLANK: i32 = 7;
const GAMMA: i32 = 8;
const ADJUST: i32 = 9;
const ERRNO: i32 = 10;
const CREATE_QUEUE: i32 = 11;

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
    fn stack(&mut self, op: u8, r: u8, offset: u32) {
        self.bytes.extend([
            0x48 | if r >= 8 { 4 } else { 0 },
            op,
            0x84 | ((r & 7) << 3),
            0x24,
        ]);
        self.bytes.extend(offset.to_le_bytes());
    }
    fn word(&mut self, at: u32, value: u64) {
        self.reg(0, value);
        self.stack(0x89, 0, at);
    }
    fn ptr(&mut self, r: u8, at: u32) {
        self.stack(0x8d, r, at);
    }
    fn path(&mut self, r: u8, at: i32) {
        rip(
            &mut self.bytes,
            ENTRY,
            &[0x48 | if r >= 8 { 4 } else { 0 }, 0x8d, ((r & 7) << 3) | 5],
            at,
        );
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
    fn check(&mut self, at: u32, value: u64) {
        self.stack(0x8b, 0, at);
        self.expect(0, value);
    }
    fn finish(mut self) -> Vec<u8> {
        self.reg(0, 42);
        self.bytes
            .extend([0x48, 0x81, 0xc4, 0, 8, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.bytes, self.failed);
        assert!(self.bytes.len() < 0x6000);
        self.bytes
    }
}
#[derive(Clone, Copy)]
enum Scope {
    Canonical,
    Observed,
    WrongModule,
    WrongLibrary,
    WrongMajor,
    WrongMinor,
    WrongVersion,
}
fn image(code: &[u8], scope: Scope) -> Input {
    let imports = [
        ("Up36PTk687E", 1),
        ("IWIBBdTHit4", 2),
        ("rKBUtgRrtbk", 1),
        ("U46NwOiJpys", 1),
        ("uquVH4-Du78", 1),
        ("zgXifHT9ErY", 0),
        ("HXzjK9yI30k", 0),
        ("1FZBKy8HeNU", 0),
        ("DYhhWbJSeRg", 0),
        ("pv9CI5VC+R0", 0),
        ("9BcDykPmo1I", 2),
        ("D0OdFMjp46I", 2),
    ];
    let pair = match scope {
        Scope::Canonical => ("VideoOut", "VideoOut"),
        Scope::WrongModule => ("VideoOut", "libSceVideoOut"),
        Scope::WrongLibrary => ("libSceVideoOut", "VideoOut"),
        _ => ("libSceVideoOut", "libSceVideoOut"),
    };
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x18000, 0);
    bytes[0x400..0x1000].fill(0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0x10000u64.to_le_bytes());
    put(&mut bytes, 104, &0x10000u64.to_le_bytes());
    put(&mut bytes, 56, &3u16.to_le_bytes());
    put(&mut bytes, 176, &1u32.to_le_bytes());
    put(&mut bytes, 180, &6u32.to_le_bytes());
    for (at, v) in [
        (184, DATA as u64),
        (192, DATA as u64),
        (208, 0x4000),
        (216, 0x4000),
        (224, 0x4000),
    ] {
        put(&mut bytes, at, &v.to_le_bytes());
    }
    put(&mut bytes, ENTRY as usize, code);
    put(&mut bytes, DATA as usize, b"original-video-alias-queue\0");
    let mut names = vec![0];
    let mut offsets = Vec::new();
    for (module, library) in [pair, ("VideoOut", "VideoOut"), ("libkernel", "libkernel")] {
        let m = names.len() as u64;
        names.extend(module.as_bytes());
        names.push(0);
        let l = names.len() as u64;
        names.extend(library.as_bytes());
        names.push(0);
        offsets.push((m, l));
    }
    let count = imports.len();
    for (index, (nid, group)) in imports.into_iter().enumerate() {
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
    assert!(0x400 + names.len() < 0x600 && 0x600 + (count + 1) * 24 <= 0x800);
    put(&mut bytes, 0x400, &names);
    let mut dynamic = vec![
        (5i64, 0x400u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, (count as u64 + 1) * 24),
        (11, 24),
    ];
    for (index, (m, l)) in offsets.into_iter().enumerate() {
        let major = if index == 0 && matches!(scope, Scope::WrongMajor) {
            2u64
        } else {
            1
        };
        let minor = if index == 0 && matches!(scope, Scope::WrongMinor) {
            2u64
        } else {
            1
        };
        let version = if index == 0 && matches!(scope, Scope::WrongVersion) {
            2u64
        } else {
            1
        };
        let id = 2 + index as u64 * 2;
        dynamic.push((0x6100_000f, m | (minor << 32) | (major << 40) | (id << 48)));
        dynamic.push((0x6100_0015, l | (version << 32) | ((id + 1) << 48)));
    }
    dynamic.extend([(7, 0x800), (8, count as u64 * 24), (9, 24), (0, 0)]);
    for at in [152, 160] {
        put(&mut bytes, at, &((dynamic.len() * 16) as u64).to_le_bytes());
    }
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
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
fn open(p: &mut Code) {
    p.reg(7, 255);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, 0);
    p.call(OPEN);
    p.expect(0, 1);
}
fn pending(p: &mut Code, handle: u64, value: u64) {
    p.reg(7, handle);
    p.reg(6, u64::MAX); // The one-argument ABI must not read an imaginary output.
    p.call(PENDING);
    p.expect(0, value);
}
fn successful(scope: Scope) -> Input {
    let mut p = Code::new();
    open(&mut p);
    pending(&mut p, 1, 0);
    pending(&mut p, 0, 0x8029_000b);
    p.word(32, 0);
    p.ptr(7, 32);
    p.path(6, DATA);
    p.call(CREATE_QUEUE);
    p.expect(0, 0);
    p.stack(0x8b, 7, 32);
    p.reg(6, 1);
    p.reg(2, 0x1234);
    p.call(ADD_EVENT);
    p.expect(0, 0);
    // XMM0 carries the float. Only four destination bytes may change.
    p.word(72, GUARD);
    p.word(80, GUARD);
    p.word(88, GUARD);
    p.reg(0, 0x3fc0_0000);
    p.bytes.extend([0x66, 0x0f, 0x6e, 0xc0]);
    p.ptr(7, 80);
    p.reg(6, u64::MAX);
    p.call(GAMMA);
    p.expect(0, 0);
    p.check(72, GUARD);
    p.check(80, (GUARD & !0xffff_ffff) | 0x3fc0_0000);
    p.check(88, GUARD);
    p.reg(7, 1);
    p.ptr(6, 80);
    p.call(ADJUST);
    p.expect(0, 0);
    // Real clock-dependent fields are not given fabricated numeric counts.
    p.word(120, GUARD);
    p.word(168, GUARD);
    p.reg(7, 1);
    p.ptr(6, 128);
    p.call(VBLANK);
    p.expect(0, 0);
    p.check(120, GUARD);
    p.check(160, 0);
    p.check(168, GUARD);
    p.reg(7, 1);
    p.reg(6, 0);
    p.call(VBLANK);
    p.expect(0, 0x8029_0002);
    p.reg(7, 0);
    p.ptr(6, 128);
    p.call(VBLANK);
    p.expect(0, 0x8029_000b);
    p.reg(7, 0);
    p.call(GAMMA);
    p.expect(0, 0x8029_0002);
    // Actual GPU-readable CPU pixels, registered and captured without a window.
    p.word(40, 0);
    p.ptr(7, 40);
    p.reg(6, 0x40000);
    p.reg(2, 0x33);
    p.reg(1, 0);
    p.call(MAP);
    p.expect(0, 0);
    // Align the actual selected source to64KiB strictly inside its0x40000 mapping.
    p.stack(0x8b, 12, 40);
    p.bytes.extend([0x49, 0x81, 0xc4, 0xff, 0xff, 1, 0]);
    p.bytes.extend([0x49, 0x81, 0xe4, 0, 0, 0xff, 0xff]);
    p.bytes.extend([0x4c, 0x89, 0xe7]);
    p.reg(0, 0xff12_3456_ff12_3456);
    p.reg(1, 65536 / 8);
    p.bytes.extend([0xf3, 0x48, 0xab]);
    for at in (256..336).step_by(8) {
        p.word(at, 0);
    }
    p.word(264, 64u64 << 32);
    p.word(272, 64);
    p.word(288, 0x8000_0000_2200_0000);
    p.stack(0x89, 12, 384);
    for at in [392, 400, 408] {
        p.word(at, 0);
    }
    p.word(0, 0);
    p.word(8, 0);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 0);
    p.ptr(1, 384);
    p.reg(8, 1);
    p.ptr(9, 256);
    p.call(REGISTER);
    p.expect(0, 0);
    pending(&mut p, 1, 0);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 1);
    p.reg(1, 0x1234);
    p.call(FLIP);
    p.expect(0, 0);
    // Capturing a Frame is real pending work, not a display acknowledgement.
    pending(&mut p, 1, 1);
    pending(&mut p, 1, 1);
    p.call(ERRNO);
    p.stack(0x89, 0, 512);
    p.bytes.extend([0xc7, 0, 37, 0, 0, 0]);
    p.word(528, 0x1f84);
    p.bytes.extend([0x0f, 0xae, 0x94, 0x24]);
    p.bytes.extend(528u32.to_le_bytes());
    for (i, v) in [
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
        0x8877_6655_4433_2211,
        0x89ab_cdef_0123_4567,
    ]
    .into_iter()
    .enumerate()
    {
        p.word(544 + i as u32 * 8, v);
    }
    p.bytes.extend([0xc5, 0x7c, 0x10, 0xbc, 0x24]);
    p.bytes.extend(544u32.to_le_bytes());
    p.bytes.extend([0xdb, 0xe3]);
    p.word(592, 0x8000_0000_0000_0007);
    p.word(600, 0x3fff);
    p.bytes.extend([0xdb, 0xac, 0x24]);
    p.bytes.extend(592u32.to_le_bytes());
    pending(&mut p, 1, 1);
    // Consumers use retained YMM15, MXCSR, precise80 and actual current errno.
    p.bytes.extend([0xc5, 0x7c, 0x11, 0xbc, 0x24]);
    p.bytes.extend(544u32.to_le_bytes());
    for (i, v) in [
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210,
        0x8877_6655_4433_2211,
        0x89ab_cdef_0123_4567,
    ]
    .into_iter()
    .enumerate()
    {
        p.check(544 + i as u32 * 8, v);
    }
    p.word(528, GUARD);
    p.bytes.extend([0x0f, 0xae, 0x9c, 0x24]);
    p.bytes.extend(528u32.to_le_bytes());
    p.check(528, (GUARD & !0xffff_ffff) | 0x1f84);
    p.word(608, GUARD);
    p.word(616, GUARD);
    p.bytes.extend([0xdb, 0xbc, 0x24]);
    p.bytes.extend(608u32.to_le_bytes());
    p.check(608, 0x8000_0000_0000_0007);
    p.check(616, (GUARD & !0xffff) | 0x3fff);
    p.call(ERRNO);
    p.stack(0x8b, 10, 512);
    p.bytes.extend([0x4c, 0x39, 0xd0, 0x0f, 0x85, 0, 0, 0, 0]);
    p.failed.push(p.bytes.len() - 4);
    p.bytes.extend([0x8b, 0]);
    p.expect(0, 37);
    p.reg(7, 1);
    p.call(CLOSE);
    p.expect(0, 0);
    pending(&mut p, 1, 0x8029_000b);
    image(&p.finish(), scope)
}
#[test]
fn native_video_aliases_use_exact_imported_abi_real_pending_frames_and_guarded_outputs() {
    for scope in [Scope::Canonical, Scope::Observed] {
        let (status, fields, text) = run(&successful(scope));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["video_flip_ready_with_frame"], "1", "{text}");
        assert_eq!(fields["video_flip_acknowledged"], "0", "{text}");
        for key in [
            "graphics_command_buffers",
            "graphics_draw_attempts",
            "graphics_dispatch_attempts",
            "window_frames_presented",
            "window_frames_acknowledged",
        ] {
            assert_eq!(fields[key], "0", "{key}: {text}");
        }
    }
    println!(
        "video_alias_native_available=true video_alias_native_programs=2 video_alias_native_pending_frames=2"
    );
}
#[test]
fn native_video_aliases_refuse_wrong_scopes_versions_and_actual_output_permissions() {
    let mut cases = Vec::new();
    for scope in [
        Scope::WrongModule,
        Scope::WrongLibrary,
        Scope::WrongMajor,
        Scope::WrongMinor,
        Scope::WrongVersion,
    ] {
        let mut p = Code::new();
        pending(&mut p, 1, 0);
        cases.push((
            image(&p.finish(), scope),
            if matches!(scope, Scope::WrongModule | Scope::WrongLibrary) {
                "unavailable-function"
            } else {
                "import-version-mismatch"
            },
            None,
        ));
    }
    for role in [GAMMA, VBLANK, ADJUST] {
        let mut p = Code::new();
        open(&mut p);
        if role == GAMMA {
            p.reg(0, 0x3f80_0000);
            p.bytes.extend([0x66, 0x0f, 0x6e, 0xc0]);
            p.path(7, ENTRY);
        } else {
            p.reg(7, 1);
            if role == VBLANK {
                p.path(6, ENTRY);
            } else {
                p.reg(6, u64::MAX);
            }
        }
        p.call(role);
        cases.push((
            image(&p.finish(), Scope::Observed),
            "runtime-service",
            Some("graphics-memory"),
        ));
    }
    assert_eq!(cases.len(), 8);
    for (source, expected, failure) in cases {
        let (status, fields, text) = run(&source);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], expected, "{text}");
        if let Some(failure) = failure {
            assert_eq!(fields["runtime_failure"], failure, "{text}");
        }
        assert!(!fields.contains_key("value"), "{text}");
        for key in [
            "graphics_command_buffers",
            "window_frames_presented",
            "window_frames_acknowledged",
        ] {
            assert_eq!(fields[key], "0", "{key}: {text}");
        }
    }
    println!("video_alias_native_refusal_programs=8 video_alias_native_refusals=8");
}
