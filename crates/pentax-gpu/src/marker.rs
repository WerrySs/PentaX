// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from pinned209dffe marker producers/consuming handlers; see NOTICE.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerKind {
    Push,
    Pop,
}

/// Bounded source annotation, without logging or retaining detached text pointers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Marker {
    kind: MarkerKind,
    payload_words: usize,
}
impl Marker {
    pub fn kind(&self) -> MarkerKind {
        self.kind
    }
    pub fn payload_words(&self) -> usize {
        self.payload_words
    }
}

pub(crate) fn packet_words(raw: u32) -> Option<usize> {
    if raw == 0xc000_1030 {
        return Some(2);
    }
    if raw & 0xc000_ffff == 0xc000_102c && (raw >> 16) & 0x3fff != 0x3fff {
        return Some(((raw >> 16) & 0x3fff) as usize + 2);
    }
    None
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Marker, &'static str> {
    if bytes.len() < 4 {
        return Err("truncated marker header");
    }
    let raw = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let words = packet_words(raw).ok_or("marker producer header")?;
    if bytes.len() < words * 4 {
        return Err("truncated marker payload");
    }
    if raw == 0xc000_1030 {
        if bytes[4..8] != [0; 4] {
            return Err("pop marker producer payload");
        }
        return Ok(Marker {
            kind: MarkerKind::Pop,
            payload_words: 1,
        });
    }
    // The source captures length before growth, then copies current bytes.
    // A callback can alter the terminator. Never interpret a host C string or
    // read beyond this exact admitted packet; annotation has no GPU mutation.
    Ok(Marker {
        kind: MarkerKind::Push,
        payload_words: words - 1,
    })
}
