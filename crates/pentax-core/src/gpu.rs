// SPDX-License-Identifier: GPL-2.0-only

//! Synchronous command/resource bridge over one exclusive native guest pause.
//! No source/backing pairing, host pointers or running CPU can be substituted.
use pentax_gpu::compute::{self, Limits, PreparedCompute, Usage};
use pentax_platform::{
    native_memory::{
        NATIVE_GUEST_PAGE, NativeMemoryError, NativeMemoryWrite, NativeRange,
        NativeTransactionLimits, PausedNativeMemory,
    },
    vulkan::{GpuLimits, GpuUsage, VulkanError, run_compute},
};
use std::fmt;

mod agc_defaults;
pub mod driver;
pub mod graphics;
mod private_compute_shader_failure;
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
struct CommandMemory<'a, 'owner> {
    paused: &'a PausedNativeMemory<'owner>,
    commands: &'a [compute::CommandRange],
}
fn preflight_resources(
    memory: &PausedNativeMemory<'_>,
    count: usize,
    resource: impl Fn(usize) -> (u64, u64, bool, bool),
    commands: &[compute::CommandRange],
    maximum_work: u64,
) -> Result<u64, Error> {
    if count > 4 {
        return Err(Error::Limit);
    }
    memory.preflight_gpu_read(0, 0).map_err(Error::Memory)?;
    let mut work = 0u64;
    for index in 0..count {
        let (address, size, read, written) = resource(index);
        let charge = memory
            .preflight_distinct_backing(
                address,
                size,
                maximum_work.checked_sub(work).ok_or(Error::Limit)?,
            )
            .map_err(Error::Memory)?;
        work = work.checked_add(charge).ok_or(Error::Limit)?;
        if read {
            memory
                .preflight_gpu_read(address, size)
                .map_err(Error::Memory)?;
        }
        if written {
            memory
                .preflight_gpu_write(address, size)
                .map_err(Error::Memory)?;
        }
        for other in 0..index {
            let (at, bytes, _, writes) = resource(other);
            if written || writes {
                let charge = memory
                    .preflight_disjoint_backing(
                        NativeRange { address, size },
                        NativeRange {
                            address: at,
                            size: bytes,
                        },
                        maximum_work.checked_sub(work).ok_or(Error::Limit)?,
                    )
                    .map_err(Error::Memory)?;
                work = work.checked_add(charge).ok_or(Error::Limit)?;
            }
        }
        if written {
            for command in commands {
                let charge = memory
                    .preflight_disjoint_backing(
                        NativeRange { address, size },
                        NativeRange {
                            address: command.address,
                            size: command.size,
                        },
                        maximum_work.checked_sub(work).ok_or(Error::Limit)?,
                    )
                    .map_err(Error::Memory)?;
                work = work.checked_add(charge).ok_or(Error::Limit)?;
            }
        }
    }
    Ok(work)
}
fn preflight_native_buffers(
    paused: &PausedNativeMemory<'_>,
    buffers: &[compute::resources::BufferSpan],
    commands: &[compute::CommandRange],
    work: u64,
) -> Result<u64, compute::Error> {
    for span in buffers {
        let access = span.requirement();
        if access.read {
            paused
                .preflight_gpu_read(span.address(), span.byte_count() as u64)
                .map_err(|_| compute::Error::Memory(compute::MemoryError::ReadDenied))?;
        }
        if access.written {
            paused
                .preflight_gpu_write(span.address(), span.byte_count() as u64)
                .map_err(|_| compute::Error::Memory(compute::MemoryError::WriteDenied))?;
        }
    }
    preflight_resources(
        paused,
        buffers.len(),
        |index| {
            let span = buffers[index];
            let access = span.requirement();
            (
                span.address(),
                span.byte_count() as u64,
                access.read,
                access.written,
            )
        },
        commands,
        work,
    )
    .map_err(|error| match error {
        Error::Limit | Error::Memory(NativeMemoryError::BudgetExceeded) => {
            compute::Error::Limit("compute resource preflight")
        }
        Error::Memory(NativeMemoryError::PermissionDenied(_)) => {
            compute::Error::Memory(compute::MemoryError::WriteDenied)
        }
        _ => compute::Error::Memory(compute::MemoryError::Unavailable),
    })
}

