// SPDX-License-Identifier: GPL-2.0-only
//! Original native consumers of real mounted files and durable transfer data.
use super::*;

const CANARY: u64 = 0x9d35_0cab_7e24_816f;

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax-original-save-directories-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        assert!(
            self.0
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pentax-original-save-directories-")
        );
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn stack(p: &mut Program, opcode: u8, register: u8, offset: u32) {
    if offset <= 127 {
        p.stack(opcode, register, offset as u8);
    } else {
        p.code.extend([
            0x48 | if register >= 8 { 4 } else { 0 },
            opcode,
            0x84 | ((register & 7) << 3),
            0x24,
        ]);
        p.code.extend(offset.to_le_bytes());
    }
}
fn pointer(p: &mut Program, register: u8, offset: u32) {
    stack(p, 0x8d, register, offset);
}
fn store(p: &mut Program, offset: u32, value: u64) {
    p.reg(0, value);
    stack(p, 0x89, 0, offset);
}
fn word(p: &mut Program, offset: u32, value: u64) {
    stack(p, 0x8b, 0, offset);
    p.expect(value);
}

fn prepare() -> Program {
    let mut p = Program::new();
    stack(&mut p, 0x89, 4, 240);
    p.reg(12, CANARY);
    for offset in (0..112).step_by(8) {
        store(&mut p, offset, 0);
    }
    for offset in (120..=192).step_by(8) {
        store(&mut p, offset, CANARY);
    }
    p
}
fn preserve(p: &mut Program) {
    p.code.extend([0x48, 0x89, 0xe0]);
    stack(p, 0x8b, 10, 240);
    p.code.extend([0x4c, 0x39, 0xd0]);
    checked(&mut p.code, &mut p.branches);
    p.code.extend([0x4c, 0x89, 0xe0]);
    p.expect(CANARY);
}

