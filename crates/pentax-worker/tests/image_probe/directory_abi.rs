// SPDX-License-Identifier: GPL-2.0-only

//! Original native consumers of packed directory bytes and source call widths.

use super::*;

struct Directory {
    path: PathBuf,
    names: Vec<String>,
}
impl Directory {
    fn new(names: &[&str]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax-directory-abi-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        for name in names {
            std::fs::write(path.join(name), 42_u64.to_le_bytes()).unwrap();
        }
        Self {
            path,
            names: names.iter().map(|name| name.to_string()).collect(),
        }
    }
    fn binding(&self) -> OsString {
        let mut value = OsString::from("/app0=");
        value.push(&self.path);
        value
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        for name in &self.names {
            assert_eq!(
                std::fs::read(self.path.join(name)).unwrap(),
                42_u64.to_le_bytes()
            );
            std::fs::remove_file(self.path.join(name)).unwrap();
        }
        std::fs::remove_dir(&self.path).unwrap();
    }
}

impl Program {
    fn directory_frame() -> Self {
        let mut p = Self::new();
        p.code[4..8].copy_from_slice(&4096_u32.to_le_bytes());
        p
    }
    fn directory_memory(&mut self, opcode: u8, reg: u8, offset: i32) {
        self.code.extend([
            0x48 | if reg >= 8 { 4 } else { 0 },
            opcode,
            0x84 | ((reg & 7) << 3),
            0x24,
        ]);
        self.code.extend_from_slice(&offset.to_le_bytes());
    }
    fn directory_ptr(&mut self, reg: u8, offset: i32) {
        self.directory_memory(0x8d, reg, offset);
    }
    fn directory_word(&mut self, offset: i32, expected: u64) {
        self.directory_memory(0x8b, 0, offset);
        self.expect(expected);
    }
    fn directory_dword(&mut self, offset: i32, expected: u64) {
        self.code.extend([0x8b, 0x84, 0x24]);
        self.code.extend_from_slice(&offset.to_le_bytes());
        self.expect(expected);
    }
    fn finish_directory(mut self, data: bool) -> (Vec<u8>, u64) {
        if data {
            self.load(0, 32);
        } else {
            self.reg(0, 42);
        }
        self.code
            .extend([0x48, 0x81, 0xc4, 0, 0x10, 0, 0, 0x5d, 0xc3]);
        finish_checks(&mut self.code, self.branches);
        assert!(
            self.code.len() < 0x500,
            "directory consumer exceeds original code extent"
        );
        (self.code, self.calls)
    }
}

fn directory_image(code: &[u8], nids: &[&str]) -> Input {
    let original = image(code, nids);
    let mut bytes = original.bytes.clone();
    put(&mut bytes, NAME as usize, b"/app0/\0");
    put(&mut bytes, MISSING as usize, b"/app0/abc\0");
    create_input(bytes)
}
fn run(directory: &Directory, main: &Input) -> (i32, HashMap<String, String>, String) {
    probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-directory".into(),
        directory.binding(),
    ])
}

