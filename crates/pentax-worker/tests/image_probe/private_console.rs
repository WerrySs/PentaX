// SPDX-License-Identifier: GPL-2.0-only
//! Original actual guest writes remain private binary output after stopping.

use super::*;

const WORDS: [u64; 3] = [
    0x7f1b_0a80_5850_ff00,
    0xff0d_0180_5245_00fe,
    0x0a80_10ff_4e00_497f,
];

struct ConsoleFiles(PathBuf);
impl ConsoleFiles {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..16 {
            let path = std::env::temp_dir().join(format!(
                "pentax-private-console-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => panic!("cannot create original console directory"),
            }
        }
        panic!("original console directory collisions exceeded bound")
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for ConsoleFiles {
    fn drop(&mut self) {
        for name in ["returned.bin", "stopped.bin", "unavailable.bin"] {
            let path = self.path(name);
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

fn original(stopped: bool) -> Input {
    let mut p = Program::new();
    for (index, word) in WORDS.into_iter().enumerate() {
        p.reg(0, word);
        p.save(16 + index as u8 * 8);
    }
    // The pinned POSIX write path accepts all three standard descriptors,
    // including descriptor zero's retained compatibility output.
    for (index, descriptor) in [1, 2, 0].into_iter().enumerate() {
        p.reg(7, descriptor);
        p.ptr(6, 16 + index as u8 * 8);
        p.reg(2, 8);
        p.call(0);
        p.expect(8);
    }
    p.reg(0, 42);
    p.code.extend([0x48, 0x81, 0xc4, 0, 1, 0, 0, 0x5d]);
    p.code.extend(if stopped {
        &[0x0f, 0x0b][..]
    } else {
        &[0xc3][..]
    });
    finish_checks(&mut p.code, p.branches);
    assert!(p.code.len() < 0x500);
    image(&p.code, &["FxVZqBAA7ks"])
}

fn run(source: &Input, output: Option<&std::path::Path>) -> (i32, HashMap<String, String>, String) {
    let mut arguments = vec![
        OsString::from("--image"),
        source.path.as_os_str().to_owned(),
    ];
    if let Some(path) = output {
        arguments.extend([
            OsString::from("--private-console"),
            path.as_os_str().to_owned(),
        ]);
    }
    probe_arguments(&arguments)
}

fn expected() -> Vec<u8> {
    let mut bytes = b"PXCNSL01".to_vec();
    for _ in 0..3 {
        bytes.extend(8_u32.to_le_bytes());
    }
    for word in WORDS {
        bytes.extend(word.to_le_bytes());
    }
    bytes
}

fn read_bounded(path: &std::path::Path) -> Vec<u8> {
    assert!(std::fs::metadata(path).unwrap().len() <= 12_308);
    std::fs::read(path).unwrap()
}

fn without_durations(mut fields: HashMap<String, String>) -> HashMap<String, String> {
    for name in [
        "sample_window_ns",
        "sample_plan_ns",
        "sample_dispatch_ns",
        "service_ns",
    ] {
        assert!(fields.remove(name).unwrap().parse::<u64>().is_ok());
    }
    fields
}

#[test]
fn private_console_exports_actual_binary_streams_without_changing_guest_outcomes() {
    let files = ConsoleFiles::new();
    let returned = original(false);
    let (off_status, off_fields, off_text) = run(&returned, None);
    assert_eq!(off_status, 0, "{off_text}");
    assert_eq!(off_fields["value"], "42");
    assert_eq!(off_fields["service_stops"], "3");
    assert_eq!(off_fields["output_bytes"], "24");
    assert!(!off_fields.contains_key("private_console_status"));
    assert!(std::fs::read_dir(&files.0).unwrap().next().is_none());

    let path = files.path("returned.bin");
    let (on_status, mut on_fields, on_text) = run(&returned, Some(&path));
    assert_eq!(on_status, off_status, "{on_text}");
    assert_eq!(
        on_fields.remove("private_console_status").unwrap(),
        "written"
    );
    assert_eq!(read_bounded(&path), expected());
    assert_eq!(without_durations(on_fields), without_durations(off_fields));
    // The helper enforces valid UTF-8 and empty host stderr. Raw 0xff/0x80
    // payload bytes escaping to either public stream would fail before here.
    for text in [&off_text, &on_text] {
        assert!(!text.contains("PXCNSL01"));
        assert!(!text.contains("pentax-private-console"));
        assert!(!text.contains("returned.bin"));
    }

    let stopped = original(true);
    let stopped_path = files.path("stopped.bin");
    let (status, fields, text) = run(&stopped, Some(&stopped_path));
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "unsupported-instruction");
    assert_eq!(fields["instruction"], "Ud2");
    assert_eq!(fields["private_console_status"], "written");
    assert_eq!(fields["service_stops"], "3");
    assert_eq!(fields["output_bytes"], "24");
    assert!(!fields.contains_key("value"));
    assert_eq!(read_bounded(&stopped_path), expected());

    let (status, fields, text) = run(&returned, Some(&path));
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "private-console-open");
    assert_eq!(fields["block_attempts"], "0");
    assert_eq!(fields["native_instructions"], "0");
    assert!(!fields.contains_key("private_console_status"));
    assert!(
        !text
            .lines()
            .any(|line| line.starts_with("PENTAX_WORKER\t1\tRUNNING\t"))
    );
    assert!(!text.contains("returned.bin"));
    assert_eq!(read_bounded(&path), expected());

    // Immutable intake finishes before opening the private file; malformed
    // executable metadata then fails preparation without a session capture.
    let mut malformed = returned.bytes.clone();
    malformed[0] = 0;
    let malformed = create_input(malformed);
    let unavailable = files.path("unavailable.bin");
    let (status, fields, text) = run(&malformed, Some(&unavailable));
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["private_console_status"], "unavailable");
    assert_eq!(fields["native_instructions"], "0");
    assert!(!fields.contains_key("value"));
    assert!(read_bounded(&unavailable).is_empty());
}
