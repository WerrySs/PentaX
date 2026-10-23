// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: paused-owner guest draw preparation and writeback.
// Public KytyPS5/KytyPS5@7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3
// graphicsRun.cpp DrawIndexAuto/DrawIndex and renderDraw.cpp
// DrawAuto/DrawIndex/EmitDrawPrimitives.

//! Actual declared-layout guest draws. A draw completion is issued only after
//! its Vulkan fence AND transactional writeback on the original paused owner.
//! The one-level RenderTarget64KB path reuses VideoOut's exact pixel mapping.
//! Presentation completion, depth, blending and writable descriptors remain separate.

use pentax_gpu::{
    compute::CommandRange,
    graphics::{self, DecodedTarget, Draw, Limits, TargetGeometry, Usage},
};
use pentax_platform::{
    native_memory::{
        NATIVE_GUEST_PAGE, NativeMemoryError, NativeMemoryWrite, NativeOwnerId, NativeRange,
        NativeTransactionLimits, PausedNativeMemory,
    },
    vulkan::{GpuLimits, GpuUsage, VulkanError},
};
use std::{
    fmt,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_SUBMISSION: AtomicU64 = AtomicU64::new(1);

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "graphics/actual_draw_tests.rs"]
mod actual_draw_tests;

#[cfg(test)]
#[path = "graphics/tiled_tests.rs"]
mod tiled_tests;

#[derive(Debug)]
pub enum Error {
    Prepare(graphics::Error),
    Memory(NativeMemoryError),
    Vulkan(VulkanError),
    Limit,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepare(error) => write!(f, "graphics preparation: {error}"),
            Self::Memory(_) => f.write_str("graphics guest memory admission failed"),
            Self::Vulkan(error) => write!(f, "graphics execution: {error}"),
            Self::Limit => f.write_str("graphics execution budget exceeded"),
        }
    }
}
impl std::error::Error for Error {}

use super::video::{ImageLayout, PresentationLimits, SurfaceLayout, detile_32, retile_32_into};
pub use pentax_gpu::graphics::TargetLayout;

/// Nonforgeable within the public API. Numeric submission IDs and a public
/// report alone are not completion authority. This attests a completed draw,
/// not that its target remains unchanged after the owner resumes.
pub struct GraphicsCompletion {
    owner: NativeOwnerId,
    target: NativeRange,
    submission: u64,
    geometry: TargetGeometry,
}
impl fmt::Debug for GraphicsCompletion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphicsCompletion")
            .field("target_bytes", &self.target.size)
            .field("layout", &self.layout())
            .finish_non_exhaustive()
    }
}
impl GraphicsCompletion {
    pub fn owner(&self) -> NativeOwnerId {
        self.owner
    }
    pub fn target(&self) -> NativeRange {
        self.target
    }
    pub fn layout(&self) -> TargetLayout {
        self.geometry.layout()
    }
    pub fn geometry(&self) -> TargetGeometry {
        self.geometry
    }
    pub fn submission(&self) -> u64 {
        self.submission
    }
}

#[derive(Debug)]
pub struct Report {
    lod_counters: Option<pentax_gpu::lod_stats::Counters>,
    pub preparation: Usage,
    pub gpu: GpuUsage,
    pub written_bytes: usize,
    pub preflight_work: u64,
    pub completion: GraphicsCompletion,
}

