// SPDX-License-Identifier: GPL-2.0-only
//! Explicit launch plans and one owned, nonblocking sibling-CLI session.
//! Private input paths and raw subprocess output are never UI diagnostics.
use crate::library::LibraryEntry;
use pentax_platform::supervised_process::{self, State, SupervisedProcess};
use same_file::Handle;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Input,
    Storage,
    Overlap,
    Preferences,
    Limit,
    Sibling,
    Unsupported,
    Process,
    Protocol,
    Restart,
}
impl Error {
    pub fn message(self) -> &'static str {
        match self {
            Self::Input => "The selected executable or a required local module is unavailable.",
            Self::Storage => "PentaX could not prepare this title's local session folders.",
            Self::Overlap => "Session storage must be separate from the game folder.",
            Self::Preferences => "Repair the saved preferences before starting a session.",
            Self::Limit => "The session reached a resource or time limit.",
            Self::Sibling => {
                "PentaX CLI is missing. Install the complete PentaX application together."
            }
            Self::Unsupported => "Starting a session currently requires Windows x64.",
            Self::Process => "The session could not continue. Its processes are being stopped.",
            Self::Protocol => {
                "The session returned an unexpected response. Its processes are being stopped."
            }
            Self::Restart => "Restart PentaX before starting another session.",
        }
    }
}
pub struct Plan {
    args: Vec<OsString>,
}
impl Plan {
    /// This runs on a background preparation job. Only the selected executable
    /// and two named optional module candidates receive metadata checks.
    pub fn prepare(entry: &LibraryEntry, config: &Path) -> Result<Self, Error> {
        if [entry.path.as_path(), config].iter().any(|path| {
            path.as_os_str().as_encoded_bytes().len() > 32768 || path.components().count() > 256
        }) {
            return Err(Error::Limit);
        }
        let image = entry.path.canonicalize().map_err(|_| Error::Input)?;
        if !fs::metadata(&image).is_ok_and(|m| m.is_file()) {
            return Err(Error::Input);
        }
        let root = image.parent().ok_or(Error::Input)?;
        let config = future_path(config)?;
        if overlap(&config, root)? {
            return Err(Error::Overlap);
        }
        let key = title_key(&image, entry.title_id.as_deref())?;
        let config = ensure_directory(&config, root)?;
        let runtime = ensure_child(&config, "runtime", root)?;
        let title = ensure_child(&runtime, &key, root)?;
        let save = ensure_child(&title, "save", root)?;
        let backing = ensure_child(&title, "backing", root)?;
        if overlap(&save, &backing)? {
            return Err(Error::Overlap);
        }
        let mut args = vec![OsString::from("probe-entry")];
        pair(&mut args, "--image", image.as_os_str());
        pair(&mut args, "--memory-profile", "runtime");
        pair(&mut args, "--execution-profile", "interactive");
        pair(&mut args, "--present-window", "yes");
        pair(&mut args, "--data-directory", binding("/app0", root));
        pair(&mut args, "--save-directory", save.as_os_str());
        if let Some(title) = entry.title_id.as_deref() {
            let title = pentax_config::TitleId::new(title).map_err(|_| Error::Preferences)?;
            pair(&mut args, "--save-title-id", title.as_str());
        }
        pair(
            &mut args,
            "--physical-backing-directory",
            backing.as_os_str(),
        );
        for (name, guest, option) in [
            ("libc.prx", "/app0/sce_module/libc.prx", "--preload-libc-as"),
            (
                "libSceNpCppWebApi.prx",
                "/app0/sce_module/libSceNpCppWebApi.prx",
                "--module-file",
            ),
        ] {
            let path = root.join("sce_module").join(name);
            match fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    let actual = path.canonicalize().map_err(|_| Error::Input)?;
                    if !contains(root, &actual)? {
                        return Err(Error::Input);
                    }
                    pair(&mut args, option, binding(guest, &actual));
                    if option == "--preload-libc-as" {
                        pair(&mut args, "--libc-memory-builtins", "checked");
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(Error::Input),
            }
        }
        let preferences = config.join("global.toml");
        match fs::metadata(&preferences) {
            Ok(metadata) if metadata.is_file() && metadata.len() <= 65536 => {
                pair(&mut args, "--config", preferences.as_os_str())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(Error::Preferences),
        }
        if args
            .iter()
            .try_fold(0usize, |n, arg| n.checked_add(arg.as_encoded_bytes().len()))
            .is_none_or(|n| n > 32768)
        {
            return Err(Error::Limit);
        }
        Ok(Self { args })
    }
}
fn pair(args: &mut Vec<OsString>, option: &str, value: impl AsRef<std::ffi::OsStr>) {
    args.push(option.into());
    args.push(value.as_ref().to_owned());
}
fn binding(guest: &str, path: &Path) -> OsString {
    let mut value = OsString::from(guest);
    value.push("=");
    value.push(path);
    value
}
fn title_key(image: &Path, title: Option<&str>) -> Result<String, Error> {
    if let Some(title) = title {
        return pentax_config::TitleId::new(title.to_owned())
            .map(|id| format!("title-{id}"))
            .map_err(|_| Error::Preferences);
    }
    let digest = Sha256::digest(image.as_os_str().as_encoded_bytes());
    let mut key = String::from("local-");
    for byte in digest {
        use std::fmt::Write;
        write!(&mut key, "{byte:02x}").map_err(|_| Error::Limit)?;
    }
    Ok(key)
}
/// Canonicalize the existing prefix before creating anything. New components
/// are normal names only; an absent directory cannot conceal an existing link.
fn future_path(path: &Path) -> Result<PathBuf, Error> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(Error::Storage);
    }
    let mut at = path.to_owned();
    let mut tail = Vec::new();
    loop {
        match fs::symlink_metadata(&at) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tail.push(at.file_name().ok_or(Error::Storage)?.to_owned());
                if !at.pop() {
                    return Err(Error::Storage);
                }
            }
            Err(_) => return Err(Error::Storage),
        }
    }
    let mut resolved = at.canonicalize().map_err(|_| Error::Storage)?;
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    Ok(resolved)
}
fn contains(root: &Path, candidate: &Path) -> Result<bool, Error> {
    let root_id = Handle::from_path(root).map_err(|_| Error::Storage)?;
    for ancestor in candidate.ancestors() {
        match Handle::from_path(ancestor) {
            Ok(id) if id == root_id => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(Error::Storage),
        }
    }
    Ok(false)
}
fn overlap(left: &Path, right: &Path) -> Result<bool, Error> {
    let left = future_path(left)?;
    let right = future_path(right)?;
    if left.starts_with(&right) || right.starts_with(&left) {
        return Ok(true);
    }
    Ok((left.exists() && contains(&left, &right)?) || (right.exists() && contains(&right, &left)?))
}
fn ensure_directory(path: &Path, game: &Path) -> Result<PathBuf, Error> {
    if overlap(path, game)? {
        return Err(Error::Overlap);
    }
    fs::create_dir_all(path).map_err(|_| Error::Storage)?;
    let actual = path.canonicalize().map_err(|_| Error::Storage)?;
    if !fs::metadata(&actual).is_ok_and(|m| m.is_dir()) || overlap(&actual, game)? {
        return Err(Error::Overlap);
    }
    Ok(actual)
}
fn ensure_child(parent: &Path, name: &str, game: &Path) -> Result<PathBuf, Error> {
    let path = parent.join(name);
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        let link = metadata.file_type().is_symlink();
        #[cfg(windows)]
        let link = {
            use std::os::windows::fs::MetadataExt;
            link || metadata.file_attributes() & 0x400 != 0
        };
        if link || !metadata.is_dir() {
            return Err(Error::Storage);
        }
    }
    let actual = ensure_directory(&path, game)?;
    if !contains(parent, &actual)? {
        return Err(Error::Storage);
    }
    Ok(actual)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Summary {
    Active,
    Stopping,
    Finished,
    Stopped,
    MissingService,
    MissingInstruction,
    RuntimeStop,
    PresentationStop,
    Blocked,
    Limit,
    Failed,
    Restart,
}
impl Summary {
    pub fn message(self) -> &'static str {
        match self {
            Self::Active => "Session active. A window opens when a supported frame is ready.",
            Self::Stopping => "Stopping session…",
            Self::Finished => "Session finished.",
            Self::Stopped => "Session stopped.",
            Self::MissingService => "Startup stopped at a service that is not supported yet.",
            Self::MissingInstruction => {
                "Startup stopped at an instruction that is not supported yet."
            }
            Self::RuntimeStop => "The runtime stopped this session.",
            Self::PresentationStop => "The display session could not continue.",
            Self::Blocked => "The session stopped while waiting for an unavailable event.",
            Self::Limit => "The session reached its resource or time limit.",
            Self::Failed => "The session could not finish normally.",
            Self::Restart => Error::Restart.message(),
        }
    }
    pub fn is_error(self) -> bool {
        !matches!(
            self,
            Self::Active | Self::Stopping | Self::Finished | Self::Stopped
        )
    }
}

