// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from pinned KytyPS5 AgcDcbContextStateOp/CpOpContextState.
// See NOTICE for primary revision, source paths and bounded profile.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Clear,
    Push,
    Pop,
    PushClear,
}

pub(crate) fn packet_words(raw: u32) -> Option<usize> {
    match raw {
        0xc001_1068 => Some(3),
        0xc003_1068 => Some(5),
        _ => None,
    }
}

/// Admit only the first segment emitted by the reviewed producer. Remaining
/// zero-NOP segments keep their existing opaque NOP framing.
pub(crate) fn decode(bytes: &[u8]) -> Result<Operation, &'static str> {
    let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    if bytes.len() < 4 {
        return Err("truncated context state packet");
    }
    let count = packet_words(word(0)).ok_or("context state packet header")?;
    if bytes.len() < count * 4 {
        return Err("truncated context state packet");
    }
    let operation = match (count, word(1)) {
        (5, 0) => Operation::Clear,
        (5, 1) => Operation::Push,
        (3, 2) => Operation::Pop,
        (5, 3) => Operation::PushClear,
        _ => return Err("context state operation/extent"),
    };
    if (2..count).any(|i| word(i) != 0) {
        return Err("context state padding");
    }
    Ok(operation)
}
