// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded guest-owned initial-thread errno and outputs.
// Adapted from pinned libKernel.cpp and errno.cpp/h; see NOTICE.

use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Kernel, Service};

/// POSIX_PTHREAD_CALL is independent of the file wrappers' -1/errno domain.
/// This conversion never reads, allocates or writes a guest errno cell.
pub(crate) fn pthread_result(result: Result<u64, Failure>) -> Result<u64, Failure> {
    match result {
        Err(Failure::Errno(errno)) => {
            let number = errno as u32;
            Ok(u64::from(if (1..=101).contains(&number) {
                number
            } else {
                1062
            }))
        }
        result => result,
    }
}

impl Kernel {
    /// Publish only the pinned mspace errors, preserving the active thread's
    /// previous cell and backing ownership on every recoverable refusal.
    /// This internal delegate consumes no additional service attempt.
    pub(crate) fn publish_mspace_errno(
        &mut self,
        number: u32,
        guest: &mut impl GuestAccess,
    ) -> Result<(), Failure> {
        if !matches!(number, 12 | 22) {
            return Err(Failure::Unsupported("mspace errno number"));
        }
        self.budget.work(12)?;
        let bytes = number.to_le_bytes();
        if let Some(address) = self.threads.active().errno {
            guest.preflight_posix_errno(Some(address))?;
            self.budget.write(
                guest,
                &[GuestWrite {
                    address,
                    bytes: &bytes,
                }],
            )?;
        } else {
            guest.preflight_posix_errno(None)?;
            // Initialize the actual number in the sole MapZeroed transaction.
            // There is no separately published zero cell or second write.
            let address = self.publish_runtime_data(4, 0, guest, |_, output| {
                output.copy_from_slice(&bytes);
                Ok(crate::RuntimeDataOutputs::new())
            })?;
            self.threads.active_mut().errno = Some(address);
        }
        Ok(())
    }

    pub(crate) fn errno_address(&mut self, guest: &mut impl GuestAccess) -> Result<u64, Failure> {
        self.budget.work(12)?;
        guest.preflight_posix_errno(None)?;
        let address = if let Some(address) = self.threads.active().errno {
            address
        } else {
            let address = self.memory.runtime_word(0, guest, &mut self.budget)?;
            self.threads.active_mut().errno = Some(address);
            address
        };
        guest.preflight_posix_errno(Some(address))?;
        Ok(address)
    }

    pub(crate) fn invoke_posix(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.budget.work(12)?;
        guest.preflight_posix_errno(None)?;
        use Service::*;
        // These are direct internal delegates, never a second Kernel::invoke:
        // one wrapper call consumes exactly one service attempt.
        let (result, wide) = match service {
            PosixOpen => (
                self.files.invoke(Open, args, guest, &mut self.budget),
                false,
            ),
            PosixClose => {
                let descriptor = args.0[0] as u32 as i32;
                (
                    if descriptor < 0 {
                        Err(Errno::BadFd.into())
                    } else if descriptor <= 2 {
                        Ok(0)
                    } else {
                        self.files.invoke(Close, args, guest, &mut self.budget)
                    },
                    false,
                )
            }
            PosixRead => {
                let descriptor = args.0[0] as u32 as i32;
                (
                    if args.0[1] == 0 {
                        Err(Errno::Fault.into())
                    } else if descriptor < 0 || descriptor == 1 || descriptor == 2 {
                        Err(Errno::BadFd.into())
                    } else if descriptor == 0 {
                        if args.0[2] == 0 {
                            Ok(0)
                        } else {
                            Err(Failure::Unsupported("POSIX standard input"))
                        }
                    } else {
                        self.files.invoke(Read, args, guest, &mut self.budget)
                    },
                    true,
                )
            }
            PosixWrite => {
                let descriptor = args.0[0] as u32 as i32;
                (
                    if descriptor < 0 {
                        Err(Errno::BadFd.into())
                    } else {
                        self.write_output(args, guest, descriptor <= 2)
                    },
                    true,
                )
            }
            PosixPwrite => (
                self.files.invoke(Pwrite, args, guest, &mut self.budget),
                true,
            ),
            PosixFtruncate => (
                self.files.invoke(Ftruncate, args, guest, &mut self.budget),
                false,
            ),
            PosixMkdir => (
                self.files.invoke(Mkdir, args, guest, &mut self.budget),
                false,
            ),
            PosixPread => (
                self.files.invoke(Pread, args, guest, &mut self.budget),
                true,
            ),
            PosixLseek => (
                self.files.invoke(Lseek, args, guest, &mut self.budget),
                true,
            ),
            // The source fstat wrapper explicitly returns int64_t although its
            // kernel delegate returns int. Its -1 must therefore sign-extend.
            PosixFstat => (
                self.files.invoke(Fstat, args, guest, &mut self.budget),
                true,
            ),
            PosixStat => (
                self.files.invoke(Stat, args, guest, &mut self.budget),
                false,
            ),
            PosixClockGettime => (
                self.time
                    .invoke(ClockGettime, args, guest, &mut self.budget),
                false,
            ),
            PosixClockGetres => (
                self.time.invoke(ClockGetres, args, guest, &mut self.budget),
                false,
            ),
            PosixNanosleep => (
                self.time.invoke(Nanosleep, args, guest, &mut self.budget),
                false,
            ),
            AprResolveFilepathsToIdsAndFileSizes
            | AprResolveFilepathsToIds
            | AprGetFileStat
            | AprGetFileSize
            | AprResolveFilepathsWithPrefixToIds
            | AprResolveFilepathsWithPrefixToIdsAndFileSizes
            | AprResolveFilepathsToIdsForEach
            | AprResolveFilepathsToIdsAndFileSizesForEach
            | AprResolveFilepathsWithPrefixToIdsForEach
            | AprResolveFilepathsWithPrefixToIdsAndFileSizesForEach => (
                self.files
                    .invoke_apr_import(service, args, guest, &mut self.budget),
                false,
            ),
            AprSubmitCommandBufferAndGetResult
            | AprSubmitCommandBuffer
            | AprSubmitCommandBufferAndGetId
            | AprWaitCommandBuffer => (
                self.ampr
                    .submit(service, args, &self.files, guest, &mut self.budget),
                false,
            ),
            _ => return Err(Failure::Unsupported("POSIX wrapper profile")),
        };
        match result {
            Err(Failure::Errno(errno)) => {
                let address = self.errno_address(guest)?;
                // Pinned KernelToPosix admits kernel errno1..101, otherwise1062.
                // Only a supported source errno reaches this conversion.
                let number = errno as u32;
                let number = if (1..=101).contains(&number) {
                    number
                } else {
                    1062
                };
                self.budget.write(
                    guest,
                    &[GuestWrite {
                        address,
                        bytes: &number.to_le_bytes(),
                    }],
                )?;
                Ok(if wide { u64::MAX } else { u64::from(u32::MAX) })
            }
            result => result, // Success preserves errno; typed failures stay failures.
        }
    }
}

#[cfg(test)]
#[path = "posix/mspace_errno_tests.rs"]
mod mspace_errno_tests;
