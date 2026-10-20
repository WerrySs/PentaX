// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance@209dffe9032d72749257ce18680d506b891284de,
// src/loader/systemContent.cpp, blob b86fcff76047852df9fe10b47c6a8c070602e640.
// See NOTICE and upstream provenance. Parsing follows the literal source
// searches, not a general XML grammar or a guessed package format.

use crate::Failure;
use crate::budget::Budget;

const MAX_INPUT: usize = 16 * 1024 * 1024;
const MAX_CHUNKS: usize = 1000;

/// The strict complete prefix replaces the source's unchecked short reads.
/// A binary count of zero is still a valid result; the XML/manifest1000-ID
/// limit does not apply to this independently encoded u16 count.
pub(super) fn binary(bytes: &[u8]) -> Option<u16> {
    if bytes.len() < 12 || &bytes[..4] != b"plgo" {
        return None;
    }
    Some(u16::from_le_bytes([bytes[10], bytes[11]]))
}

pub(super) fn xml(bytes: &[u8], budget: &mut Budget) -> Result<Option<u16>, Failure> {
    // The first default-tag pass and the disjoint chunk-tag pass each perform
    // only fixed-length searches and a bounded number of decimal/space scans.
    // Sixty-four byte-comparison units per input byte covers all those passes,
    // including the separate decimal validation/conversion of the first tag.
    charge(bytes, 64, budget)?;
    let mut maximum = 0;
    let mut found = false;
    if let Some(start) = find(bytes, b"<chunks", 0)
        && let Some(end) = find(bytes, b">", start)
        && let Some(default) = attribute(&bytes[start..end], b"default_chunk")
    {
        maximum = default;
        found = true;
    }

    let mut position = 0;
    while let Some(start) = find(bytes, b"<chunk", position) {
        let name_end = start + 6;
        if name_end == bytes.len() || !matches!(bytes[name_end], b' ' | b'\t' | b'\r' | b'\n') {
            position = name_end;
            continue;
        }
        let Some(end) = find(bytes, b">", name_end) else {
            break;
        };
        if let Some(id) = attribute(&bytes[start..end], b"id") {
            maximum = maximum.max(id);
            found = true;
        }
        position = end + 1;
    }
    if !found || maximum >= MAX_CHUNKS as u32 {
        return Ok(None);
    }
    Ok(Some((maximum + 1) as u16))
}

pub(super) fn manifest(bytes: &[u8], budget: &mut Budget) -> Result<Option<u16>, Failure> {
    // Every byte position is considered, including overlapping record windows.
    // A record has four marker and four value bytes;16 units per byte also
    // covers its fixed scalar bookkeeping without an input-sized allocation.
    charge(bytes, 16, budget)?;
    let mut seen = [0u64; MAX_CHUNKS.div_ceil(64)];
    let mut maximum = 0usize;
    let mut found = false;
    for window in bytes.windows(8) {
        if &window[..4] != b"\x08id\0" {
            continue;
        }
        let id = u32::from_le_bytes([window[4], window[5], window[6], window[7]]);
        if id < MAX_CHUNKS as u32 {
            let id = id as usize;
            seen[id / 64] |= 1u64 << (id % 64);
            maximum = maximum.max(id);
            found = true;
        }
    }
    if !found {
        return Ok(None);
    }
    budget.work((maximum + 1) as u64)?;
    if (0..=maximum).any(|id| seen[id / 64] & (1u64 << (id % 64)) == 0) {
        return Ok(None);
    }
    Ok(Some((maximum + 1) as u16))
}

fn charge(bytes: &[u8], per_byte: u64, budget: &mut Budget) -> Result<(), Failure> {
    if bytes.len() > MAX_INPUT {
        return Err(Failure::Limit("PlayGo package input bytes"));
    }
    budget.work(bytes.len() as u64 * per_byte + 1)
}

fn find(bytes: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| start + offset)
}

