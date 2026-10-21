// SPDX-License-Identifier: GPL-2.0-only
//! Logical boundaries that a private two-iteration native trace must not skip.

use super::StartupProfile;

pub(super) struct Boundaries {
    pub blocks: u64,
    pub turn_remaining: u64,
    pub report_progress: bool,
    pub progress_interval: u64,
    pub reported_blocks: u64,
    pub has_consumer: bool,
    pub frame_pump_blocks: Option<u64>,
}

impl Boundaries {
    /// Both original loop-entry positions must precede every pending boundary.
    /// The boundary after the second block is handled by the ordinary loop.
    pub fn maximum_span(&self) -> u64 {
        let Some(second) = self.blocks.checked_add(1) else {
            return 1;
        };
        if self.turn_remaining < 2
            || StartupProfile::sampled(self.report_progress, self.blocks)
            || StartupProfile::sampled(self.report_progress, second)
        {
            return 1;
        }
        if self.report_progress {
            let remaining = self
                .reported_blocks
                .checked_add(self.progress_interval.max(1))
                .and_then(|next| next.checked_sub(self.blocks));
            if remaining.is_none_or(|remaining| remaining < 2) {
                return 1;
            }
        }
        if self.has_consumer {
            let remaining = self
                .frame_pump_blocks
                .and_then(|old| self.blocks.checked_sub(old))
                .and_then(|elapsed| 1024_u64.checked_sub(elapsed));
            if remaining.is_none_or(|remaining| remaining < 2) {
                return 1;
            }
        }
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordinary(blocks: u64) -> Boundaries {
        Boundaries {
            blocks,
            turn_remaining: u64::MAX,
            report_progress: true,
            progress_interval: 8192,
            reported_blocks: 0,
            has_consumer: true,
            frame_pump_blocks: Some(0),
        }
    }

    #[test]
    fn pairs_leave_sampling_frame_progress_and_turn_observation_positions_unskipped() {
        assert_eq!(ordinary(1).maximum_span(), 2);
        assert_eq!(ordinary(0).maximum_span(), 1); // The current block is sampled.
        assert_eq!(ordinary(1022).maximum_span(), 2);
        assert_eq!(ordinary(1023).maximum_span(), 1);
        let mut before_sample = ordinary(4092);
        before_sample.has_consumer = false;
        assert_eq!(before_sample.maximum_span(), 1); // Next block is sampled.
        before_sample.blocks = 4093;
        assert_eq!(before_sample.maximum_span(), 1);
        before_sample.blocks = 4094;
        assert_eq!(before_sample.maximum_span(), 2);
        let mut before_report = ordinary(8190);
        before_report.has_consumer = false;
        assert_eq!(before_report.maximum_span(), 2);
        before_report.blocks = 8191;
        assert_eq!(before_report.maximum_span(), 1);
        for remaining in [0, 1, 2] {
            let mut turn = ordinary(1);
            turn.turn_remaining = remaining;
            assert_eq!(turn.maximum_span(), if remaining < 2 { 1 } else { 2 });
        }
    }

    #[test]
    fn absent_or_exhausted_boundaries_never_wrap_into_a_larger_trace_allowance() {
        let mut boundary = ordinary(1);
        boundary.frame_pump_blocks = None;
        assert_eq!(boundary.maximum_span(), 1);
        boundary.frame_pump_blocks = Some(2);
        assert_eq!(boundary.maximum_span(), 1);
        boundary.has_consumer = false;
        assert_eq!(boundary.maximum_span(), 2);
        boundary.progress_interval = 1;
        boundary.reported_blocks = 1;
        assert_eq!(boundary.maximum_span(), 1);
        boundary.reported_blocks = u64::MAX;
        assert_eq!(boundary.maximum_span(), 1);
        boundary.report_progress = false;
        boundary.blocks = u64::MAX;
        assert_eq!(boundary.maximum_span(), 1);
    }
}
