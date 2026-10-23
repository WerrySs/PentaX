// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned Rust Vulkan port; see NOTICE.

//! Owned Vulkan resources ported from the pinned Kyty renderer. Public submission
//! accepts closed original factories or opaque prepared compiler products.
//! No caller supplies SPIR-V, Vulkan handles, guest addresses or host pointers.
//! Driver calls are an unsafe boundary, not a shader or process sandbox.

use std::fmt;

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod backend;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod commands;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod compute;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod device;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod graphics;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod lod_stats;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod pipeline;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod programs;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod resources;

/// Fenced readback from a checked guest-command compiler product. Debug keeps
/// guest bytes private; only explicit callers can inspect the returned payload.
pub struct ComputeBufferReadback {
    resource_index: usize,
    bytes: Vec<u8>,
}
impl ComputeBufferReadback {
    pub fn resource_index(&self) -> usize {
        self.resource_index
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
pub struct ComputeReadback {
    resource_count: usize,
    buffers: Vec<ComputeBufferReadback>,
    usage: GpuUsage,
}
impl std::fmt::Debug for ComputeReadback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputeReadback")
            .field("resources", &self.resource_count)
            .field("outputs", &self.buffers.len())
            .field("usage", &self.usage)
            .finish()
    }
}
impl ComputeReadback {
    pub fn resource_count(&self) -> usize {
        self.resource_count
    }
    pub fn outputs(&self) -> &[ComputeBufferReadback] {
        &self.buffers
    }
    /// Exact legacy single-output view, never a successful fabricated empty payload.
    pub fn bytes(&self) -> Option<&[u8]> {
        if let [buffer] = self.buffers.as_slice() {
            (self.resource_count == 1 && buffer.resource_index == 0)
                .then_some(buffer.bytes.as_slice())
        } else {
            None
        }
    }
    pub fn usage(&self) -> GpuUsage {
        self.usage
    }
}

/// Execute a bounded prepared compute product, never caller-supplied SPIR-V.
/// Keep the guest paused until the readback is committed on its original owner.
pub fn run_compute(
    prepared: &pentax_gpu::compute::PreparedCompute,
    limits: &GpuLimits,
) -> Result<ComputeReadback, VulkanError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        compute::run(prepared, *limits)
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = (prepared, limits);
        Err(VulkanError::UnsupportedHost)
    }
}

#[cfg(test)]
mod compute_readback_tests {
    use super::*;

    #[test]
    fn single_output_of_a_multi_resource_dispatch_is_not_a_legacy_single_buffer() {
        let mut result = ComputeReadback {
            resource_count: 2,
            buffers: vec![ComputeBufferReadback {
                resource_index: 0,
                bytes: vec![1, 2, 3, 4],
            }],
            usage: GpuUsage::default(),
        };
        assert_eq!(result.resource_count(), 2);
        assert_eq!(result.outputs()[0].resource_index(), 0);
        assert_eq!(result.bytes(), None);
        result.resource_count = 1;
        assert_eq!(result.bytes(), Some(&[1, 2, 3, 4][..]));
        result.buffers.clear();
        assert_eq!(result.bytes(), None); // A read-only dispatch has no fabricated output.
    }
}

/// Fenced RGBA8 result of an actual prepared guest draw. Construction is private:
/// callers cannot substitute arbitrary pixels for a completed GPU submission.
pub struct GraphicsReadback {
    bytes: Vec<u8>,
    usage: GpuUsage,
    lod_counters: Option<pentax_gpu::lod_stats::Counters>,
}
impl fmt::Debug for GraphicsReadback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphicsReadback")
            .field("bytes", &self.bytes.len())
            .field("usage", &self.usage)
            .finish()
    }
}
impl GraphicsReadback {
    /// Present only when this call read actual counter words after its GPU fence.
    pub fn lod_counters(&self) -> Option<&pentax_gpu::lod_stats::Counters> {
        self.lod_counters.as_ref()
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn usage(&self) -> GpuUsage {
        self.usage
    }
}

/// Upload the actual linear target, execute its prepared guest vertex/pixel
/// shaders and return fenced pixels. An indexed product uploads its actual
/// U16/U32 snapshot and uses a triangle-list indexed draw with no restart or
/// base-vertex offset. Optional read-only VS/PS raw buffers use the exact owned
/// snapshots and stage manifest bindings. The initialized-LOD entry additionally
/// admits only the exact manifest counter buffer.
/// This does not present a window or accept a tiled surface. Keep the original
/// guest owner paused through writeback.
pub fn run_graphics(
    prepared: &pentax_gpu::graphics::PreparedDraw,
    limits: &GpuLimits,
) -> Result<GraphicsReadback, VulkanError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        graphics::run(prepared, *limits)
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = (prepared, limits);
        Err(VulkanError::UnsupportedHost)
    }
}