const LINE_BYTES: usize = 16384;
// Closed labels emitted by image_probe; unknown diagnostics never imply a limit.
fn resource_limit(code: Option<&[u8]>, budget: Option<&[u8]>, failure: Option<&[u8]>) -> bool {
    matches!(
        (code, failure),
        (Some(b"execution-budget"), _)
            | (
                Some(b"native-dispatch"),
                Some(b"dispatch-budget" | b"memory-budget")
            )
            | (
                Some(b"runtime-service"),
                Some(b"runtime-budget" | b"guest-memory-budget" | b"graphics-budget")
            )
    ) || matches!(
        (code, budget),
        (
            Some(b"native-dispatch"),
            Some(
                b"dispatch-blocks"
                    | b"dispatch-source-bytes"
                    | b"dispatch-emitted-bytes"
                    | b"dispatch-work"
                    | b"dispatch-memory-analysis"
                    | b"dispatch-memory-checks"
                    | b"dispatch-memory-bytes"
                    | b"dispatch-instructions"
                    | b"dispatch-call-work"
                    | b"dispatch-nested-calls"
                    | b"dispatch-source-declarations"
                    | b"dispatch-other"
            )
        ) | (
            Some(b"runtime-service"),
            Some(
                b"kernel-stops"
                    | b"native-service-requests"
                    | b"provider-work"
                    | b"graphics-provider-work"
                    | b"source-declarations"
                    | b"source-declaration-bytes"
                    | b"source-declaration-work"
                    | b"dynamic-module-work"
                    | b"module-input-bytes"
                    | b"module-read-work"
                    | b"module-publication-work"
                    | b"dynamic-module-window"
                    | b"runtime-other"
                    | b"kernel-attempts"
                    | b"kernel-work"
                    | b"kernel-guest-bytes"
                    | b"kernel-scratch"
                    | b"kernel-handles"
                    | b"kernel-mapped-bytes"
                    | b"kernel-directory-bytes"
                    | b"kernel-name-bytes"
                    | b"kernel-output-bytes"
                    | b"kernel-output-transfer"
                    | b"kernel-other"
            )
        )
    )
}
struct Protocol {
    line: [u8; LINE_BYTES],
    used: usize,
    result: Option<Summary>,
}
impl Protocol {
    fn new() -> Self {
        Self {
            line: [0; LINE_BYTES],
            used: 0,
            result: None,
        }
    }
    fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        for &byte in bytes {
            if byte == b'\n' {
                let line = self.line[..self.used]
                    .strip_suffix(b"\r")
                    .unwrap_or(&self.line[..self.used]);
                if self.result.is_some() {
                    return Err(Error::Protocol);
                }
                let mut words = line
                    .strip_prefix(b"PENTAX_WORKER\t1\t")
                    .ok_or(Error::Protocol)?
                    .split(|b| *b == b'\t');
                match words.next() {
                    Some(b"READY" | b"RUNNING" | b"PROGRESS") => {}
                    Some(b"RESULT") => {
                        let outcome = words.next();
                        let mut code = None;
                        let mut budget = None;
                        let mut failure = None;
                        for word in words {
                            let field = if let Some(value) = word.strip_prefix(b"code=") {
                                Some((&mut code, value))
                            } else if let Some(value) = word.strip_prefix(b"budget=") {
                                Some((&mut budget, value))
                            } else {
                                word.strip_prefix(b"runtime_failure=")
                                    .map(|value| (&mut failure, value))
                            };
                            if let Some((field, value)) = field
                                && field.replace(value).is_some()
                            {
                                return Err(Error::Protocol);
                            }
                        }
                        self.result = Some(match code {
                            _ if resource_limit(code, budget, failure) => Summary::Limit,
                            Some(b"outer-return") if outcome == Some(b"exit") => Summary::Finished,
                            Some(b"unavailable-function") => Summary::MissingService,
                            Some(b"unsupported-instruction" | b"unsupported-memory-footprint") => {
                                Summary::MissingInstruction
                            }
                            Some(b"frame-session") => Summary::PresentationStop,
                            Some(b"thread-deadlock") => Summary::Blocked,
                            Some(b"runtime-service") => Summary::RuntimeStop,
                            _ => Summary::Failed,
                        });
                    }
                    _ => return Err(Error::Protocol),
                }
                self.used = 0;
            } else {
                if self.used == self.line.len() {
                    return Err(Error::Limit);
                }
                self.line[self.used] = byte;
                self.used += 1;
            }
        }
        Ok(())
    }
}