fn attribute(tag: &[u8], name: &[u8]) -> Option<u32> {
    // Only the first literal substring is considered: no token boundary, XML
    // entity decoding, retry at later duplicates, UTF8 requirement or sign.
    let mut position = find(tag, name, 0)? + name.len();
    while position < tag.len() && matches!(tag[position], b' ' | b'\t') {
        position += 1;
    }
    if tag.get(position) != Some(&b'=') {
        return None;
    }
    position += 1;
    while position < tag.len() && matches!(tag[position], b' ' | b'\t') {
        position += 1;
    }
    let quote = *tag.get(position)?;
    if !matches!(quote, b'\'' | b'"') {
        return None;
    }
    position += 1;
    let start = position;
    while position < tag.len() && tag[position].is_ascii_digit() {
        position += 1;
    }
    if position == start || tag.get(position) != Some(&quote) {
        return None;
    }
    let mut value = 0u32;
    for digit in &tag[start..position] {
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(*digit - b'0'))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Limits;

    fn budget() -> Budget {
        Budget::new(Limits::default()).unwrap()
    }
    fn record(id: u32) -> [u8; 8] {
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(b"\x08id\0");
        bytes[4..].copy_from_slice(&id.to_le_bytes());
        bytes
    }

    #[test]
    fn binary_requires_whole_header_and_preserves_zero_and_full_u16_domain() {
        let mut bytes = [0xa5; 14];
        bytes[..4].copy_from_slice(b"plgo");
        bytes[10..12].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(binary(&bytes[..11]), None);
        assert_eq!(binary(&bytes), Some(0));
        bytes[10..12].copy_from_slice(&0x1234u16.to_le_bytes());
        assert_eq!(binary(&bytes[..12]), Some(0x1234));
        bytes[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(binary(&bytes), Some(u16::MAX));
        bytes[0] = b'P';
        assert_eq!(binary(&bytes), None);
    }

    #[test]
    fn xml_literal_first_occurrences_holes_and_numeric_boundaries_are_not_normalized() {
        let mut work = budget();
        // Raw nonUTF8 is allowed. The default prefix need not be an exact XML
        // tag name; the id substring inside badid is also the source match.
        assert_eq!(
            xml(
                b"\xff<chunksibling default_chunk='4'><chunk badid \t= \t\"9\"/><chunk id='0'/>",
                &mut work
            ),
            Ok(Some(10))
        );
        // Only the first chunks/default and the first id substring are used.
        // A newline beside '=' is not the source's space-or-tab grammar.
        assert_eq!(xml(b"<chunks default_chunk=\n'998'><chunks default_chunk='998'><chunk id='2' id='999'/><chunk id='3'/>", &mut work), Ok(Some(4)));
        assert_eq!(
            xml(b"<chunk id='4294967296'/><chunk\nid='0002'/>", &mut work),
            Ok(Some(3))
        );
        assert_eq!(
            xml(b"<chunks default_chunk='999'/>", &mut work),
            Ok(Some(1000))
        );
        assert_eq!(
            xml(b"<chunk id='1'/><chunk id='1000'/>", &mut work),
            Ok(None)
        );
        assert_eq!(xml(b"<chunk id='4294967295'/>", &mut work), Ok(None));
        assert_eq!(
            xml(b"<chunk/><chunk><chunk id='+1'/><chunk id='1'", &mut work),
            Ok(None)
        );
        let mut denied = Budget::new(Limits {
            max_work: 0,
            ..Limits::default()
        })
        .unwrap();
        assert!(matches!(
            xml(b"<chunk id='0'/>", &mut denied),
            Err(Failure::Limit(_))
        ));
        assert_eq!(denied.usage().work, 0);
    }

    #[test]
    fn manifest_scans_unaligned_complete_windows_and_requires_contiguous_valid_ids() {
        let mut work = budget();
        let mut bytes = vec![0xff];
        bytes.extend_from_slice(&record(2));
        bytes.extend_from_slice(&record(0));
        bytes.extend_from_slice(&record(1000)); // ignored, not a global error
        assert_eq!(manifest(&bytes, &mut work), Ok(None));
        bytes.extend_from_slice(&record(1));
        assert_eq!(manifest(&bytes, &mut work), Ok(Some(3)));
        assert_eq!(manifest(&record(0)[..7], &mut work), Ok(None));
        assert_eq!(manifest(&record(1000), &mut work), Ok(None));
        let mut all = Vec::new();
        for id in (0..1000).rev() {
            all.extend_from_slice(&record(id));
        }
        assert_eq!(manifest(&all, &mut work), Ok(Some(1000)));
        // Removing id256's exact record proves little-endian values and the
        // interior hole matter even though both endpoints remain present.
        let offset = (999 - 256) * 8;
        all[offset..offset + 8].fill(0xff);
        assert_eq!(manifest(&all, &mut work), Ok(None));
        let mut denied = Budget::new(Limits {
            max_work: 0,
            ..Limits::default()
        })
        .unwrap();
        assert!(matches!(
            manifest(&record(0), &mut denied),
            Err(Failure::Limit(_))
        ));
        assert_eq!(denied.usage().work, 0);
    }
}