#[test]
fn native_directory_source_records_base_offsets_and_state_continue_to_file_data() {
    // Slots: open, getdirentries, getdents, lseek, fstat, read, close, stat.
    let directory = Directory::new(&["abc"]);
    let mut p = Program::directory_frame();
    p.path(7, NAME);
    p.reg(6, 0x20000);
    p.reg(2, 0);
    p.call(0);
    p.expect(3);
    p.save(8);
    p.set(64, 85);
    p.load(7, 8);
    p.directory_ptr(6, 512);
    p.reg(2, 513);
    p.ptr(1, 64);
    p.call(1);
    p.expect(512);
    p.word(64, 0);
    p.directory_word(512, 0x0104_000c_48fc_91ec);
    p.directory_word(524, 0x0204_000c_6696_98f4);
    p.directory_word(536, 0x0308_01e8_85af_ae58);
    p.directory_word(544, 0x0063_6261);
    p.load(7, 8);
    p.directory_ptr(6, 128);
    p.call(4);
    p.expect(0);
    p.directory_word(136, 0o040777);
    p.directory_word(200, 512);
    p.directory_word(208, 1);
    p.load(7, 8);
    p.reg(6, 1);
    p.reg(2, 512);
    p.call(2);
    p.expect(0); // EOF never dereferences non-null buffer.
    p.load(7, 8);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(3);
    p.expect(0);
    p.load(7, 8);
    p.directory_ptr(6, 512);
    p.reg(2, 1);
    p.call(5);
    p.expect(1);
    p.load(7, 8);
    p.directory_ptr(6, 512);
    p.reg(2, 512);
    p.ptr(1, 64);
    p.call(1);
    p.expect(0x8002_0016);
    p.word(64, 0);
    p.load(7, 8);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(3);
    p.expect(0);
    p.load(7, 8);
    p.directory_ptr(6, 512);
    p.reg(2, 511);
    p.call(2);
    p.expect(0x8002_0016);
    p.load(7, 8);
    p.call(6);
    p.expect(0);
    p.path(7, MISSING);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(0);
    p.expect(3);
    p.save(8);
    p.load(7, 8);
    p.ptr(6, 32);
    p.reg(2, 8);
    p.call(5);
    p.expect(8);
    p.word(32, 42);
    p.load(7, 8);
    p.call(6);
    p.expect(0);
    p.path(7, NAME);
    p.directory_ptr(6, 128);
    p.call(7);
    p.expect(0);
    p.directory_word(200, 512);
    let (code, calls) = p.finish_directory(true);
    let main = directory_image(
        &code,
        &[
            "1G3lF1Gg1k8",
            "taRWhTJFTgE",
            "j2AIqSqJP0w",
            "oib76F-12fk",
            "kBwCPsYX-m4",
            "Cg4srZ6TKbU",
            "UK2Tl2DWUns",
            "eV9wAD2riIA",
        ],
    );
    let (status, fields, text) = run(&directory, &main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_directory_multiblock_reads_round_counts_and_keep_each_real_base_offset() {
    let names = ["a".repeat(255), "b".repeat(255)];
    let directory = Directory::new(&[&names[0], &names[1]]);
    let mut p = Program::directory_frame();
    p.path(7, NAME);
    p.reg(6, 0);
    p.reg(2, 0);
    p.call(0);
    p.expect(3);
    p.save(8);
    for (base, header_offset, header) in [(0, 540, 0xff08_01e8), (512, 516, 0xff08_0200)] {
        p.load(7, 8);
        p.directory_ptr(6, 512);
        p.reg(2, 1023);
        p.ptr(1, 64);
        p.call(1);
        p.expect(512);
        p.word(64, base);
        p.directory_dword(header_offset, header);
    }
    p.load(7, 8);
    p.directory_ptr(6, 512);
    p.reg(2, 512);
    p.ptr(1, 64);
    p.call(1);
    p.expect(0);
    p.word(64, 1024);
    p.load(7, 8);
    p.reg(6, 0);
    p.reg(2, 2);
    p.call(2);
    p.expect(1024);
    p.load(7, 8);
    p.call(3);
    p.expect(0);
    let (code, calls) = p.finish_directory(false);
    let main = directory_image(
        &code,
        &["1G3lF1Gg1k8", "taRWhTJFTgE", "oib76F-12fk", "UK2Tl2DWUns"],
    );
    let (status, fields, text) = run(&directory, &main);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn native_directory_unavailable_pread_and_bad_backing_stop_without_a_register_result() {
    let directory = Directory::new(&[]);
    for pread in [true, false] {
        let mut p = Program::directory_frame();
        p.path(7, NAME);
        p.reg(6, 0x20000);
        p.reg(2, 0);
        p.call(0);
        p.save(8);
        p.load(7, 8);
        p.directory_ptr(6, 512);
        p.reg(2, 512);
        p.reg(1, if pread { 0 } else { 1 });
        p.call(1);
        let (code, _) = p.finish_directory(false);
        let main = directory_image(
            &code,
            &[
                "1G3lF1Gg1k8",
                if pread { "+r3rMFwItV4" } else { "taRWhTJFTgE" },
            ],
        );
        let (status, fields, text) = run(&directory, &main);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "runtime-service");
        assert_eq!(fields["service_stops"], "2");
        assert!(!fields.contains_key("value"));
    }
}
