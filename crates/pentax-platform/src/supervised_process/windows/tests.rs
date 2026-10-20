// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded diagnostics and actual private-job process trees.
use super::*;
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::Duration,
};

static PROCESS_TEST: Mutex<()> = Mutex::new(());

fn quoted(value: &str) -> Result<Vec<u16>, Error> {
    let mut result = reserve::<u16>(MAX_COMMAND_UNITS, MAX_COMMAND_UNITS * 2)?;
    quote(&mut result, value.encode_utf16())?;
    Ok(result)
}

#[test]
fn windows_command_words_preserve_empty_unicode_quotes_and_trailing_slashes_without_shell() {
    assert_eq!(quoted("").unwrap(), [34, 34]);
    assert_eq!(quoted("a b\\").unwrap(), [34, 97, 32, 98, 92, 92, 34]);
    assert_eq!(quoted("a\"b").unwrap(), [34, 97, 92, 34, 98, 34]);
    assert_eq!(
        quoted("\u{03bb}\t\\\"").unwrap(),
        [34, 0x03bb, 9, 92, 92, 92, 34, 34]
    );
    assert_eq!(quoted("x\0y"), Err(Error::InvalidArguments));
    assert_eq!(
        quoted(&"\\".repeat(MAX_COMMAND_UNITS)),
        Err(Error::InvalidArguments)
    );
    let original = Path::new(r"Z:\original-fixture\pentax-cli.exe");
    assert!(command(original, &vec![OsString::new(); MAX_ARGUMENTS + 1]).is_err());
    let (app, line) = command(original, &["".into(), "a b\\".into(), "a\"b".into()]).unwrap();
    let expected = format!("\"{}\" \"\" \"a b\\\\\" \"a\\\"b\"\0", original.display());
    assert_eq!(line, expected.encode_utf16().collect::<Vec<_>>());
    assert_eq!(
        app,
        original
            .as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    );
    assert!(command(Path::new("pentax-cli.exe"), &[]).is_err());
}

#[test]
fn actual_private_diagnostic_pipe_preserves_partial_chunks_and_discards_excess_only_on_shutdown() {
    let (read, write) = pipe().unwrap();
    let mut reader = Reader {
        handle: read,
        captured: 0,
        maximum: 5,
        eof: false,
    };
    let mut output = [0xa5; 4];
    assert_eq!(reader.read(&mut output, false), Ok(0));
    assert_eq!(output, [0xa5; 4]);
    let original = [0, 1, 2, 0xff, 4, 5, 6];
    let mut count = 0;
    // SAFETY: Fresh empty private pipe, a tiny bounded message fits its known
    // buffer, and no other writer exists. No guest or UI pause is involved.
    let written = unsafe {
        WriteFile(
            write.raw(),
            original.as_ptr(),
            original.len() as u32,
            &mut count,
            null_mut(),
        )
    };
    assert_ne!(written, 0);
    assert_eq!(count, original.len() as u32);
    assert_eq!(reader.read(&mut output, false), Ok(4));
    assert_eq!(output, [0, 1, 2, 0xff]);
    output.fill(0xa5);
    assert_eq!(reader.read(&mut output, false), Ok(1));
    assert_eq!(output, [4, 0xa5, 0xa5, 0xa5]);
    assert_eq!(reader.read(&mut output, false), Err(Error::OutputLimit));
    assert_eq!(reader.captured, 5);
    output.fill(0xa5);
    assert_eq!(reader.read(&mut output, true), Ok(0));
    assert_eq!(output, [0xa5; 4]);
    assert_eq!(reader.captured, 5);
    assert!(!reader.eof);
    drop(write);
    assert_eq!(reader.read(&mut output, true), Ok(0));
    assert!(reader.eof);
}

const STREAM_CHILD: &str = "supervised_process::windows::tests::original_stream_payload";
const TREE_CHILD: &str = "supervised_process::windows::tests::original_tree_payload";
const GRANDCHILD: &str = "supervised_process::windows::tests::original_grandchild_payload";

fn args(name: &str) -> Vec<OsString> {
    [
        "--exact",
        name,
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ]
    .into_iter()
    .map(OsString::from)
    .collect()
}
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|part| part == needle)
}

