// SPDX-License-Identifier: GPL-2.0-only
//! Original finite programs exercise the interactive policy without an open-ended test.

use super::*;

fn original(iterations: u32, stopped: bool) -> Input {
    // MOV ECX,count; DEC ECX; JNZ DEC; MOV EAX,42; RET or UD2.
    let mut code = vec![0xb9];
    code.extend(iterations.to_le_bytes());
    code.extend([0xff, 0xc9, 0x75, 0xfc, 0xb8, 42, 0, 0, 0]);
    code.extend(if stopped {
        &[0x0f, 0x0b][..]
    } else {
        &[0xc3][..]
    });
    input(&code, false)
}

fn original_write() -> Input {
    let mut program = Program::new();
    program.reg(0, 0xff80_1b0a_7135_0029);
    program.save(16);
    program.reg(7, 1);
    program.ptr(6, 16);
    program.reg(2, 8);
    program.call(0);
    program.expect(8);
    let (code, calls) = program.finish(None);
    assert_eq!(calls, 1);
    image(&code, &["FxVZqBAA7ks"])
}

fn run(source: &Input, profile: Option<&str>) -> (i32, HashMap<String, String>, String) {
    let mut arguments = vec!["--image".into(), source.path.as_os_str().to_owned()];
    if let Some(profile) = profile {
        arguments.extend(["--execution-profile".into(), profile.into()]);
    }
    probe_arguments(&arguments)
}

fn same_guest(left: &HashMap<String, String>, right: &HashMap<String, String>) {
    for key in [
        "code",
        "block_attempts",
        "native_instructions",
        "native_entries",
        "service_stops",
        "segment_ops",
        "string_elements",
        "string_bytes",
        "vector_ops",
        "vector_bytes",
        "integer_ops",
        "integer_bytes",
        "window_configured",
        "window_frames_presented",
        "window_frames_acknowledged",
    ] {
        assert_eq!(left[key], right[key], "{key}");
    }
}

fn run_cli(source: &Input, stopped: bool) {
    let cli = std::env::var_os("PENTAX_TEST_CLI").unwrap_or_else(|| {
        let worker = std::env::var_os("PENTAX_TEST_WORKER")
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_pentax-worker").into());
        PathBuf::from(worker)
            .with_file_name("pentax-cli.exe")
            .into_os_string()
    });
    let mut child = Command::new(cli)
        .args(["probe-entry", "--image"])
        .arg(&source.path)
        .args(["--execution-profile", "interactive"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(0x0800_0000)
        .spawn()
        .expect("original interactive fixture requires the matching sibling CLI");
    fn read_bounded(mut reader: impl std::io::Read) -> (Vec<u8>, bool) {
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
    let output = std::thread::spawn(move || read_bounded(stdout));
    let errors = std::thread::spawn(move || read_bounded(stderr));
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = output.join();
            let _ = errors.join();
            panic!("original interactive CLI exceeded its finite test deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = child.wait().unwrap();
    let (stdout, output_overflow) = output.join().unwrap();
    let (stderr, error_overflow) = errors.join().unwrap();
    assert!(!output_overflow && !error_overflow);
    let text = String::from_utf8(stdout).unwrap();
    assert!(
        text.lines()
            .all(|line| line.starts_with("PENTAX_WORKER\t1\t"))
    );
    assert!(
        text.lines()
            .any(|line| line.starts_with("PENTAX_WORKER\t1\tRUNNING\tmode=entry-probe"))
    );
    let results: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tRESULT\t"))
        .collect();
    assert_eq!(results.len(), 1, "{text}");
    if stopped {
        assert_eq!(status.code(), Some(1), "{text}");
        assert!(results[0].starts_with(
            "PENTAX_WORKER\t1\tRESULT\tstopped\tmode=entry-probe\tcode=unsupported-instruction\t"
        ));
        assert!(results[0].contains("\tinstruction=Ud2"));
        assert!(!results[0].contains("\tvalue="));
        assert_eq!(
            String::from_utf8(stderr).unwrap().trim_end(),
            "PentaX: entry worker stopped; inspect its result category above"
        );
    } else {
        assert_eq!(status.code(), Some(0), "{text}");
        assert!(stderr.is_empty());
        assert!(
            results[0].starts_with(
                "PENTAX_WORKER\t1\tRESULT\texit\tmode=entry-probe\tcode=outer-return\t"
            )
        );
        assert!(results[0].contains("\tvalue=42"));
    }
    assert!(!text.contains(".elf"));
    assert!(!text.contains("original-module"));
}

#[test]
fn interactive_policy_retains_finite_returns_typed_stops_and_the_default_probe_limit() {
    let short = original(17, false);
    let (default_status, default_fields, default_text) = run(&short, None);
    let (interactive_status, interactive_fields, interactive_text) =
        run(&short, Some("interactive"));
    assert_eq!(default_status, 0, "{default_text}");
    assert_eq!(interactive_status, 0, "{interactive_text}");
    assert_eq!(default_fields["value"], "42");
    assert_eq!(interactive_fields["value"], "42");
    same_guest(&default_fields, &interactive_fields);
    assert_eq!(interactive_fields["block_attempts"], "18");
    assert_eq!(interactive_fields["native_instructions"], "19");
    run_cli(&short, false);

    // Traverse the actual CPU->provider->kernel path, not just a zero-service
    // loop. Independent lifetime ledgers must admit a first real request.
    let write = original_write();
    let (default_status, default_fields, default_text) = run(&write, None);
    let (interactive_status, interactive_fields, interactive_text) =
        run(&write, Some("interactive"));
    assert_eq!(default_status, 0, "{default_text}");
    assert_eq!(interactive_status, 0, "{interactive_text}");
    same_guest(&default_fields, &interactive_fields);
    for fields in [&default_fields, &interactive_fields] {
        assert_eq!(fields["service_stops"], "1");
        assert_eq!(fields["output_bytes"], "8");
        assert_eq!(fields["value"], "42");
    }
    // Binary bytes are accepted by the retained output sink, never reflected
    // into stdout; probe_arguments rejects invalid UTF-8 and raw host stderr.
    for text in [default_text, interactive_text] {
        assert!(!text.contains("71350029"));
        assert!(
            text.lines()
                .all(|line| line.starts_with("PENTAX_WORKER\t1\t"))
        );
    }

    let longer = original(12_288, false);
    let (status, fields, text) = run(&longer, None);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["code"], "native-dispatch");
    assert_eq!(fields["budget"], "dispatch-blocks");
    assert_eq!(fields["block_attempts"], "10000");
    assert!(!fields.contains_key("value"));
    let (status, fields, text) = run(&longer, Some("interactive"));
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["block_attempts"], "12289");
    assert_eq!(fields["native_instructions"], "12290");

    let stopped = original(17, true);
    let (default_status, default_fields, default_text) = run(&stopped, None);
    let (interactive_status, interactive_fields, interactive_text) =
        run(&stopped, Some("interactive"));
    assert_eq!(default_status, 2, "{default_text}");
    assert_eq!(interactive_status, 2, "{interactive_text}");
    assert_eq!(interactive_fields["code"], "unsupported-instruction");
    assert_eq!(interactive_fields["instruction"], "Ud2");
    assert!(!interactive_fields.contains_key("value"));
    same_guest(&default_fields, &interactive_fields);
    run_cli(&stopped, true);
    for text in [default_text, interactive_text] {
        assert!(!text.contains(".elf"));
        assert!(!text.contains("original-module"));
    }
}
