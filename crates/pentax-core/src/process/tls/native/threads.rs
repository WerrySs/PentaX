// SPDX-License-Identifier: GPL-2.0-only

//! Fresh TLS instances from retained, relocated initialization templates.

use super::*;
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE};
use crate::process::tls::{
    StaticTlsLimits, TlsBlock, TlsBudget, TlsStorageRange, Work, round_pages,
};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativePermissions,
    NativeTransactionLimits,
};

fn native(error: NativeMemoryError) -> TlsError {
    TlsError::Memory(MemoryError::Native(error))
}
fn allocation() -> TlsError {
    TlsError::Memory(MemoryError::BudgetExceeded)
}

pub(super) fn allocate_initializers(
    templates: &[TlsTemplateLayout],
    limits: &StaticTlsLimits,
) -> Result<Vec<Vec<u8>>, TlsError> {
    let mut size = 0_u64;
    for template in templates {
        size = size
            .checked_add(template.initialized_size)
            .ok_or(TlsError::AddressOverflow)?;
        if size > limits.max_initialized_bytes || size > limits.max_work_steps {
            return Err(TlsError::Budget {
                kind: TlsBudget::InitializedBytes,
                limit: limits.max_initialized_bytes.min(limits.max_work_steps),
            });
        }
    }
    let mut initializers = Vec::new();
    initializers
        .try_reserve_exact(templates.len())
        .map_err(|_| allocation())?;
    for template in templates {
        let length = usize::try_from(template.initialized_size).map_err(|_| allocation())?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| allocation())?;
        bytes.resize(length, 0);
        initializers.push(bytes);
    }
    Ok(initializers)
}