#[test]
fn actual_job_drains_root_output_and_cancellation_reaps_a_running_descendant_before_exit() {
    let _guard = PROCESS_TEST.lock().unwrap();
    let executable = std::env::current_exe().unwrap();
    let limits = Limits {
        run_timeout: Duration::from_secs(10),
        ..Default::default()
    };
    // Private original-fixture constructor only: the public API still accepts
    // exactly the sibling CLI, never an arbitrary path from the desktop/guest.
    for tree in [false, true] {
        let mut process = SupervisedProcess::spawn(
            &executable,
            &args(if tree { TREE_CHILD } else { STREAM_CHILD }),
            limits,
        )
        .unwrap();
        let mut out = [0; PUMP_BYTES];
        let mut err = [0; PUMP_BYTES];
        let mut captured_out = [0; super::super::OUTPUT_CAP];
        let mut captured_err = [0; super::super::OUTPUT_CAP];
        let mut out_len = 0;
        let mut err_len = 0;
        let mut cancel = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = if cancel {
                process.shutdown_poll(&mut out, &mut err)
            } else {
                process.poll(&mut out, &mut err)
            }
            .unwrap();
            captured_out[out_len..out_len + status.stdout_len]
                .copy_from_slice(&out[..status.stdout_len]);
            captured_err[err_len..err_len + status.stderr_len]
                .copy_from_slice(&err[..status.stderr_len]);
            out_len += status.stdout_len;
            err_len += status.stderr_len;
            if tree && !cancel && contains(&captured_out[..out_len], b"original-grandchild-ready") {
                assert!(process.inner.as_ref().unwrap().descendants().unwrap() >= 2);
                process.request_cancel().unwrap();
                cancel = true;
            }
            if let State::Exited(code) = status.state {
                if tree {
                    assert!(cancel);
                    assert_eq!(code, 1);
                } else {
                    assert_eq!(code, 0);
                    assert!(contains(&captured_out[..out_len], b"original-output-ready"));
                    assert!(contains(&captured_err[..err_len], b"original-error-ready"));
                }
                assert!(process.inner.is_none());
                assert_eq!(process.poll(&mut out, &mut err).unwrap(), Status::CLOSED);
                assert_eq!(ACTIVE.load(Ordering::Acquire), 0);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "original supervised process did not reach actual root/job/EOF completion"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

#[test]
fn finished_root_with_retained_diagnostic_writer_has_finite_grace_without_forged_eof() {
    let _guard = PROCESS_TEST.lock().unwrap();
    let executable = std::env::current_exe().unwrap();
    let limits = Limits::default();
    let mut process = SupervisedProcess::spawn(&executable, &args(STREAM_CHILD), limits).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let inner = process.inner.as_mut().unwrap();
        if inner.exit().unwrap().is_some() && inner.descendants().unwrap() == 0 {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let inner = process.inner.as_mut().unwrap();
    let mut out = [0; PUMP_BYTES];
    let mut err = [0; PUMP_BYTES];
    for _ in 0..32 {
        inner.stderr.read(&mut err, true).unwrap();
        if inner.stderr.eof {
            break;
        }
    }
    assert!(inner.stderr.eof);
    // Keep one real writer open after independently observing the real root
    // exit and empty job. Only the deadline is modeled; no process, job count
    // or EOF observation is fabricated, and no production hook is introduced.
    let (read, retained_writer) = pipe().unwrap();
    inner.stdout = Reader {
        handle: read,
        captured: 0,
        maximum: super::super::OUTPUT_CAP,
        eof: false,
    };
    inner.stopping = Some(
        Instant::now()
            .checked_sub(limits.shutdown_grace + Duration::from_secs(1))
            .unwrap(),
    );
    assert_eq!(
        inner.poll(&mut out, &mut err, true),
        Err(Error::RestartRequired)
    );
    assert!(!inner.stdout.eof);
    assert_eq!(inner.exit().unwrap(), Some(0));
    assert_eq!(inner.descendants().unwrap(), 0);
    // Actual closure, even after a deadline, is independently observable. The
    // public wrapper may now release its owner without poisoning later tests.
    drop(retained_writer);
    assert_eq!(
        inner.poll(&mut out, &mut err, true).unwrap().state,
        State::Exited(0)
    );
    assert!(inner.stdout.eof);
    assert_eq!(
        process.poll(&mut out, &mut err).unwrap().state,
        State::Exited(0)
    );
    assert!(process.inner.is_none());
    assert_eq!(ACTIVE.load(Ordering::Acquire), 0);
}

#[test]
#[ignore = "original child payload selected only by its supervised parent test"]
fn original_stream_payload() {
    println!("original-output-ready");
    eprintln!("original-error-ready");
}
#[test]
#[ignore = "original child payload selected only by its supervised parent test"]
fn original_tree_payload() {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(args(GRANDCHILD))
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    // This original descendant is deliberately created through normal process
    // creation, exercising automatic inheritance of the already admitted job.
    // If the outer proof fails, both payloads still have finite lifetimes.
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
    }
}
#[test]
#[ignore = "original grandchild payload selected only by its supervised parent test"]
fn original_grandchild_payload() {
    println!("original-grandchild-ready");
    std::thread::sleep(Duration::from_secs(10));
}
