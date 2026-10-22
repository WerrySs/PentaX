// SPDX-License-Identifier: GPL-2.0-only
//! Original actual-worker terminal snapshots. Private register data is read
//! only from explicit fresh test files, never from public protocol fields.

use super::*;

const MAIN_VALUE: u64 = 0x1122_3344_5566_7788;
const CHILD_VALUE: u64 = 0x8877_6655_4433_2211;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..16 {
            let path = std::env::temp_dir().join(format!(
                "pentax-private-thread-states-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => panic!("cannot create original context directory"),
            }
        }
        panic!("original context directory collisions exceeded bound")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        for name in [
            "baseline.bin",
            "samples.bin",
            "states.bin",
            "child.bin",
            "returned.bin",
        ] {
            let path = self.path(name);
            if path.exists() {
                std::fs::remove_file(path).unwrap();
            }
        }
        std::fs::remove_dir(&self.0).unwrap();
    }
}

fn run(
    source: &Input,
    contexts: Option<&std::path::Path>,
    samples: Option<&std::path::Path>,
) -> (i32, HashMap<String, String>, String) {
    let mut args = vec![
        "--image".into(),
        source.path.as_os_str().to_owned(),
        "--execution-profile".into(),
        "extended-startup".into(),
    ];
    for (option, path) in [
        ("--private-thread-states", contexts),
        ("--private-samples", samples),
    ] {
        if let Some(path) = path {
            args.push(option.into());
            args.push(path.as_os_str().to_owned());
        }
    }
    run_arguments(&args, true)
}

