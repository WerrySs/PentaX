// SPDX-License-Identifier: GPL-2.0-only

//! Synchronous command/resource bridge over one exclusive native guest pause.
//! No source/backing pairing, host pointers or running CPU can be substituted.
use pentax_gpu::compute::{self, Limits, PreparedCompute, Usage};
use pentax_platform::{
    native_memory::{
        NativeMemoryError, NativeMemoryWrite, NativeRange, NativeTransactionLimits,
        PausedNativeMemory,
    },
    vulkan::{GpuLimits, GpuUsage, VulkanError, run_compute},
};
use std::fmt;

mod agc_defaults;
pub mod driver;
pub mod graphics;
pub mod shaders;
pub mod video;

#[derive(Debug)]
pub enum Error {
    Prepare(compute::Error),
    Memory(NativeMemoryError),
    Vulkan(VulkanError),
    Allocation,
    Limit,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prepare(e) => write!(f, "GPU preparation: {e}"),
            Self::Memory(_) => f.write_str("GPU guest memory admission failed"),
            Self::Vulkan(e) => write!(f, "GPU execution: {e}"),
            Self::Allocation => f.write_str("GPU command allocation failed"),
            Self::Limit => f.write_str("GPU command budget exceeded"),
        }
    }
}
impl std::error::Error for Error {}
#[derive(Clone, Copy, Debug)]
pub struct Report {
    pub preparation: Usage,
    pub gpu: GpuUsage,
    pub buffer_bytes: usize,
    pub written_bytes: usize,
    pub preflight_work: u64,
}
struct Memory<'a, 'owner>(&'a PausedNativeMemory<'owner>);
impl compute::Memory for Memory<'_, '_> {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), compute::MemoryError> {
        self.0.read_gpu(address, output).map_err(|e| match e {
            NativeMemoryError::PermissionDenied(_) => compute::MemoryError::ReadDenied,
            _ => compute::MemoryError::Unavailable,
        })
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), compute::MemoryError> {
        if read {
            self.0
                .preflight_gpu_read(address, output.len() as u64)
                .map_err(|error| match error {
                    NativeMemoryError::PermissionDenied(_) => compute::MemoryError::ReadDenied,
                    _ => compute::MemoryError::Unavailable,
                })?;
        }
        if written {
            self.0
                .preflight_gpu_write(address, output.len() as u64)
                .map_err(|error| match error {
                    NativeMemoryError::PermissionDenied(_) => compute::MemoryError::WriteDenied,
                    _ => compute::MemoryError::Unavailable,
                })?;
        }
        self.0
            .snapshot_gpu_buffer(address, output, read, written)
            .map_err(|_| compute::MemoryError::Unavailable)
    }
}
/// Prepare and run while keeping this exact owner paused through fence and
/// writeback. Failed preparation/admission publishes no GPU or guest effects.
pub fn execute_compute(
    paused: &mut PausedNativeMemory<'_>,
    commands: NativeRange,
    limits: &Limits,
    gpu_limits: &GpuLimits,
) -> Result<Report, Error> {
    execute_compute_admitted(paused, commands, limits, gpu_limits, |_, _, _| Ok(()))
}

// Session accounting runs after preparation and memory admission, before any
// Vulkan work. A failed execution keeps its conservative admission charged.
fn execute_compute_admitted(
    paused: &mut PausedNativeMemory<'_>,
    commands: NativeRange,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<Report, Error> {
    let length = usize::try_from(commands.size).map_err(|_| Error::Limit)?;
    if length == 0 || length > limits.max_command_bytes || length > limits.max_host_bytes {
        return Err(Error::Limit);
    }
    paused
        .preflight_gpu_read(commands.address, commands.size)
        .map_err(Error::Memory)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| Error::Allocation)?;
    if bytes.capacity() > limits.max_host_bytes || bytes.capacity() > limits.max_command_bytes {
        return Err(Error::Limit);
    }
    bytes.resize(length, 0);
    paused
        .read_gpu(commands.address, &mut bytes)
        .map_err(Error::Memory)?;
    let excess = bytes.capacity() - bytes.len();
    let mut preparation_limits = *limits;
    preparation_limits.max_host_bytes = limits
        .max_host_bytes
        .checked_sub(excess)
        .ok_or(Error::Limit)?;
    let prepared: PreparedCompute =
        compute::prepare(&bytes, &Memory(paused), &preparation_limits).map_err(Error::Prepare)?;
    let mut preparation = prepared.usage();
    preparation.host_bytes = preparation
        .host_bytes
        .checked_add(excess)
        .ok_or(Error::Limit)?;
    run_prepared(
        paused,
        prepared,
        preparation,
        limits,
        gpu_limits,
        &[],
        admit,
    )
}

fn execute_dispatch_admitted(
    paused: &mut PausedNativeMemory<'_>,
    dispatch: &compute::Dispatch,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    commands: &[compute::CommandRange],
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<Report, Error> {
    let outcome =
        compute::prepare_dispatch(dispatch, &Memory(paused), limits).map_err(Error::Prepare)?;
    let (product, preparation) = outcome.into_parts();
    let prepared = match product {
        Some(prepared) => prepared,
        None => {
            admit(preparation, 0, false)?;
            return Ok(Report {
                preparation,
                gpu: GpuUsage::default(),
                buffer_bytes: 0,
                written_bytes: 0,
                preflight_work: 0,
            });
        }
    };
    run_prepared(
        paused,
        prepared,
        preparation,
        limits,
        gpu_limits,
        commands,
        admit,
    )
}

fn run_prepared(
    paused: &mut PausedNativeMemory<'_>,
    prepared: PreparedCompute,
    preparation: Usage,
    limits: &Limits,
    gpu_limits: &GpuLimits,
    commands: &[compute::CommandRange],
    admit: impl FnOnce(Usage, u64, bool) -> Result<(), Error>,
) -> Result<Report, Error> {
    let address = prepared.buffer_address();
    let size = prepared.input().len() as u64;
    let remainder = limits
        .max_work
        .checked_sub(prepared.usage().work)
        .ok_or(Error::Limit)?;
    let mut work = paused
        .preflight_distinct_backing(address, size, remainder)
        .map_err(Error::Memory)?;
    if prepared.writes_buffer() {
        paused
            .preflight_gpu_write(address, size)
            .map_err(Error::Memory)?;
        for command in commands {
            let charge = paused
                .preflight_disjoint_backing(
                    NativeRange { address, size },
                    NativeRange {
                        address: command.address,
                        size: command.size,
                    },
                    remainder.checked_sub(work).ok_or(Error::Limit)?,
                )
                .map_err(Error::Memory)?;
            work = work.checked_add(charge).ok_or(Error::Limit)?;
        }
    }
    admit(preparation, work, true)?;
    let completed = run_compute(&prepared, gpu_limits).map_err(Error::Vulkan)?;
    let mut written_bytes = 0;
    if prepared.writes_buffer() {
        if completed.bytes().len() != size as usize {
            return Err(Error::Limit);
        }
        paused
            .apply_gpu_writes(
                &[NativeMemoryWrite {
                    address,
                    bytes: completed.bytes(),
                }],
                &NativeTransactionLimits {
                    max_outputs: 1,
                    max_output_bytes: size,
                    max_preflight_steps: work,
                    ..Default::default()
                },
            )
            .map_err(Error::Memory)?;
        written_bytes = size as usize;
    }
    Ok(Report {
        preparation,
        gpu: completed.usage(),
        buffer_bytes: size as usize,
        written_bytes,
        preflight_work: work,
    })
}
