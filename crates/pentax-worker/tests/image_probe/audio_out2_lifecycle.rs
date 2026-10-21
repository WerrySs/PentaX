// SPDX-License-Identifier: GPL-2.0-only
//! Original headless PCM, resource and cooperative pacing consumers.
//! No physical audio device or audible-output assertion is made.

use super::*;

const ENTRY: i32 = 0x1000;
const CHILD: i32 = 0x4000;
const FRAME: u32 = 0x5000;
const FRAMES: u64 = 4096;
const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;
const VECTOR: [u64; 4] = [
    0x0123_4567_89ab_cdef,
    0xfedc_ba98_7654_3210,
    0x8877_6655_4433_2211,
    0x89ab_cdef_0123_4567,
];
const IMPORTS: [&str; 17] = [
    "t5YrizufpQc", // ContextResetParam
    "0x6o1VVAYSY", // ContextCreate
    "on6ZH7Abo10", // ContextDestroy
    "PE2zHMqLSHs", // ContextAdvance
    "aII9h5nli9U", // ContextPush
    "R7d0F1g2qsU", // ContextGetQueueLevel
    "xywYcRB7nbQ", // UserCreate
    "IaZXJ9M79uo", // UserDestroy
    "JK2wamZPzwM", // PortCreate
    "cd+Rtw+D1x8", // PortDestroy
    "gatEUKG+Ea4", // PortGetState
    "8XTArSPyWHk", // PortSetAttributes
    "6UgtwV+0zb4", // PthreadCreate
    "BpFoboUJoZU", // CreateEventFlag
    "JTvBflhYazQ", // WaitEventFlag
    "IOnSvHzqu6A", // SetEventFlag
    "8mql9OcQnd4", // DeleteEventFlag
];

