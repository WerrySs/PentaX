// SPDX-License-Identifier: GPL-2.0-only

//! Separate worker supervision with bounded output, deadline and guaranteed reap.

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT: u64 = 64 * 1024;

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

pub(super) fn run(args: &[OsString]) -> Result<(), Box<dyn Error>> {
    if args.is_empty()
        || !args.len().is_multiple_of(2)
        || args.len() / 2 > 125 + 256 + 7
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
            "use probe-entry --image <path> [--memory-profile compact|runtime] [--physical-backing-directory <host-directory>] [--execution-profile probe|startup|extended-startup|long-startup] [--present-window yes|no] [--preload-libc <path> | --preload-libc-as <guest-path>=<host-path>] [--libc-memory-builtins off|checked] [--image <path> ...] [--module-file <guest-path>=<host-path> ...] [--data-file <guest-path>=<host-path> ...] [--data-directory <guest-mount>=<host-directory> ...] [--save-directory <host-directory>]; first image is primary".into(),
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
}
