// SPDX-License-Identifier: GPL-2.0-only

//! Final-only queue snapshot. Pending waits are not readiness or deadlock proof.

use super::Scheduling;

#[derive(Debug, Default)]
pub(in crate::native_session) struct Observations {
    threads: u64,
    waiting: u64,
    returned: u64,
    main_state: Option<&'static str>,
    main_wait: Option<&'static str>,
    active_role: Option<&'static str>,
}

impl Observations {
    pub(in crate::native_session) fn protocol_fields(&self) -> String {
        format!(
            "\tscheduling_threads={}\tscheduling_pending_waits={}\tscheduling_returned={}\tscheduling_main_state={}\tscheduling_main_wait={}\tscheduling_active_role={}",
            self.threads,
            self.waiting,
            self.returned,
            self.main_state.unwrap_or("unobserved"),
            self.main_wait.unwrap_or("none"),
            self.active_role.unwrap_or("unobserved"),
        )
    }
}

impl Scheduling<'_, '_> {
    /// Visit the bounded queue once after an epoch stops, outside native entry.
    /// Do not call readiness checks: those can advance retained wait state.
    pub(in crate::native_session) fn observations(&self) -> Observations {
        let mut snapshot = Observations {
            threads: self.threads.len() as u64,
            active_role: Some(if self.is_main() { "main" } else { "child" }),
            ..Default::default()
        };
        for thread in &self.threads {
            snapshot.waiting += u64::from(thread.waiting.is_some());
            snapshot.returned += u64::from(thread.returned);
        }
        if let Some(main) = self.threads.first() {
            snapshot.main_state = Some(if main.returned {
                "returned"
            } else if main.waiting.is_some() {
                "pending-wait"
            } else {
                "unparked"
            });
            // The label comes exclusively from the admitted public catalogue,
            // never a guest name, argument, identifier or decoded source byte.
            snapshot.main_wait = main.waiting_service;
        }
        snapshot
    }
}
