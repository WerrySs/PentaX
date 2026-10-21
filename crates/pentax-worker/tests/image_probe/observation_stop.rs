// SPDX-License-Identifier: GPL-2.0-only
//! Original guest loops stop cooperatively and retain private terminal output.
use super::private_thread_states::{rows, run_arguments};
use super::*;

const WORD: u64 = 0xff80_1b0a_254e_0029;
const SENTINEL: u64 = 0x1122_3344_5566_7788;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..16 {
            let path = std::env::temp_dir().join(format!(
                "pentax-observation-stop-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => panic!("cannot create original observation directory"),
            }
        }
        panic!("original observation directory collisions exceeded bound")
    }
    fn path(&self, name: &str, kind: &str) -> PathBuf {
        self.0.join(format!("{name}-{kind}.bin"))
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        for name in ["loop", "return", "fault", "invalid"] {
            for kind in ["samples", "states", "console"] {
                let path = self.path(name, kind);
                if path.exists() {
                    std::fs::remove_file(path).unwrap();
                }
            }
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

fn original(kind: &str) -> Input {
    let mut program = Program::new();
    program.reg(0, WORD);
    program.save(16);
    program.reg(7, 1);
    program.ptr(6, 16);
    program.reg(2, 8);
    program.call(0);
    program.expect(8);
    program.reg(12, SENTINEL);
    program.reg(13, 0);
    if kind == "return" {
        let (code, calls) = program.finish(None);
        assert_eq!(calls, 1);
        return image(&code, &["FxVZqBAA7ks"]);
    }
    if kind == "loop" {
        // Real INC/JMP loop, with no in-body service or host-side shortcut.
        let begin = program.code.len();
        program.code.extend([0x49, 0xff, 0xc5, 0xe9]);
        program
            .code
            .extend((begin as i32 - (program.code.len() as i32 + 4)).to_le_bytes());
    } else {
        assert_eq!(kind, "fault");
        program.code.extend([0x0f, 0x0b]);
    }
    finish_checks(&mut program.code, program.branches);
    assert!(program.code.len() < 0x500);
    image(&program.code, &["FxVZqBAA7ks"])
}

fn arguments(source: &Input) -> Vec<OsString> {
    vec![
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "interactive".into(),
        "--stop-after-seconds".into(),
        "1".into(),
    ]
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn bounded(path: &std::path::Path, maximum: u64) -> Vec<u8> {
    assert!(std::fs::metadata(path).unwrap().len() <= maximum);
    std::fs::read(path).unwrap()
}

#[test]
fn graceful_observation_cli_stop_retains_private_artifacts_and_early_outcome_precedence() {
    let files = Files::new();
    for kind in ["loop", "return", "fault"] {
        let source = original(kind);
        let mut args = arguments(&source);
        for (option, suffix) in [
            ("--private-samples", "samples"),
            ("--private-thread-states", "states"),
            ("--private-console", "console"),
        ] {
            args.extend([option.into(), files.path(kind, suffix).into_os_string()]);
        }
        let (status, fields, text) = run_arguments(&args, true);
        assert_eq!(status, if kind == "return" { 0 } else { 1 }, "{text}");
        assert_eq!(
            fields["code"],
            match kind {
                "loop" => "observation-duration",
                "return" => "outer-return",
                _ => "unsupported-instruction",
            }
        );
        assert_eq!(fields["service_stops"], "1");
        assert_eq!(fields["output_bytes"], "8");
        assert_eq!(fields["private_console_status"], "written");
        assert_eq!(fields["private_samples_status"], "written");
        assert!(fields["native_entries"].parse::<u64>().unwrap() > 0);
        if kind == "return" {
            assert_eq!(fields["value"], "42");
            assert_eq!(fields["private_thread_states_status"], "unavailable");
            assert_eq!(
                std::fs::metadata(files.path(kind, "states")).unwrap().len(),
                0
            );
        } else {
            assert!(!fields.contains_key("value"));
            assert!(!fields.contains_key("budget"));
            assert_eq!(fields["private_thread_states_status"], "written");
            let states = rows(&files.path(kind, "states"));
            assert_eq!(states.len(), 1);
            assert_eq!(&states[0][..8], &[0; 8]); // Primary module, main role.
            assert!((0x800..0xd00).contains(&u64_at(&states[0], 8)));
            assert_eq!(u64_at(&states[0], 16 + 12 * 8), SENTINEL);
            if kind == "loop" {
                assert!(u64_at(&states[0], 16 + 13 * 8) > 0);
            } else {
                assert_eq!(fields["instruction"], "Ud2");
            }
        }
        let mut console = b"PXCNSL01".to_vec();
        console.extend(8_u32.to_le_bytes());
        console.extend([0; 8]);
        console.extend(WORD.to_le_bytes());
        assert_eq!(bounded(&files.path(kind, "console"), 12_308), console);
        let samples = bounded(&files.path(kind, "samples"), 1552);
        assert_eq!(&samples[..8], b"PXSAMP01");
        let count = u32::from_le_bytes(samples[8..12].try_into().unwrap()) as usize;
        assert!(count <= 64);
        assert_eq!(&samples[12..16], &[0; 4]);
        assert_eq!(samples.len(), 16 + count * 24);
        let mut recent = 0;
        for row in samples[16..].chunks_exact(24) {
            assert_eq!(&row[..8], &[0; 8]);
            assert!((0x800..0xd00).contains(&u64_at(row, 8)));
            recent += u64_at(row, 16);
        }
        assert!((1..=64).contains(&recent));
        for forbidden in [
            "PXCTX001",
            "PXSAMP01",
            "PXCNSL01",
            "pentax-observation-stop-",
            "-states.bin",
            "-console.bin",
            "-samples.bin",
        ] {
            assert!(!text.contains(forbidden));
        }
        assert!(
            text.lines()
                .filter(|line| line.contains("\tPROGRESS\t"))
                .all(|line| !line.contains("private_") && !line.contains("observation-duration"))
        );
    }
    // Check the actual worker status too: CLI's existing stopped route always
    // exits1, whereas only this intentional worker stop changes2 to1.
    let source = original("loop");
    let (status, fields, text) = run_arguments(&arguments(&source), false);
    assert_eq!(status, 1, "{text}");
    assert_eq!(fields["code"], "observation-duration");
    assert!(!fields.contains_key("value"));
    let mut invalid = arguments(&source);
    invalid[3] = "probe".into();
    invalid.extend([
        "--private-thread-states".into(),
        files.path("invalid", "states").into_os_string(),
    ]);
    let (status, fields, text) = run_arguments(&invalid, false);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "image-arguments");
    assert_eq!(fields["native_instructions"], "0");
    assert!(!text.contains("\tRUNNING\t"));
    assert!(!files.path("invalid", "states").exists());
    println!(
        "graceful_observation_cli_available=true graceful_observation_cli_programs=5 graceful_observation_private_written=8"
    );
}