fn address(p: &mut Program, register: u8, offset: i32) {
    p.code.extend([
        0x48 | if register >= 8 { 4 } else { 0 },
        0x8d,
        0x84 | ((register & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn load(p: &mut Program, register: u8, offset: i32) {
    p.code.extend([
        0x48 | if register >= 8 { 4 } else { 0 },
        0x8b,
        0x84 | ((register & 7) << 3),
        0x24,
    ]);
    p.code.extend(offset.to_le_bytes());
}
fn save(p: &mut Program, offset: i32) {
    p.code.extend([0x48, 0x89, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn store(p: &mut Program, offset: i32, value: u64) {
    p.reg(0, value);
    save(p, offset);
}
fn word(p: &mut Program, offset: i32, expected: u64) {
    load(p, 0, offset);
    p.expect(expected);
}
fn call(p: &mut Program, base: i32, slot: i32) {
    rip(&mut p.code, base, &[0xff, 0x15], 0xf00 + slot * 8);
    p.calls += 1;
}
fn finish(mut p: Program, frame: u32, result: u64) -> Vec<u8> {
    p.reg(0, result);
    p.code.extend([0x48, 0x81, 0xc4]);
    p.code.extend(frame.to_le_bytes());
    p.code.extend([0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    p.code
}
fn seed_context(p: &mut Program, child: bool) {
    let (save_rsp, seed, mxcsr) = if child { (8, 16, 120) } else { (56, 64, 96) };
    p.stack(0x89, 4, save_rsp);
    for (index, value) in VECTOR.into_iter().enumerate() {
        store(
            p,
            i32::from(seed) + index as i32 * 8,
            value ^ u64::from(child),
        );
    }
    store(
        p,
        i32::from(mxcsr),
        (CANARY & !0xffff_ffff) | if child { 0x3f80 } else { 0x5f80 },
    );
    p.code.extend([0x0f, 0xae, 0x54, 0x24, mxcsr]);
    p.code.extend([0xc5, 0x7e, 0x6f, 0x7c, 0x24, seed]);
}
fn check_context(p: &mut Program, child: bool) {
    let (save_rsp, output, mxcsr) = if child { (8, 64, 120) } else { (56, 64, 96) };
    p.code.extend([0xc5, 0x7e, 0x7f, 0x7c, 0x24, output]);
    for (index, value) in VECTOR.into_iter().enumerate() {
        word(
            p,
            i32::from(output) + index as i32 * 8,
            value ^ u64::from(child),
        );
    }
    p.code.extend([0x0f, 0xae, 0x5c, 0x24, mxcsr]);
    word(
        p,
        i32::from(mxcsr),
        (CANARY & !0xffff_ffff) | if child { 0x3f80 } else { 0x5f80 },
    );
    p.code.extend([0x48, 0x89, 0xe0]);
    p.load(10, save_rsp);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
}
fn queue(p: &mut Program, depth: u64) {
    load(p, 7, 0);
    address(p, 6, 432);
    address(p, 2, 436);
    call(p, ENTRY, 5);
    p.expect(0);
    word(p, 432, depth << 32);
    word(p, 424, CANARY);
    word(p, 440, CANARY);
}
fn port_state(p: &mut Program, channels: u64) {
    p.reg(0, CANARY);
    for offset in (344..=416).step_by(8) {
        save(p, offset);
    }
    load(p, 7, 16);
    address(p, 6, 352);
    call(p, ENTRY, 10);
    p.expect(0);
    word(p, 352, (127 << 32) | (channels << 16) | 1);
    for offset in (360..416).step_by(8) {
        word(p, offset, 0);
    }
    word(p, 344, CANARY);
    word(p, 416, CANARY);
}

fn parent(pcm: bool) -> Program {
    let mut p = Program::new();
    p.code[4..8].copy_from_slice(&FRAME.to_le_bytes());
    p.reg(0, CANARY);
    for offset in (0..=440).step_by(8) {
        save(&mut p, offset);
    }
    for offset in [0, 8, 16, 24, 32, 40, 48] {
        store(&mut p, offset, 0);
    }
    seed_context(&mut p, false);
    // Actual per-thread self pointers remain guest values, never host handles.
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    save(&mut p, 112);
    address(&mut p, 7, 128);
    call(&mut p, ENTRY, 0);
    p.expect(0);
    store(&mut p, 128, if pcm { 1 << 32 } else { 0 });
    store(&mut p, 136, if pcm { 2 << 32 } else { 1 << 32 });
    store(&mut p, 144, (1 << 32) | FRAMES);
    address(&mut p, 7, 128);
    p.reg(6, u64::MAX); // Public source ignores the caller buffer and its size.
    p.reg(2, u64::MAX);
    address(&mut p, 1, 0);
    call(&mut p, ENTRY, 1);
    p.expect(0);
    word(&mut p, 0, 1);
    queue(&mut p, if pcm { 2 } else { 1 });

    if pcm {
        p.reg(7, u64::MAX); // Opaque u32 user label; no account or PSN claim.
        address(&mut p, 6, 8);
        call(&mut p, ENTRY, 6);
        p.expect(0);
        word(&mut p, 8, 1);
        store(&mut p, 192, 0x0000_0100_a5a5_0100); // Object FloatMono.
        store(&mut p, 200, 0xa5a5_a5a5_0000_bb80); // 48000 Hz, opaque flags.
        load(&mut p, 0, 8);
        save(&mut p, 208);
        load(&mut p, 7, 0);
        address(&mut p, 6, 192);
        address(&mut p, 2, 16);
        call(&mut p, ENTRY, 8);
        p.expect(0);
        word(&mut p, 16, 1);
        port_state(&mut p, 1);
        load(&mut p, 7, 8);
        call(&mut p, ENTRY, 7);
        p.expect(0x8026_8007); // The real live port keeps its user BUSY.

        address(&mut p, 7, 512);
        p.reg(0, 0x3f80_0000);
        p.reg(1, FRAMES);
        p.code.extend([0xf3, 0xab]); // Original bounded PCM float32 samples.
        address(&mut p, 0, 512);
        save(&mut p, 328);
        store(&mut p, 336, 0x3f00_0000); // Gain0.5 and ambisonics0.
        for (at, id, value, size) in [(256, 1, 336, 4), (280, 8, 340, 4), (304, 0, 328, 8)] {
            store(&mut p, at, (CANARY & !0xffff_ffff) | id);
            address(&mut p, 0, value);
            save(&mut p, at + 8);
            store(&mut p, at + 16, size);
        }
        load(&mut p, 7, 16);
        address(&mut p, 6, 256);
        p.reg(2, 3);
        call(&mut p, ENTRY, 11);
        p.expect(0);
        // The retained snapshot must not borrow these subsequently changed
        // guest samples; literal mixing values are checked by kernel fixtures.
        store(&mut p, 512, 0x7fc0_0000_7fc0_0000);
        load(&mut p, 7, 0);
        call(&mut p, ENTRY, 3); // Unlock this real object's non-PCM attributes.
        p.expect(0);
        store(&mut p, 264, 0); // Gain's now-live null value must be rejected.
        load(&mut p, 7, 16);
        address(&mut p, 6, 256);
        p.reg(2, 1);
        call(&mut p, ENTRY, 11);
        p.expect(0x8026_8001); // A no-op Advance would wrongly skip this check.
    } else {
        address(&mut p, 7, 48);
        rip(&mut p.code, ENTRY, &[0x48, 0x8d, 0x35], NAME);
        p.reg(2, 0x20);
        p.reg(1, 0);
        p.reg(8, 0);
        call(&mut p, ENTRY, 13);
        p.expect(0);
    }

    load(&mut p, 7, 0);
    p.reg(6, u64::from(pcm));
    call(&mut p, ENTRY, 4); // First block has no artificial leading delay.
    p.expect(0);
    address(&mut p, 7, 40);
    p.reg(6, 0);
    rip(&mut p.code, ENTRY, &[0x48, 0x8d, 0x15], CHILD);
    address(&mut p, 1, 0);
    p.reg(8, 0);
    call(&mut p, ENTRY, 12); // Existing scheduler forces the real child first.
    p.expect(0);
    word(&mut p, 24, 1);
    word(&mut p, 32, 0); // Its blocking push has not returned yet.
    if pcm {
        load(&mut p, 7, 0);
        call(&mut p, ENTRY, 2); // Must defer behind the child's paced lease.
        p.expect(0);
    } else {
        load(&mut p, 7, 48);
        p.reg(6, 1);
        p.reg(2, 0x21);
        address(&mut p, 1, 432);
        p.reg(8, 0);
        call(&mut p, ENTRY, 14); // Real event wait, not a guest polling loop.
        p.expect(0);
        word(&mut p, 432, 1);
        load(&mut p, 7, 48);
        call(&mut p, ENTRY, 16);
        p.expect(0);
        load(&mut p, 7, 0);
        call(&mut p, ENTRY, 2);
        p.expect(0);
    }
    word(&mut p, 24, 2);
    word(&mut p, 32, 42);
    check_context(&mut p, false);
    if pcm {
        load(&mut p, 7, 8);
        call(&mut p, ENTRY, 7); // Cascade released the port's user reference.
        p.expect(0);
        port_state(&mut p, 2); // Unknown-port output is the source default.
        load(&mut p, 7, 16);
        call(&mut p, ENTRY, 9);
        p.expect(0);
    }
    queue(&mut p, 4); // Unknown context writes0/4, preserving both canaries.
    load(&mut p, 7, 0);
    call(&mut p, ENTRY, 3);
    p.expect(0);
    load(&mut p, 7, 0);
    p.reg(6, 0);
    call(&mut p, ENTRY, 4);
    p.expect(0x8026_8008); // Unknown nonblocking push is never fake success.
    load(&mut p, 7, 0);
    call(&mut p, ENTRY, 2);
    p.expect(0);
    p
}

fn child(pcm: bool) -> Program {
    let mut p = Program::new();
    p.stack(0x89, 7, 0);
    seed_context(&mut p, true);
    p.load(11, 0);
    p.code.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]);
    p.code.extend([
        0x4d, 0x8b, 0x53, 112, 0x4c, 0x39, 0xd0, 0x0f, 0x84, 0, 0, 0, 0,
    ]);
    p.branches.push(p.code.len() - 4); // Different actual TLS self pointer.
    p.code.extend([0x49, 0xc7, 0x43, 24, 1, 0, 0, 0]);
    p.code.extend([0x49, 0x8b, 0x3b]);
    p.reg(6, 1);
    call(&mut p, CHILD, 4);
    p.expect(0);
    check_context(&mut p, true);
    p.load(11, 0);
    p.code.extend([0x49, 0xc7, 0x43, 24, 2, 0, 0, 0]);
    p.code.extend([0x49, 0xc7, 0x43, 32, 42, 0, 0, 0]);
    if !pcm {
        p.code.extend([0x49, 0x8b, 0x7b, 48]);
        p.reg(6, 1);
        call(&mut p, CHILD, 15);
        p.expect(0);
    }
    p
}

fn original_image(parent: Program, child: Program) -> (Input, u64) {
    let calls = parent.calls + child.calls;
    let mut code = finish(parent, FRAME, 42);
    assert!(code.len() < (CHILD - ENTRY) as usize);
    code.resize((CHILD - ENTRY) as usize, 0xcc);
    code.extend(finish(child, 0x100, 73));
    let template = scoped_system_image(&[], &[], "libSceAudioOut", "libSceAudioOut2", 1);
    let mut bytes = template.bytes.clone();
    bytes.resize(0xc000, 0);
    bytes[0x400..0xa00].fill(0);
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 96, &0xc000_u64.to_le_bytes());
    put(&mut bytes, 104, &0xc000_u64.to_le_bytes());
    put(&mut bytes, ENTRY as usize, &code);
    put(&mut bytes, NAME as usize, b"original-headless-audio\0");
    let module = b"libSceAudioOut\0";
    let library = b"libSceAudioOut2\0";
    let kernel = b"libkernel\0";
    let library_at = 1 + module.len();
    let kernel_at = library_at + library.len();
    let mut names = vec![0];
    names.extend(module);
    names.extend(library);
    names.extend(kernel);
    for (index, nid) in IMPORTS.iter().enumerate() {
        let row = 0x600 + (index + 1) * 24;
        put(&mut bytes, row, &(names.len() as u32).to_le_bytes());
        bytes[row + 4] = 0x12;
        names.extend(format!("{nid}#{}\0", if index < 12 { "D#C" } else { "F#E" }).bytes());
        put(
            &mut bytes,
            0x800 + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x808 + index * 24,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(names.len() < 0x200);
    put(&mut bytes, 0x400, &names);
    let entries = [
        (5_i64, 0x400_u64),
        (10, names.len() as u64),
        (6, 0x600),
        (39, (IMPORTS.len() as u64 + 1) * 24),
        (11, 24),
        (0x6100_000f, 1 | (1 << 32) | (1 << 40) | (2 << 48)),
        (0x6100_0015, library_at as u64 | (1 << 32) | (3 << 48)),
        (
            0x6100_000f,
            kernel_at as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x6100_0015, kernel_at as u64 | (1 << 32) | (5 << 48)),
        (7, 0x800),
        (8, IMPORTS.len() as u64 * 24),
        (9, 24),
        (0, 0),
    ];
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(entries.len() as u64 * 16).to_le_bytes());
    }
    (create_input(bytes), calls)
}

#[test]
fn native_audio_out2_headless_pcm_lease_and_full_queue_preserve_real_thread_continuations() {
    for pcm in [true, false] {
        let (source, calls) = original_image(parent(pcm), child(pcm));
        assert_eq!(calls, if pcm { 21 } else { 15 });
        let (status, fields, text) = probe(&source);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["phase"], "main-entry", "{text}");
        assert_modeled_vector_counters(&fields, [(8, 144), (4, 16), (4, 16)]);
        assert_eq!(
            fields["string_elements"],
            if pcm { "4096" } else { "0" },
            "{text}"
        );
        assert_eq!(
            fields["audio_mix_frames"],
            if pcm { "8192" } else { "0" },
            "{text}"
        );
        assert_eq!(
            fields["audio_logical_output_batches"],
            if pcm { "2" } else { "0" },
            "{text}"
        );
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        assert!(
            !fields["kernel_handles_by_type"]
                .split(',')
                .any(|value| value.starts_with("audio2:")),
            "{text}"
        );
        assert!(
            fields["native_instructions"].parse::<u64>().unwrap() > 0,
            "{text}"
        );
    }
}
