// SPDX-License-Identifier: GPL-2.0-only
//! Reviewed Win32 ownership, quoting, explicit inheritance and zero-time polls.
//! Process/job contracts: CreateProcessW, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
//! AssignProcessToJobObject, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, PeekNamedPipe,
//! TerminateJobObject and WaitForSingleObject (timeout0). No shell is involved.

use super::{Error, Limits, MAX_ARGUMENTS, MAX_COMMAND_UNITS, PUMP_BYTES, State, Status};
use std::{
    ffi::OsString,
    mem::{size_of, size_of_val},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::Path,
    ptr::{null, null_mut},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    Storage::FileSystem::*,
    System::{JobObjects::*, Pipes::*, Threading::*},
};

static ACTIVE: AtomicU8 = AtomicU8::new(0); // 0 vacant, 1 owned, 2 restart required
static QUARANTINE: OnceLock<Mutex<Option<Inner>>> = OnceLock::new();

struct Permit;
impl Permit {
    fn acquire() -> Result<Self, Error> {
        ACTIVE
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|state| {
                if state == 2 {
                    Error::RestartRequired
                } else {
                    Error::AlreadyRunning
                }
            })
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        let _ = ACTIVE.compare_exchange(1, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}
struct Handle(OwnedHandle);
impl Handle {
    fn take(raw: HANDLE, operation: &'static str) -> Result<Self, Error> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            return Err(os(operation));
        }
        // SAFETY: Every private call transfers one newly created owned handle;
        // no public raw-handle constructor, pseudo or borrowed handle is used.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(raw) }))
    }
    fn raw(&self) -> HANDLE {
        self.0.as_raw_handle()
    }
    fn no_inherit(&self) -> Result<(), Error> {
        // SAFETY: Only this new live parent read handle's local bit is changed.
        if unsafe { SetHandleInformation(self.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
            return Err(os("parent inheritance"));
        }
        Ok(())
    }
}
fn os(operation: &'static str) -> Error {
    // SAFETY: Capture the immediately preceding Win32 error, without payloads.
    Error::OperatingSystem {
        operation,
        code: unsafe { GetLastError() },
    }
}
fn inherited_attributes() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: 1,
    }
}
fn pipe() -> Result<(Handle, Handle), Error> {
    let mut read = null_mut();
    let mut write = null_mut();
    let attributes = inherited_attributes();
    // SAFETY: New anonymous byte pair, default owner ACL, fixed bounded buffer.
    // The read endpoint's inheritance is immediately cleared before launch.
    if unsafe { CreatePipe(&mut read, &mut write, &attributes, PUMP_BYTES as u32) } == 0 {
        return Err(os("diagnostic pipe"));
    }
    let read = Handle::take(read, "diagnostic read handle")?;
    let write = Handle::take(write, "diagnostic write handle")?;
    read.no_inherit()?;
    Ok((read, write))
}
fn null_input() -> Result<Handle, Error> {
    let attributes = inherited_attributes();
    // SAFETY: Fixed Windows NUL device, read-only owned handle. No console,
    // ambient file name, user-selected path or foreign stdin is inherited.
    Handle::take(
        unsafe {
            CreateFileW(
                [78_u16, 85, 76, 0].as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                &attributes,
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        },
        "null input",
    )
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new(handles: &[HANDLE; 3]) -> Result<Self, Error> {
        let mut bytes = 0;
        // SAFETY: Exact count-only query for one explicit inheritance list.
        let _ = unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes) };
        if bytes == 0 || bytes > 4096 {
            return Err(Error::Limit);
        }
        let words = bytes.div_ceil(size_of::<usize>());
        let mut storage = reserve::<usize>(words, 8192)?;
        storage.resize(words, 0);
        let mut result = Self {
            storage,
            initialized: false,
        };
        // SAFETY: Aligned, bounded storage remains stable through process creation.
        if unsafe { InitializeProcThreadAttributeList(result.raw(), 1, 0, &mut bytes) } == 0 {
            return Err(os("attribute list"));
        }
        result.initialized = true;
        // SAFETY: Exactly three new distinct inheritable handles; this array
        // remains live until the synchronous CreateProcessW call returns.
        if unsafe {
            UpdateProcThreadAttribute(
                result.raw(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                size_of::<[HANDLE; 3]>(),
                null_mut(),
                null(),
            )
        } == 0
        {
            return Err(os("explicit handle list"));
        }
        Ok(result)
    }
    fn raw(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.storage.as_mut_ptr().cast()
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: Exactly the successfully initialized owned attribute list.
            unsafe { DeleteProcThreadAttributeList(self.raw()) };
        }
    }
}
fn reserve<T>(count: usize, maximum_bytes: usize) -> Result<Vec<T>, Error> {
    if count
        .checked_mul(size_of::<T>())
        .is_none_or(|n| n > maximum_bytes)
    {
        return Err(Error::Limit);
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .map_err(|_| Error::Allocation)?;
    if output
        .capacity()
        .checked_mul(size_of::<T>())
        .is_none_or(|n| n > maximum_bytes)
    {
        return Err(Error::Limit);
    }
    Ok(output)
}
fn push(output: &mut Vec<u16>, unit: u16) -> Result<(), Error> {
    if output.len() >= MAX_COMMAND_UNITS - 1 {
        return Err(Error::InvalidArguments);
    }
    output.push(unit);
    Ok(())
}
fn quote(output: &mut Vec<u16>, argument: impl Iterator<Item = u16>) -> Result<(), Error> {
    push(output, 34)?;
    let mut slashes = 0_usize;
    for unit in argument {
        if unit == 0 {
            return Err(Error::InvalidArguments);
        }
        if unit == 92 {
            slashes += 1;
            if slashes >= MAX_COMMAND_UNITS {
                return Err(Error::InvalidArguments);
            }
            continue;
        }
        for _ in 0..if unit == 34 { slashes * 2 + 1 } else { slashes } {
            push(output, 92)?;
        }
        slashes = 0;
        push(output, unit)?;
    }
    for _ in 0..slashes * 2 {
        push(output, 92)?;
    }
    push(output, 34)
}
fn command(program: &Path, args: &[OsString]) -> Result<(Vec<u16>, Vec<u16>), Error> {
    if !program.is_absolute() || args.len() > MAX_ARGUMENTS {
        return Err(Error::InvalidArguments);
    }
    let mut app = reserve::<u16>(MAX_COMMAND_UNITS, MAX_COMMAND_UNITS * 2)?;
    let mut line = reserve::<u16>(MAX_COMMAND_UNITS, MAX_COMMAND_UNITS * 2)?;
    for unit in program.as_os_str().encode_wide() {
        if unit == 0 || unit == 34 {
            return Err(Error::InvalidArguments);
        }
        push(&mut app, unit)?;
    }
    if app.is_empty() {
        return Err(Error::InvalidArguments);
    }
    app.push(0);
    quote(&mut line, program.as_os_str().encode_wide())?;
    for argument in args {
        push(&mut line, 32)?;
        quote(&mut line, argument.encode_wide())?;
    }
    line.push(0);
    Ok((app, line))
}

struct Reader {
    handle: Handle,
    captured: usize,
    maximum: usize,
    eof: bool,
}
impl Reader {
    fn read(&mut self, output: &mut [u8], closing: bool) -> Result<usize, Error> {
        if self.eof {
            return Ok(0);
        }
        let mut available = 0;
        // SAFETY: Metadata-only zero-wait query on the unique new read endpoint.
        if unsafe {
            PeekNamedPipe(
                self.handle.raw(),
                null_mut(),
                0,
                null_mut(),
                &mut available,
                null_mut(),
            )
        } == 0
        {
            // SAFETY: Immediate last-error query for this exact peek.
            let code = unsafe { GetLastError() };
            if code == ERROR_BROKEN_PIPE {
                self.eof = true;
                return Ok(0);
            }
            return Err(Error::OperatingSystem {
                operation: "diagnostic peek",
                code,
            });
        }
        if available == 0 {
            return Ok(0);
        }
        let remaining = self.maximum - self.captured;
        if remaining == 0 && !closing {
            return Err(Error::OutputLimit);
        }
        let mut discard = [0; PUMP_BYTES];
        let target = if remaining == 0 {
            &mut discard[..]
        } else {
            output
        };
        let count = (available as usize)
            .min(target.len())
            .min(PUMP_BYTES)
            .min(if remaining == 0 {
                PUMP_BYTES
            } else {
                remaining
            });
        let mut actual = 0;
        // SAFETY: Sole reader, at least count bytes were already buffered, and
        // no other reader can remove them. Only bounded local storage is used;
        // the synchronous read does not wait for a pipe writer to produce more.
        if unsafe {
            ReadFile(
                self.handle.raw(),
                target.as_mut_ptr(),
                count as u32,
                &mut actual,
                null_mut(),
            )
        } == 0
        {
            return Err(os("diagnostic read"));
        }
        let actual = actual as usize;
        if actual == 0 || actual > count {
            return Err(Error::Closed);
        }
        if remaining == 0 {
            return Ok(0);
        } // shutdown drains excess without retaining it
        self.captured += actual;
        Ok(actual)
    }
}

struct Inner {
    process: Handle,
    job: Handle,
    initial_thread: Option<Handle>,
    assigned: bool,
    stdout: Reader,
    stderr: Reader,
    started: Instant,
    stopping: Option<Instant>,
    exit_code: Option<u32>,
    limits: Limits,
    _permit: Permit,
}
impl Inner {
    fn exit(&mut self) -> Result<Option<u32>, Error> {
        if let Some(code) = self.exit_code {
            return Ok(Some(code));
        }
        // SAFETY: Exact newly owned root process, always a zero-time query.
        match unsafe { WaitForSingleObject(self.process.raw(), 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut code = 0;
                // SAFETY: The process is actually signaled; even259 is final.
                if unsafe { GetExitCodeProcess(self.process.raw(), &mut code) } == 0 {
                    return Err(os("exit code"));
                }
                self.exit_code = Some(code);
                Ok(Some(code))
            }
            _ => Err(os("process status")),
        }
    }
    fn descendants(&self) -> Result<u32, Error> {
        if !self.assigned {
            return Ok(0);
        }
        let mut info = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        // SAFETY: Read-only exact-size accounting for this private job. No
        // process identifiers/paths are enumerated or exposed to the caller.
        if unsafe {
            QueryInformationJobObject(
                self.job.raw(),
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of_val(&info) as u32,
                null_mut(),
            )
        } == 0
        {
            return Err(os("job status"));
        }
        Ok(info.ActiveProcesses)
    }
    fn stop(&mut self) -> Result<(), Error> {
        self.stopping.get_or_insert_with(Instant::now);
        if self.exit()?.is_some() && self.descendants()? == 0 {
            return Ok(());
        }
        // SAFETY: Before assignment the exact suspended root is the only child;
        // afterward the private no-breakaway job owns every actual descendant.
        // A successful terminate request is never a completion observation.
        let stopped = unsafe {
            if self.assigned {
                TerminateJobObject(self.job.raw(), 1)
            } else {
                TerminateProcess(self.process.raw(), 1)
            }
        };
        if stopped == 0 {
            return Err(os("terminate owned session"));
        }
        Ok(())
    }
    fn poll(
        &mut self,
        stdout: &mut [u8],
        stderr: &mut [u8],
        closing: bool,
    ) -> Result<Status, Error> {
        let root = self.exit()?;
        let descendants = self.descendants()?;
        // A vanished CLI must not leave a worker behind. Stop only this owned
        // job; do not wait for, enumerate or manipulate unrelated processes.
        if root.is_some() && descendants != 0 && self.stopping.is_none() {
            self.stop()?;
        }
        if !closing && self.stopping.is_none() && self.started.elapsed() >= self.limits.run_timeout
        {
            self.stop()?;
            return Err(Error::Timeout);
        }
        let stopping = self.stopping.is_some();
        let stdout_len = self.stdout.read(stdout, closing || stopping)?;
        let stderr_len = self.stderr.read(stderr, closing || stopping)?;
        if let Some(code) = root
            && descendants == 0
            && self.stdout.eof
            && self.stderr.eof
        {
            return Ok(Status {
                stdout_len,
                stderr_len,
                state: State::Exited(code),
            });
        }
        if self
            .stopping
            .is_some_and(|start| start.elapsed() >= self.limits.shutdown_grace)
        {
            return Err(Error::RestartRequired);
        }
        Ok(Status {
            stdout_len,
            stderr_len,
            state: if stopping {
                State::Stopping
            } else {
                State::Running
            },
        })
    }
    fn quarantine(self) {
        ACTIVE.store(2, Ordering::Release);
        // The unique Permit prevents another retained session. Never drop an
        // unresolved owner even if an internal invariant was violated.
        if let Err(owner) = QUARANTINE.set(Mutex::new(Some(self))) {
            std::mem::forget(owner);
            std::process::abort();
        }
    }
}

/// One safe launcher owner. No method blocks waiting for exit, output or
/// termination. Dropping unresolved work retains one poisoned process/job owner
/// until real reaping or process teardown, and permanently refuses a new run.
pub struct SupervisedProcess {
    inner: Option<Inner>,
}
impl SupervisedProcess {
    pub fn spawn_cli(args: &[OsString], limits: Limits) -> Result<Self, Error> {
        limits.validate()?;
        if args.len() > MAX_ARGUMENTS {
            return Err(Error::InvalidArguments);
        }
        let executable = std::env::current_exe().map_err(|_| Error::SiblingUnavailable)?;
        let directory = executable
            .parent()
            .filter(|path| path.is_absolute())
            .ok_or(Error::SiblingUnavailable)?;
        let program = directory.join("pentax-cli.exe");
        Self::spawn(&program, args, limits)
    }
    fn spawn(program: &Path, args: &[OsString], limits: Limits) -> Result<Self, Error> {
        let permit = Permit::acquire()?;
        let (application, mut line) = command(program, args)?;
        let (stdout_read, stdout_write) = pipe()?;
        let (stderr_read, stderr_write) = pipe()?;
        let input = null_input()?;
        let handles = [input.raw(), stdout_write.raw(), stderr_write.raw()];
        let mut attributes = Attributes::new(&handles)?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = input.raw();
        startup.StartupInfo.hStdOutput = stdout_write.raw();
        startup.StartupInfo.hStdError = stderr_write.raw();
        startup.lpAttributeList = attributes.raw();
        // SAFETY: New unnamed noninheritable job, private to this launch owner.
        let job = Handle::take(unsafe { CreateJobObjectW(null(), null()) }, "private job")?;
        let mut policy = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        policy.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        // SAFETY: Only this new job receives kill-on-close. No breakaway, CPU,
        // memory, pagefile, scheduling, priority or affinity policy is changed.
        if unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&policy as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of_val(&policy) as u32,
            )
        } == 0
        {
            return Err(os("job policy"));
        }
        let mut child = PROCESS_INFORMATION::default();
        // SAFETY: Explicit absolute sibling application, bounded correctly quoted
        // mutable command line, zero CRT-reserved fields, exactly three private
        // inherited standard handles. Suspended root cannot escape admission.
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                null(),
                null(),
                1,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | CREATE_SUSPENDED,
                null(),
                null(),
                &startup.StartupInfo,
                &mut child,
            )
        } == 0
        {
            return Err(os("create sibling CLI"));
        }
        let process = Handle::take(child.hProcess, "root process handle")?;
        let thread = Handle::take(child.hThread, "root thread handle")?;
        // No spare parent writer can hide EOF after the actual child exits.
        drop(input);
        drop(stdout_write);
        drop(stderr_write);
        let mut inner = Inner {
            process,
            job,
            initial_thread: Some(thread),
            assigned: false,
            stdout: Reader {
                handle: stdout_read,
                captured: 0,
                maximum: limits.max_stdout_bytes,
                eof: false,
            },
            stderr: Reader {
                handle: stderr_read,
                captured: 0,
                maximum: limits.max_stderr_bytes,
                eof: false,
            },
            started: Instant::now(),
            stopping: None,
            exit_code: None,
            limits,
            _permit: permit,
        };
        // SAFETY: Exact newly created suspended root and this private owned job.
        if unsafe { AssignProcessToJobObject(inner.job.raw(), inner.process.raw()) } == 0 {
            let error = os("assign private job");
            return Self::failed_launch(inner, error);
        }
        inner.assigned = true;
        // SAFETY: The root's initial thread is retained and still suspended;
        // all ownership and the non-breakaway job are established before resume.
        if unsafe { ResumeThread(inner.initial_thread.as_ref().ok_or(Error::Closed)?.raw()) }
            == u32::MAX
        {
            let error = os("resume sibling CLI");
            return Self::failed_launch(inner, error);
        }
        inner.initial_thread = None;
        Ok(Self { inner: Some(inner) })
    }
    fn failed_launch(mut inner: Inner, error: Error) -> Result<Self, Error> {
        let stopped = inner.stop(); // checked request; failures never release ownership
        let mut out = [0; PUMP_BYTES];
        let mut err = [0; PUMP_BYTES];
        if inner
            .poll(&mut out, &mut err, true)
            .is_ok_and(|status| matches!(status.state, State::Exited(_)))
        {
            return Err(stopped.err().unwrap_or(error));
        }
        inner.quarantine();
        Err(Error::RestartRequired)
    }
    pub fn poll(&mut self, stdout: &mut [u8], stderr: &mut [u8]) -> Result<Status, Error> {
        self.poll_inner(stdout, stderr, false)
    }
    pub fn request_cancel(&mut self) -> Result<(), Error> {
        self.inner.as_mut().map_or(Ok(()), Inner::stop)
    }
    pub fn shutdown_poll(&mut self, stdout: &mut [u8], stderr: &mut [u8]) -> Result<Status, Error> {
        let stop = self.request_cancel();
        // Even a failed request must reach the actual exit/deadline checks.
        // Returning early here could hide an unresolved owner indefinitely.
        let status = self.poll_inner(stdout, stderr, true)?;
        if !matches!(status.state, State::Exited(_) | State::Closed) {
            stop?;
        }
        Ok(status)
    }
    fn poll_inner(
        &mut self,
        stdout: &mut [u8],
        stderr: &mut [u8],
        closing: bool,
    ) -> Result<Status, Error> {
        if stdout.is_empty() || stderr.is_empty() {
            return Err(Error::Limit);
        }
        let Some(inner) = self.inner.as_mut() else {
            return Ok(Status::CLOSED);
        };
        let result = inner.poll(stdout, stderr, closing);
        match result {
            Ok(status) => {
                if matches!(status.state, State::Exited(_)) {
                    self.inner = None;
                }
                Ok(status)
            }
            Err(Error::RestartRequired) => {
                if let Some(inner) = self.inner.take() {
                    inner.quarantine();
                }
                Err(Error::RestartRequired)
            }
            Err(error) => {
                let stop = inner.stop();
                if inner
                    .stopping
                    .is_some_and(|start| start.elapsed() >= inner.limits.shutdown_grace)
                {
                    if let Some(inner) = self.inner.take() {
                        inner.quarantine();
                    }
                    return Err(Error::RestartRequired);
                }
                stop?;
                Err(error)
            }
        }
    }
    /// Try-lock plus bounded zero-time observations only. A clean observation
    /// releases retained handles; quarantine still permanently blocks relaunch.
    pub fn poll_quarantined_cleanup() -> Result<State, Error> {
        let Some(owner) = QUARANTINE.get() else {
            return Ok(State::Closed);
        };
        let mut owner = match owner.try_lock() {
            Ok(owner) => owner,
            Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return Ok(State::Stopping),
        };
        let Some(inner) = owner.as_mut() else {
            return Ok(State::Closed);
        };
        let stop = inner.stop();
        let mut out = [0; PUMP_BYTES];
        let mut err = [0; PUMP_BYTES];
        let result = inner.poll(&mut out, &mut err, true)?;
        if matches!(result.state, State::Exited(_)) {
            *owner = None;
        } else {
            stop?;
        }
        Ok(result.state)
    }
}
impl Drop for SupervisedProcess {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            // Drop cannot return an OS error. Both request outcomes retain the
            // owner unless the independent complete exit/drain check succeeds.
            let _ = inner.stop();
            let mut out = [0; PUMP_BYTES];
            let mut err = [0; PUMP_BYTES];
            // A failed termination request is harmless only when actual
            // root/job/EOF observations independently prove full closure.
            if !inner
                .poll(&mut out, &mut err, true)
                .is_ok_and(|status| matches!(status.state, State::Exited(_)))
            {
                inner.quarantine();
            }
        }
    }
}

#[cfg(test)]
mod tests;
