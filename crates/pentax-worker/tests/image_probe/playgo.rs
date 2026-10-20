// SPDX-License-Identifier: GPL-2.0-only
//! Original native PlayGo consumers with an explicit generated package header.

use super::*;

const CANARY: u64 = 0xa5a5_a5a5_a5a5_a5a5;

fn save_word(p: &mut Program, offset: u8, value: u64) {
    p.reg(0, value);
    p.save(offset);
}

fn run(mut p: Program, imports: &[&str], scope: &str) {
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d, 0xc3]);
    finish_checks(&mut p.code, p.branches);
    assert!(
        p.code.len() < 0x700,
        "Original code must end before its GOT"
    );
    let mut main = scoped_system_image(&p.code, imports, scope, scope, 1);
    // PlayGo's exact source module version is1.0; the shared helper uses1.1.
    put(
        &mut main.bytes,
        0x258,
        &(1_u64 | (1 << 40) | (2 << 48)).to_le_bytes(),
    );
    // The helper installs unused filesystem names inside its code window.
    put(&mut main.bytes, 0x800, &p.code);
    std::fs::write(&main.path, &main.bytes).unwrap();
    let mut header = vec![0; 12];
    header[..4].copy_from_slice(b"plgo");
    header[10..12].copy_from_slice(&3_u16.to_le_bytes());
    let data = create_input(header);
    let mut binding = OsString::from("/app0/sce_sys/playgo-chunk.dat=");
    binding.push(&data.path);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-file".into(),
        binding,
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["service_stops"], p.calls.to_string(), "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

#[test]
fn native_observed_playgo_aliases_read_real_metadata_and_preserve_stateless_lifecycle() {
    let mut p = Program::new();
    save_word(&mut p, 32, CANARY);
    p.ptr(7, 32);
    p.reg(6, 0);
    p.call(2); // Open succeeds before Initialize, using only the explicit file.
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0001);
    save_word(&mut p, 0, u64::MAX); // Opaque init buffer is never dereferenced.
    save_word(&mut p, 8, 0xa5a5_a5a5_0020_0000);
    p.ptr(7, 0);
    p.call(0);
    p.expect(0);
    p.reg(7, 0xeeee_0000_0000_0001);
    p.reg(6, 0xdddd_0000_0000_0002);
    p.call(4);
    p.expect(0);
    save_word(&mut p, 64, CANARY);
    p.reg(7, 1);
    p.ptr(6, 64);
    p.reg(2, 0xcccc_0000_0000_0008);
    p.ptr(1, 68); // Count-last store replaces the final ID and its tail.
    p.call(5);
    p.expect(0);
    p.word(64, 0x0000_0003_0001_0000);
    save_word(&mut p, 80, CANARY);
    p.reg(7, 1);
    p.reg(6, u64::MAX); // ToDo output is ignored even at positive capacity.
    p.reg(2, 1);
    p.ptr(1, 80);
    p.call(6);
    p.expect(0);
    p.word(80, 0xa5a5_a5a5_0000_0000);
    p.reg(7, 0xaaaa_0000_0000_0001);
    p.call(3);
    p.expect(0);
    p.call(1);
    p.expect(0);
    p.ptr(7, 32);
    p.reg(6, 0);
    p.call(2); // Terminate/Close do not discard the cached chunk count.
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0001);
    run(
        p,
        &[
            "ts6GlZOKRrE",
            "MPe0EeBGM-E",
            "M1Gma1ocrGE",
            "Uco1I0dlDi8",
            "4AAcTU9R3XM",
            "73fF1MFU8hA",
            "Nn7zKwnA5q0",
        ],
        "libScePlayGo",
    );
}

