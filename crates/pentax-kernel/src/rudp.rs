// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: process-owned source bootstrap receipt and handler.
// Adapted from pinned libRudp.cpp; see NOTICE and upstream provenance.

use crate::budget::Budget;
use crate::{CallArgs, Failure, Kernel, Service};

#[derive(Default)]
pub(crate) struct RudpState {
    initialized: bool,
    handler: (u64, u64),
}
impl RudpState {
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.work(1)?;
        match service {
            Service::RudpInit => {
                // Pinned source ignores both pool arguments and retains its
                // handler across repeated initialization. No pool is touched.
                self.initialized = true;
            }
            Service::RudpEnableInternalIOThread => {
                // This pinned export is an explicit no-op, including before
                // Init. It creates no thread and certifies no packet transport.
            }
            Service::RudpSetEventHandler => {
                // Source accepts null to clear the callback and retains arg
                // even then. Registration neither validates nor invokes code.
                self.handler = (args.0[0], args.0[1]);
            }
            _ => return Err(Failure::Unsupported("RUDP bootstrap service")),
        }
        Ok(0)
    }
}
impl Kernel {
    /// Source initialization receipt only; not evidence of an I/O thread,
    /// packet transport, connectivity or actual guest event delivery.
    #[must_use]
    pub fn rudp_bootstrap_initialized(&self) -> bool {
        self.rudp.initialized
    }
    /// Whether the bootstrap profile retains a non-null guest handler.
    /// Its registration never proves that guest code executed.
    #[must_use]
    pub fn rudp_event_handler_registered(&self) -> bool {
        self.rudp.handler.0 != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestGuest, kernel};

    fn call(
        owner: &mut Kernel,
        service: Service,
        values: [u64; 6],
        guest: &mut TestGuest,
    ) -> Result<u64, Failure> {
        owner.invoke_library(service, CallArgs(values), guest)
    }
    #[test]
    fn rudp_bootstrap_retains_source_receipts_handler_and_null_argument_without_touching_memory() {
        let mut owner = kernel(Default::default());
        let mut guest = TestGuest::new();
        guest.refuse = true;
        assert!(!owner.rudp_bootstrap_initialized());
        assert!(!owner.rudp_event_handler_registered());
        let before = owner.usage();
        assert_eq!(
            call(
                &mut owner,
                Service::RudpEnableInternalIOThread,
                [u64::MAX; 6],
                &mut guest
            ),
            Ok(0)
        );
        assert!(!owner.rudp_bootstrap_initialized());
        assert_eq!(
            call(
                &mut owner,
                Service::RudpSetEventHandler,
                [u64::MAX, 71, 0, 0, 0, 0],
                &mut guest
            ),
            Ok(0)
        );
        assert_eq!(owner.rudp.handler, (u64::MAX, 71));
        assert!(owner.rudp_event_handler_registered());
        for pool in [0, u64::MAX] {
            assert_eq!(
                call(
                    &mut owner,
                    Service::RudpInit,
                    [pool, u64::MAX, 0, 0, 0, 0],
                    &mut guest
                ),
                Ok(0)
            );
            assert!(owner.rudp_bootstrap_initialized());
            assert_eq!(owner.rudp.handler, (u64::MAX, 71));
        }
        assert_eq!(
            call(
                &mut owner,
                Service::RudpSetEventHandler,
                [0, u64::MAX, 0, 0, 0, 0],
                &mut guest
            ),
            Ok(0)
        );
        assert_eq!(owner.rudp.handler, (0, u64::MAX));
        assert!(!owner.rudp_event_handler_registered());
        assert_eq!(owner.usage().bytes_read, before.bytes_read);
        assert_eq!(owner.usage().bytes_written, before.bytes_written);
        assert_eq!(owner.usage().handles_live, before.handles_live);
        assert!(!kernel(Default::default()).rudp_bootstrap_initialized());
    }
    #[test]
    fn rudp_bootstrap_resource_refusal_precedes_state_publication() {
        let mut owner = kernel(crate::Limits {
            max_work: 0,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        for service in [Service::RudpInit, Service::RudpSetEventHandler] {
            assert!(matches!(
                call(&mut owner, service, [u64::MAX; 6], &mut guest),
                Err(Failure::Limit(_))
            ));
            assert!(!owner.rudp_bootstrap_initialized());
            assert_eq!(owner.rudp.handler, (0, 0));
        }
    }
}
