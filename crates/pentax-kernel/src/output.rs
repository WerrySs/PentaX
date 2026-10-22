// SPDX-License-Identifier: GPL-2.0-only

//! Bounded owned standard-stream output, independent of the worker protocol.

use crate::{CallArgs, Errno, Failure, GuestAccess, Kernel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputStream {
    /// The pinned LibKernel::write wrapper also accepts descriptor zero.
    StdinCompatibility,
    Stdout,
    Stderr,
}
impl OutputStream {
    fn index(self) -> usize {
        match self {
            Self::StdinCompatibility => 0,
            Self::Stdout => 1,
            Self::Stderr => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputUsage {
    pub accepted_bytes: u64,
    pub retained_bytes: usize,
}

#[derive(Default)]
pub(crate) struct OutputState {
    buffers: [Vec<u8>; 3],
    usage: OutputUsage,
}

impl Kernel {
    pub fn output_usage(&self) -> OutputUsage {
        self.output.usage
    }

    /// Borrow a bounded suffix for application-owned diagnostics, without
    /// consuming output, changing its budget or publishing it to host streams.
    pub fn output_tail(&self, stream: OutputStream, maximum: usize) -> &[u8] {
        let bytes = &self.output.buffers[stream.index()];
        &bytes[bytes.len().saturating_sub(maximum)..]
    }

    /// Transfer accepted bytes to the owning application. No implicit logging,
    /// decoding, host stdout/stderr write or guest-file mutation occurs here.
    pub fn take_output(&mut self, stream: OutputStream) -> Vec<u8> {
        let bytes = std::mem::take(&mut self.output.buffers[stream.index()]);
        self.output.usage.retained_bytes -= bytes.len();
        bytes
    }

    pub(crate) fn write_output(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
        compatibility: bool,
    ) -> Result<u64, Failure> {
        let [descriptor, address, length, ..] = args.0;
        let descriptor = descriptor as u32 as i32;
        // KernelWrite checks null before descriptor validity, even at length0.
        // The POSIX wrapper's standard-descriptor branch accepts empty writes.
        if address == 0 && (!compatibility || length != 0) {
            return Err(Errno::Fault.into());
        }
        if descriptor >= 3 {
            return self.files.write_save_file(
                crate::Service::Write,
                args.0,
                guest,
                &mut self.budget,
            );
        }
        if !(0..=2).contains(&descriptor) || descriptor == 0 && !compatibility {
            // Saved-directory descriptors use their separate access checks;
            // this branch retains the original standard-stream rejection.
            return Err(Errno::BadFd.into());
        }
        let count = usize::try_from(length).map_err(|_| Failure::Limit("output transfer"))?;
        if count > self.budget.limits.max_io_bytes || count > i32::MAX as usize {
            return Err(Failure::Limit("output transfer"));
        }
        let retained = self
            .output
            .usage
            .retained_bytes
            .checked_add(count)
            .filter(|size| *size <= self.budget.limits.max_output_bytes)
            .ok_or(Failure::Limit("output bytes"))?;
        let accepted = self
            .output
            .usage
            .accepted_bytes
            .checked_add(length)
            .ok_or(Failure::Limit("output bytes"))?;
        if count == 0 {
            return Ok(0);
        }
        let mut bytes = self.budget.buffer(count)?;
        self.budget.read(guest, address, &mut bytes)?;
        let destination = &mut self.output.buffers[descriptor as usize];
        destination
            .try_reserve_exact(count)
            .map_err(|_| Failure::Allocation)?;
        // No fallible operation follows publication. Binary bytes, including
        // embedded NULs, remain intact until explicitly drained by the owner.
        destination.extend_from_slice(&bytes);
        self.output.usage = OutputUsage {
            accepted_bytes: accepted,
            retained_bytes: retained,
        };
        Ok(length)
    }
}
