// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned graphics command/state preparation.
// Source contracts and deliberate limits are documented in graphics/README.md.

//! Nonindexed guest draw preparation without GPU effects or host pointers.
//! A command plan is not execution authority. The runtime must retain the same
//! owner, admit actual GPU permissions and publish completed readback atomically.

use crate::RegisterSpace;
use crate::compute::CommandRange;
use crate::compute::{Memory, MemoryError};
use pentax_shader_compiler::CompiledShader;
use std::fmt;

mod commands;
mod prepare;
mod state;
pub use commands::{CommandLimits, CommandPlan, CommandUsage, Draw, plan_commands};
pub use prepare::prepare_draw;
pub use state::GraphicsState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Limit(&'static str),
    Allocation,
    Memory(MemoryError),
    Packet {
        buffer: usize,
        dword: usize,
        reason: &'static str,
    },
    Unsupported(&'static str),
    MissingRegister {
        space: RegisterSpace,
        register: u32,
    },
    Compile,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "graphics submission: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_shader_words: usize,
    pub max_spirv_words: usize,
    pub max_target_bytes: usize,
    pub max_dimension: u32,
    pub max_vertices: u64,
    pub max_work: u64,
    pub max_host_bytes: usize,
    pub compiler_phase_bytes: usize,
    pub compiler_phase_work: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_shader_words: 256,
            max_spirv_words: 32 * 1024,
            max_target_bytes: 16 * 1024 * 1024,
            max_dimension: 4096,
            max_vertices: 1024 * 1024,
            max_work: 128 * 1024 * 1024,
            max_host_bytes: 128 * 1024 * 1024,
            compiler_phase_bytes: 8 * 1024 * 1024,
            compiler_phase_work: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub work: u64,
    pub host_bytes: usize,
    pub shader_words: usize,
    pub vertices: u64,
    pub target_bytes: usize,
}

/// Real compiler products and retained target contents, never a canned shader.
/// Private fields prohibit substituting unrelated SPIR-V or target snapshots.
pub struct PreparedDraw {
    vertex: CompiledShader,
    pixel: CompiledShader,
    input: Vec<u8>,
    vertex_push: [u8; 128],
    pixel_push: [u8; 128],
    target_address: u64,
    width: u32,
    height: u32,
    vertex_count: u32,
    instance_count: u32,
    shader_ranges: [CommandRange; 2],
    usage: Usage,
}
impl fmt::Debug for PreparedDraw {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedDraw")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("vertex_count", &self.vertex_count)
            .field("instance_count", &self.instance_count)
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}
impl PreparedDraw {
    pub fn vertex(&self) -> &CompiledShader {
        &self.vertex
    }
    pub fn pixel(&self) -> &CompiledShader {
        &self.pixel
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn target_address(&self) -> u64 {
        self.target_address
    }
    pub fn input(&self) -> &[u8] {
        &self.input
    }
    pub fn vertex_count(&self) -> u32 {
        self.vertex_count
    }
    pub fn first_vertex(&self) -> u32 {
        0
    }
    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }
    pub fn first_instance(&self) -> u32 {
        0
    }
    pub fn vertex_push_data(&self) -> &[u8; 128] {
        &self.vertex_push
    }
    pub fn pixel_push_data(&self) -> &[u8; 128] {
        &self.pixel_push
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    /// Exact terminated source spans, not an owner or backing certificate.
    /// Native writeback must exclude physical aliases of both source ranges.
    pub fn shader_ranges(&self) -> [CommandRange; 2] {
        self.shader_ranges
    }
}

struct Budget<'a> {
    limits: &'a Limits,
    usage: Usage,
}
impl Budget<'_> {
    fn work(&mut self, units: u64) -> Result<(), Error> {
        self.usage.work = self
            .usage
            .work
            .checked_add(units)
            .filter(|n| *n <= self.limits.max_work)
            .ok_or(Error::Limit("work"))?;
        Ok(())
    }
    fn host(&mut self, bytes: usize) -> Result<(), Error> {
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_add(bytes)
            .filter(|n| *n <= self.limits.max_host_bytes)
            .ok_or(Error::Limit("host bytes"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
