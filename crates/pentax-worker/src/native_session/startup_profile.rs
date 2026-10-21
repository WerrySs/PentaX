// SPDX-License-Identifier: GPL-2.0-only

//! Fixed aggregate observations; no guest addresses, identifiers or source data.
//! Stage times sample one block in 4093, while service time covers every call.

use super::{NativeSessionError, charge};
use std::time::Instant;

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
    blocks: block_samples::BlockSamples,
}

/// Exact counts for the first 64 public kernel labels, plus an explicit
/// overflow bucket. Other admitted providers have their own aggregate bucket.
/// The top four are selected only when emitting a bounded progress record.
#[derive(Debug)]
pub(super) struct ServiceProfile {
    counts: [Option<(&'static str, u64, u64)>; 64],
    other: u64,
    overflow: u64,
    last: Option<&'static str>,
}

impl Default for ServiceProfile {
    fn default() -> Self {
        Self {
            counts: [None; 64],
            other: 0,
            overflow: 0,
            last: None,
        }
    }
}

impl ServiceProfile {
    pub fn record(
        &mut self,
        name: Option<&'static str>,
        nonzero_return: bool,
        work: &mut u64,
        limit: u64,
    ) -> Result<(), NativeSessionError> {
        self.last = name;
        let Some(name) = name else {
            self.other = self.other.saturating_add(1);
            return Ok(());
        };
        for slot in &mut self.counts {
            charge(
                work,
                8 + slot.map_or(0, |(label, _, _)| label.len().min(name.len()) as u64),
                limit,
                "admission work",
            )?;
            match slot {
                Some((label, count, nonzero)) if *label == name => {
                    *count = count.saturating_add(1);
                    *nonzero = nonzero.saturating_add(u64::from(nonzero_return));
                    return Ok(());
                }
                None => {
                    *slot = Some((name, 1, u64::from(nonzero_return)));
                    return Ok(());
                }
                _ => {}
            }
        }
        self.overflow = self.overflow.saturating_add(1);
        Ok(())
    }

    fn protocol_fields(&self) -> String {
        let mut sorted = self.counts;
        sorted.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.map_or(0, |(_, n, _)| n)));
        let top = sorted
            .iter()
            .flatten()
            .take(4)
            .map(|(name, count, _)| format!("{name}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        // Nonzero observations need their own ranking: the most frequent calls
        // may all return zero and otherwise hide rarer nonzero outcomes. These
        // are raw return observations, not an error classification (IDs and
        // other successful values can be nonzero).
        sorted.sort_unstable_by_key(|entry| {
            std::cmp::Reverse(entry.map_or((0, 0), |(_, calls, nonzero)| (nonzero, calls)))
        });
        let nonzero = sorted
            .iter()
            .flatten()
            .filter(|(_, _, count)| *count != 0)
            .take(4)
            .map(|(name, _, count)| format!("{name}:{count}"))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "service_top={top}\tservice_nonzero_top={nonzero}\tservice_last={}\tservice_other={}\tservice_overflow={}",
            self.last.unwrap_or("other-provider"),
            self.other,
            self.overflow
        )
    }
}

impl StartupProfile {
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

    pub fn service_finished(&mut self, start: Option<Instant>) {
        if let Some(start) = start {
            Self::accumulate(&mut self.service_ns, start.elapsed().as_nanos());
        }
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
                    .record(Some(label), false, &mut work, u64::MAX)
                    .unwrap();
            }
        }
        // Both a successful nonzero identity and another nonzero result remain
        // visible. The observer deliberately assigns neither an errno meaning.
        profile
            .record(Some("PthreadGetthreadid"), true, &mut work, u64::MAX)
            .unwrap();
        profile
            .record(Some("Open"), true, &mut work, u64::MAX)
            .unwrap();
        profile
            .record(Some("Open"), true, &mut work, u64::MAX)
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
        assert!(fields.ends_with("service_last=Open\tservice_other=0\tservice_overflow=0"));
        assert_eq!(
            ServiceProfile::default().protocol_fields(),
            "service_top=\tservice_nonzero_top=\tservice_last=other-provider\tservice_other=0\tservice_overflow=0"
        );
    }
}