#[test]
fn native_playgo_locus_aliases_entry_progress_and_validated_todo_follow_source_order() {
    let mut p = Program::new();
    save_word(&mut p, 0, 0x0002_0002_0001_0000);
    save_word(&mut p, 16, CANARY);
    p.reg(7, 1);
    p.ptr(6, 0);
    p.reg(2, 3);
    p.ptr(1, 16);
    p.call(0);
    p.expect(0);
    p.word(16, 0xa5a5_a5a5_a503_0303);
    save_word(&mut p, 32, 0);
    p.reg(7, 1);
    p.ptr(6, 32);
    p.reg(2, 3);
    p.ptr(1, 34); // Iteration0 modifies iteration1's input ID to3.
    p.call(0);
    p.expect(0x80b2_000c);
    p.word(32, 0x0000_0000_0003_0000);
    save_word(&mut p, 64, CANARY);
    save_word(&mut p, 72, CANARY);
    p.reg(7, 1);
    p.ptr(6, 0);
    p.reg(2, 3);
    p.ptr(1, 64);
    p.call(5);
    p.expect(0);
    p.word(64, 3);
    p.word(72, 3);
    save_word(&mut p, 96, CANARY);
    p.reg(7, 1);
    p.ptr(6, 0);
    p.reg(2, 3);
    p.ptr(1, 96);
    p.call(4);
    p.expect(0);
    p.word(96, 0);
    save_word(&mut p, 104, CANARY);
    p.reg(7, 1);
    p.ptr(6, 104);
    p.call(1);
    p.expect(0);
    p.word(104, 0xa5a5_a5a5_0000_0002);
    save_word(&mut p, 112, 0xff03_0002);
    p.reg(7, 1);
    p.ptr(6, 112);
    p.reg(2, 1);
    p.call(2);
    p.expect(0); // Reserved byte0xff is ignored.
    p.reg(7, 1);
    p.ptr(6, 0);
    p.reg(2, 3);
    p.reg(1, 0xdddd_0000_0000_0003);
    p.call(3);
    p.expect(0); // Signed low8 locus3, not the entire register.
    p.reg(1, 1);
    p.call(3);
    p.expect(0x80b2_0010);
    run(
        p,
        &[
            "uWIYLFkkwqk",
            "rvBSfTimejE",
            "gUPGiOQ1tmQ",
            "-Q1-u1a7p0g",
            "v6EZ-YWRdMs",
            "-RJWNMK3fC8",
        ],
        "PlayGo",
    );
}

#[test]
fn native_playgo_optional_masks_unread_opaque_option_and_install_chunk_list_have_exact_tails() {
    let mut p = Program::new();
    save_word(&mut p, 0, CANARY);
    p.reg(7, 1);
    p.ptr(6, 0);
    p.call(0);
    p.expect(0);
    p.word(0, u64::MAX);
    save_word(&mut p, 8, CANARY);
    p.reg(7, 1);
    p.reg(6, 0);
    p.ptr(2, 8);
    p.call(1);
    p.expect(0);
    p.word(8, u64::MAX);
    save_word(&mut p, 16, CANARY);
    p.reg(7, 1);
    p.reg(6, 0xaaaa_0000_0000_0001);
    p.ptr(2, 16);
    p.call(1);
    p.expect(0);
    p.word(16, 31);
    save_word(&mut p, 24, CANARY);
    p.reg(7, 1);
    p.reg(6, 1);
    p.ptr(2, 24);
    p.call(4);
    p.expect(0);
    p.word(24, 31);
    p.reg(7, 1);
    p.reg(6, 1);
    p.reg(2, u64::MAX);
    p.call(2); // A nonnull option is never read by PrefetchOptionalChunk.
    p.expect(0);
    save_word(&mut p, 32, CANARY);
    save_word(&mut p, 40, CANARY);
    p.reg(7, 1);
    p.ptr(6, 32);
    p.reg(2, 3);
    p.ptr(1, 40);
    p.call(3);
    p.expect(0);
    p.word(32, 0xa5a5_0002_0001_0000);
    p.word(40, 0xa5a5_a5a5_0000_0003);
    save_word(&mut p, 56, CANARY);
    p.reg(7, 1);
    p.reg(6, 2);
    p.ptr(2, 56);
    p.call(1);
    p.expect(0x80b2_0024);
    p.word(56, CANARY);
    run(
        p,
        &[
            "3OMbYZBaa50",
            "g4AZyxpSAlA",
            "HVAa744ecdw",
            "8-e7E989rCU",
            "IfiN+-oeVWI",
        ],
        "PlayGo",
    );
}
