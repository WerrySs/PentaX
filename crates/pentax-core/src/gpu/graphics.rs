// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: paused-owner guest draw preparation and writeback.
// Public KytyPS5/KytyPS5@7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3
// graphicsRun.cpp DrawIndexAuto and renderDraw.cpp DrawAuto/EmitDrawPrimitives.

//! Actual linear-target guest draws. A draw completion is issued only after
//! its Vulkan fence AND transactional writeback on the original paused owner.
//! Tiled registration/presentation, depth, blending and descriptor resources
//! remain separate explicitly unsupported profiles, not successful no-ops.

use pentax_gpu::{
    compute::CommandRange,
    graphics::{self, Draw, Limits, Usage},
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

/// The exact data representation written by this execution slice. It cannot be
/// substituted for a registered RenderTarget64KB surface without real conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetLayout {
    LinearRgba8,
}

/// Nonforgeable within the public API. Numeric submission IDs and a public
/// report alone are not completion authority. This attests a completed draw,
/// not that its target remains unchanged after the owner resumes.
pub struct GraphicsCompletion {
    owner: NativeOwnerId,
    target: NativeRange,
    submission: u64,
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
        TargetLayout::LinearRgba8
    }
    pub fn submission(&self) -> u64 {
        self.submission
    }
}

#[derive(Debug)]
pub struct Report {
    pub preparation: Usage,
    pub gpu: GpuUsage,
    pub written_bytes: usize,
    pub preflight_work: u64,
    pub completion: GraphicsCompletion,
}

/// No Vulkan call happens until the entire destination and its disjointness
/// from retained command sources are admitted. The exclusive pause lasts across
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
    let prepared =
        graphics::prepare_draw(draw, &super::Memory(paused), limits).map_err(Error::Prepare)?;
    let preparation = prepared.usage();
    let target = NativeRange {
        address: prepared.target_address(),
        size: prepared.input().len() as u64,
    };
    let remaining = limits
        .max_work
        .checked_sub(preparation.work)
        .ok_or(Error::Limit)?;
    let shader_ranges = prepared.shader_ranges();
    let work = preflight_target(paused, target, command_ranges, &shader_ranges, remaining)?;
    // Reserve the completion serial before effects. No post-write allocation,
    // increment overflow or other fallible step can lose a committed receipt.
    let submission = NEXT_SUBMISSION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |serial| {
            serial.checked_add(1)
        })
        .map_err(|_| Error::Limit)?;
    admit(preparation, work, true)?;
    let completed =
        pentax_platform::vulkan::run_graphics(&prepared, gpu_limits).map_err(Error::Vulkan)?;
    if completed.bytes().len() != prepared.input().len() {
        return Err(Error::Limit);
    }
    // apply_gpu_writes performs its own complete permission/alias admission.
    // Metadata cannot change while this exact exclusive pause is retained.
    paused
        .apply_gpu_writes(
            &[NativeMemoryWrite {
                address: target.address,
                bytes: completed.bytes(),
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
        preparation,
        gpu: completed.usage(),
        written_bytes: prepared.input().len(),
        preflight_work: work,
        completion: GraphicsCompletion {
            owner: paused.owner_id(),
            target,
            submission,
        },
    })
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
