// SPDX-License-Identifier: GPL-2.0-only
use super::*;

#[test]
fn native_calendar_conversions_use_configured_offsets_and_source_output_widths() {
    let mut p = Program::new();
    p.call(0);
    p.save(0);
    p.set_errno(37);
    p.ptr(7, 48);
    p.call(1);
    p.expect(0);
    let zone = u64::from((-330_i32) as u32) | (4_u64 << 32);
    p.word(48, zone);
    p.reg(7, (-10_i64) as u64);
    p.ptr(6, 16);
    p.ptr(2, 24);
    p.ptr(1, 40);
    p.call(2);
    p.expect(0);
    p.word(16, 21_590);
    p.word(24, (-10_i64) as u64);
    p.word(32, 19_800 | (1_800_u64 << 32));
    p.word(40, 1_800);
    p.set(72, 0);
    p.reg(7, 21_590);
    p.reg(6, u64::MAX); // Source ignores the reserved argument.
    p.ptr(2, 56);
    p.ptr(1, 64);
    p.ptr(8, 72);
    p.call(3);
    p.expect(0);
    p.word(56, (-10_i64) as u64);
    p.word(64, zone);
    p.word(72, 1_800);
    p.reg(7, 0);
    p.call(1);
    p.expect(0x8002_000e);
    p.errno(37);
    let (code, _) = p.finish(None);
    let source = image(
        &code,
        &["9BcDykPmo1I", "kOcnerypnQA", "-o5uEDpN+oY", "0NTHN1NKONI"],
    );
    let config = create_input(b"schema_version = 1\n[system]\ntimezone_offset_minutes = 330\ndaylight_saving = true\ndaylight_saving_offset_minutes = 30\n".to_vec());
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--config".into(),
        config.path.as_os_str().to_owned(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
}
