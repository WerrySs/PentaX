// SPDX-License-Identifier: GPL-2.0-only

//! Separate worker supervision with bounded output, deadline and guaranteed reap.

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

const MAX_OUTPUT: u64 = 64 * 1024;
const MAX_RECORD: usize = 16 * 1024;

struct Worker {
    child: Child,
    reaped: bool,
}
impl Drop for Worker {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
fn collect(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_OUTPUT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err(io::Error::other("worker output limit exceeded"));
    }
    Ok(bytes)
}

struct Record {
    bytes: [u8; MAX_RECORD],
    used: usize,
}
impl Record {
    fn empty() -> Self {
        Self {
            bytes: [0; MAX_RECORD],
            used: 0,
        }
    }
}

#[derive(Default)]
struct Completion {
    result: bool,
    outer_return: bool,
}
impl Completion {
    fn record(&mut self, record: &[u8]) -> io::Result<()> {
        let invalid = || io::Error::other("worker protocol record refused");
        if self.result {
            return Err(invalid());
        }
        let line = record.strip_suffix(b"\r").unwrap_or(record);
        let mut words = line
            .strip_prefix(b"PENTAX_WORKER\t1\t")
            .ok_or_else(invalid)?
            .split(|byte| *byte == b'\t');
        let kind = words.next().ok_or_else(invalid)?;
        let outcome = if kind == b"RESULT" {
            Some(words.next().ok_or_else(invalid)?)
        } else {
            None
        };
        if !matches!(kind, b"READY" | b"RUNNING" | b"PROGRESS" | b"RESULT")
            || outcome.is_some_and(|value| {
                !matches!(value, b"exit" | b"stopped" | b"rejected" | b"unsupported")
            })
        {
            return Err(invalid());
        }
        let mut code = None;
        let mut mode = None;
        let mut fields = 0;
        for word in words {
            let at = word
                .iter()
                .position(|byte| *byte == b'=')
                .ok_or_else(invalid)?;
            let (key, value) = (&word[..at], &word[at + 1..]);
            if key.is_empty()
                || key == b"detail"
                || !key
                    .iter()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
                || value.is_empty() && !matches!(key, b"service_top" | b"service_nonzero_top")
                || !value.iter().all(|byte| {
                    byte.is_ascii_alphanumeric()
                        || matches!(
                            *byte,
                            b'_' | b'-' | b':' | b',' | b'.' | b'(' | b')' | b'+' | b'<' | b'>'
                        )
                })
            {
                return Err(invalid());
            }
            if key == b"code" && code.replace(value).is_some()
                || key == b"mode" && mode.replace(value).is_some()
            {
                return Err(invalid());
            }
            fields += 1;
        }
        if fields == 0
            || matches!(kind, b"RUNNING" | b"PROGRESS") && mode != Some(&b"entry-probe"[..])
            || matches!(outcome, Some(b"exit" | b"stopped"))
                && (mode != Some(&b"entry-probe"[..]) || code.is_none())
        {
            return Err(invalid());
        }
        self.result = outcome.is_some();
        self.outer_return = outcome == Some(&b"exit"[..]) && code == Some(&b"outer-return"[..]);
        Ok(())
    }
}

// At most two complete records are queued. A new record is validated before
// publication; no arbitrary worker stderr, partial record or free-form error
// crosses into the public CLI stream.
fn stream_records(mut reader: impl Read, output: SyncSender<Record>) -> io::Result<Completion> {
    let mut record = Record::empty();
    let mut completion = Completion::default();
    let mut input = [0; 4096];
    loop {
        let count = reader.read(&mut input)?;
        if count == 0 {
            if record.used != 0 {
                return Err(io::Error::other("worker protocol ended inside a record"));
            }
            return Ok(completion);
        }
        for &byte in &input[..count] {
            if byte == b'\n' {
                completion.record(&record.bytes[..record.used])?;
                if record.used == MAX_RECORD {
                    return Err(io::Error::other("worker protocol record limit exceeded"));
                }
                record.bytes[record.used] = b'\n';
                record.used += 1;
                output
                    .send(record)
                    .map_err(|_| io::Error::other("worker protocol consumer closed"))?;
                record = Record::empty();
            } else {
                if record.used == MAX_RECORD - 1 {
                    return Err(io::Error::other("worker protocol record limit exceeded"));
                }
                record.bytes[record.used] = byte;
                record.used += 1;
            }
        }
    }
}

