// SPDX-License-Identifier: GPL-2.0-only

//! Exact GET_LOD_STATS packet data, without execution or write authority.
//!
//! Producer layout follows public KytyPS5/KytyPS5 revision
//! 7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3, AgcDcbGetLodStats.
//! Completion-size behavior follows public Jetsku/KytyPS5 revision
//! b719d67ce6bcfd21d402bd415f3e4f68e4793534, LodStatsCounter::Report.

use core::fmt;

pub const HEADER: u32 = 0xc003_8e00;
const CONTROL_MASK: u32 = 0x300f_fffc;
const REPORT_BYTES: usize = 64 + 256 * 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Header,
    AddressAlignment,
    ReservedControl,
}

/// Packet scalars only. In particular, a decoded destination is not a lease.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Request {
    destination: u64,
    size: u32,
    control: u32,
}

impl Request {
    pub fn decode(words: [u32; 5]) -> Result<Self, Error> {
        if words[0] != HEADER {
            return Err(Error::Header);
        }
        if words[2] & 63 != 0 {
            return Err(Error::AddressAlignment);
        }
        if words[4] & !CONTROL_MASK != 0 {
            return Err(Error::ReservedControl);
        }
        Ok(Self {
            destination: u64::from(words[2]) | (u64::from(words[3]) << 32),
            size: words[1],
            control: words[4],
        })
    }

    pub const fn destination(self) -> u64 {
        self.destination
    }

    pub const fn size(self) -> u32 {
        self.size
    }

    pub const fn cache_policy(self) -> u8 {
        ((self.control >> 28) & 3) as u8
    }

    pub const fn reset_count(self) -> u8 {
        ((self.control >> 10) & 255) as u8
    }

    pub const fn interval(self) -> u8 {
        ((self.control >> 2) & 255) as u8
    }

    pub const fn force_reset(self) -> bool {
        self.control & (1 << 18) != 0
    }

    pub const fn report_and_reset(self) -> bool {
        self.control & (1 << 19) != 0
    }

    pub const fn reset_requested(self) -> bool {
        self.force_reset() || self.report_and_reset()
    }

    /// Byte extent for completion publication, assuming initialization and an
    /// actual completed GPU snapshot. This never grants a guest write or makes
    /// the first initialization packet publishable. Cache/reset-count/interval
    /// policy admission is separately owned by the ordered runtime.
    pub const fn completion_bytes(self) -> usize {
        if self.destination == 0 || self.size < 4 {
            0
        } else if self.size as u64 > REPORT_BYTES as u64 {
            REPORT_BYTES
        } else {
            self.size as usize
        }
    }

    /// An initialized empty probe preserves counters and performs no write.
    pub const fn is_empty_probe(self) -> bool {
        self.completion_bytes() == 0 && !self.reset_requested()
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Neither the guest destination nor other private packet values escape.
        f.debug_struct("LodStatsRequest").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_packet_fields_and_completion_extents_preserve_the_source_contract() {
        let request =
            Request::decode([0xc003_8e00, 0x41, 0x5566_7780, 0x1122_3344, 0x2008_d008]).unwrap();
        assert_eq!(request.destination(), 0x1122_3344_5566_7780);
        assert_eq!((request.size(), request.cache_policy()), (65, 2));
        assert_eq!((request.reset_count(), request.interval()), (52, 2));
        assert!(request.report_and_reset());
        assert!(!request.force_reset());
        assert_eq!(request.completion_bytes(), 65);
        assert!(!request.is_empty_probe());
        assert_eq!(format!("{request:?}"), "LodStatsRequest { .. }");
        for (size, expected) in [
            (0, 0),
            (3, 0),
            (4, 4),
            (2111, 2111),
            (2112, 2112),
            (u32::MAX, 2112),
        ] {
            let value = Request::decode([HEADER, size, 64, 0, 0]).unwrap();
            assert_eq!(value.completion_bytes(), expected);
            assert_eq!(value.is_empty_probe(), expected == 0);
        }
        for reset in [1 << 18, 1 << 19, 3 << 18] {
            let value = Request::decode([HEADER, 0, 0, 0, reset]).unwrap();
            assert_eq!(value.completion_bytes(), 0);
            assert!(value.reset_requested());
            assert!(!value.is_empty_probe());
        }
        let no_destination = Request::decode([HEADER, u32::MAX, 0, 0, 0]).unwrap();
        assert!(no_destination.is_empty_probe());
    }

    #[test]
    fn malformed_packet_encodings_refuse_without_normalizing_reserved_data() {
        for header in [0, HEADER ^ 1, HEADER ^ (1 << 16), HEADER ^ (1 << 30)] {
            assert_eq!(Request::decode([header, 0, 0, 0, 0]), Err(Error::Header));
        }
        for bit in 0..6 {
            assert_eq!(
                Request::decode([HEADER, 0, 1 << bit, 0, 0]),
                Err(Error::AddressAlignment)
            );
        }
        for bit in 0..32 {
            if CONTROL_MASK & (1 << bit) == 0 {
                assert_eq!(
                    Request::decode([HEADER, 0, 0, 0, 1 << bit]),
                    Err(Error::ReservedControl)
                );
            }
        }
        let all = Request::decode([HEADER, u32::MAX, 0xffff_ffc0, u32::MAX, 0x300f_fffc]).unwrap();
        assert_eq!(
            (all.cache_policy(), all.reset_count(), all.interval()),
            (3, 255, 255)
        );
        assert!(all.force_reset() && all.report_and_reset());
        // Encoding is data: ownership and canonical-address checks belong to
        // the runtime immediately before an actual publication attempt.
        assert_eq!(all.destination(), 0xffff_ffff_ffff_ffc0);
    }
}
