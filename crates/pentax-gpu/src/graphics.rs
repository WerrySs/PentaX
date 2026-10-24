// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned graphics command/state preparation.
// Source contracts and deliberate limits are documented in graphics/README.md.

//! Guest triangle draw preparation without GPU effects or host pointers.
//! A command plan is not execution authority. The runtime must retain the same
//! owner, admit actual GPU permissions and publish completed readback atomically.

use crate::RegisterSpace;
use crate::compute::CommandRange;
pub use crate::compute::MemoryAction;
use crate::compute::{Memory, MemoryError};
use pentax_shader_compiler::CompiledShader;
pub use pentax_shader_compiler::{InputInterface, VertexInstanceContract};
use std::fmt;

mod commands;
mod draw_indirect;
pub use draw_indirect::{IndirectDraw, NativeIndirectOffsets};
mod indirect;
pub use indirect::{CaptureFailure, CaptureUsage, CapturedCommands, CapturedComputeCommands};
mod ordered;
pub use ordered::{OrderedCommands, OrderedStep};
mod linkage;
mod prepare;
mod state;
mod textures;
pub use commands::{
    Action, CommandLimits, CommandLocation, CommandPlan, CommandUsage, Draw, Flip, plan_commands,
    plan_commands_with_compute,
};
pub use linkage::{ShaderInterfaces, validate_shader_interfaces};
pub use prepare::prepare_draw;
pub use prepare::{prepare_draw_with_target, prepare_draw_with_target_and_lod};
pub use state::GraphicsState;
pub use textures::{PreparedSampler, PreparedTexture, UploadedMip};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Limit(&'static str),
    Allocation,
    Memory(MemoryError),
    SourceMemory {
        address: u64,
        error: MemoryError,
    },
    SourcePacket {
        address: u64,
        dword: usize,
        reason: &'static str,
    },
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
    /// Independent pixel staging ceiling: retained LOAD/input plus the
    /// backend's upload and completed readback allowance. Compiler IR is
    /// bounded separately by `max_host_bytes` and `compiler_phase_bytes`.
    pub max_pixel_bytes: usize,
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
            max_pixel_bytes: 64 * 1024 * 1024,
            max_host_bytes: 128 * 1024 * 1024,
            compiler_phase_bytes: 8 * 1024 * 1024,
            compiler_phase_work: 4 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub work: u64,
    /// Actual vector capacities retained in the returned prepared draw.
    pub host_bytes: usize,
    /// Greatest admitted simultaneous host capacity, including temporary
    /// compiler envelopes while those calls are running (not allocations).
    pub peak_host_bytes: usize,
    /// Actual LOAD/input vector capacities included in `host_bytes`.
    pub pixel_bytes: usize,
    pub shader_words: usize,
    pub vertices: u64,
    pub target_bytes: usize,
}

/// Declared byte representation, not native memory or completion authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetLayout {
    LinearRgba8,
    RenderTarget64KbRgba8,
}

/// Immutable geometry validated from this draw's actual register snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetGeometry {
    layout: TargetLayout,
    width: u32,
    height: u32,
    pitch: u32,
    guest_bytes: usize,
    tight_bytes: usize,
}
impl TargetGeometry {
    pub const fn layout(self) -> TargetLayout {
        self.layout
    }
    pub const fn width(self) -> u32 {
        self.width
    }
    pub const fn height(self) -> u32 {
        self.height
    }
    pub const fn pitch(self) -> u32 {
        self.pitch
    }
    pub const fn guest_bytes(self) -> usize {
        self.guest_bytes
    }
    pub const fn tight_bytes(self) -> usize {
        self.tight_bytes
    }
}

/// Remaining conversion admission after all retained preparation allocations.
/// The decoder has no guest or native memory authority.
#[derive(Clone, Copy, Debug)]
pub struct TargetDecodeLimits {
    pub max_host_bytes: usize,
    pub max_work: u64,
}
pub struct DecodedTarget {
    pub bytes: Vec<u8>,
    pub work: u64,
}