impl PreparedNativeTlsSession<'_, '_> {
    pub(crate) fn rollback_last_thread_instance(
        &mut self,
        thread_pointer: u64,
    ) -> Result<(), TlsError> {
        let instance = self
            .instances
            .last()
            .filter(|instance| {
                self.instances.len() > 1 && instance.thread_pointer == thread_pointer
            })
            .ok_or(TlsError::InvalidStorage {
                instance: self.instances.len(),
                reason: "thread rollback order",
            })?;
        let start = instance
            .storage
            .map_or(thread_pointer, |storage| storage.address);
        let end = thread_pointer
            .checked_add(PAGE_SIZE)
            .ok_or(TlsError::AddressOverflow)?;
        let result = self
            .memory
            .pause()
            .map_err(TlsError::Memory)?
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::Unmap {
                        address: start,
                        size: end - start,
                    },
                    writes: &[],
                },
                &NativeTransactionLimits::default(),
            );
        if let Err(error) = result {
            self.memory.space.abandon();
            return Err(native(error));
        }
        self.instances.pop();
        Ok(())
    }
    /// Create fresh storage in a caller-assigned, exclusively reserved TLS
    /// window of this owner. The caller must keep the window out of other
    /// allocators, including its two unmapped guards. No CPU, stack, pthread
    /// handle or running-thread state is created by this operation.
    ///
    /// Initialized bytes come from the relocated templates retained at source
    /// publication, never from another live thread. Metadata is published only
    /// after the complete RW/NX mapping and writes succeed atomically.
    pub fn append_thread_instance(&mut self, thread_pointer: u64) -> Result<usize, TlsError> {
        let index = self.instances.len();
        let invalid = |reason| TlsError::InvalidStorage {
            instance: index,
            reason,
        };
        if index as u64 >= self.limits.max_instances {
            return Err(TlsError::Budget {
                kind: TlsBudget::Instances,
                limit: self.limits.max_instances,
            });
        }
        if self.thread_control != ThreadControlBlockPolicy::KytyObservedSelfPointerV1 {
            return Err(invalid("thread storage requires explicit TCB profile"));
        }
        if thread_pointer == 0
            || thread_pointer >= ADDRESS_LIMIT
            || !thread_pointer.is_multiple_of(PAGE_SIZE)
        {
            return Err(invalid("guest TP must be page-aligned and in range"));
        }
        let mut work = Work {
            used: 0,
            limit: self.limits.max_work_steps,
        };
        work.charge((self.templates.len() + self.instances.len() + 1) as u64)?;
        let extent = self
            .templates
            .iter()
            .map(|template| template.tp_offset)
            .max()
            .unwrap_or(0)
            .max(self.limits.reserved_static_bytes);
        let extent = round_pages(extent)?;
        let storage_start = thread_pointer
            .checked_sub(extent)
            .ok_or(TlsError::AddressOverflow)?;
        let lower = storage_start
            .checked_sub(PAGE_SIZE)
            .ok_or(TlsError::AddressOverflow)?;
        let upper = thread_pointer
            .checked_add(PAGE_SIZE)
            .ok_or(TlsError::AddressOverflow)?;
        let end = upper
            .checked_add(PAGE_SIZE)
            .filter(|end| *end <= ADDRESS_LIMIT)
            .ok_or(TlsError::AddressOverflow)?;
        let mapped = extent
            .checked_add(PAGE_SIZE)
            .ok_or(TlsError::AddressOverflow)?;
        let mut aggregate = mapped;
        for old in &self.instances {
            let old_bytes = old
                .storage
                .map_or(0, |storage| storage.size)
                .checked_add(if old.thread_control_block.is_some() {
                    PAGE_SIZE
                } else {
                    0
                })
                .ok_or(TlsError::AddressOverflow)?;
            aggregate = aggregate
                .checked_add(old_bytes)
                .ok_or(TlsError::AddressOverflow)?;
            if let Some([before, after]) = old.guard_pages
                && lower
                    < after
                        .checked_add(PAGE_SIZE)
                        .ok_or(TlsError::AddressOverflow)?
                && before < end
            {
                return Err(TlsError::StorageOverlap { instance: index });
            }
        }
        if aggregate > self.limits.max_storage_bytes {
            return Err(TlsError::Budget {
                kind: TlsBudget::StorageBytes,
                limit: self.limits.max_storage_bytes,
            });
        }
        work.charge(mapped)?;
        if self.initializers.len() != self.templates.len() {
            return Err(invalid("TLS initializer history"));
        }
        let mut blocks = Vec::new();
        blocks
            .try_reserve_exact(self.templates.len())
            .map_err(|_| allocation())?;
        let mut writes = Vec::new();
        writes
            .try_reserve_exact(self.templates.len() + 1)
            .map_err(|_| allocation())?;
        let header = thread_pointer.to_le_bytes();
        writes.push(NativeMemoryWrite {
            address: thread_pointer,
            bytes: &header,
        });
        for (template, bytes) in self.templates.iter().zip(&self.initializers) {
            let address = thread_pointer
                .checked_sub(template.tp_offset)
                .ok_or(TlsError::AddressOverflow)?;
            let block_end = address
                .checked_add(template.mem_size)
                .ok_or(TlsError::AddressOverflow)?;
            if address < storage_start
                || block_end > thread_pointer
                || address % template.alignment != template.alignment_bias
                || bytes.len() as u64 != template.initialized_size
                || template.initialized_size > template.mem_size
            {
                return Err(invalid("thread TLS block geometry"));
            }
            work.charge(bytes.len() as u64)?;
            writes.push(NativeMemoryWrite { address, bytes });
            blocks.push(TlsBlock {
                module: template.module,
                runtime_id: template.runtime_id,
                address,
                size: template.mem_size,
            });
        }
        self.instances
            .try_reserve_exact(1)
            .map_err(|_| allocation())?;
        let mut paused = self.memory.pause().map_err(TlsError::Memory)?;
        paused
            .preflight_reserved_unmapped(lower, end - lower)
            .map_err(native)?;
        paused
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::MapZeroed {
                        address: storage_start,
                        size: mapped,
                        permissions: NativePermissions::READ_WRITE,
                    },
                    writes: &writes,
                },
                &NativeTransactionLimits {
                    max_outputs: self.templates.len() + 1,
                    max_output_bytes: self
                        .limits
                        .max_initialized_bytes
                        .checked_add(8)
                        .ok_or(TlsError::AddressOverflow)?,
                    max_preflight_steps: self.limits.max_work_steps.saturating_sub(work.used),
                    ..Default::default()
                },
            )
            .map_err(native)?;
        self.instances.push(StaticTlsInstance {
            thread_pointer,
            thread_control_block: Some(TlsStorageRange {
                address: thread_pointer,
                size: 64,
            }),
            storage: (extent != 0).then_some(TlsStorageRange {
                address: storage_start,
                size: extent,
            }),
            guard_pages: Some([lower, upper]),
            blocks,
        });
        Ok(index)
    }
}