/// Continue an explicitly initialized logical counter interval through an actual
/// instrumented draw. Initial words are data, not completion or guest authority.
pub fn run_graphics_with_lod(
    prepared: &pentax_gpu::graphics::PreparedDraw,
    limits: &GpuLimits,
    initial: &pentax_gpu::lod_stats::Counters,
) -> Result<GraphicsReadback, VulkanError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        graphics::run_with_lod(prepared, *limits, Some(initial))
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = (prepared, limits, initial);
        Err(VulkanError::UnsupportedHost)
    }
}

/// Initialize both counter planes with real GPU fills and fenced readback.
/// This never writes a guest report or supplies a report-ready marker.
pub fn run_lod_reset(limits: &GpuLimits) -> Result<pentax_gpu::lod_stats::Counters, VulkanError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        lod_stats::reset(*limits)
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = limits;
        Err(VulkanError::UnsupportedHost)
    }
}

/// Original programs assembled in Rust, never a game or firmware shader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginalGpuFixture {
    /// Eight invocations write their lane index plus nine to a real SSBO.
    ComputeBuffer,
    /// A vertex/pixel pair draws an opaque red full-screen triangle into RGBA8.
    TriangleRgba,
}

/// Retained admission policy. Driver-internal allocations are outside these
/// application-visible byte/object budgets. A fence timeout cannot bound a
/// malfunctioning driver call or the final safe device-idle cleanup.
#[derive(Clone, Copy, Debug)]
pub struct GpuLimits {
    pub max_device_bytes: u64,
    pub max_host_bytes: u64,
    pub max_shader_words: usize,
    pub max_objects: u32,
    pub max_commands: u32,
    pub max_work: u64,
    /// Conservatively charged separately for decode and compile, per shader.
    pub compiler_phase_bytes: usize,
    /// Conservatively charged separately for decode and compile, per shader.
    pub compiler_phase_work: u64,
    pub fence_timeout_ns: u64,
}

