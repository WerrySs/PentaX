// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from pinned209dffe AGC producers and explicit barrier events.
// See NOTICE. This payload is not a fence, event signal or execution authority.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynchronizationKind {
    Acquire,
    Event,
}

/// One exact admitted synchronization packet, retained at its stream position.
/// No constructor accepts detached host controls or backing authority.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Synchronization {
    kind: SynchronizationKind,
    payload: [u32; 7],
    words: usize,
}
impl fmt::Debug for Synchronization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Synchronization")
            .field("kind", &self.kind)
            .field("payload_words", &self.words)
            .finish_non_exhaustive()
    }
}
impl Synchronization {
    pub fn kind(&self) -> SynchronizationKind {
        self.kind
    }
    /// Exact producer payload for bounded audit; never a native pointer.
    pub fn payload(&self) -> &[u32] {
        &self.payload[..self.words]
    }
}

pub(crate) fn packet_words(raw: u32) -> Option<usize> {
    match raw {
        0xc006_1050 => Some(8),
        0xc000_4600 => Some(2),
        0xc002_4600 => Some(4),
        _ => None,
    }
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Synchronization, &'static str> {
    if bytes.len() < 4 {
        return Err("truncated synchronization header");
    }
    let word =
        |index: usize| u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    let raw = word(0);
    let count = packet_words(raw).ok_or("synchronization packet profile")?;
    if bytes.len() < count * 4 {
        return Err("truncated synchronization packet");
    }
    let mut payload = [0; 7];
    for (index, value) in payload[..count - 1].iter_mut().enumerate() {
        *value = word(index + 1);
    }
    let kind = if raw == 0xc006_1050 {
        // The source stores truncated cache coordinates, not memory operands.
        // No address/size dereference or cache-state fabrication is performed.
        if payload[2] != 0 || payload[4] != 0 {
            return Err("acquire producer reserved words");
        }
        SynchronizationKind::Acquire
    } else {
        // Addressed query events and the upstream ignored event family have
        // no established result implementation in this bounded profile.
        if count != 2 {
            return Err("addressed event queries are unsupported");
        }
        let control = payload[0];
        let event = control & 63;
        let index = (control >> 8) & 7;
        if control & !0x73f != 0 {
            return Err("event write control profile");
        }
        match event {
            7 | 15 | 16 if matches!(index, 0 | 4) => {}
            0x16 | 0x31 | 0x2a | 0x2c | 0x2e if index == 0 => {}
            _ => return Err("event has no admitted synchronization effect"),
        }
        SynchronizationKind::Event
    };
    Ok(Synchronization {
        kind,
        payload,
        words: count - 1,
    })
}