fn run_selected_cli(executable: &std::path::Path, args: &[OsString]) -> (i32, Vec<u8>, Vec<u8>) {
    use pentax_platform::supervised_process::{Limits, State, SupervisedProcess};
    let args: Vec<_> = std::iter::once(OsString::from("probe-entry"))
        .chain(args.iter().cloned())
        .collect();
    let mut process = SupervisedProcess::spawn_selected_cli(
        executable,
        &args,
        Limits {
            run_timeout: Some(Duration::from_secs(30)),
            max_stdout_bytes: Some(32 * 1024),
            max_stderr_bytes: Some(32 * 1024),
            ..Limits::default()
        },
    )
    .expect("original private-state fixture requires the selected CLI");
    let mut stdout = Vec::with_capacity(32 * 1024);
    let mut stderr = Vec::with_capacity(32 * 1024);
    let mut out = [0; 4096];
    let mut err = [0; 4096];
    loop {
        let status = match process.poll(&mut out, &mut err) {
            Ok(status) => status,
            Err(error) => {
                // Cancellation owns the exact suspended-before-admission job,
                // including the CLI's interactive worker. Never join readers
                // or kill descendants discovered through an ambient census.
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    if process
                        .shutdown_poll(&mut out, &mut err)
                        .is_ok_and(|status| {
                            matches!(status.state, State::Exited(_) | State::Closed)
                        })
                        || Instant::now() >= deadline
                    {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                // Any unresolved owner retains the existing poisoned job;
                // dropping it neither releases a live tree nor waits forever.
                panic!("original private-state CLI supervision failed: {error}");
            }
        };
        assert!(stdout.len() + status.stdout_len <= 32 * 1024);
        assert!(stderr.len() + status.stderr_len <= 32 * 1024);
        stdout.extend_from_slice(&out[..status.stdout_len]);
        stderr.extend_from_slice(&err[..status.stderr_len]);
        if let State::Exited(code) = status.state {
            // Exited requires actual root exit, empty job and both real EOFs.
            return (i32::try_from(code).unwrap(), stdout, stderr);
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn run_arguments(
    args: &[OsString],
    cli: bool,
) -> (i32, HashMap<String, String>, String) {
    // The platform supervisor owns one process tree per test process.
    static RUN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = RUN.lock().unwrap();
    let interactive = args
        .chunks_exact(2)
        .any(|pair| pair[0] == "--execution-profile" && pair[1] == "interactive");
    let worker = || {
        std::env::var_os("PENTAX_TEST_WORKER")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_pentax-worker").into())
    };
    let executable = if cli {
        std::env::var_os("PENTAX_TEST_CLI").unwrap_or_else(|| {
            let worker = std::env::var_os("PENTAX_TEST_WORKER")
                .unwrap_or_else(|| env!("CARGO_BIN_EXE_pentax-worker").into());
            PathBuf::from(worker)
                .with_file_name("pentax-cli.exe")
                .into_os_string()
        })
    } else {
        worker()
    };
    let (status, stdout, stderr) = if cli {
        run_selected_cli(std::path::Path::new(&executable), args)
    } else {
        let mut child = Command::new(executable)
            .arg("probe-entry")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x0800_0000)
            .spawn()
            .expect("original private-state fixture requires the matching sibling CLI");
        fn drain(mut reader: impl std::io::Read) -> (Vec<u8>, bool) {
            let mut retained = Vec::with_capacity(32 * 1024);
            let mut buffer = [0; 4096];
            let mut overflow = false;
            loop {
                let count = reader.read(&mut buffer).unwrap();
                if count == 0 {
                    return (retained, overflow);
                }
                let keep = count.min(32 * 1024 - retained.len());
                retained.extend_from_slice(&buffer[..keep]);
                overflow |= keep != count;
            }
        }
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let output = std::thread::spawn(move || drain(stdout));
        let errors = std::thread::spawn(move || drain(stderr));
        let deadline = Instant::now() + Duration::from_secs(30);
        while child.try_wait().unwrap().is_none() {
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = output.join();
                let _ = errors.join();
                panic!("original private-state CLI exceeded its finite deadline");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let status = child.wait().unwrap();
        let (stdout, output_overflow) = output.join().unwrap();
        let (stderr, error_overflow) = errors.join().unwrap();
        assert!(!output_overflow && !error_overflow);
        (status.code().unwrap(), stdout, stderr)
    };
    if status == 0 || !cli {
        assert!(stderr.is_empty());
    } else {
        assert_eq!(
            String::from_utf8(stderr).unwrap().trim_end(),
            if interactive {
                "PentaX: entry worker stopped; inspect its result category above"
            } else {
                "PentaX: entry probe stopped; inspect its result category above"
            }
        );
    }
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.lines()
            .all(|line| line.starts_with("PENTAX_WORKER\t1\t"))
    );
    let result: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tRESULT\t"))
        .collect();
    assert_eq!(result.len(), 1);
    let fields = result[0]
        .split('\t')
        .skip(4)
        .map(|field| field.split_once('=').unwrap())
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    (status, fields, text)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
pub(super) fn rows(path: &std::path::Path) -> Vec<Vec<u8>> {
    let length = std::fs::metadata(path).unwrap().len();
    assert!(matches!(length, 176 | 336));
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"PXCTX001");
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0);
    let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    assert!((1..=2).contains(&count));
    assert_eq!(bytes.len(), 16 + 160 * count);
    let rows: Vec<_> = bytes[16..].chunks_exact(160).map(<[u8]>::to_vec).collect();
    for row in &rows {
        assert_eq!(u64_at(row, 152), 0);
    }
    rows
}

fn without_durations(mut fields: HashMap<String, String>) -> HashMap<String, String> {
    for key in [
        "sample_window_ns",
        "sample_plan_ns",
        "sample_dispatch_ns",
        "service_ns",
        "service_time_top",
        "service_other_ns",
        "service_overflow_ns",
        "service_unattributed_ns",
    ] {
        fields.remove(key);
    }
    fields
}
fn private_only(text: &str) {
    for forbidden in [
        "PXCTX001",
        "PXSAMP01",
        "states.bin",
        "child.bin",
        "pentax-private-thread-states-",
    ] {
        assert!(!text.contains(forbidden));
    }
    assert!(
        text.lines()
            .filter(|line| line.contains("\tPROGRESS\t"))
            .all(|line| !line.contains("private_thread_states"))
    );
}

#[test]
fn private_thread_states_preserve_stop_samples_and_fresh_file_capability() {
    let files = Files::new();
    let mut code = vec![0x49, 0xbc]; // MOV R12,main; MOV R13,child; CMP R13,R13; UD2.
    code.extend(MAIN_VALUE.to_le_bytes());
    code.extend([0x49, 0xbd]);
    code.extend(CHILD_VALUE.to_le_bytes());
    code.extend([0x4d, 0x39, 0xed, 0x0f, 0x0b]);
    let mut bytes = module_bytes(&code, false, None);
    put(&mut bytes, 68, &1_u32.to_le_bytes()); // PF_X only, as allowed by execution.
    let source = create_input(bytes);
    let baseline = files.path("baseline.bin");
    let samples = files.path("samples.bin");
    let states = files.path("states.bin");
    let (off_status, off, off_text) = run(&source, None, Some(&baseline));
    assert_eq!(off_status, 1, "{off_text}");
    assert_eq!(off["code"], "unsupported-instruction");
    assert_eq!(off["instruction"], "Ud2");
    assert!(!off.contains_key("private_thread_states_status"));
    assert!(!off.contains_key("private_thread_states_main"));
    assert!(!off.contains_key("private_thread_states_active_child"));
    assert!(!states.exists());
    let (status, mut on, text) = run(&source, Some(&states), Some(&samples));
    assert_eq!(status, off_status, "{text}");
    assert_eq!(
        on.remove("private_thread_states_status").unwrap(),
        "written"
    );
    assert_eq!(on.remove("private_thread_states_main").unwrap(), "captured");
    assert_eq!(
        on.remove("private_thread_states_active_child").unwrap(),
        "unavailable-context"
    );
    assert_eq!(without_durations(on), without_durations(off));
    let retained = std::fs::read(&states).unwrap();
    let captured = rows(&states);
    assert_eq!(captured.len(), 1);
    let row = &captured[0];
    assert_eq!(&row[..8], &[0; 8]); // Actual main in primary module zero.
    assert_eq!(u64_at(row, 8), 0x117);
    assert_eq!(u64_at(row, 16 + 12 * 8), MAIN_VALUE);
    assert_eq!(u64_at(row, 16 + 13 * 8), CHILD_VALUE);
    assert_eq!(u64_at(row, 144), 0x246);
    let before = std::fs::read(&baseline).unwrap();
    assert_eq!(&before[..8], b"PXSAMP01");
    assert_eq!(std::fs::read(samples).unwrap(), before);
    private_only(&text);
    let (status, fields, text) = run(&source, Some(&states), None);
    assert_eq!(status, 1, "{text}");
    assert_eq!(fields["code"], "private-thread-states-open");
    assert_eq!(fields["native_instructions"], "0");
    assert_eq!(fields["block_attempts"], "0");
    assert!(!text.contains("\tRUNNING\t"));
    assert_eq!(std::fs::read(states).unwrap(), retained);
    private_only(&text);

    // A returned PC is the actual outer marker, not initialized image bytes.
    // Omit the artifact rather than inventing a plausible relative PC.
    let returned = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let path = files.path("returned.bin");
    let (status, fields, text) = run(&returned, Some(&path), None);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["private_thread_states_status"], "unavailable");
    assert_eq!(
        fields["private_thread_states_main"],
        "no-initialized-executable"
    );
    assert_eq!(
        fields["private_thread_states_active_child"],
        "unavailable-context"
    );
    assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
    println!(
        "private_thread_states_execute_only_available=true\tprivate_thread_states_execute_only_rows=1"
    );
}

#[test]
fn private_thread_states_capture_parked_main_and_actual_active_child_before_stop() {
    let files = Files::new();
    let mut p = Program::new();
    p.set(0, 0);
    p.reg(12, MAIN_VALUE);
    p.ptr(7, 0); // pthread_create(thread*,null,child,null,null).
    p.reg(6, 0);
    p.path(2, 0xc00);
    p.reg(1, 0);
    p.reg(8, 0);
    p.call(0);
    p.expect(0);
    let (mut code, _) = p.finish(None);
    assert!(code.len() < 0x400);
    code.resize(0x400, 0xcc);
    let mut child = vec![0x49, 0xbc];
    child.extend(CHILD_VALUE.to_le_bytes());
    child.extend([0x49, 0xbd]);
    child.extend(MAIN_VALUE.to_le_bytes());
    child.extend([0x4d, 0x39, 0xed, 0x0f, 0x0b]);
    code.extend(child);
    let source = image(&code, &["6UgtwV+0zb4"]);
    let (off_status, off, off_text) = run(&source, None, None);
    assert_eq!(off_status, 1, "{off_text}");
    assert_eq!(off["phase"], "thread-entry");
    assert_eq!(off["scheduling_threads"], "2");
    assert_eq!(off["service_stops"], "1");
    let path = files.path("child.bin");
    let (status, mut on, text) = run(&source, Some(&path), None);
    assert_eq!(status, off_status, "{text}");
    assert_eq!(
        on.remove("private_thread_states_status").unwrap(),
        "written"
    );
    assert_eq!(on.remove("private_thread_states_main").unwrap(), "captured");
    assert_eq!(
        on.remove("private_thread_states_active_child").unwrap(),
        "captured"
    );
    assert_eq!(without_durations(on), without_durations(off));
    let captured = rows(&path);
    assert_eq!(captured.len(), 2);
    let main = &captured[0];
    let child = &captured[1];
    assert_eq!(&main[4..8], &0_u32.to_le_bytes());
    assert_eq!(&child[4..8], &1_u32.to_le_bytes());
    assert_eq!(&child[..4], &0_u32.to_le_bytes());
    assert_eq!(u64_at(child, 8), 0xc17);
    assert_eq!(u64_at(main, 16 + 12 * 8), MAIN_VALUE);
    assert_eq!(u64_at(child, 16 + 12 * 8), CHILD_VALUE);
    assert_eq!(u64_at(child, 16 + 13 * 8), MAIN_VALUE);
    assert_ne!(u64_at(main, 16 + 4 * 8), u64_at(child, 16 + 4 * 8));
    assert_eq!(u64_at(child, 144), 0x246);
    private_only(&text);
    println!("private_thread_states_cli_available=true\tprivate_thread_states_cli_rows=2");
}
