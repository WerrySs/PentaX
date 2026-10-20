// SPDX-License-Identifier: GPL-2.0-only
use super::*;

const GAME_PATH: &str = "/app0/sce_module/libSceNpCppWebApi.prx";
const SYSMODULE_NIDS: &[&str] = &["g8cM39EUZ6o", "eR2bZFAAU0Q", "fMP5NHUOaMk", "hHrGoGoNf+s"];

fn game_consumer(p: Program) -> (Input, u64) {
    let (code, calls) = p.finish(None);
    (
        scoped_system_image(&code, SYSMODULE_NIDS, "Sysmodule", "Sysmodule", 1),
        calls,
    )
}

fn counter_is(p: &mut Program, value: u64) {
    p.code.extend([0x64, 0x48, 0x8b, 4, 0x25, 0x28, 0, 0, 0]);
    p.expect(value);
}

#[test]
fn native_sysmodule_game_file_starts_once_then_retains_without_returning_a_kernel_handle() {
    let mut p = Program::new();
    p.reg(7, 0x115);
    p.call(0);
    p.expect(0); // Sysmodule returns zero, not the new physical module handle.
    counter_is(&mut p, 1);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(16);
    p.reg(7, 0xabcd_0115);
    p.reg(6, 1 << 32);
    p.reg(2, 2 << 32);
    p.reg(1, 3 << 32);
    p.ptr(8, 16);
    p.call(3);
    p.expect(0);
    p.word(16, 0xa5a5_a5a5_0000_0000);
    for (slot, result) in [(1, 0), (2, 0), (1, 0), (2, 0x805a_1001), (0, 0)] {
        p.reg(7, 0x115);
        p.call(slot);
        p.expect(result);
    }
    counter_is(&mut p, 1); // Unload/reload references do not restart the image.
    let (main, calls) = game_consumer(p);
    let mut initializer = vec![0x48, 0x09, 0xf7, 0x48, 0x09, 0xd7, 0x75, 0x1b];
    initializer.extend([
        0x64, 0x48, 0x8b, 4, 0x25, 0x28, 0, 0, 0, 0x83, 0xc0, 1, 0x64, 0x48, 0x89, 4, 0x25, 0x28,
        0, 0, 0, 0xb8, 7, 0, 0, 0, 0xc3, 0x0f, 0x0b,
    ]);
    // A real positive initializer result must not leak to the Sysmodule caller.
    let library = module_input(&initializer, false, Some(0x100));
    let (status, fields, text) = probe_dynamic(&main, &[(GAME_PATH, &library)]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_sysmodule_game_file_failures_never_complete_or_resume_the_caller() {
    let mut p = Program::new();
    p.reg(7, 0x115);
    p.call(0);
    p.expect(0);
    let (main, _) = game_consumer(p);
    for library in [
        module_input(&[0x0f, 0x0b], false, Some(0x100)),
        module_input(&[0xb8, 0xff, 0xff, 0xff, 0xff, 0xc3], false, Some(0x100)),
        retain_preload(false),
    ] {
        let (status, fields, text) = probe_dynamic(&main, &[(GAME_PATH, &library)]);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["initializer_calls"], "0");
        assert!(!fields.contains_key("value"));
    }
    let invalid = create_input(b"original unread invalid module".to_vec());
    let mut p = Program::new();
    p.reg(7, 0x114); // Adjacent unlisted ID cannot select the supplied capability.
    p.call(0);
    p.expect(0);
    let (main, _) = game_consumer(p);
    let (status, fields, text) = probe_dynamic(&main, &[(GAME_PATH, &invalid)]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["initializer_calls"], "0");
}

#[test]
fn native_sysmodule_game_file_reentrant_self_load_keeps_the_outer_initializer_pending() {
    let mut init = Program::new();
    init.reg(7, 0x115);
    init.call(2);
    init.expect(0x805a_1001); // Outer load has not published its reference yet.
    init.reg(7, 0x115);
    init.call(0); // Already mapped; do not recursively start the same initializer.
    init.expect(0);
    let (initializer, inner_calls) = game_consumer(init);
    let mut bytes = initializer.bytes.clone();
    put(&mut bytes, 152, &(12_u64 * 16).to_le_bytes());
    put(&mut bytes, 160, &(12_u64 * 16).to_le_bytes());
    put(&mut bytes, 0x2a0, &12_i64.to_le_bytes());
    put(&mut bytes, 0x2a8, &0x800_u64.to_le_bytes());
    let library = create_input(bytes);
    let mut p = Program::new();
    for (slot, result) in [(0, 0), (1, 0), (2, 0), (1, 0), (2, 0x805a_1001)] {
        p.reg(7, 0x115);
        p.call(slot);
        p.expect(result);
    }
    let (main, outer_calls) = game_consumer(p);
    let (status, fields, text) = probe_dynamic(&main, &[(GAME_PATH, &library)]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["initializer_calls"], "1");
    assert_eq!(
        fields["service_stops"],
        (inner_calls + outer_calls).to_string()
    );
}

#[test]
fn native_sysmodule_references_keep_exact_scopes_and_int32_results() {
    const UNLOADED: u64 = 0x805a_1001;
    let mut p = Program::new();
    for (slot, result) in [(2, UNLOADED), (1, UNLOADED), (0, 0)] {
        p.reg(7, 37);
        p.call(slot);
        p.expect(result);
    }
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(16);
    p.reg(7, 0xabcd_0025); // Same uint16 module identity.
    p.reg(6, 1 << 32);
    p.reg(2, 2 << 32);
    p.reg(1, 3 << 32);
    p.ptr(8, 16);
    p.call(3);
    p.expect(0);
    p.word(16, 0xa5a5_a5a5_0000_0000);
    for (slot, result) in [(1, 0), (2, 0), (1, 0), (2, UNLOADED), (1, UNLOADED)] {
        p.reg(7, 37);
        p.call(slot);
        p.expect(result);
    }
    let (code, calls) = p.finish(None);
    let nids = ["g8cM39EUZ6o", "eR2bZFAAU0Q", "fMP5NHUOaMk", "hHrGoGoNf+s"];
    for (module, library, version, expected) in [
        ("Sysmodule", "Sysmodule", 1, "exit"),
        ("libSceSysmodule", "Sysmodule", 1, "exit"),
        ("Sysmodule", "libSceSysmodule", 1, "exit"),
        ("libSceSysmodule", "libSceSysmodule", 1, "exit"),
        ("libkernel", "libkernel", 1, "unavailable-function"),
        (
            "libSceSysmodule",
            "libSceSysmodule",
            2,
            "import-version-mismatch",
        ),
    ] {
        let source = scoped_system_image(&code, &nids, module, library, version);
        let (status, fields, text) = probe(&source);
        if expected == "exit" {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], expected, "{text}");
        }
    }
}
