// SPDX-License-Identifier: GPL-2.0-only

//! Fixed aggregate observations; no guest addresses, identifiers or source data.
//! Stage times sample one block in 4093, while service time covers every call.

use super::{NativeSessionError, charge};
use std::{fmt::Write, time::Instant};

mod block_samples;

#[derive(Debug, Default)]
pub(super) struct StartupProfile {
    pub samples: u64,
    pub window_ns: u64,
    pub plan_ns: u64,
    pub dispatch_ns: u64,
    pub service_ns: u64,
    pub native_body_blocks: u64,
    pub exit_only_blocks: u64,
    pub services: ServiceProfile,
    builtin_routes: [[BuiltinRouteMetric; 8]; 3],
    builtin_route_overflowed: bool,
    blocks: block_samples::BlockSamples,
}

/// Exact counts for the first 64 public kernel labels, plus an explicit
/// overflow bucket. Other admitted providers have their own aggregate bucket.
/// Count and elapsed-time top fours are selected only when emitting a bounded record.
#[derive(Debug)]
pub(super) struct ServiceProfile {
    counts: [Option<(&'static str, u64, u64, u64)>; 64],
    other: u64,
    overflow: u64,
    other_ns: u64,
    overflow_ns: u64,
    unattributed_ns: u64,
    last: Option<&'static str>,
}

impl Default for ServiceProfile {
    fn default() -> Self {
        Self {
            counts: [None; 64],
            other: 0,
            overflow: 0,
            other_ns: 0,
            overflow_ns: 0,
            unattributed_ns: 0,
            last: None,
        }
    }
}

impl ServiceProfile {
    pub fn record(
        &mut self,
        name: Option<&'static str>,
        nonzero_return: bool,
        elapsed_ns: u64,
        work: &mut u64,
        limit: u64,
    ) -> Result<(), NativeSessionError> {
        self.last = name;
        let Some(name) = name else {
            self.other = self.other.saturating_add(1);
            self.other_ns = self.other_ns.saturating_add(elapsed_ns);
            return Ok(());
        };
        for slot in &mut self.counts {
            if let Err(error) = charge(
                work,
                8 + slot.map_or(0, |(label, _, _, _)| label.len().min(name.len()) as u64),
                limit,
                "admission work",
            ) {
                // The action was measured, but this observation was not admitted.
                self.unattributed_ns = self.unattributed_ns.saturating_add(elapsed_ns);
                return Err(error);
            }
            match slot {
                Some((label, count, nonzero, nanos)) if *label == name => {
                    *count = count.saturating_add(1);
                    *nonzero = nonzero.saturating_add(u64::from(nonzero_return));
                    *nanos = nanos.saturating_add(elapsed_ns);
                    return Ok(());
                }
                None => {
                    *slot = Some((name, 1, u64::from(nonzero_return), elapsed_ns));
                    return Ok(());
                }
                _ => {}
            }
        }
        self.overflow = self.overflow.saturating_add(1);
        self.overflow_ns = self.overflow_ns.saturating_add(elapsed_ns);
        Ok(())
    }

    /// No successful public label is available for a failed action or a libc
    /// pre-effect fallback. Keep its measured time without changing old counts.
    pub fn record_unattributed(&mut self, elapsed_ns: u64) {
        self.unattributed_ns = self.unattributed_ns.saturating_add(elapsed_ns);
    }

    fn protocol_fields(&self) -> String {
        let mut sorted = self.counts;
        sorted.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.map_or(0, |(_, n, _, _)| n)));
        let top = sorted
            .iter()
            .flatten()
            .take(4)
            .map(|(name, count, _, _)| format!("{name}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        // Nonzero observations need their own ranking: the most frequent calls
        // may all return zero and otherwise hide rarer nonzero outcomes. These
        // are raw return observations, not an error classification (IDs and
        // other successful values can be nonzero).
        sorted.sort_unstable_by_key(|entry| {
            std::cmp::Reverse(entry.map_or((0, 0), |(_, calls, nonzero, _)| (nonzero, calls)))
        });
        let nonzero = sorted
            .iter()
            .flatten()
            .filter(|(_, _, count, _)| *count != 0)
            .take(4)
            .map(|(name, _, count, _)| format!("{name}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        let mut fields = format!(
            "service_top={top}\tservice_nonzero_top={nonzero}\tservice_last={}\tservice_other={}\tservice_overflow={}",
            self.last.unwrap_or("other-provider"),
            self.other,
            self.overflow
        );
        // Reuse the same fixed stack table. Formatting directly into the existing
        // output string needs no third Vec or per-label temporary allocation.
        sorted.sort_unstable_by_key(|entry| {
            std::cmp::Reverse(entry.map_or(0, |(_, _, _, nanos)| nanos))
        });
        fields.push_str("\tservice_time_top=");
        let mut separator = "";
        for (name, _, _, nanos) in sorted.iter().flatten().filter(|row| row.3 != 0).take(4) {
            write!(fields, "{separator}{name}:{nanos}").expect("String formatting is infallible");
            separator = ",";
        }
        if separator.is_empty() {
            fields.push_str("none");
        }
        write!(
            fields,
            "\tservice_other_ns={}\tservice_overflow_ns={}\tservice_unattributed_ns={}",
            self.other_ns, self.overflow_ns, self.unattributed_ns
        )
        .expect("String formatting is infallible");
        fields
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct BuiltinRouteMetric {
    count: u64,
    nanoseconds: u64,
}

#[derive(Clone, Copy)]
pub(super) enum BuiltinRouteOutcome {
    Substituted,
    Fallback,
    /// The route may fail after helper completion during its checked RET.
    Refused,
}

impl StartupProfile {
    /// Optional fixed aggregates reuse the already measured route wall time.
    /// None means diagnostics disabled; no guest budget or outcome is changed.
    pub fn record_builtin_route(
        &mut self,
        operation: usize,
        requested_bytes: Option<u64>,
        nanoseconds: u64,
        outcome: BuiltinRouteOutcome,
    ) {
        let Some(bytes) = requested_bytes else { return };
        if self.builtin_route_overflowed {
            return;
        }
        let row = match outcome {
            BuiltinRouteOutcome::Substituted => match bytes {
                0 => 0,
                1..=64 => 1,
                65..=4096 => 2,
                4097..=65536 => 3,
                65537..=1048576 => 4,
                _ => 5,
            },
            BuiltinRouteOutcome::Fallback => 6,
            BuiltinRouteOutcome::Refused => 7,
        };
        let Some(cells) = self.builtin_routes.get_mut(operation) else {
            self.builtin_route_overflowed = true;
            return;
        };
        let cell = &mut cells[row];
        let Some(count) = cell.count.checked_add(1) else {
            self.builtin_route_overflowed = true;
            return;
        };
        let Some(elapsed) = cell.nanoseconds.checked_add(nanoseconds) else {
            self.builtin_route_overflowed = true;
            return;
        };
        *cell = BuiltinRouteMetric {
            count,
            nanoseconds: elapsed,
        };
    }

    /// Terminal fields only; progress output and original totals stay intact.
    pub fn append_builtin_route_fields(&self, output: &mut String) {
        const OPERATIONS: [&str; 3] = ["memcpy", "memmove", "memset"];
        const ROWS: [&str; 8] = [
            "zero",
            "1_64",
            "65_4096",
            "4097_65536",
            "65537_1048576",
            "over_1048576",
            "fallback",
            "refused",
        ];
        for (name, cells) in OPERATIONS.into_iter().zip(&self.builtin_routes) {
            write!(output, "\tlibc_builtin_route_{name}=")
                .expect("String formatting is infallible");
            let mut separator = "";
            for (label, cell) in ROWS.into_iter().zip(cells) {
                write!(
                    output,
                    "{separator}{label}:{}:{}",
                    cell.count, cell.nanoseconds
                )
                .expect("String formatting is infallible");
                separator = ",";
            }
        }
        write!(
            output,
            "\tlibc_builtin_route_overflowed={}",
            self.builtin_route_overflowed
        )
        .expect("String formatting is infallible");
    }
    /// Explicit local output capability only; never part of protocol fields.
    pub fn enable_private_samples(&mut self) {
        self.blocks.enable_private_samples();
    }

    pub fn capture_private_samples(&mut self, source: &pentax_linker::SymbolicLinkPlan<'_>) {
        self.blocks.capture_private_samples(source);
    }

    pub fn private_samples_bytes(&self) -> Option<&[u8]> {
        self.blocks.private_samples_bytes()
    }

    pub fn begin_sample_epoch(&mut self, epoch: pentax_linker::SourceIdentity) {
        self.blocks.begin_epoch(epoch);
    }

    pub fn prepare_block_sample(
        &mut self,
        epoch: pentax_linker::SourceIdentity,
        block: &pentax_codegen::MaterializedBlockPlan<'_, '_, '_>,
        maximum_records: u64,
        is_main: bool,
    ) -> Option<block_samples::PendingSample> {
        self.blocks.prepare(epoch, block, maximum_records, is_main)
    }

    pub fn complete_block_sample(&mut self, sample: block_samples::PendingSample) {
        self.blocks.complete(sample);
    }

    pub fn final_sample_fields(&self) -> String {
        self.blocks.protocol_fields()
    }

    pub fn sampled(enabled: bool, blocks: u64) -> bool {
        enabled && blocks.is_multiple_of(4093)
    }

    pub fn checkpoint(stamp: &mut Option<Instant>, total: &mut u64) {
        if let Some(previous) = stamp {
            let now = Instant::now();
            Self::accumulate(total, now.duration_since(*previous).as_nanos());
            *previous = now;
        }
    }

    pub fn service_finished(&mut self, start: Option<Instant>) -> u64 {
        start.map_or(0, |start| self.service_elapsed(start.elapsed().as_nanos()))
    }

    fn service_elapsed(&mut self, nanos: u128) -> u64 {
        let nanos = u64::try_from(nanos).unwrap_or(u64::MAX);
        self.service_ns = self.service_ns.saturating_add(nanos);
        nanos
    }

    fn accumulate(total: &mut u64, nanos: u128) {
        *total = total.saturating_add(u64::try_from(nanos).unwrap_or(u64::MAX));
    }

    pub fn protocol_fields(&self) -> String {
        format!(
            "profile_samples={}\tsample_window_ns={}\tsample_plan_ns={}\tsample_dispatch_ns={}\tservice_ns={}\tnative_body_blocks={}\texit_only_blocks={}\t{}",
            self.samples,
            self.window_ns,
            self.plan_ns,
            self.dispatch_ns,
            self.service_ns,
            self.native_body_blocks,
            self.exit_only_blocks,
            self.services.protocol_fields(),
        )
    }
}

#[cfg(test)]
mod service_profile_tests {
    use super::ServiceProfile;

    #[test]
    fn infrequent_nonzero_returns_survive_hot_zero_calls_without_error_inference() {
        let mut profile = ServiceProfile::default();
        let mut work = 0;
        for label in [
            "PthreadMutexLock",
            "PthreadMutexUnlock",
            "pthread_mutex_lock",
            "pthread_mutex_unlock",
        ] {
            for _ in 0..16 {
                profile
                    .record(Some(label), false, 0, &mut work, u64::MAX)
                    .unwrap();
            }
        }
        // Both a successful nonzero identity and another nonzero result remain
        // visible. The observer deliberately assigns neither an errno meaning.
        profile
            .record(Some("PthreadGetthreadid"), true, 0, &mut work, u64::MAX)
            .unwrap();
        profile
            .record(Some("Open"), true, 0, &mut work, u64::MAX)
            .unwrap();
        profile
            .record(Some("Open"), true, 0, &mut work, u64::MAX)
            .unwrap();
        let fields = profile.protocol_fields();
        let top = fields
            .split('\t')
            .find(|part| part.starts_with("service_top="))
            .unwrap();
        assert!(!top.contains("Open") && !top.contains("PthreadGetthreadid"));
        let nonzero = fields
            .split('\t')
            .find(|part| part.starts_with("service_nonzero_top="))
            .unwrap();
        assert_eq!(nonzero, "service_nonzero_top=Open:2,PthreadGetthreadid:1");
        assert!(fields.contains("service_last=Open\tservice_other=0\tservice_overflow=0\t"));
        assert_eq!(
            ServiceProfile::default().protocol_fields(),
            "service_top=\tservice_nonzero_top=\tservice_last=other-provider\tservice_other=0\tservice_overflow=0\tservice_time_top=none\tservice_other_ns=0\tservice_overflow_ns=0\tservice_unattributed_ns=0"
        );
    }
}

#[cfg(test)]
mod service_timing_tests;

#[cfg(test)]
#[path = "startup_profile/libc_builtin_timing_tests.rs"]
mod libc_builtin_timing_tests;