impl Default for GpuLimits {
    fn default() -> Self {
        Self {
            max_device_bytes: 64 * 1024 * 1024,
            max_host_bytes: 128 * 1024 * 1024,
            max_shader_words: 1024 * 1024,
            max_objects: 64,
            max_commands: 128,
            max_work: 64 * 1024 * 1024,
            compiler_phase_bytes: 8 * 1024 * 1024,
            compiler_phase_work: 4 * 1024 * 1024,
            fence_timeout_ns: 5_000_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuUsage {
    pub device_bytes: u64,
    pub host_bytes: u64,
    pub shader_words: usize,
    pub objects: u32,
    pub commands: u32,
    pub work: u64,
}

/// Readback is published only after the fence and all expected values agree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuReport {
    fixture: OriginalGpuFixture,
    bytes: Vec<u8>,
    usage: GpuUsage,
}

impl GpuReport {
    pub const fn fixture(&self) -> OriginalGpuFixture {
        self.fixture
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub const fn usage(&self) -> GpuUsage {
        self.usage
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VulkanError {
    UnsupportedHost,
    LoaderUnavailable,
    Unsupported(&'static str),
    Limit(&'static str),
    Allocation,
    Compile,
    InvalidCompilerContract(&'static str),
    Driver { operation: &'static str, code: i32 },
    UnexpectedReadback,
}

impl fmt::Display for VulkanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHost => f.write_str("Vulkan diagnostic requires Windows x86-64"),
            Self::LoaderUnavailable => f.write_str("system Vulkan loader is unavailable"),
            Self::Unsupported(s) => write!(f, "unsupported Vulkan profile: {s}"),
            Self::Limit(s) => write!(f, "Vulkan admission limit: {s}"),
            Self::Allocation => f.write_str("Vulkan host allocation failed"),
            Self::Compile => f.write_str("original AMD shader compilation failed"),
            Self::InvalidCompilerContract(s) => write!(f, "unsupported compiler contract: {s}"),
            Self::Driver { operation, code } => {
                write!(f, "{operation} failed with Vulkan code {code}")
            }
            Self::UnexpectedReadback => {
                f.write_str("GPU readback disagreed with the original program")
            }
        }
    }
}
impl std::error::Error for VulkanError {}

/// Execute only the selected closed original factory. No Vulkan API is called
/// by validation-only unit tests. Run this diagnostic in an isolated worker.
pub fn run_original(
    fixture: OriginalGpuFixture,
    limits: &GpuLimits,
) -> Result<GpuReport, VulkanError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        backend::run(fixture, *limits)
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        let _ = (fixture, limits);
        Err(VulkanError::UnsupportedHost)
    }
}

#[cfg(any(all(target_os = "windows", target_arch = "x86_64"), test))]
struct Budget {
    limits: GpuLimits,
    usage: GpuUsage,
}
#[cfg(any(all(target_os = "windows", target_arch = "x86_64"), test))]
impl Budget {
    fn new(limits: GpuLimits) -> Self {
        Self {
            limits,
            usage: GpuUsage::default(),
        }
    }
    fn device(&mut self, bytes: u64) -> Result<(), VulkanError> {
        let next = self
            .usage
            .device_bytes
            .checked_add(bytes)
            .ok_or(VulkanError::Limit("device bytes"))?;
        if next > self.limits.max_device_bytes {
            return Err(VulkanError::Limit("device bytes"));
        }
        self.usage.device_bytes = next;
        Ok(())
    }
    fn host(&mut self, bytes: u64) -> Result<(), VulkanError> {
        let next = self
            .usage
            .host_bytes
            .checked_add(bytes)
            .ok_or(VulkanError::Limit("host bytes"))?;
        if next > self.limits.max_host_bytes {
            return Err(VulkanError::Limit("host bytes"));
        }
        self.usage.host_bytes = next;
        Ok(())
    }
    fn work(&mut self, units: u64) -> Result<(), VulkanError> {
        let next = self
            .usage
            .work
            .checked_add(units)
            .ok_or(VulkanError::Limit("work"))?;
        if next > self.limits.max_work {
            return Err(VulkanError::Limit("work"));
        }
        self.usage.work = next;
        Ok(())
    }
    fn object(&mut self) -> Result<(), VulkanError> {
        let next = self
            .usage
            .objects
            .checked_add(1)
            .ok_or(VulkanError::Limit("objects"))?;
        if next > self.limits.max_objects {
            return Err(VulkanError::Limit("objects"));
        }
        self.usage.objects = next;
        self.work(1)
    }
    fn command(&mut self, count: u32) -> Result<(), VulkanError> {
        let next = self
            .usage
            .commands
            .checked_add(count)
            .ok_or(VulkanError::Limit("commands"))?;
        if next > self.limits.max_commands {
            return Err(VulkanError::Limit("commands"));
        }
        self.usage.commands = next;
        self.work(u64::from(count))
    }
    fn shader(&mut self, words: usize) -> Result<(), VulkanError> {
        let next = self
            .usage
            .shader_words
            .checked_add(words)
            .ok_or(VulkanError::Limit("shader words"))?;
        if next > self.limits.max_shader_words {
            return Err(VulkanError::Limit("shader words"));
        }
        self.usage.shader_words = next;
        self.work(words as u64)
    }
    fn buffer(&mut self, bytes: usize) -> Result<Vec<u8>, VulkanError> {
        self.host(bytes as u64)?;
        self.work(bytes as u64)?;
        let mut out = Vec::new();
        out.try_reserve_exact(bytes)
            .map_err(|_| VulkanError::Allocation)?;
        // Allocators may round reserve_exact upward. Retain and charge the
        // actual capacity, including owned draw readback, rather than its len.
        self.host((out.capacity() - bytes) as u64)?;
        out.resize(bytes, 0);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_limits_are_checked_before_charging_or_allocating() {
        let mut budget = Budget::new(GpuLimits {
            max_device_bytes: 7,
            max_host_bytes: 3,
            max_work: 8,
            max_objects: 1,
            max_commands: 2,
            max_shader_words: 2,
            ..GpuLimits::default()
        });
        assert_eq!(budget.buffer(4), Err(VulkanError::Limit("host bytes")));
        assert_eq!(budget.usage.host_bytes, 0);
        budget.device(7).unwrap();
        assert_eq!(budget.device(1), Err(VulkanError::Limit("device bytes")));
        budget.object().unwrap();
        assert_eq!(budget.object(), Err(VulkanError::Limit("objects")));
        budget.command(2).unwrap();
        assert_eq!(budget.command(1), Err(VulkanError::Limit("commands")));
        budget.shader(2).unwrap();
        assert_eq!(budget.shader(1), Err(VulkanError::Limit("shader words")));
        assert_eq!(budget.work(u64::MAX), Err(VulkanError::Limit("work")));
    }
}
