// SPDX-License-Identifier: GPL-2.0-only

//! Bounded serialization of the public LOD-feedback report format.
//!
//! Reference: public Jetsku/KytyPS5 revision
//! b719d67ce6bcfd21d402bd415f3e4f68e4793534, lodStatsReport.h.
//! These are data conversions, not collection or completion proofs. Callers
//! must establish ownership and actual GPU completion before publishing a
//! report. Initializing counter storage does not authorize a report write.

use core::fmt;

pub const COUNTER_COUNT: usize = 256;
pub const STORAGE_ENTRIES: usize = COUNTER_COUNT + 1;
pub const COUNTER_WORDS: usize = STORAGE_ENTRIES * 2;
pub const COUNTER_BYTES: usize = COUNTER_WORDS * 4;
pub const REPORT_HEADER_BYTES: usize = 64;
pub const REPORT_BYTES: usize = REPORT_HEADER_BYTES + COUNTER_COUNT * 8;
pub const UNSAMPLED: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    CounterWordCount,
    CounterByteCount,
}

/// Initial upload data only. No samples or GPU completion are asserted.
pub const fn reset_words() -> [u32; COUNTER_WORDS] {
    let mut words = [0; COUNTER_WORDS];
    let mut index = 0;
    while index < STORAGE_ENTRIES {
        words[index] = UNSAMPLED;
        index += 1;
    }
    words
}

/// An owned copy of counter words. This type grants no guest-write authority.
#[derive(Clone)]
pub struct Counters {
    words: [u32; COUNTER_WORDS],
}

impl fmt::Debug for Counters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Counters").finish_non_exhaustive()
    }
}

impl Counters {
    /// Exact owned snapshot data for a subsequent real GPU upload.
    /// This observation grants no collection, completion or guest-write authority.
    pub const fn as_words(&self) -> &[u32; COUNTER_WORDS] {
        &self.words
    }

    pub fn from_words(words: &[u32]) -> Result<Self, Error> {
        let words = words.try_into().map_err(|_| Error::CounterWordCount)?;
        Ok(Self { words })
    }

    pub fn from_le_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() != COUNTER_BYTES {
            return Err(Error::CounterByteCount);
        }
        let mut words = [0; COUNTER_WORDS];
        for (word, raw) in words.iter_mut().zip(bytes.chunks_exact(4)) {
            *word = u32::from_le_bytes(raw.try_into().expect("four-byte counter chunk"));
        }
        Ok(Self { words })
    }

    /// Serialize a snapshot. The caller must separately attest its GPU origin
    /// and completion; the header is format data, never such an attestation.
    pub fn pack_report(&self) -> Report {
        let mut bytes = [0; REPORT_BYTES];
        bytes[..4].copy_from_slice(&1_u32.to_le_bytes());
        for counter in 0..COUNTER_COUNT {
            let finest = self.words[counter];
            let level = if finest == UNSAMPLED {
                15
            } else {
                finest.min(14)
            };
            let samples = self.words[STORAGE_ENTRIES + counter].min(0x00ff_ffff);
            let entry = (u64::from(level) << 56) | ((counter as u64) << 24) | u64::from(samples);
            let start = REPORT_HEADER_BYTES + counter * 8;
            bytes[start..start + 8].copy_from_slice(&entry.to_le_bytes());
        }
        Report { bytes }
    }
}

/// Serialized data only; not a fence, a ready ticket or a guest-memory lease.
pub struct Report {
    bytes: [u8; REPORT_BYTES],
}

impl Report {
    pub const fn as_bytes(&self) -> &[u8; REPORT_BYTES] {
        &self.bytes
    }
}

impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Report").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_layout_consumes_counter_ids_saturation_and_the_actual_finest_words() {
        let mut words = reset_words();
        words[0] = 0;
        words[1] = 14;
        words[2] = 15;
        words[254] = u32::MAX - 1;
        words[255] = 7;
        words[STORAGE_ENTRIES] = 0x0100_0000;
        words[STORAGE_ENTRIES + 1] = 0x00ff_ffff;
        words[STORAGE_ENTRIES + 2] = 9;
        words[STORAGE_ENTRIES + 254] = u32::MAX;
        words[STORAGE_ENTRIES + 255] = 0x123456;
        // The extra shader sink must never become a 257th guest entry.
        words[256] = 3;
        words[STORAGE_ENTRIES + 256] = 123;
        let report = Counters::from_words(&words).unwrap().pack_report();
        let raw = report.as_bytes();
        assert_eq!(raw.len(), 2112);
        assert_eq!(&raw[..4], &[1, 0, 0, 0]);
        assert!(raw[4..64].iter().all(|byte| *byte == 0));
        for (counter, expected) in [
            (0, 0x0000_0000_00ff_ffff_u64),
            (1, 0x0e00_0000_01ff_ffff),
            (2, 0x0e00_0000_0200_0009),
            (3, 0x0f00_0000_0300_0000),
            (254, 0x0e00_0000_feff_ffff),
            (255, 0x0700_0000_ff12_3456),
        ] {
            let start = 64 + counter * 8;
            assert_eq!(&raw[start..start + 8], &expected.to_le_bytes());
        }
    }

    #[test]
    fn reset_and_little_endian_snapshots_are_exact_and_owned() {
        let mut words = reset_words();
        assert!(words[..257].iter().all(|word| *word == u32::MAX));
        assert!(words[257..].iter().all(|word| *word == 0));
        words[17] = 6;
        words[257 + 17] = 0x123456;
        let mut bytes: Vec<_> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
        let counters = Counters::from_le_bytes(&bytes).unwrap();
        assert_eq!(counters.as_words(), &words);
        bytes.fill(0xff);
        words.fill(0);
        let report = counters.pack_report();
        assert_eq!(
            &report.as_bytes()[64 + 17 * 8..64 + 18 * 8],
            &0x0600_0000_1112_3456_u64.to_le_bytes()
        );
        assert_eq!(format!("{counters:?}"), "Counters { .. }");
        assert_eq!(format!("{report:?}"), "Report { .. }");
    }

    #[test]
    fn malformed_snapshot_extents_are_refused_instead_of_truncated_or_padded() {
        for length in [0, COUNTER_WORDS - 1, COUNTER_WORDS + 1] {
            assert!(matches!(
                Counters::from_words(&vec![0; length]),
                Err(Error::CounterWordCount)
            ));
        }
        for length in [0, COUNTER_BYTES - 1, COUNTER_BYTES + 1] {
            assert!(matches!(
                Counters::from_le_bytes(&vec![0; length]),
                Err(Error::CounterByteCount)
            ));
        }
    }
}
