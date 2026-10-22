// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from public KytyPS5 agc.cpp and CpOpDispatchReset/ClearState.
// See NOTICE for exact revision and source paths.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reset {
    Processor,
    Context,
}

pub(crate) fn is_header(raw: u32) -> bool {
    matches!(raw, 0xc000_1024 | 0xc000_1200)
}

/// Exact bounded producer packets, not generic NOP tags or context operations.
/// CLEAR_STATE's low four bits do not select push/pop or a different operation.
pub(crate) fn decode(raw: u32, payload: u32) -> Result<Reset, &'static str> {
    match (raw, payload) {
        (0xc000_1024, 0) => Ok(Reset::Processor),
        (0xc000_1200, value) if value & !15 == 0 => Ok(Reset::Context),
        _ => Err("queue reset packet profile"),
    }
}