/// Exact source index element representations. No endian or restart flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexType {
    U16,
    U32,
}
impl IndexType {
    pub const fn byte_width(self) -> usize {
        match self {
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }
}

/// Immutable owned index payload from one fully admitted draw. Its range is
/// metadata, not guest ownership, permission or completion authority.
pub struct PreparedIndices {
    range: CommandRange,
    index_type: IndexType,
    count: u32,
    bytes: Vec<u8>,
}

/// An immutable, exact-base GPU-readable raw-buffer snapshot. Its numeric
/// range is metadata; the native adapter must separately exclude physical
/// aliases of every writable target while retaining the original pause.
pub struct PreparedBuffer {
    range: CommandRange,
    bytes: Vec<u8>,
}
impl fmt::Debug for PreparedBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedBuffer")
            .field("capacity", &self.bytes.capacity())
            .finish_non_exhaustive()
    }
}
impl PreparedBuffer {
    pub fn range(&self) -> CommandRange {
        self.range
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl PreparedIndices {
    pub fn range(&self) -> CommandRange {
        self.range
    }
    pub fn index_type(&self) -> IndexType {
        self.index_type
    }
    pub fn count(&self) -> u32 {
        self.count
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
impl fmt::Debug for PreparedIndices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedIndices")
            .field("index_type", &self.index_type)
            .field("count", &self.count)
            .field("capacity", &self.bytes.capacity())
            .finish_non_exhaustive()
    }
}

/// Real compiler products and retained target contents, never a canned shader.
/// Private fields prohibit substituting unrelated SPIR-V or target snapshots.
pub struct PreparedDraw {
    vertex: CompiledShader,
    pixel: CompiledShader,
    input: Vec<u8>,
    target_snapshot: Option<Vec<u8>>,
    target_geometry: TargetGeometry,
    vertex_push: [u8; 128],
    pixel_push: [u8; 128],
    target_address: u64,
    width: u32,
    height: u32,
    vertex_count: u32,
    instance_count: u32,
    native_offsets: Option<NativeIndirectOffsets>,
    shader_ranges: [CommandRange; 2],
    indices: Option<PreparedIndices>,
    vertex_buffer: Option<PreparedBuffer>,
    pixel_buffer: Option<PreparedBuffer>,
    pixel_images: Vec<PreparedTexture>,
    pixel_samplers: Vec<PreparedSampler>,
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
    pub fn pixel_images(&self) -> &[PreparedTexture] {
        &self.pixel_images
    }
    pub fn pixel_samplers(&self) -> &[PreparedSampler] {
        &self.pixel_samplers
    }
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
    pub fn target_geometry(&self) -> TargetGeometry {
        self.target_geometry
    }
    /// The original full padded LOAD snapshot. Linear targets reuse input.
    pub fn target_snapshot(&self) -> &[u8] {
        self.target_snapshot.as_deref().unwrap_or(&self.input)
    }
    /// Consume preparation after the backend fence to recover owned padded
    /// storage for an in-place checked reverse conversion. No allocation.
    pub fn into_target_snapshot(self) -> Vec<u8> {
        self.target_snapshot.unwrap_or(self.input)
    }
    pub fn vertex_count(&self) -> u32 {
        self.vertex_count
    }
    /// Indexed draws count index elements as the conservative vertex-work
    /// upper bound; the backend must use the separate actual index payload.
    pub fn indices(&self) -> Option<&PreparedIndices> {
        self.indices.as_ref()
    }
    pub fn vertex_buffer(&self) -> Option<&PreparedBuffer> {
        self.vertex_buffer.as_ref()
    }
    pub fn pixel_buffer(&self) -> Option<&PreparedBuffer> {
        self.pixel_buffer.as_ref()
    }
    /// The admitted compiler executes native guest fetches: patched SGPRs carry
    /// indirect offsets, so applying Vulkan offsets again would double them.
    pub fn vertex_offset(&self) -> i32 {
        0
    }
    pub fn native_indirect_offsets(&self) -> Option<NativeIndirectOffsets> {
        self.native_offsets
    }
    /// Check the opaque prepared mode against the actual compiler contracts.
    /// This does not waive any emitted feature or resource requirement.
    pub fn validate_native_instance_contract(&self) -> Result<(), Error> {
        use pentax_shader_compiler::VertexInstanceContract;
        let expected = if self.native_offsets.is_some() {
            VertexInstanceContract::NativeIndirectFirstInstanceZero
        } else {
            VertexInstanceContract::Dynamic
        };
        if self.vertex.manifest().vertex_instance_contract() == expected
            && self.pixel.manifest().vertex_instance_contract() == VertexInstanceContract::Dynamic
        {
            Ok(())
        } else {
            Err(Error::Unsupported("native indirect instance contract"))
        }
    }
    pub fn first_vertex(&self) -> u32 {
        0
    }
    pub fn instance_count(&self) -> u32 {
        self.instance_count
    }
    pub fn first_instance(&self) -> u32 {
        // Native indirect first-instance is already in the patched user SGPR.
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
        self.peak(bytes)?;
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_add(bytes)
            .ok_or(Error::Limit("host bytes"))?;
        Ok(())
    }
    fn peak(&mut self, temporary: usize) -> Result<(), Error> {
        let peak = self
            .usage
            .host_bytes
            .checked_add(temporary)
            .filter(|n| *n <= self.limits.max_host_bytes)
            .ok_or(Error::Limit("host bytes"))?;
        self.usage.peak_host_bytes = self.usage.peak_host_bytes.max(peak);
        Ok(())
    }
    fn release(&mut self, bytes: usize) -> Result<(), Error> {
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_sub(bytes)
            .ok_or(Error::Limit("host accounting"))?;
        Ok(())
    }
    fn pixels(&mut self, retained: usize, future: usize) -> Result<(), Error> {
        retained
            .checked_add(future)
            .filter(|n| *n <= self.limits.max_pixel_bytes)
            .ok_or(Error::Limit("pixel staging bytes"))?;
        self.usage.pixel_bytes = retained;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