impl compute::Memory for CommandMemory<'_, '_> {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), compute::MemoryError> {
        compute::Memory::read(&Memory(self.paused), address, output)
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), compute::MemoryError> {
        compute::Memory::snapshot_buffer(&Memory(self.paused), address, output, read, written)
    }
    fn preflight_buffers(
        &self,
        buffers: &[compute::resources::BufferSpan],
        work: u64,
    ) -> Result<u64, compute::Error> {
        preflight_native_buffers(self.paused, buffers, self.commands, work)
    }
}
impl compute::Memory for Memory<'_, '_> {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), compute::MemoryError> {
        self.0.read_gpu(address, output).map_err(|e| match e {
            NativeMemoryError::PermissionDenied(_) => compute::MemoryError::ReadDenied,
            _ => compute::MemoryError::Unavailable,
        })
    }
    fn preflight_buffers(
        &self,
        buffers: &[compute::resources::BufferSpan],
        work: u64,
    ) -> Result<u64, compute::Error> {
        preflight_native_buffers(self.0, buffers, &[], work)
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
    let prepared: PreparedCompute = compute::prepare(
        &bytes,
        &CommandMemory {
            paused,
            commands: &[compute::CommandRange {
                address: commands.address,
                size: commands.size,
            }],
        },
        &preparation_limits,
    )
    .map_err(Error::Prepare)?;
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
        &[compute::CommandRange {
            address: commands.address,
            size: commands.size,
        }],
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
        match compute::prepare_dispatch(dispatch, &CommandMemory { paused, commands }, limits) {
            Ok(outcome) => outcome,
            Err(error) => {
                private_compute_shader_failure::capture_from_env(paused, dispatch, &error);
                return Err(Error::Prepare(error));
            }
        };
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

fn output_transaction_work(buffers: &[compute::PreparedBuffer]) -> Result<u64, Error> {
    let mut pages = 0u64;
    let mut count = 0u64;
    for buffer in buffers.iter().filter(|buffer| buffer.written()) {
        let bytes = buffer.input().len() as u64;
        let end = buffer
            .address()
            .checked_add(bytes)
            .filter(|end| bytes != 0 && *end != 0)
            .ok_or(Error::Limit)?;
        let visits = (end - 1) / NATIVE_GUEST_PAGE - buffer.address() / NATIVE_GUEST_PAGE + 1;
        pages = pages.checked_add(visits).ok_or(Error::Limit)?;
        count = count.checked_add(1).ok_or(Error::Limit)?;
    }
    count
        .checked_mul(count)
        .and_then(|n| n.checked_add(pages.checked_mul(4)?))
        .and_then(|n| n.checked_add(pages.checked_mul(pages)?))
        .ok_or(Error::Limit)
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
    let buffers = prepared.buffers();
    let remainder = limits
        .max_work
        .checked_sub(prepared.usage().work)
        .ok_or(Error::Limit)?;
    let work = preflight_resources(
        paused,
        buffers.len(),
        |index| {
            let buffer = &buffers[index];
            (
                buffer.address(),
                buffer.input().len() as u64,
                buffer.read(),
                buffer.written(),
            )
        },
        commands,
        remainder,
    )?;
    let mut total = 0usize;
    let mut written_bytes = 0usize;
    let mut count = 0usize;
    for buffer in buffers {
        total = total
            .checked_add(buffer.input().len())
            .ok_or(Error::Limit)?;
        if buffer.written() {
            count += 1;
            written_bytes = written_bytes
                .checked_add(buffer.input().len())
                .ok_or(Error::Limit)?;
        }
    }
    if total > limits.max_buffer_bytes {
        return Err(Error::Limit);
    }
    let transaction_work = output_transaction_work(buffers)?;
    let preflight_charge = work
        .checked_mul(2)
        .and_then(|charge| charge.checked_add(transaction_work))
        .filter(|charge| *charge <= remainder)
        .ok_or(Error::Limit)?;
    let required_metadata = count
        .checked_mul(std::mem::size_of::<NativeMemoryWrite<'_>>())
        .ok_or(Error::Limit)?;
    preparation
        .host_bytes
        .checked_add(required_metadata)
        .filter(|bytes| *bytes <= limits.max_host_bytes)
        .ok_or(Error::Limit)?;
    prepared
        .retained_host_bytes()
        .map_err(Error::Prepare)?
        .checked_add(required_metadata)
        .filter(|bytes| *bytes <= limits.max_host_bytes)
        .ok_or(Error::Limit)?;
    let mut storage = Vec::<NativeMemoryWrite<'_>>::new();
    storage
        .try_reserve_exact(count)
        .map_err(|_| Error::Allocation)?;
    let storage_bytes = storage
        .capacity()
        .checked_mul(std::mem::size_of::<NativeMemoryWrite<'_>>())
        .ok_or(Error::Limit)?;
    let retained = prepared
        .retained_host_bytes()
        .map_err(Error::Prepare)?
        .checked_add(storage_bytes)
        .ok_or(Error::Limit)?;
    let mut preparation = preparation;
    preparation.host_bytes = preparation
        .host_bytes
        .checked_add(storage_bytes)
        .filter(|bytes| *bytes <= limits.max_host_bytes)
        .ok_or(Error::Limit)?;
    let mut gpu_limits = *gpu_limits;
    gpu_limits.max_host_bytes = gpu_limits.max_host_bytes.min(
        limits
            .max_host_bytes
            .checked_sub(retained)
            .ok_or(Error::Limit)? as u64,
    );
    admit(preparation, preflight_charge, true)?;
    let completed = run_compute(&prepared, &gpu_limits).map_err(Error::Vulkan)?;
    if completed.resource_count() != buffers.len() {
        return Err(Error::Vulkan(VulkanError::UnexpectedReadback));
    }
    // No host allocation after GPU effects: this exact metadata capacity was
    // admitted before submit, and every output byte came from fenced readback.
    let mut writes = storage;
    let mut returned = completed.outputs().iter();
    for (index, buffer) in buffers
        .iter()
        .enumerate()
        .filter(|(_, buffer)| buffer.written())
    {
        let output = returned
            .next()
            .ok_or(Error::Vulkan(VulkanError::UnexpectedReadback))?;
        if output.resource_index() != index || output.bytes().len() != buffer.input().len() {
            return Err(Error::Vulkan(VulkanError::UnexpectedReadback));
        }
        writes.push(NativeMemoryWrite {
            address: buffer.address(),
            bytes: output.bytes(),
        });
    }
    if returned.next().is_some() {
        return Err(Error::Vulkan(VulkanError::UnexpectedReadback));
    }
    // Re-prove current owner, complete permissions and all physical exclusions
    // before the ONE atomic publication. Its complete transaction walker was
    // admitted separately before GPU submission, including cross-output pages.
    // A failed output never publishes peers.
    preflight_resources(
        paused,
        buffers.len(),
        |index| {
            let buffer = &buffers[index];
            (
                buffer.address(),
                buffer.input().len() as u64,
                buffer.read(),
                buffer.written(),
            )
        },
        commands,
        work,
    )?;
    if !writes.is_empty() {
        paused
            .apply_gpu_writes(
                &writes,
                &NativeTransactionLimits {
                    max_outputs: count,
                    max_output_bytes: written_bytes as u64,
                    max_preflight_steps: transaction_work,
                    ..Default::default()
                },
            )
            .map_err(Error::Memory)?;
    }
    Ok(Report {
        preparation,
        gpu: completed.usage(),
        buffer_bytes: total,
        written_bytes,
        preflight_work: preflight_charge,
    })
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "gpu/wave64_hardware_tests.rs"]
mod wave64_hardware_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "gpu/compute_resources_tests.rs"]
mod compute_resources_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "gpu/lds_hardware_tests.rs"]
mod lds_hardware_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "gpu/storage_barrier_hardware_tests.rs"]
mod storage_barrier_hardware_tests;
