// SPDX-License-Identifier: GPL-2.0-only
//! Original register-loop observations are exported only by explicit opt-in.
//! The file contains bounded module-relative metadata, never source bytes.

use super::*;

struct SampleFiles(PathBuf);

impl SampleFiles {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..16 {
            let path = std::env::temp_dir().join(format!(
                "pentax-private-samples-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => panic!("cannot create original sample directory"),
            }
        }
        panic!("original sample directory collisions exceeded bound")
    }

    fn path(&self, stopped: bool) -> PathBuf {
        self.0.join(if stopped {
            "stopped.bin"
        } else {
            "returned.bin"
        })
    }
}

impl Drop for SampleFiles {
    fn drop(&mut self) {
        for stopped in [false, true] {
            let path = self.path(stopped);
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

fn original(stopped: bool) -> Input {
    // MOV ECX,12288; DEC ECX; JNZ to DEC; MOV EAX,42; RET or UD2.
    // All 12,288 loop bodies execute a real ordinary register instruction.
    // Samples at completed-block ordinals 0,4093,8186,12279 retain the first
    // entry once and the exact steady-loop entry three times.
    let mut code = vec![0xb9];
    code.extend(12_288_u32.to_le_bytes());
    code.extend([0xff, 0xc9, 0x75, 0xfc, 0xb8, 42, 0, 0, 0]);
    code.extend(if stopped {
        &[0x0f, 0x0b][..]
    } else {
        &[0xc3][..]
    });
    input(&code, false)
}

fn run(source: &Input, output: Option<&std::path::Path>) -> (i32, HashMap<String, String>, String) {
    let mut arguments = vec![
        OsString::from("--image"),
        source.path.as_os_str().to_owned(),
        OsString::from("--execution-profile"),
        OsString::from("extended-startup"),
    ];
    if let Some(output) = output {
        arguments.push("--private-samples".into());
        arguments.push(output.as_os_str().to_owned());
    }
    probe_arguments(&arguments)
}

fn check_samples(path: &std::path::Path, fields: &HashMap<String, String>) -> Vec<u8> {
    assert!((16..=2048).contains(&std::fs::metadata(path).unwrap().len()));
    let bytes = std::fs::read(path).unwrap();
    assert!((16..=2048).contains(&bytes.len()));
    assert_eq!(&bytes[..8], b"PXSAMP01");
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let flags = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
    assert!(count <= 64);
    assert_eq!(bytes.len(), 16 + 24 * count);
    assert_eq!(flags, 0);
    // No program names or absolute PCs are needed to validate these original
    // entries: the ELF's initialized RX segment begins at module offset zero.
    let mut rows = Vec::new();
    for row in bytes[16..].chunks_exact(24) {
        let module = u32::from_le_bytes(row[..4].try_into().unwrap());
        let role = u32::from_le_bytes(row[4..8].try_into().unwrap());
        let offset = u64::from_le_bytes(row[8..16].try_into().unwrap());
        let occurrences = u64::from_le_bytes(row[16..24].try_into().unwrap());
        assert_eq!(module, 0);
        assert_eq!(role, 0);
        assert!(matches!(offset, 0x100 | 0x105));
        assert!(occurrences > 0);
        rows.push((offset, occurrences));
    }
    rows.sort_unstable();
    assert_eq!(rows, [(0x100, 1), (0x105, 3)]);
    assert_eq!(fields["sample_completed"], "4");
    assert_eq!(fields["sample_recent"], "4");
    assert_eq!(fields["sample_distinct"], "2");
    assert_eq!(
        rows.iter().map(|(_, count)| count).sum::<u64>(),
        fields["sample_recent"].parse::<u64>().unwrap()
    );
    bytes
}

#[test]
fn private_samples_opt_in_exports_completed_relative_rows_and_preserves_guest_outcomes() {
    let files = SampleFiles::new();
    let returned = original(false);
    let (off_status, off_fields, off_text) = run(&returned, None);
    assert_eq!(off_status, 0, "{off_text}");
    assert_eq!(off_fields["value"], "42");
    assert_eq!(off_fields["block_attempts"], "12289");
    assert_eq!(off_fields["native_instructions"], "12290");
    assert!(!off_fields.contains_key("private_samples_status"));
    assert!(std::fs::read_dir(&files.0).unwrap().next().is_none());

    let returned_path = files.path(false);
    let (on_status, mut on_fields, on_text) = run(&returned, Some(&returned_path));
    assert_eq!(on_status, off_status, "{on_text}");
    assert_eq!(
        on_fields.remove("private_samples_status").unwrap(),
        "written"
    );
    let retained = check_samples(&returned_path, &on_fields);
    assert!(!on_text.contains("pentax-private-samples"));
    assert!(!on_text.contains("returned.bin"));
    assert_eq!(
        on_text
            .lines()
            .filter(|line| !line.contains("\tRESULT\t"))
            .collect::<Vec<_>>(),
        off_text
            .lines()
            .filter(|line| !line.contains("\tRESULT\t"))
            .collect::<Vec<_>>()
    );
    // The optional metadata encoding is separately charged to the existing
    // observer allowance. This original composition has six modules and ten
    // PT_LOAD segments: primary1, kernel3, graphics1, JSON2, unavailable1,
    // libc2. Fixed staging has a 16-byte header and 64 possible 24-byte rows.
    const STAGING: u64 = 16 + 64 * 24;
    const PRIVATE_WORK: u64 =
        STAGING + 1 + (6 + 1) + (10 * 8 + 16) * 64 + 64 * 64 * 24 + 2 * STAGING;
    let off_work = off_fields["sample_observer_work"].parse::<u64>().unwrap();
    let on_work = on_fields
        .remove("sample_observer_work")
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert_eq!(on_work.checked_sub(off_work), Some(PRIVATE_WORK));
    // Guest execution and every other closed field remain unchanged. Only
    // preexisting real clock durations differ between these process runs.
    let mut off_fields = off_fields;
    off_fields.remove("sample_observer_work").unwrap();
    for fields in [&mut off_fields, &mut on_fields] {
        for key in [
            "sample_window_ns",
            "sample_plan_ns",
            "sample_dispatch_ns",
            "service_ns",
        ] {
            assert!(fields.remove(key).unwrap().parse::<u64>().is_ok());
        }
    }
    assert_eq!(on_fields, off_fields);

    let stopped = original(true);
    let stopped_path = files.path(true);
    let (stop_status, stop_fields, stop_text) = run(&stopped, Some(&stopped_path));
    assert_eq!(stop_status, 2, "{stop_text}");
    assert_eq!(stop_fields["code"], "unsupported-instruction");
    assert_eq!(stop_fields["instruction"], "Ud2");
    assert_eq!(stop_fields["native_instructions"], "12290");
    assert!(!stop_fields.contains_key("value"));
    assert_eq!(stop_fields["private_samples_status"], "written");
    assert_eq!(check_samples(&stopped_path, &stop_fields), retained);
    assert!(!stop_text.contains("stopped.bin"));

    // Reusing an existing output must fail before even RUNNING or a first
    // native instruction, and must preserve the exact already written file.
    let (collision_status, collision_fields, collision_text) = run(&returned, Some(&returned_path));
    assert_eq!(collision_status, 2, "{collision_text}");
    assert_eq!(collision_fields["code"], "private-samples-open");
    assert_eq!(collision_fields["block_attempts"], "0");
    assert_eq!(collision_fields["native_instructions"], "0");
    assert!(
        !collision_text
            .lines()
            .any(|line| line.starts_with("PENTAX_WORKER\t1\tRUNNING\t"))
    );
    assert!(!collision_text.contains("returned.bin"));
    assert_eq!(std::fs::read(returned_path).unwrap(), retained);
}
