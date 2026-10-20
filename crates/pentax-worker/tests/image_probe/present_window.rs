// SPDX-License-Identifier: GPL-2.0-only
//! Original guest CPU pixels through the real worker-owned presentation path.
//! This opt-in fixture requires an actual display; it never substitutes a
//! transport mock, elapsed-time tick, or numeric presentation receipt.

use super::*;

const NIDS: [&str; 8] = [
    "Up36PTk687E", // VideoOutOpen
    "rKBUtgRrtbk", // VideoOutRegisterBuffers2
    "U46NwOiJpys", // VideoOutSubmitFlip
    "SbU3dwp80lQ", // VideoOutGetFlipStatus
    "zgXifHT9ErY", // VideoOutIsFlipPending
    "uquVH4-Du78", // VideoOutClose
    "IWIBBdTHit4", // KernelMapFlexibleMemory, independent libkernel scope
    "QcteRwbsnV0", // KernelUsleep, genuine cooperative timed park
];
const ATTRIBUTE: i32 = 0xd00;
const PIXELS: u64 = 0xff9f_5f1f_ff9f_5f1f;

fn fail_if(p: &mut Program, opcode: u8) {
    p.code.extend([0x0f, opcode, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4);
}

fn original_presenter_image() -> Input {
    let mut p = Program::new();
    // The allocation itself is guest-requested CPU/GPU RW, with enough spare
    // backing for a complete 64KiB tile after rounding its base up to 64KiB.
    p.set(16, 0);
    p.ptr(7, 16);
    p.reg(6, 0x30000);
    p.reg(2, 0x33);
    p.reg(1, 0);
    p.call(6);
    p.expect(0);
    p.load(12, 16);
    p.code.extend([0x49, 0x81, 0xc4, 0xff, 0xff, 0, 0]); // ADD R12,65535
    p.code.extend([0x49, 0x81, 0xe4, 0, 0, 0xff, 0xff]); // AND R12,-65536
    p.code.extend([0x4c, 0x89, 0xe7]); // MOV RDI,R12
    p.reg(0, PIXELS);
    p.reg(1, 65536 / 8);
    // The actual guest writes every padded source byte. Uniform original
    // RGBA8 pixels work for the declared tile layout without a fake linear
    // reinterpretation, a host fill, or a retained commercial image.
    p.code.extend([0xf3, 0x48, 0xab]); // REP STOSQ
    p.code.extend([0x49, 0x8b, 0x04, 0x24]); // MOV RAX,[R12]
    p.expect(PIXELS);

    p.reg(7, 255);
    p.reg(6, 0);
    p.reg(2, 0);
    p.reg(1, 0);
    p.call(0);
    p.expect(1);
    p.set(0, 0); // seventh argument: category0, no DCC
    p.set(8, 0); // eighth argument: reserved0
    p.code.extend([0x4c, 0x89, 0xe0]); // MOV RAX,R12
    p.save(32); // real registered surface address
    for offset in [40, 48, 56] {
        p.set(offset, 0);
    }
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 0);
    p.ptr(1, 32);
    p.reg(8, 1);
    p.path(9, ATTRIBUTE);
    p.call(1);
    p.expect(0);

    // Source status starts at count0. Registration/exporting pixels alone
    // cannot advance it; only the completed consumer acknowledgement can.
    p.reg(7, 1);
    p.ptr(6, 0);
    p.call(3);
    p.expect(0);
    p.word(0, 0);
    p.reg(7, 1);
    p.reg(6, 0);
    p.reg(2, 1); // CPU/vsync flip, not GPU-EOP
    p.reg(1, 7);
    p.call(2);
    p.expect(0);

    p.reg(13, 0);
    let again = p.code.len();
    p.reg(7, 1);
    p.ptr(6, 0);
    p.call(3);
    p.expect(0);
    p.code.extend([0x49, 0xff, 0xc5]); // INC R13: finite observations
    p.load(0, 0);
    p.code.extend([0x48, 0x85, 0xc0]); // TEST RAX,RAX
    p.code.extend([0x0f, 0x85, 0, 0, 0, 0]);
    let done = p.code.len() - 4;
    p.code.extend([0x49, 0x81, 0xfd, 200, 0, 0, 0]); // CMP R13,200
    fail_if(&mut p, 0x83); // JAE failure: no timing-only success
    p.reg(7, 5_000);
    p.call(7);
    p.expect(0);
    p.code.push(0xe9);
    let at = p.code.len();
    p.code
        .extend((again as i32 - (at as i32 + 4)).to_le_bytes());
    let end = p.code.len();
    put(
        &mut p.code,
        done,
        &(end as i32 - (done as i32 + 4)).to_le_bytes(),
    );

    p.expect(1); // exactly one completed core flip, never just a swap submission
    p.word(24, 7); // original full-width flip argument survives the transport
    p.word(48, 0); // GPU-pending + pending i32 words both zero
    p.word(56, 0); // current buffer0 plus reserved neighbor0
    p.load(0, 40); // same process-clock counter domain as the original submit
    p.load(10, 64);
    p.code.extend([0x4c, 0x39, 0xd0]);
    fail_if(&mut p, 0x82); // JB failure
    p.reg(7, 1);
    p.reg(6, 0);
    p.call(4);
    p.expect(0);
    // Another real native service/read does not duplicate the receipt.
    p.reg(7, 1);
    p.ptr(6, 0);
    p.call(3);
    p.expect(0);
    p.word(0, 1);
    p.reg(7, 1);
    p.call(5);
    p.expect(0);
    let (code, static_calls) = p.finish(None);
    assert_eq!(static_calls, 10);

    let mut input = scoped_system_image(&code, &NIDS, "VideoOut", "VideoOut", 1);
    // Preserve the ordinary original ELF builder and its table limits. Only
    // these two kernel imports get independent exact module/library IDs.
    let mut names = b"\0VideoOut\0VideoOut\0".to_vec();
    let mut kernel_name = 0;
    for (index, nid) in NIDS.iter().enumerate() {
        if index == 6 {
            kernel_name = names.len() as u64;
            names.extend_from_slice(b"libkernel\0");
        }
        put(
            &mut input.bytes,
            0x600 + (index + 1) * 24,
            &(names.len() as u32).to_le_bytes(),
        );
        names.extend_from_slice(
            format!("{nid}#{}\0", if index < 6 { "D#C" } else { "F#E" }).as_bytes(),
        );
    }
    assert!(names.len() < 0x200);
    input.bytes[0x400..0x600].fill(0);
    put(&mut input.bytes, 0x400, &names);
    put(&mut input.bytes, 0x218, &(names.len() as u64).to_le_bytes());
    for (at, tag, value) in [
        (
            0x2a0,
            0x6100_000f_i64,
            kernel_name | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x2b0, 0x6100_0015_i64, kernel_name | (1 << 32) | (5 << 48)),
        (0x2c0, 0, 0),
    ] {
        put(&mut input.bytes, at, &tag.to_le_bytes());
        put(&mut input.bytes, at + 8, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut input.bytes, at, &(13_u64 * 16).to_le_bytes());
    }
    let mut attribute = [0; 80];
    attribute[12..16].copy_from_slice(&64_u32.to_le_bytes());
    attribute[16..20].copy_from_slice(&64_u32.to_le_bytes());
    attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000_u64.to_le_bytes());
    put(&mut input.bytes, ATTRIBUTE as usize, &attribute);
    std::fs::write(&input.path, &input.bytes).unwrap();
    input
}

#[test]
#[ignore = "requires a visible primary monitor and actual Vulkan presentation completion"]
fn original_native_cpu_flip_is_presented_by_local_window_and_acknowledged_once() {
    let input = original_presenter_image();
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        input.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "startup".into(),
        "--present-window".into(),
        "yes".into(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["window_configured"], "1", "{text}");
    assert_eq!(fields["window_frames_presented"], "1", "{text}");
    assert_eq!(fields["window_frames_acknowledged"], "1", "{text}");
    assert!(fields["native_instructions"].parse::<u64>().unwrap() > 0);
    // Eight fixed calls plus n status observations and n-1 actual timed waits.
    // No implicit clock/equeue service is invented for the real core ack.
    let calls = fields["service_stops"].parse::<u64>().unwrap();
    assert!((9..=407).contains(&calls) && calls % 2 == 1, "{text}");
    println!("local_presenter_composed_available=true local_presenter_composed_frames=1");
}
