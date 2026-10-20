// SPDX-License-Identifier: GPL-2.0-only
// Adapted from authorized KytyPS5-Performance@209dffe libs.cpp:156..283.
// Git blob c366dadb7c02e94ed3f67c1c951dec53bd294694; see NOTICE/provenance.
//! Silent content-service initialization. No export/search/delete backend.

use crate::{CallArgs, Failure, Service, budget::Budget};

#[derive(Default)]
pub(crate) struct ContentState {
    initialized: [bool; 3],
}

impl ContentState {
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let (index, invalid) = match service {
            Service::ContentExportInit2 => (0, 0x809d_3016),
            Service::ContentSearchInit => (1, 0x809d_1003),
            Service::ContentDeleteInitialize => (2, 0x809d_5001),
            _ => return Err(Failure::Unsupported("content initialization service")),
        };
        budget.work(1)?;
        if args.0[0] == 0 {
            return Ok(invalid);
        }
        // Verified LOGF macro does not evaluate its arguments when silent.
        // Allocator pointers and sizes are logging-only fields in this source:
        // no guest read, callback invocation, allocation or host action occurs.
        self.initialized[index] = true;
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestGuest, kernel};
    use crate::{GuestAccess, GuestError, GuestWrite, Limits, MemoryEffect};

    struct NoAccess;
    impl GuestAccess for NoAccess {
        fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
            panic!("logging-only content read");
        }
        fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
            panic!("content output or mapping");
        }
    }

    #[test]
    fn content_null_errors_and_silent_opaque_params_keep_three_independent_flags() {
        let mut owner = kernel(Limits::default());
        let census = owner.handle_usage();
        for (index, service, invalid) in [
            (0, Service::ContentExportInit2, 0x809d3016),
            (1, Service::ContentSearchInit, 0x809d1003),
            (2, Service::ContentDeleteInitialize, 0x809d5001),
        ] {
            let prior = owner.content.initialized;
            assert_eq!(
                owner.invoke_library(service, CallArgs([0; 6]), &mut NoAccess),
                Ok(invalid)
            );
            assert_eq!(owner.content.initialized, prior);
            for pointer in [u64::MAX, 1, 0x8000_0000_0000_0000] {
                assert_eq!(
                    owner.invoke_library(service, CallArgs([pointer; 6]), &mut NoAccess),
                    Ok(0)
                );
                let mut expected = prior;
                expected[index] = true;
                assert_eq!(owner.content.initialized, expected);
                assert_eq!(owner.handle_usage(), census);
            }
            assert_eq!(
                owner.invoke_library(service, CallArgs([0; 6]), &mut NoAccess),
                Ok(invalid)
            );
            assert!(owner.content.initialized[index]);
        }
        assert_eq!(owner.content.initialized, [true; 3]);
        assert_eq!(
            (
                owner.usage().bytes_read,
                owner.usage().bytes_written,
                owner.usage().mapped_bytes
            ),
            (0, 0, 0)
        );
    }

    #[test]
    fn content_limits_precede_state_mutation_and_library_calls_preserve_errno() {
        for service in [
            Service::ContentExportInit2,
            Service::ContentSearchInit,
            Service::ContentDeleteInitialize,
        ] {
            let mut limited = kernel(Limits {
                max_work: 1,
                ..Limits::default()
            });
            assert_eq!(
                limited.invoke_library(service, CallArgs([u64::MAX; 6]), &mut NoAccess),
                Err(Failure::Limit("work"))
            );
            assert_eq!(limited.content.initialized, [false; 3]);
            let mut attempts = kernel(Limits {
                max_attempts: 1,
                ..Limits::default()
            });
            assert!(
                attempts
                    .invoke_library(service, CallArgs([0; 6]), &mut NoAccess)
                    .is_ok()
            );
            assert_eq!(
                attempts.invoke_library(service, CallArgs([1; 6]), &mut NoAccess),
                Err(Failure::Limit("attempts"))
            );
            assert_eq!(attempts.content.initialized, [false; 3]);
        }
        let mut owner = kernel(Limits::default());
        let mut guest = TestGuest::new();
        let errno = owner
            .invoke_library(Service::ErrnoLocation, CallArgs([0; 6]), &mut guest)
            .unwrap();
        guest
            .apply(
                MemoryEffect::None,
                &[GuestWrite {
                    address: errno,
                    bytes: &37_u32.to_le_bytes(),
                }],
            )
            .unwrap();
        let usage = owner.usage();
        for service in [
            Service::ContentExportInit2,
            Service::ContentSearchInit,
            Service::ContentDeleteInitialize,
        ] {
            for pointer in [0, u64::MAX] {
                let _ = owner
                    .invoke_library(service, CallArgs([pointer; 6]), &mut guest)
                    .unwrap();
                assert_eq!(guest.get(errno, 4), 37_u32.to_le_bytes());
            }
        }
        assert_eq!(
            (
                owner.usage().bytes_read,
                owner.usage().bytes_written,
                owner.usage().handles_live
            ),
            (usage.bytes_read, usage.bytes_written, usage.handles_live)
        );
    }
}
