// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: bounded current-owner ordered register execution.

use super::{Failure, PausedNativeMemory, compute};
use std::mem::size_of;

// Inline sealed state avoids a fallible allocation after an earlier effect.
#[allow(clippy::large_enum_variant)]
pub(super) enum ComputeCommands {
    Static {
        plan: compute::CommandPlan,
        at: usize,
        held: usize,
        work: u64,
    },
    Ordered {
        plan: compute::OrderedCommands,
        held: usize,
        work: u64,
    },
}
// Inline sealed state avoids a fallible allocation after an earlier effect.
#[allow(clippy::large_enum_variant)]
pub(super) enum ComputeStep {
    Array,
    Command(usize),
    Dispatch(compute::Dispatch),
}
impl ComputeCommands {
    pub(super) fn prepare(
        memory: &PausedNativeMemory<'_>,
        root: compute::CommandRange,
        initial: compute::ComputeState,
        limits: &compute::CommandLimits,
    ) -> Result<Self, Failure> {
        let size = usize::try_from(root.size).map_err(|_| Failure::Limit)?;
        if size > limits.max_command_bytes {
            return Err(Failure::Limit);
        }
        let expected = size
            .checked_add(size_of::<Vec<u8>>())
            .ok_or(Failure::Limit)?;
        if expected > limits.max_host_bytes {
            return Err(Failure::Limit);
        }
        let work = root.size.checked_mul(3).ok_or(Failure::Limit)?;
        if work > limits.max_work {
            return Err(Failure::Limit);
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(size)
            .map_err(|_| Failure::Allocation)?;
        let held = bytes
            .capacity()
            .checked_add(size_of::<Vec<u8>>())
            .ok_or(Failure::Limit)?;
        let mut limits = *limits;
        limits.max_host_bytes = limits
            .max_host_bytes
            .checked_sub(held)
            .ok_or(Failure::Limit)?;
        limits.max_work = limits.max_work.checked_sub(work).ok_or(Failure::Limit)?;
        bytes.resize(size, 0);
        memory
            .read_gpu(root.address, &mut bytes)
            .map_err(|_| Failure::Memory)?;
        if pentax_gpu::contains_indirect_registers(&[&bytes]).map_err(Failure::Preparation)? {
            let plan = compute::OrderedCommands::prepare(&bytes, root, initial, &limits)
                .map_err(Failure::Preparation)?;
            // Capture storage is gone before execution. Its admitted allocation
            // and copy/walk work remain charged through the complete product.
            Ok(Self::Ordered { plan, held, work })
        } else {
            struct Snapshot<'a, 'owner> {
                root: compute::CommandRange,
                bytes: &'a [u8],
                memory: &'a PausedNativeMemory<'owner>,
            }
            impl compute::Memory for Snapshot<'_, '_> {
                fn read(
                    &self,
                    address: u64,
                    output: &mut [u8],
                ) -> Result<(), compute::MemoryError> {
                    if address == self.root.address && output.len() == self.bytes.len() {
                        output.copy_from_slice(self.bytes);
                        Ok(())
                    } else {
                        compute::Memory::read(&super::super::Memory(self.memory), address, output)
                    }
                }
            }
            let plan = compute::plan_commands(
                &initial,
                root,
                &Snapshot {
                    root,
                    bytes: &bytes,
                    memory,
                },
                &limits,
            )
            .map_err(Failure::Preparation)?;
            Ok(Self::Static {
                plan,
                at: 0,
                held,
                work,
            })
        }
    }
    pub(super) fn usage(&self) -> compute::CommandUsage {
        match self {
            Self::Static {
                plan, held, work, ..
            } => {
                let mut usage = plan.usage();
                usage.host_bytes += held;
                usage.work += work;
                usage
            }
            Self::Ordered { plan, held, work } => {
                let mut usage = plan.usage();
                usage.host_bytes += held;
                usage.work += work;
                usage
            }
        }
    }
    pub(super) fn command_ranges(&self) -> &[compute::CommandRange] {
        match self {
            Self::Static { plan, .. } => plan.command_ranges(),
            Self::Ordered { plan, .. } => plan.command_ranges(),
        }
    }
    pub(super) fn next_step(&mut self) -> Result<Option<ComputeStep>, Failure> {
        match self {
            Self::Static { plan, at, .. } => {
                if plan.command(*at).is_none() {
                    return Ok(None);
                }
                let index = *at;
                *at += 1;
                Ok(Some(ComputeStep::Command(index)))
            }
            Self::Ordered { plan, .. } => {
                Ok(plan
                    .next_step()
                    .map_err(Failure::Preparation)?
                    .map(|step| match step {
                        compute::OrderedStep::Array => ComputeStep::Array,
                        compute::OrderedStep::Memory(index) => ComputeStep::Command(index),
                        compute::OrderedStep::Dispatch(dispatch) => ComputeStep::Dispatch(dispatch),
                    }))
            }
        }
    }
    pub(super) fn command(&self, index: usize) -> Result<compute::Command<'_>, Failure> {
        match self {
            Self::Static { plan, .. } => {
                plan.command(index)
                    .ok_or(Failure::Preparation(compute::Error::Unsupported(
                        "compute action index",
                    )))
            }
            Self::Ordered { plan, .. } => plan.memory_command(index).map_err(Failure::Preparation),
        }
    }
    pub(super) fn resolve_array(&mut self, memory: &PausedNativeMemory<'_>) -> Result<(), Failure> {
        match self {
            Self::Ordered { plan, .. } => plan
                .resolve_array(&super::super::Memory(memory))
                .map_err(Failure::Preparation),
            _ => Err(Failure::Preparation(compute::Error::Unsupported(
                "static register array",
            ))),
        }
    }
    pub(super) fn final_state(&self) -> Result<compute::ComputeState, Failure> {
        match self {
            Self::Static { plan, .. } => Ok(plan.final_state()),
            Self::Ordered { plan, .. } => plan.final_state().map_err(Failure::Preparation),
        }
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "register_execution_tests.rs"]
mod tests;
