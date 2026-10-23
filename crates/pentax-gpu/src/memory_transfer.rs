// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Original bounded Rust adaptation of public KytyPS5 7b7f9a70
// CpOpCopyData/CpOpDmaData and CommandProcessor::DmaData; see NOTICE.
//! Sealed transfer metadata. Admission reads no source/destination payload.

#[derive(Clone, Copy)]
enum Source {
    Memory(u64),
    Fill(u32),
    Empty,
}

/// A checked immutable packet, never independent backing/queue authority.
pub struct Transfer {
    source: Source,
    destination: u64,
    bytes: usize,
    words: [u32; 7],
    word_count: usize,
}
impl std::fmt::Debug for Transfer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transfer")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
impl Transfer {
    pub fn byte_count(&self) -> usize {
        self.bytes
    }
    pub fn destination(&self) -> u64 {
        self.destination
    }
    pub fn source_address(&self) -> Option<u64> {
        if let Source::Memory(address) = self.source {
            Some(address)
        } else {
            None
        }
    }
    pub fn fill_word(&self) -> Option<u32> {
        if let Source::Fill(word) = self.source {
            Some(word)
        } else {
            None
        }
    }
    /// Exact captured control/address words for explicit auditing, not Debug/logs.
    pub fn packet(&self) -> &[u32] {
        &self.words[..self.word_count]
    }
}
pub(crate) fn packet_words(header: u32) -> Option<usize> {
    match header {
        0xc004_4000 => Some(6),
        0xc005_5000 => Some(7),
        _ => None,
    }
}
fn extent(address: u64, bytes: usize, alignment: usize) -> Result<(), &'static str> {
    if address == 0
        || !address.is_multiple_of(alignment as u64)
        || address
            .checked_add(bytes as u64)
            .is_none_or(|end| end > 1 << 47)
    {
        return Err("transfer extent/alignment profile");
    }
    Ok(())
}
pub(crate) fn decode(input: &[u8]) -> Result<Transfer, &'static str> {
    let header = input.get(..4).ok_or("transfer header")?;
    let header = u32::from_le_bytes(header.try_into().unwrap());
    let count = packet_words(header).ok_or("transfer header profile")?;
    if input.len() != count * 4 {
        return Err("transfer packet extent");
    }
    let mut words = [0; 7];
    for (out, input) in words.iter_mut().zip(input.chunks_exact(4)) {
        *out = u32::from_le_bytes(input.try_into().unwrap());
    }
    let control = words[1];
    let source = u64::from(words[2]) | u64::from(words[3]) << 32;
    let destination = u64::from(words[4]) | u64::from(words[5]) << 32;
    let mut result = Transfer {
        source: Source::Empty,
        destination,
        bytes: 0,
        words,
        word_count: count,
    };
    if header == 0xc005_5000 {
        // Match the primary engine validation, then its early zero-byte return.
        // Encoded engine is one bit. All other control/address fields are opaque
        // on this path, including otherwise unsupported selectors/reserved bits.
        // The exact encoded engine field is one bit, proving the primary's
        // required 0..=1 engine domain before considering the byte count.
        let size = (words[6] & 0x03ff_ffff) as usize;
        if size == 0 {
            return Ok(result);
        }
        let allowed = 1 | 3 << 13 | 3 << 20 | 3 << 25 | 3 << 29 | 1 << 31;
        if control & !allowed != 0 {
            return Err("DMA control profile");
        }
        let src = (control >> 29 & 3) | (words[6] >> 24 & 4) | (words[6] >> 25 & 8);
        let dst = (control >> 20 & 3) | (words[6] >> 25 & 4) | (words[6] >> 26 & 8);
        // Source's discard/query/register/nowhere/GDS domains are not modeled.
        if destination as u32 == 0x3022c || !matches!(dst, 0 | 3) || !size.is_multiple_of(4) {
            return Err("DMA destination/count profile");
        }
        extent(destination, size, 4)?;
        result.bytes = size;
        result.source = match src {
            0 | 3 => {
                extent(source, size, 4)?;
                Source::Memory(source)
            }
            2 => Source::Fill(source as u32),
            _ => return Err("DMA source profile"),
        };
    } else {
        let allowed = 15 | 15 << 8 | 3 << 13 | 1 << 16 | 1 << 20 | 3 << 25 | 1 << 30;
        if control & !allowed != 0 {
            return Err("COPY control profile");
        }
        let src = (control & 15) << 1 | (control >> 30 & 1);
        let dst = (control >> 8 & 15) << 1;
        let size = if control & (1 << 16) != 0 { 8 } else { 4 };
        if !matches!(dst, 2 | 4) || destination as u32 == 0x3022c {
            return Err("COPY destination profile");
        }
        extent(destination, size, size)?;
        result.bytes = size;
        result.source = match src {
            2 | 4 | 5 => {
                extent(source, size, size)?;
                Source::Memory(source)
            }
            10 | 11 if size == 4 => Source::Fill(source as u32),
            _ => return Err("COPY source/width profile"),
        };
    }
    Ok(result)
}
