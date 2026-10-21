// SPDX-License-Identifier: GPL-2.0-only

//! Source-bound same-ISA block translation plans. No native execution API.
//!
//! Plans retain the exact immutable linker source and policy. Ordinary admitted
//! instructions are re-encoded for a checked, disjoint cache placement; control
//! transfers and unported semantics terminate at a private UD2 site. Native
//! memory operands still require identity guest mappings and complete context
//! preservation in a future worker. These bytes are not a sandbox or permission
//! to execute, and no executable allocation or host pointer is exposed here.
//! The separate materialized path accepts caller-declared writes and retains
//! every deferred record; its distinct result never certifies TLS preparation.

#![forbid(unsafe_code)]

mod affine_loop_trace;
mod materialized;
mod memory;
mod model;
mod plan;
mod policy;
mod source;
#[cfg(test)]
mod tests;

pub use affine_loop_trace::AffineLoopTrace;
pub use iced_x86::{
    Code, ConditionCode, CpuidFeature, Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic,
    OpKind, Register,
};
pub use materialized::{
    DeclaredAppliedWrite, MaterializedBlockPlan, MaterializedSource, MaterializedSourceAssurance,
    MaterializedSourceUsage, declare_materialized_source, plan_materialized_block,
};
pub use memory::{
    DeclaredMemoryAccess, InstructionMemoryAccesses, MemoryAccessAnalyzer, MemoryAccessError,
    MemoryAccessKind, MemoryRegisterDependencies,
};
pub use model::*;
pub use plan::plan_block;
pub use policy::{CpuFeatures, TranslationPolicy};