/// Current execution authority retains the unique pause loan. It cannot be
/// constructed from a public Report or kept while its owner resumes. Returning
/// the pause consumes/drops the receipt; a flip consumes it exactly once.
pub(super) struct CurrentDraw<'borrow, 'owner> {
    memory: &'borrow mut PausedNativeMemory<'owner>,
    report: Report,
}
impl<'borrow, 'owner> CurrentDraw<'borrow, 'owner> {
    pub(super) fn take_lod_counters(&mut self) -> Option<pentax_gpu::lod_stats::Counters> {
        self.report.lod_counters.take()
    }
    pub(super) fn written_bytes(&self) -> usize {
        self.report.written_bytes
    }
    /// Shared observation retains the unique pause loan and current draw
    /// receipt. It grants neither writes nor an independently reusable pause.
    pub(super) fn paused_memory(&self) -> &PausedNativeMemory<'owner> {
        self.memory
    }
    pub(super) fn into_pause(self) -> &'borrow mut PausedNativeMemory<'owner> {
        self.memory
    }
    pub(super) fn consume<T>(
        self,
        action: impl FnOnce(
            &mut PausedNativeMemory<'owner>,
            &GraphicsCompletion,
        ) -> Result<T, super::driver::Failure>,
    ) -> (
        &'borrow mut PausedNativeMemory<'owner>,
        Result<T, super::driver::Failure>,
    ) {
        let result = action(self.memory, &self.report.completion);
        (self.memory, result)
    }
}
pub(super) fn execute_draw_current<'borrow, 'owner>(
    memory: &'borrow mut PausedNativeMemory<'owner>,
    draw: &Draw,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    command_ranges: &[CommandRange],
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<CurrentDraw<'borrow, 'owner>, Error> {
    let report = execute_draw_admitted(memory, draw, limits, gpu_limits, command_ranges, admit)?;
    Ok(CurrentDraw { memory, report })
}

pub(super) fn execute_draw_current_with_lod<'borrow, 'owner>(
    memory: &'borrow mut PausedNativeMemory<'owner>,
    draw: &Draw,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    command_ranges: &[CommandRange],
    initial: Option<&pentax_gpu::lod_stats::Counters>,
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<CurrentDraw<'borrow, 'owner>, Error> {
    if initial.is_none() {
        return execute_draw_current(memory, draw, limits, gpu_limits, command_ranges, admit);
    }
    let report = execute_draw_admitted_with_lod(
        memory,
        draw,
        limits,
        gpu_limits,
        command_ranges,
        initial,
        admit,
    )?;
    Ok(CurrentDraw { memory, report })
}

/// No Vulkan call happens until the entire destination and its disjointness
/// from retained command/shader/index and stage-buffer sources are admitted. The exclusive pause lasts across
/// preparation, execution, fence and all-or-nothing guest publication.
pub fn execute_draw(
    paused: &mut PausedNativeMemory<'_>,
    draw: &Draw,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    command_ranges: &[CommandRange],
) -> Result<Report, Error> {
    execute_draw_admitted(
        paused,
        draw,
        limits,
        gpu_limits,
        command_ranges,
        |_, _, _| Ok(()),
    )
}

/// Driver/session cumulative admission and poison marking must happen before
/// GPU work can occur. A later failure must not replay an earlier queue state.
pub(super) fn execute_draw_admitted(
    paused: &mut PausedNativeMemory<'_>,
    draw: &Draw,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    command_ranges: &[CommandRange],
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<Report, Error> {
    execute_draw_admitted_with_lod(
        paused,
        draw,
        limits,
        gpu_limits,
        command_ranges,
        None,
        admit,
    )
}

pub(super) fn execute_draw_admitted_with_lod(
    paused: &mut PausedNativeMemory<'_>,
    draw: &Draw,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    command_ranges: &[CommandRange],
    initial: Option<&pentax_gpu::lod_stats::Counters>,
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<Report, Error> {
    let prepared = graphics::prepare_draw_with_target_and_lod(
        draw,
        &super::Memory(paused),
        limits,
        initial.is_some(),
        |snapshot, geometry, remaining| {
            let mut work = 0;
            let bytes = detile_32(
                snapshot,
                image_layout(geometry),
                0,
                &PresentationLimits {
                    max_frame_bytes: limits.max_target_bytes,
                    max_held_bytes: remaining.max_host_bytes,
                    max_work: remaining.max_work,
                },
                &mut work,
            )
            .map_err(|error| match error {
                super::driver::Failure::Limit => graphics::Error::Limit("target LOAD conversion"),
                super::driver::Failure::Allocation => graphics::Error::Allocation,
                _ => graphics::Error::Unsupported("target LOAD conversion"),
            })?;
            Ok(DecodedTarget { bytes, work })
        },
    )
    .map_err(Error::Prepare)?;
    let geometry = prepared.target_geometry();
    let mut preparation = prepared.usage();
    let reverse_work = if geometry.layout() == TargetLayout::RenderTarget64KbRgba8 {
        (geometry.tight_bytes() as u64)
            .checked_mul(4)
            .ok_or(Error::Limit)?
    } else {
        0
    };
    // Reserve reverse-conversion work before GPU effects, including the full
    // offset-validation pass. Publication never discovers a new work quota.
    preparation.work = preparation
        .work
        .checked_add(reverse_work)
        .filter(|n| *n <= limits.max_work)
        .ok_or(Error::Limit)?;
    // The platform separately charges its upload and completed Vec capacities.
    // Compiler scratch is dead, and exact compiled products/LOAD/input remain.
    // Upload/readback must satisfy BOTH the independent graphics workspace
    // ceiling and the session's simultaneous pixel/transport ceiling.
    let remaining_host = limits
        .max_host_bytes
        .checked_sub(preparation.host_bytes)
        .ok_or(Error::Limit)?;
    let remaining_pixels = limits
        .max_pixel_bytes
        .checked_sub(preparation.pixel_bytes)
        .ok_or(Error::Limit)?;
    let mut execution_limits = *gpu_limits;
    execution_limits.max_host_bytes = execution_limits
        .max_host_bytes
        .min(remaining_host as u64)
        .min(remaining_pixels as u64);
    let texture_upload_bytes = prepared
        .pixel_images()
        .iter()
        .try_fold(0u64, |sum, texture| {
            sum.checked_add(texture.pixels().len() as u64)
        })
        .ok_or(Error::Limit)?;
    let counter_transfer_bytes = if prepared.pixel().manifest().lod_feedback_binding().is_some() {
        (pentax_gpu::lod_stats::COUNTER_BYTES as u64)
            .checked_mul(2)
            .ok_or(Error::Limit)?
    } else {
        0
    };
    let resource_upload_bytes = [prepared.vertex_buffer(), prepared.pixel_buffer()]
        .into_iter()
        .flatten()
        .try_fold(
            texture_upload_bytes
                .checked_add(counter_transfer_bytes)
                .ok_or(Error::Limit)?,
            |bytes, resource| bytes.checked_add(resource.bytes().len() as u64),
        )
        .ok_or(Error::Limit)?;
    if (geometry.tight_bytes() as u64)
        .checked_mul(2)
        .and_then(|n| {
            n.checked_add(
                prepared
                    .indices()
                    .map_or(0, |indices| indices.bytes().len() as u64),
            )
        })
        .and_then(|n| n.checked_add(resource_upload_bytes))
        .is_none_or(|n| n > execution_limits.max_host_bytes)
    {
        return Err(Error::Limit);
    }
    let target = NativeRange {
        address: prepared.target_address(),
        size: geometry.guest_bytes() as u64,
    };
    let remaining = limits
        .max_work
        .checked_sub(preparation.work)
        .ok_or(Error::Limit)?;
    let shader_ranges = prepared.shader_ranges();
    let mut work = preflight_target(paused, target, command_ranges, &shader_ranges, remaining)?;
    if let Some(indices) = prepared.indices() {
        let index_work = preflight_indices(
            paused,
            indices.range(),
            target,
            remaining.checked_sub(work).ok_or(Error::Limit)?,
        )?;
        work = work.checked_add(index_work).ok_or(Error::Limit)?;
    }
    if let Some(arguments) = draw.indirect_arguments() {
        // The live argument snapshot is data, not backing authority. Keep its
        // complete current owner/rights and physical output exclusion checked.
        let argument_work = preflight_read_source(
            paused,
            arguments,
            target,
            remaining.checked_sub(work).ok_or(Error::Limit)?,
        )?;
        work = work.checked_add(argument_work).ok_or(Error::Limit)?;
    }
    // Immutable copies alone do not authorize GPU access or target writeback.
    // Resolve both stages against this same paused owner and actual backing.
    // Their mutual read/read aliases remain legal and each upload is charged.
    for resource in [prepared.vertex_buffer(), prepared.pixel_buffer()]
        .into_iter()
        .flatten()
    {
        let resource_work = preflight_read_source(
            paused,
            resource.range(),
            target,
            remaining.checked_sub(work).ok_or(Error::Limit)?,
        )?;
        work = work.checked_add(resource_work).ok_or(Error::Limit)?;
    }
    // Complete same-owner permissions and physical alias exclusions also cover
    // each whole sampled-image backing, including source padding and mip tails.
    for resource in prepared.pixel_images() {
        let resource_work = preflight_read_source(
            paused,
            resource.source(),
            target,
            remaining.checked_sub(work).ok_or(Error::Limit)?,
        )?;
        work = work.checked_add(resource_work).ok_or(Error::Limit)?;
    }
    // Reserve the completion serial before effects. No post-write allocation,
    // increment overflow or other fallible step can lose a committed receipt.
    let submission = NEXT_SUBMISSION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |serial| {
            serial.checked_add(1)
        })
        .map_err(|_| Error::Limit)?;
    admit(preparation, work, true)?;
    let completed = match initial {
        Some(counters) => {
            pentax_platform::vulkan::run_graphics_with_lod(&prepared, &execution_limits, counters)
        }
        None => pentax_platform::vulkan::run_graphics(&prepared, &execution_limits),
    }
    .map_err(Error::Vulkan)?;
    let lod_counters = completed.lod_counters().cloned();
    if completed.bytes().len() != geometry.tight_bytes() {
        return Err(Error::Limit);
    }
    let mut baseline = prepared.into_target_snapshot();
    let bytes = if geometry.layout() == TargetLayout::RenderTarget64KbRgba8 {
        let mut work = 0;
        retile_32_into(
            completed.bytes(),
            image_layout(geometry),
            &mut baseline,
            &PresentationLimits {
                max_frame_bytes: limits.max_target_bytes,
                max_held_bytes: limits.max_pixel_bytes,
                max_work: reverse_work,
            },
            &mut work,
        )
        .map_err(|_| Error::Limit)?;
        baseline.as_slice()
    } else {
        completed.bytes()
    };
    // apply_gpu_writes performs its own complete permission/alias admission.
    // Metadata cannot change while this exact exclusive pause is retained.
    paused
        .apply_gpu_writes(
            &[NativeMemoryWrite {
                address: target.address,
                bytes,
            }],
            &NativeTransactionLimits {
                max_outputs: 1,
                max_output_bytes: target.size,
                max_preflight_steps: remaining,
                ..Default::default()
            },
        )
        .map_err(Error::Memory)?;
    Ok(Report {
        lod_counters,
        preparation,
        gpu: completed.usage(),
        written_bytes: geometry.guest_bytes(),
        preflight_work: work,
        completion: GraphicsCompletion {
            owner: paused.owner_id(),
            target,
            submission,
            geometry,
        },
    })
}

fn image_layout(geometry: TargetGeometry) -> ImageLayout {
    ImageLayout {
        kind: match geometry.layout() {
            TargetLayout::LinearRgba8 => SurfaceLayout::Linear,
            TargetLayout::RenderTarget64KbRgba8 => SurfaceLayout::RenderTarget64Kb,
        },
        width: geometry.width(),
        height: geometry.height(),
        pitch: geometry.pitch(),
    }
}

fn preflight_target(
    paused: &PausedNativeMemory<'_>,
    target: NativeRange,
    commands: &[CommandRange],
    shaders: &[CommandRange],
    maximum: u64,
) -> Result<u64, Error> {
    if target.size == 0 || target.address.checked_add(target.size).is_none() {
        return Err(Error::Limit);
    }
    // Charge the outer scan before visiting any supplied range. Backing helpers
    // charge all actual page comparisons independently under the same remainder.
    let last = target
        .address
        .checked_add(target.size - 1)
        .ok_or(Error::Limit)?;
    let pages = last / NATIVE_GUEST_PAGE - target.address / NATIVE_GUEST_PAGE + 1;
    let mut work = commands
        .len()
        .checked_add(shaders.len())
        .and_then(|n| u64::try_from(n).ok())
        .and_then(|n| {
            pages
                .checked_mul(2)
                .and_then(|visits| n.checked_add(visits))
        })
        .ok_or(Error::Limit)?;
    if work > maximum {
        return Err(Error::Limit);
    }
    paused
        .preflight_gpu_read(target.address, target.size)
        .map_err(Error::Memory)?;
    paused
        .preflight_gpu_write(target.address, target.size)
        .map_err(Error::Memory)?;
    let distinct = paused
        .preflight_distinct_backing(
            target.address,
            target.size,
            maximum.checked_sub(work).ok_or(Error::Limit)?,
        )
        .map_err(Error::Memory)?;
    work = work.checked_add(distinct).ok_or(Error::Limit)?;
    for command in commands.iter().chain(shaders) {
        let charge = paused
            .preflight_disjoint_backing(
                target,
                NativeRange {
                    address: command.address,
                    size: command.size,
                },
                maximum.checked_sub(work).ok_or(Error::Limit)?,
            )
            .map_err(Error::Memory)?;
        work = work.checked_add(charge).ok_or(Error::Limit)?;
    }
    Ok(work)
}

fn preflight_indices(
    paused: &PausedNativeMemory<'_>,
    indices: CommandRange,
    target: NativeRange,
    maximum: u64,
) -> Result<u64, Error> {
    preflight_read_source(paused, indices, target, maximum)
}

fn preflight_read_source(
    paused: &PausedNativeMemory<'_>,
    source: CommandRange,
    target: NativeRange,
    maximum: u64,
) -> Result<u64, Error> {
    let last = source
        .address
        .checked_add(source.size.checked_sub(1).ok_or(Error::Limit)?)
        .ok_or(Error::Limit)?;
    let pages = last / NATIVE_GUEST_PAGE - source.address / NATIVE_GUEST_PAGE + 1;
    let mut work = pages
        .checked_add(1)
        .filter(|n| *n <= maximum)
        .ok_or(Error::Limit)?;
    paused
        .preflight_gpu_read(source.address, source.size)
        .map_err(Error::Memory)?;
    let range = NativeRange {
        address: source.address,
        size: source.size,
    };
    let cost = paused
        .preflight_disjoint_backing(
            range,
            target,
            maximum.checked_sub(work).ok_or(Error::Limit)?,
        )
        .map_err(Error::Memory)?;
    work = work.checked_add(cost).ok_or(Error::Limit)?;
    Ok(work)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

    #[test]
    fn target_admission_rejects_command_overlap_and_gpu_permissions_without_writes() {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        owner
            .map_at(
                base + 0x4000,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(true, false),
            )
            .unwrap();
        let mut paused = owner.pause().unwrap();
        paused.write(base, &[0xa5; 64]).unwrap();
        let target = NativeRange {
            address: base,
            size: 64,
        };
        assert!(preflight_target(&paused, target, &[], &[], 4096).is_ok());
        assert!(matches!(
            preflight_target(
                &paused,
                target,
                &[CommandRange {
                    address: base + 60,
                    size: 16
                }],
                &[],
                4096
            ),
            Err(Error::Memory(NativeMemoryError::Overlap))
        ));
        assert!(matches!(
            preflight_target(
                &paused,
                NativeRange {
                    address: base + 0x4000,
                    size: 64
                },
                &[],
                &[],
                4096
            ),
            Err(Error::Memory(_))
        ));
        assert!(
            preflight_target(
                &paused,
                target,
                &[CommandRange {
                    address: base + 256,
                    size: 4
                }],
                &[],
                0
            )
            .is_err()
        );
        assert!(matches!(
            preflight_target(
                &paused,
                target,
                &[],
                &[CommandRange {
                    address: base + 60,
                    size: 16
                }],
                4096
            ),
            Err(Error::Memory(NativeMemoryError::Overlap))
        ));
        let mut actual = [0; 64];
        paused.read(base, &mut actual).unwrap();
        assert_eq!(actual, [0xa5; 64]);
    }
}
