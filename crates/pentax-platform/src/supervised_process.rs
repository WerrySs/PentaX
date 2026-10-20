// SPDX-License-Identifier: GPL-2.0-only
//! Bounded sibling-CLI supervision for the desktop launcher.
//!
//! The fixed sibling executable is launched suspended into a private Windows
//! kill-on-close job before any child instruction can run. Standard streams
//! use only new private handles; no shell, ambient PATH lookup, inherited console
//! handles, frame endpoint or raw-handle adoption API is supplied.
//!
//! Polling and teardown never wait for a child or pipe peer. If termination
//! cannot actually be observed within the finite grace period, one poisoned
//! owner retains the exact process/job/handles and new launches are refused.
//! This is process ownership and failure containment, not a security sandbox.

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
use std::ffi::OsString;
use std::time::Duration;

const OUTPUT_CAP: usize = 65_536;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const PUMP_BYTES: usize = 4096;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const MAX_ARGUMENTS: usize = 256;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const MAX_COMMAND_UNITS: usize = 32_767;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub run_timeout: Duration,
    pub shutdown_grace: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            run_timeout: Duration::from_secs(3600),
            shutdown_grace: Duration::from_secs(2),
            max_stdout_bytes: OUTPUT_CAP,
            max_stderr_bytes: OUTPUT_CAP,
        }
    }
}
impl Limits {
    pub fn validate(self) -> Result<(), Error> {
        if self.run_timeout.is_zero()
            || self.run_timeout > Duration::from_secs(86_400)
            || self.shutdown_grace.is_zero()
            || self.shutdown_grace > Duration::from_secs(5)
            || self.max_stdout_bytes == 0
            || self.max_stdout_bytes > OUTPUT_CAP
            || self.max_stderr_bytes == 0
            || self.max_stderr_bytes > OUTPUT_CAP
        {
            return Err(Error::Limit);
        }
        Ok(())
    }
}

/// Exit is final only after the root is signaled, the job has no active
/// descendants, and both diagnostic streams have been completely drained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    Stopping,
    Exited(u32),
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub stdout_len: usize,
    pub stderr_len: usize,
    pub state: State,
}
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
impl Status {
    const CLOSED: Self = Self {
        stdout_len: 0,
        stderr_len: 0,
        state: State::Closed,
    };
}

/// Closed operation labels never contain executable paths or argument text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    UnsupportedHost,
    Limit,
    Allocation,
    InvalidArguments,
    SiblingUnavailable,
    AlreadyRunning,
    OutputLimit,
    Timeout,
    RestartRequired,
    Closed,
    OperatingSystem { operation: &'static str, code: u32 },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OperatingSystem { operation, code } => {
                write!(f, "supervised process {operation} failed ({code})")
            }
            other => write!(f, "supervised process: {other:?}"),
        }
    }
}
impl std::error::Error for Error {}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub use windows::SupervisedProcess;

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub struct SupervisedProcess;
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
impl SupervisedProcess {
    pub fn spawn_cli(_: &[OsString], limits: Limits) -> Result<Self, Error> {
        limits.validate()?;
        Err(Error::UnsupportedHost)
    }
    pub fn poll(&mut self, _: &mut [u8], _: &mut [u8]) -> Result<Status, Error> {
        Err(Error::UnsupportedHost)
    }
    pub fn request_cancel(&mut self) -> Result<(), Error> {
        Err(Error::UnsupportedHost)
    }
    pub fn shutdown_poll(&mut self, _: &mut [u8], _: &mut [u8]) -> Result<Status, Error> {
        Err(Error::UnsupportedHost)
    }
    pub fn poll_quarantined_cleanup() -> Result<State, Error> {
        Err(Error::UnsupportedHost)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supervision_requires_finite_deadlines_and_independent_bounded_streams() {
        let limits = Limits::default();
        limits.validate().unwrap();
        assert_eq!(
            Limits {
                run_timeout: Duration::ZERO,
                ..limits
            }
            .validate(),
            Err(Error::Limit)
        );
        assert_eq!(
            Limits {
                run_timeout: Duration::from_secs(86_401),
                ..limits
            }
            .validate(),
            Err(Error::Limit)
        );
        assert_eq!(
            Limits {
                shutdown_grace: Duration::from_secs(6),
                ..limits
            }
            .validate(),
            Err(Error::Limit)
        );
        assert_eq!(
            Limits {
                max_stdout_bytes: 0,
                ..limits
            }
            .validate(),
            Err(Error::Limit)
        );
        assert_eq!(
            Limits {
                max_stderr_bytes: OUTPUT_CAP + 1,
                ..limits
            }
            .validate(),
            Err(Error::Limit)
        );
        assert_eq!(
            Limits {
                max_stdout_bytes: 7,
                max_stderr_bytes: 19,
                ..limits
            }
            .validate(),
            Ok(())
        );
    }
}
