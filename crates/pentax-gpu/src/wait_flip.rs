// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from public KytyPS5 7b7f9a70 AgcDcbWaitUntilSafeForRendering
// and CpOpWaitFlipDone; original bounded opaque queue predicate. See NOTICE.
/// Exact selected-port/buffer wait. This descriptor grants no port authority,
/// fence, completion or presentation; the owned runtime must resolve it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaitFlip {
    handle: i32,
    buffer: i32,
}
impl WaitFlip {
    pub fn handle(self) -> i32 {
        self.handle
    }
    pub fn buffer(self) -> i32 {
        self.buffer
    }
}
pub(crate) fn packet_words(header: u32) -> Option<usize> {
    (header == 0xc005_1018).then_some(7)
}
pub(crate) fn decode(bytes: &[u8]) -> Result<WaitFlip, &'static str> {
    if bytes.len() < 28 {
        return Err("truncated wait-flip packet");
    }
    let word =
        |index: usize| u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    if word(0) != 0xc005_1018 || (3..7).any(|index| word(index) != 0) {
        return Err("wait-flip packet profile");
    }
    Ok(WaitFlip {
        handle: word(1) as i32,
        buffer: word(2) as i32,
    })
}