// Scope0 is canonical SaveData, scope1 is libkernel, scope2 is the explicit
// full-prefix native SaveData pair. No generic cross-namespace linking exists.
fn original(code: &[u8], imports: &[(&str, u8)]) -> Input {
    let nids: Vec<_> = imports.iter().map(|(nid, _)| *nid).collect();
    let template = scoped_system_image(code, &nids, "SaveData", "SaveData", 1);
    let mut bytes = template.bytes.clone();
    let mut names = b"\0SaveData\0SaveData\0".to_vec();
    let kernel = names.len() as u64;
    names.extend_from_slice(b"libkernel\0");
    let native = names.len() as u64;
    names.extend_from_slice(b"libSceSaveData_native\0");
    for (index, (nid, scope)) in imports.iter().enumerate() {
        put(
            &mut bytes,
            0x600 + (index + 1) * 24,
            &(names.len() as u32).to_le_bytes(),
        );
        let qualifier = match scope {
            0 => "D#C",
            1 => "F#E",
            2 => "H#G",
            _ => panic!("original scope"),
        };
        names.extend_from_slice(format!("{nid}#{qualifier}\0").as_bytes());
    }
    assert!(names.len() <= 0x200);
    bytes[0x400..0x600].fill(0);
    put(&mut bytes, 0x400, &names);
    put(&mut bytes, 0x218, &(names.len() as u64).to_le_bytes());
    for (at, tag, value) in [
        (
            0x2a0,
            0x6100_000f_i64,
            kernel | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (0x2b0, 0x6100_0015, kernel | (1 << 32) | (5 << 48)),
        (
            0x2c0,
            0x6100_000f,
            native | (1 << 32) | (1 << 40) | (6 << 48),
        ),
        (0x2d0, 0x6100_0015, native | (1 << 32) | (7 << 48)),
        (0x2e0, 0, 0),
    ] {
        put(&mut bytes, at, &tag.to_le_bytes());
        put(&mut bytes, at + 8, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(&mut bytes, at, &(15_u64 * 16).to_le_bytes());
    }
    put(&mut bytes, NAME as usize, b"/savedata0/roundtrip.bin\0");
    create_input(bytes)
}

fn run(root: &Root, p: Program, imports: &[(&str, u8)]) {
    let (code, calls) = p.finish(None);
    let input = original(&code, imports);
    let arguments: Vec<OsString> = vec![
        "--image".into(),
        input.path.clone().into(),
        "--save-directory".into(),
        root.0.clone().into(),
        "--save-title-id".into(),
        "PPSA12345".into(),
    ];
    let (status, fields, text) = probe_arguments(&arguments);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_eq!(fields["unavailable_functions"], "0");
}

#[test]
fn actual_save_directory_file_write_survives_unmount_and_worker_restart_then_transfer_mount_reads_it()
 {
    let root = Root::new();
    let mut p = prepare();
    p.reg(7, u64::MAX);
    p.call(0);
    p.expect(0);
    p.set(0, 1);
    p.ptr(0, 80);
    p.save(8);
    p.set(16, 48);
    p.set(24, 0);
    p.set(32, 4);
    p.set(40, 0);
    store(&mut p, 80, u64::from_le_bytes(*b"original"));
    store(&mut p, 88, u64::from_le_bytes(*b".slot\0\0\0"));
    p.ptr(7, 0);
    pointer(&mut p, 6, 128);
    p.call(1);
    p.expect(0);
    word(&mut p, 120, CANARY);
    word(&mut p, 192, CANARY);
    word(&mut p, 128, u64::from_le_bytes(*b"/savedat"));
    word(&mut p, 136, u64::from_le_bytes(*b"a0\0\0\0\0\0\0"));
    word(&mut p, 152, 1_u64 << 32);
    p.path(7, NAME);
    p.reg(6, 0x602);
    p.reg(2, 0o600);
    p.call(4);
    p.expect(3);
    p.save(112);
    store(&mut p, 80, u64::from_le_bytes(*b"original"));
    p.load(7, 112);
    p.ptr(6, 80);
    p.reg(2, 8);
    p.call(5);
    p.expect(8);
    p.load(7, 112);
    p.call(6);
    p.expect(0);
    p.reg(7, 0);
    pointer(&mut p, 6, 128);
    p.call(2);
    p.expect(0);
    p.call(3);
    p.expect(0);
    preserve(&mut p);
    run(
        &root,
        p,
        &[
            ("TywrFKCoLGY", 0),
            ("ZP4e7rlzOUk", 0),
            ("uW4vfTwMQVo", 0),
            ("yKDy8S5yLA0", 0),
            ("1G3lF1Gg1k8", 1),
            ("4wSze92BhLI", 1),
            ("UK2Tl2DWUns", 1),
        ],
    );

    let mut p = prepare();
    p.set(0, 1);
    p.ptr(0, 80);
    p.save(8);
    p.ptr(0, 96);
    p.save(16);
    store(&mut p, 24, u64::MAX); // The source ignores the opaque fingerprint.
    store(&mut p, 80, u64::from_le_bytes(*b"PPSA1234"));
    p.set(88, b'5');
    store(&mut p, 96, u64::from_le_bytes(*b"original"));
    store(&mut p, 104, u64::from_le_bytes(*b".slot\0\0\0"));
    p.ptr(7, 0);
    pointer(&mut p, 6, 128);
    p.call(0);
    p.expect(0);
    word(&mut p, 120, CANARY);
    word(&mut p, 192, CANARY);
    word(&mut p, 152, 0);
    p.path(7, NAME);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(3);
    p.expect(3);
    p.save(112);
    p.load(7, 112);
    p.ptr(6, 80);
    p.reg(2, 8);
    p.call(4);
    p.expect(8);
    p.word(80, u64::from_le_bytes(*b"original"));
    p.load(7, 112);
    p.call(5);
    p.expect(0);
    store(&mut p, 48, CANARY);
    pointer(&mut p, 7, 128);
    p.ptr(6, 0);
    p.call(2);
    p.expect(0);
    p.word(0, 48);
    p.word(8, 48);
    p.word(48, CANARY);
    p.reg(7, 0);
    pointer(&mut p, 6, 128);
    p.call(1);
    p.expect(0);
    preserve(&mut p);
    run(
        &root,
        p,
        &[
            ("WAzWTZm1H+I", 2),
            ("uW4vfTwMQVo", 0),
            ("65VH0Qaaz6s", 0),
            ("1G3lF1Gg1k8", 1),
            ("Cg4srZ6TKbU", 1),
            ("UK2Tl2DWUns", 1),
        ],
    );
    println!(
        "save_directory_native_available=true save_directory_native_programs=2 save_directory_native_restart_reads=1"
    );
}

#[test]
fn actual_transfer_mount_aliases_preserve_parameter_user_and_absent_output_order() {
    let root = Root::new();
    for import in [("WAzWTZm1H+I", 2), ("RjMlsR8EXrw", 0)] {
        let mut p = prepare();
        pointer(&mut p, 6, 128);
        p.reg(7, 0);
        p.call(0);
        p.expect(0x809f_0000);
        word(&mut p, 128, CANARY);
        p.set(0, 1);
        p.ptr(0, 80);
        p.save(8);
        p.ptr(0, 96);
        p.save(16);
        store(&mut p, 80, u64::from_le_bytes(*b"PPSA1234"));
        p.set(88, b'5');
        store(&mut p, 96, u64::from_le_bytes(*b"missing\0"));
        store(&mut p, 0, u64::from(u32::MAX));
        p.ptr(7, 0);
        pointer(&mut p, 6, 128);
        p.call(0);
        p.expect(0x809f_0011);
        word(&mut p, 128, CANARY);
        p.set(0, 1);
        p.ptr(7, 0);
        pointer(&mut p, 6, 128);
        p.call(0);
        p.expect(0x809f_0008);
        for offset in (128..192).step_by(8) {
            word(&mut p, offset, 0);
        }
        word(&mut p, 120, CANARY);
        word(&mut p, 192, CANARY);
        preserve(&mut p);
        run(&root, p, &[import]);
    }
    println!(
        "save_transfer_errors_native_available=true save_transfer_errors_native_programs=2 save_transfer_errors_native_cases=6"
    );
}