pub struct Session {
    process: Option<SupervisedProcess>,
    protocol: Protocol,
    pub summary: Summary,
    pub finished: bool,
    cancelled: bool,
    failed: Option<Summary>,
}
impl Session {
    pub fn start(plan: Plan) -> Result<Self, Error> {
        let process =
            SupervisedProcess::spawn_cli(&plan.args, supervised_process::Limits::interactive())
                .map_err(process_error)?;
        Ok(Self {
            process: Some(process),
            protocol: Protocol::new(),
            summary: Summary::Active,
            finished: false,
            cancelled: false,
            failed: None,
        })
    }
    pub fn stop(&mut self) {
        self.cancelled = true;
        self.summary = Summary::Stopping;
        if let Some(process) = &mut self.process
            && let Err(error) = process.request_cancel()
        {
            self.failure(error);
        }
    }
    fn failure(&mut self, error: supervised_process::Error) {
        let error = process_error(error);
        if error == Error::Restart {
            self.summary = Summary::Restart;
            self.finished = true;
            self.process = None;
        } else {
            self.failed.get_or_insert(if error == Error::Limit {
                Summary::Limit
            } else {
                Summary::Failed
            });
            self.summary = Summary::Stopping;
        }
    }
    pub fn poll(&mut self) {
        if self.finished {
            return;
        }
        let mut stdout = [0; 4096];
        let mut stderr = [0; 4096];
        let Some(process) = self.process.as_mut() else {
            self.finished = true;
            return;
        };
        let status = if self.cancelled || self.failed.is_some() {
            process.shutdown_poll(&mut stdout, &mut stderr)
        } else {
            process.poll(&mut stdout, &mut stderr)
        };
        match status {
            Ok(status) => {
                if self.protocol.feed(&stdout[..status.stdout_len]).is_err() {
                    self.failed = Some(Summary::Failed);
                    self.stop();
                }
                match status.state {
                    State::Running => {}
                    State::Stopping => self.summary = Summary::Stopping,
                    State::Exited(code) => {
                        self.summary = self.failed.unwrap_or({
                            if self.cancelled {
                                Summary::Stopped
                            } else if self.protocol.used != 0 {
                                Summary::Failed
                            } else {
                                match self.protocol.result {
                                    Some(Summary::Finished) if code == 0 => Summary::Finished,
                                    Some(Summary::Finished) | None => Summary::Failed,
                                    Some(result) => result,
                                }
                            }
                        });
                        self.finished = true;
                        self.process = None;
                    }
                    State::Closed => {
                        self.summary = self.failed.unwrap_or(Summary::Stopped);
                        self.finished = true;
                        self.process = None;
                    }
                }
            }
            Err(error) => self.failure(error),
        }
        // stderr is deliberately drained and discarded, never rendered/logged.
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if let Some(process) = &mut self.process {
            let _ = process.request_cancel();
        }
    }
}
fn process_error(error: supervised_process::Error) -> Error {
    use supervised_process::Error as E;
    match error {
        E::UnsupportedHost => Error::Unsupported,
        E::SiblingUnavailable => Error::Sibling,
        E::Limit | E::OutputLimit | E::Timeout => Error::Limit,
        E::RestartRequired | E::AlreadyRunning => Error::Restart,
        E::Allocation | E::InvalidArguments | E::Closed | E::OperatingSystem { .. } => {
            Error::Process
        }
    }
}

/// Cleanup never grants reopen authority: the desktop keeps its restart latch.
pub fn poll_quarantined_cleanup() {
    let _ = SupervisedProcess::poll_quarantined_cleanup();
}

#[cfg(test)]
mod tests;
