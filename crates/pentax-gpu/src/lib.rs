// SPDX-License-Identifier: GPL-2.0-or-later

//! Bounded, flat syntax inspection of a small AMD-derived GPU command packet
//! subset for PentaX.
//!
//! [`inspect`] copies one little-endian byte slice whose length is a multiple
//! of four into an owned, immutable buffer and walks it once from the front.
//! It commits fixed-size records for exact type-2 padding, zero-flag NOPs
//! (ordinary and the header-only form) and the direct context, SH and uconfig
//! relative register-write grammars, then reports either
//! [`Outcome::CompleteFlatSubset`] or the first positioned terminal stop. A
//! four-dword indirect-buffer reference is decoded as an observation and
//! always stops inspection at its header; its target is never read.
//!
//! The flat inspector is input validation. Register records are encoded
//! write intents with owner-bound value ranges: they are not applied writes,
//! known register values, a hardware-state snapshot or shader addresses. A
//! complete report is never permission to execute commands, and a stopped
//! prefix is diagnostic only; there is no resume token. Flat inspection keeps
//! no GPU state and reads no guest memory. The separate [`compute`] API prepares
//! a self-contained dispatch or bounded multi-dispatch command plans with known
//! queue registers and ordinary indirect calls/chains. Core's Graphics5Driver
//! services retain per-queue state and exclusive ownership through Vulkan
//! readback. Asynchronous scheduling, general coherence and presentation remain
//! unfinished.
//! [`graphics`] additionally prepares bounded nonindexed guest draw snapshots,
//! real compiled VS/PS and initial target contents. Its strict linear RGBA8
//! profile is documented separately; a prepared product grants no owner or
//! execution authority and does not establish title or presentation support.
//!
//! Field positions follow AMD's published Gfx9+ packet definitions, with an
//! independent public PS5 implementation used only as observation of the
//! same subset. Exact pinned sources and the precise admission and stop
//! policy are listed in the crate README. The implementation is original,
//! safe Rust. Flat inspection performs no I/O; compute uses an explicit reader
//! and the separately licensed shader compiler, with no direct file access.

#![forbid(unsafe_code)]

pub mod compute;
mod context_state;
mod error;
pub mod graphics;
mod inspect;
mod limits;
mod marker;
mod memory_sync;
mod memory_transfer;
mod packet;
mod queue_reset;
mod registers;
mod report;

pub use error::{InspectError, StopCategory, StopReason, Storage};
pub use inspect::inspect;
pub use limits::{LimitKind, Limits, RECORD_STORAGE_BYTES};
pub use packet::{Header, IndirectBuffer, RegisterSpace};
pub use registers::{IndirectRegisters, contains_indirect as contains_indirect_registers};
pub use report::{
    Accounting, Inspection, NopPayload, Outcome, Packet, PacketBody, Packets, RegisterWrite,
};

#[cfg(test)]
mod tests;
