// SPDX-License-Identifier: GPL-2.0-only

use super::*;

struct SaveRoot(PathBuf);
impl SaveRoot {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax native save {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for SaveRoot {
    fn drop(&mut self) {
        for item in std::fs::read_dir(&self.0).unwrap() {
            let path = item.unwrap().path();
            assert_eq!(path.parent(), Some(self.0.as_path()));
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn native_save_memory_persists_across_worker_processes_and_refuses_data_root_overlap() {
    let root = SaveRoot::new();
    let nids = [
        "oQySEUfgXRA",
        "QwOO7vegnV8",
        "cduy9v4YmT4",
        "wiT9jeC7xPw",
        "j8xKtiFj0SY",
        "yKDy8S5yLA0",
    ];
    for restart in [false, true] {
        let mut p = Program::new();
        p.reg(0, 1_u64 << 32); // Setup option0, user1.
        p.save(0);
        p.set(8, 64);
        for offset in [16, 24, 32, 40] {
            p.set(offset, 0);
        }
        p.ptr(7, 0);
        p.ptr(6, 48);
        p.call(0);
        p.expect(0);
        p.word(48, if restart { 64 } else { 0 });
        p.word(56, 0);
        p.word(64, 0);
        // Shared Get/Set prefix and one 64-byte-stride descriptor.
        p.set(0, 1);
        p.ptr(0, 72);
        p.save(8);
        p.ptr(0, 104);
        p.save(72);
        p.set(80, 8);
        p.set(88, 0);
        p.ptr(7, 0);
        p.call(1);
        p.expect(0);
        let payload = u64::from_le_bytes(*b"original");
        p.word(104, if restart { payload } else { 0 });
        p.reg(0, payload);
        p.save(104);
        p.ptr(7, 0);
        p.call(2);
        p.expect(0);
        // Real sync queues a successful event only after the bundle is stored.
        p.set(8, 0);
        p.ptr(7, 0);
        p.call(3);
        p.expect(0);
        p.reg(7, 0);
        p.ptr(6, 0);
        p.call(4);
        p.expect(0);
        p.word(0, 3);
        p.word(8, 1);
        p.word(16, u64::from_le_bytes(*b"UNKNOWN\0"));
        p.call(5);
        p.expect(0);
        let (code, calls) = p.finish(None);
        let input = scoped_system_image(
            &code,
            &nids,
            "libSceSaveData_native",
            "libSceSaveData_native",
            1,
        );
        let arguments = vec![
            "--image".into(),
            input.path.clone().into(),
            "--save-directory".into(),
            root.0.clone().into(),
        ];
        let (status, fields, text) = probe_arguments(&arguments);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], calls.to_string());
        assert!(root.0.join("memory-u1-s0.pxsave").is_file());
        if restart {
            let mut overlap = arguments;
            overlap.push("--data-directory".into());
            let mut binding = OsString::from("/app0=");
            binding.push(&root.0);
            overlap.push(binding);
            let (status, fields, text) = probe_arguments(&overlap);
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], "save-data-directory-overlap");
        }
    }
}

#[test]
fn native_save_initialization_transaction_events_and_termination_keep_exact_normal_and_native_scopes()
 {
    let mut p = Program::new();
    for reg in [7, 6, 2, 1, 8, 9] {
        p.reg(reg, u64::MAX);
    }
    p.call(0);
    p.expect(0);
    p.call(2);
    p.expect(1);
    p.call(3);
    p.expect(0);
    p.call(1);
    p.expect(0);
    p.call(2);
    p.expect(2);
    p.set(0, 0);
    p.set(8, 0);
    p.ptr(7, 0);
    p.ptr(6, 8);
    p.call(4);
    p.expect(0);
    p.reg(0, 0x0000_0001_ffff_fff9);
    p.save(0);
    p.ptr(7, 0);
    p.call(5);
    p.expect(0);
    p.reg(7, u64::MAX);
    p.call(0);
    p.expect(0); // Does not clear queued events.
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    for offset in (16..=120).step_by(8) {
        p.save(offset);
    }
    p.reg(7, u64::MAX);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0);
    p.word(16, 4);
    p.word(24, 0);
    p.word(112, 0);
    p.word(120, 0xa5a5_a5a5_a5a5_a5a5);
    p.reg(7, 0);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0x809f_0008);
    p.word(16, 4);
    p.reg(6, 0);
    p.call(6);
    p.expect(0x809f_0000);
    p.ptr(7, 0);
    p.call(5);
    p.expect(0);
    p.call(1);
    p.expect(0);
    p.reg(7, 0);
    p.ptr(6, 16);
    p.call(6);
    p.expect(0x809f_0008);
    p.call(2);
    p.expect(3);
    let (code, calls) = p.finish(None);
    let nids = [
        "TywrFKCoLGY",
        "yKDy8S5yLA0",
        "gjRZNnw0JPE",
        "lJUQuaKqoKY",
        "sDCBrmc61XU",
        "ie7qhZ4X0Cc",
        "j8xKtiFj0SY",
    ];
    for (module, library, version, expected) in [
        ("SaveData", "SaveData", 1, "exit"),
        ("libSceSaveData", "SaveData", 1, "exit"),
        ("SaveData", "libSceSaveData", 1, "exit"),
        ("libSceSaveData", "libSceSaveData", 1, "exit"),
        ("SaveData_native", "SaveData_native", 1, "exit"),
        ("libSceSaveData_native", "SaveData_native", 1, "exit"),
        ("SaveData_native", "libSceSaveData_native", 1, "exit"),
        ("libSceSaveData_native", "libSceSaveData_native", 1, "exit"),
        (
            "libSceSaveData_native",
            "libSceSaveData_native",
            2,
            "import-version-mismatch",
        ),
        ("SaveData_native", "SaveData", 1, "unavailable-function"),
        ("SaveData", "SaveData_native", 1, "unavailable-function"),
    ] {
        let input = scoped_system_image(&code, &nids, module, library, version);
        let (status, fields, text) = probe(&input);
        if expected == "exit" {
            assert_eq!(status, 0, "{text}");
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["service_stops"], calls.to_string());
        } else {
            assert_eq!(status, 2, "{text}");
            assert_eq!(fields["code"], expected);
        }
    }
}