fn reject_stderr(mut reader: impl Read) -> io::Result<()> {
    let mut byte = [0];
    if reader.read(&mut byte)? == 0 {
        Ok(())
    } else {
        Err(io::Error::other(
            "worker stderr refused; raw diagnostics withheld",
        ))
    }
}

// Empty is transient while the producer is alive. Only Disconnected proves
// that every validated record has been forwarded, including a queued RESULT.
fn drain_records(receiver: &Receiver<Record>, writer: &mut impl Write) -> io::Result<bool> {
    for _ in 0..2 {
        match receiver.try_recv() {
            Ok(record) => {
                writer.write_all(&record.bytes[..record.used])?;
                writer.flush()?;
            }
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => return Ok(true),
        }
    }
    Ok(false)
}

fn run_interactive(
    worker: &mut Worker,
    stdout: impl Read + Send + 'static,
    stderr: impl Read + Send + 'static,
) -> Result<(), Box<dyn Error>> {
    let (sender, receiver) = mpsc::sync_channel(2);
    let mut output = Some(
        std::thread::Builder::new()
            .name("interactive-worker-output".into())
            .spawn(move || stream_records(stdout, sender))?,
    );
    let errors = std::thread::Builder::new()
        .name("interactive-worker-errors".into())
        .spawn(move || reject_stderr(stderr));
    let mut errors = match errors {
        Ok(thread) => Some(thread),
        Err(_) => {
            drop(receiver);
            let _ = worker.child.kill();
            if worker.child.wait().is_ok() {
                worker.reaped = true;
            }
            let _ = output.take().unwrap().join();
            return Err("worker stderr reader unavailable".into());
        }
    };
    let mut completion = None;
    let mut stderr_done = false;
    let mut status = None;
    let mut writer = io::stdout().lock();
    let result: Result<(), Box<dyn Error>> = (|| {
        loop {
            let drained = drain_records(&receiver, &mut writer)?;
            if output.as_ref().is_some_and(|thread| thread.is_finished()) {
                completion = Some(
                    output
                        .take()
                        .unwrap()
                        .join()
                        .map_err(|_| "worker stdout reader failed")??,
                );
                if !completion.as_ref().is_some_and(|value| value.result) {
                    return Err("entry worker terminated before a final result".into());
                }
            }
            if errors.as_ref().is_some_and(|thread| thread.is_finished()) {
                errors
                    .take()
                    .unwrap()
                    .join()
                    .map_err(|_| "worker stderr reader failed")??;
                stderr_done = true;
            }
            if status.is_none() {
                status = worker.child.try_wait()?;
                worker.reaped = status.is_some();
            }
            if let (Some(status), Some(completion)) = (&status, &completion)
                && stderr_done
                && drained
            {
                if status.success() && completion.result && completion.outer_return {
                    return Ok(());
                }
                return Err(if completion.result {
                    "entry worker stopped; inspect its result category above"
                } else {
                    "entry worker terminated before a final result"
                }
                .into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    // Disconnect queued sends before stopping the exact child. Reader storage
    // remains owned until the child is reaped and both threads have joined.
    drop(receiver);
    if !worker.reaped {
        let _ = worker.child.kill();
        if worker.child.wait().is_ok() {
            worker.reaped = true;
        }
    }
    if let Some(thread) = output {
        let _ = thread.join();
    }
    if let Some(thread) = errors {
        let _ = thread.join();
    }
    result
}

pub(super) fn run(args: &[OsString]) -> Result<(), Box<dyn Error>> {
    if args.is_empty()
        || !args.len().is_multiple_of(2)
        || args.len() / 2 > 125 + 256 + 10
        || args
            .chunks_exact(2)
            .filter(|pair| {
                pair[0] != "--data-file"
                    && pair[0] != "--config"
                    && pair[0] != "--save-directory"
                    && pair[0] != "--physical-backing-directory"
                    && pair[0] != "--data-directory"
                    && pair[0] != "--memory-profile"
                    && pair[0] != "--execution-profile"
                    && pair[0] != "--present-window"
                    && pair[0] != "--libc-memory-builtins"
                    && pair[0] != "--private-samples"
                    && pair[0] != "--private-thread-states"
                    && pair[0] != "--private-console"
            })
            .count()
            > 125
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--data-file" || pair[0] == "--data-directory")
            .count()
            > 256
        || args[0] != "--image"
        || args.chunks_exact(2).any(|pair| {
            pair[0] != "--image"
                && pair[0] != "--config"
                && pair[0] != "--save-directory"
                && pair[0] != "--physical-backing-directory"
                && pair[0] != "--preload-libc"
                && pair[0] != "--preload-libc-as"
                && pair[0] != "--module-file"
                && pair[0] != "--data-file"
                && pair[0] != "--data-directory"
                && pair[0] != "--memory-profile"
                && pair[0] != "--execution-profile"
                && pair[0] != "--present-window"
                && pair[0] != "--libc-memory-builtins"
                && pair[0] != "--private-samples"
                && pair[0] != "--private-thread-states"
                && pair[0] != "--private-console"
        })
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--preload-libc" || pair[0] == "--preload-libc-as")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--memory-profile")
            .count()
            > 1
        || args.chunks_exact(2).any(|pair| {
            pair[0] == "--memory-profile" && pair[1] != "compact" && pair[1] != "runtime"
        })
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--execution-profile")
            .count()
            > 1
        || args.chunks_exact(2).any(|pair| {
            pair[0] == "--execution-profile"
                && pair[1] != "probe"
                && pair[1] != "startup"
                && pair[1] != "extended-startup"
                && pair[1] != "long-startup"
                && pair[1] != "interactive"
        })
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--present-window")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--present-window" && pair[1] != "yes" && pair[1] != "no")
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--libc-memory-builtins")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--private-samples")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--private-console")
            .count()
            > 1
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--private-thread-states")
            .count()
            > 1
        || args.chunks_exact(2).any(|pair| {
            pair[0] == "--libc-memory-builtins" && pair[1] != "off" && pair[1] != "checked"
        })
        || (args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--libc-memory-builtins" && pair[1] == "checked")
            && !args
                .chunks_exact(2)
                .any(|pair| pair[0] == "--preload-libc" || pair[0] == "--preload-libc-as"))
    {
        return Err(
            "use probe-entry --image <path> [--memory-profile compact|runtime] [--physical-backing-directory <host-directory>] [--execution-profile probe|startup|extended-startup|long-startup|interactive] [--present-window yes|no] [--preload-libc <path> | --preload-libc-as <guest-path>=<host-path>] [--libc-memory-builtins off|checked] [--private-samples <fresh-file>] [--private-thread-states <fresh-file>] [--private-console <fresh-file>] [--image <path> ...] [--module-file <guest-path>=<host-path> ...] [--data-file <guest-path>=<host-path> ...] [--data-directory <guest-mount>=<host-directory> ...] [--save-directory <host-directory>]; first image is primary".into(),
        );
    }
    if args
        .chunks_exact(2)
        .filter(|pair| pair[0] == "--physical-backing-directory")
        .count()
        > 1
        || (args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--physical-backing-directory")
            && !args
                .chunks_exact(2)
                .any(|pair| pair[0] == "--memory-profile" && pair[1] == "runtime"))
    {
        return Err("one physical backing directory requires the runtime memory profile".into());
    }
    if args
        .chunks_exact(2)
        .filter(|pair| pair[0] == "--config")
        .count()
        > 1
    {
        return Err("only one --config file may be selected".into());
    }
    let executable = std::env::current_exe()?.with_file_name(if cfg!(windows) {
        "pentax-worker.exe"
    } else {
        "pentax-worker"
    });
    let mut command = Command::new(executable);
    command
        .arg("probe-entry")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut worker = Worker {
        child: command.spawn()?,
        reaped: false,
    };
    let stdout = worker
        .child
        .stdout
        .take()
        .ok_or("worker stdout unavailable")?;
    let stderr = worker
        .child
        .stderr
        .take()
        .ok_or("worker stderr unavailable")?;
    if args
        .chunks_exact(2)
        .any(|pair| pair[0] == "--execution-profile" && pair[1] == "interactive")
    {
        return run_interactive(&mut worker, stdout, stderr);
    }
    let output = std::thread::Builder::new()
        .name("entry-probe-output".into())
        .spawn(move || collect(stdout))?;
    let errors = std::thread::Builder::new()
        .name("entry-probe-errors".into())
        .spawn(move || collect(stderr))?;
    let deadline_seconds = if args
        .chunks_exact(2)
        .any(|pair| pair[0] == "--execution-profile" && pair[1] == "long-startup")
    {
        300
    } else if args
        .chunks_exact(2)
        .any(|pair| pair[0] == "--execution-profile" && pair[1] == "extended-startup")
    {
        120
    } else {
        30
    };
    let deadline = Instant::now() + Duration::from_secs(deadline_seconds);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = worker.child.try_wait()? {
            worker.reaped = true;
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            worker.child.kill()?;
            let status = worker.child.wait()?;
            worker.reaped = true;
            break status;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let output = output.join().map_err(|_| "worker stdout reader failed")??;
    let errors = errors.join().map_err(|_| "worker stderr reader failed")??;
    let text = std::str::from_utf8(&output).map_err(|_| "worker protocol is not UTF-8")?;
    if text
        .lines()
        .any(|line| !line.starts_with("PENTAX_WORKER\t1\t"))
    {
        return Err("worker emitted unexpected output; raw diagnostics withheld".into());
    }
    let results: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("PENTAX_WORKER\t1\tRESULT\t"))
        .collect();
    if results.len() > 1
        || (status.success()
            && (results.len() != 1
                || !results[0].starts_with(
                    "PENTAX_WORKER\t1\tRESULT\texit\tmode=entry-probe\tcode=outer-return\t",
                )))
    {
        return Err("worker completion protocol is invalid".into());
    }
    io::stdout().write_all(&output)?;
    if timed_out {
        return Err(format!(
            "entry probe exceeded its {deadline_seconds}-second deadline; worker stopped"
        )
        .into());
    }
    if !errors.is_empty() {
        return Err("entry worker reported an internal error; raw diagnostics withheld".into());
    }
    if !status.success() {
        let reported = std::str::from_utf8(&output).is_ok_and(|text| {
            text.lines()
                .any(|line| line.starts_with("PENTAX_WORKER\t1\tRESULT\t"))
        });
        if reported {
            return Err("entry probe stopped; inspect its result category above".into());
        }
        return Err(format!(
            "entry worker terminated before a final result ({status}); progress records contain partial observations only"
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_output_collection_enforces_the_total_byte_bound() {
        assert_eq!(
            collect(io::Cursor::new(vec![b'a'; MAX_OUTPUT as usize]))
                .unwrap()
                .len(),
            MAX_OUTPUT as usize
        );
        assert!(collect(io::Cursor::new(vec![b'a'; MAX_OUTPUT as usize + 1])).is_err());
    }

    #[test]
    fn interactive_records_stream_beyond_the_diagnostic_total_with_one_terminal_result() {
        let progress = b"PENTAX_WORKER\t1\tPROGRESS\tmode=entry-probe\tblock_attempts=123456789\n";
        let mut source = Vec::new();
        for _ in 0..2048 {
            source.extend(progress);
        }
        // These two existing closed lists are empty in a real zero-service
        // entry. The interactive route must retain that finite wire format.
        source.extend(b"PENTAX_WORKER\t1\tRESULT\texit\tmode=entry-probe\tcode=outer-return\tvalue=42\tservice_top=\tservice_nonzero_top=\n");
        assert!(source.len() as u64 > MAX_OUTPUT);
        let (sender, receiver) = mpsc::sync_channel(2);
        let thread = std::thread::spawn(move || stream_records(io::Cursor::new(source), sender));
        let mut records = 0;
        while let Ok(record) = receiver.recv() {
            assert!(record.used <= MAX_RECORD);
            assert_eq!(record.bytes[record.used - 1], b'\n');
            records += 1;
        }
        let completion = thread.join().unwrap().unwrap();
        assert_eq!(records, 2049);
        assert!(completion.result && completion.outer_return);
    }

    #[test]
    fn interactive_records_refuse_partial_raw_oversize_and_post_result_output() {
        for source in [
            b"raw output\n".as_slice(),
            b"PENTAX_WORKER\t1\tRESULT\terror\tdetail=raw-data\n",
            b"PENTAX_WORKER\t1\tPROGRESS\tmode=entry-probe\tvalue=\xff\n",
            b"PENTAX_WORKER\t1\tPROGRESS\tmode=entry-probe\tvalue=raw payload\n",
            b"PENTAX_WORKER\t1\tPROGRESS\tmode=entry-probe\tvalue=\n",
            b"PENTAX_WORKER\t1\tPROGRESS\tmode=entry-probe",
            b"PENTAX_WORKER\t1\tRESULT\texit\tmode=entry-probe\tcode=outer-return\tcode=outer-return\n",
        ] {
            let (sender, receiver) = mpsc::sync_channel(2);
            assert!(stream_records(io::Cursor::new(source), sender).is_err());
            assert!(receiver.try_recv().is_err());
        }
        let (sender, receiver) = mpsc::sync_channel(2);
        assert!(stream_records(io::Cursor::new(vec![b'a'; MAX_RECORD]), sender).is_err());
        assert!(receiver.try_recv().is_err());
        let source = b"PENTAX_WORKER\t1\tRESULT\tstopped\tmode=entry-probe\tcode=unsupported-instruction\nPENTAX_WORKER\t1\tPROGRESS\tmode=entry-probe\n";
        let (sender, receiver) = mpsc::sync_channel(2);
        assert!(stream_records(io::Cursor::new(source), sender).is_err());
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err());
        assert!(reject_stderr(io::Cursor::new([])).is_ok());
        assert!(reject_stderr(io::Cursor::new(b"private payload")).is_err());
        let (sender, receiver) = mpsc::sync_channel(2);
        drop(receiver);
        assert!(
            stream_records(
                io::Cursor::new(b"PENTAX_WORKER\t1\tREADY\tbackend=windows-x86_64-image-probe\n"),
                sender
            )
            .is_err()
        );
    }

    #[test]
    fn interactive_eof_waits_for_the_final_record_queued_after_an_empty_poll() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let mut forwarded = Vec::new();
        assert!(!drain_records(&receiver, &mut forwarded).unwrap());
        let final_record =
            b"PENTAX_WORKER\t1\tRESULT\texit\tmode=entry-probe\tcode=outer-return\tvalue=42\n";
        let mut record = Record::empty();
        record.bytes[..final_record.len()].copy_from_slice(final_record);
        record.used = final_record.len();
        sender.send(record).unwrap();
        drop(sender);
        assert!(drain_records(&receiver, &mut forwarded).unwrap());
        assert_eq!(forwarded, final_record);
    }
}
