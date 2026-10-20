// SPDX-License-Identifier: GPL-2.0-only
// Ported from KytyPS5-Performance at 209dffe9032d72749257ce18680d506b891284de.
// See README.md and NOTICE for source attribution and modifications.

//! An owned, bounded **linear syntax decoder**, directly adapted from the
//! KytyPS5-Performance shader frontend. It decodes every family recognized by
//! the pinned decoder: scalar SOP1/2/K/C/P, VOP1/2/C/3/3P (including SDWA, DPP
//! and DPP8 modifier words), VINTRP, SMEM, MUBUF, MTBUF, FLAT, DS, MIMG and EXP,
//! with the source's opcode maps, aliases and field-dependent restrictions.
//!
//! Success means the supplied span was decoded with these source rules. It
//! does not validate register pairs, reserved bits, stage legality,
//! control-flow targets, shader termination, resource bindings or execution,
//! and does not claim ISA validity. ENDPGM does not hide trailing input. No
//! shader is executed or translated to a host shader language.
//!
//! Instruction views borrow their owner, including the exact source words.
//! Failed decoding exposes no partial result. Branch offsets are retained as
//! signed, span-relative metadata; they are not validated instruction targets.

#![forbid(unsafe_code)]

mod decode;
mod image;
mod memory;
mod model;
mod opcode;
mod vector;

pub use decode::{DecodeError, LimitKind, Limits, UnsupportedReason, decode, instruction_family};
pub use image::{image_address_component_layout, image_address_dword_count};
pub use model::{
    Branch, DecodedSpan, Dpp, DppKind, Export, Family, Image, ImageAddressComponent,
    ImageDimension, ImageSampleFlags, Instruction, MAX_IMAGE_NSA_ADDRESSES, MemoryAccess, Operand,
    OperandKind, SdwaSelect, SdwaUnused, ShaderStage,
};
pub use opcode::Opcode;

/// Exact source revision used for this port, rather than a hardware ISA claim.
pub const SOURCE_REVISION: &str = "209dffe9032d72749257ce18680d506b891284de";

#[cfg(test)]
mod tests;
