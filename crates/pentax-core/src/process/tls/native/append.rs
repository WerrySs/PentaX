// SPDX-License-Identifier: GPL-2.0-only

//! Append templates without replaying live TLS; stage exact future-default changes.
use super::*;
use crate::memory::service::StagingMemory;
use crate::process::ProcessError;
use crate::process::tls::TlsBlock;
use pentax_platform::native_memory::{NativeAddressSpace, NativeRange};

const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;
struct Snapshot {
    address: u64,
    bytes: Vec<u8>,
}
struct InitializerWrite {
    template: usize,
    offset: usize,
    value: u8,
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
fn invalid(reason: &'static str) -> TlsError {
    TlsError::InvalidStorage {
        instance: 0,
        reason,
    }
}
fn allocation() -> TlsError {
    TlsError::Memory(MemoryError::BudgetExceeded)
}

impl PreparedNativeTlsSession<'_, '_> {
    /// Only private core publication may extend actual TLS history. Metadata,
    /// snapshots and all new block geometry are admitted before any effect.
    #[allow(dead_code)] // Retained single-append entry for internal consumers.
    pub(crate) fn append_module(
        &mut self,
        plan: &RelocatedStaticTlsPlan<'_, '_, '_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
    ) -> Result<(), TlsError> {
        self.append_batch(plan, window, limits, |_| Ok(()))
    }

    /// Append the explicit guest plus an optional authenticated no-TLS provider
    /// in one transaction. Attestation follows new-page sealing and precedes
    /// retained-word commit, so failure restores all staged TLS storage too.
    pub(crate) fn append_batch(
        &mut self,
        plan: &RelocatedStaticTlsPlan<'_, '_, '_>,
        window: NativeRange,
        limits: &NativeLoadLimits,
        before_replacements: impl FnOnce(&mut NativeAddressSpace) -> Result<(), ProcessError>,
    ) -> Result<(), TlsError> {
        let old = self.templates.len();
        let templates = plan.templates();
        let source = plan.bindings().base_plan();
        let old_modules = self.memory.module_count();
        if source
            .modules()
            .len()
            .checked_sub(old_modules)
            .is_none_or(|count| !(1..=2).contains(&count))
            || source
                .appended_guest()
                .is_none_or(|guest| guest.index() != old_modules)
        {
            return Err(invalid("new TLS source suffix"));
        }
        if !templates.starts_with(&self.templates)
            || templates.len() > old + 1
            || templates[old..]
                .iter()
                .any(|template| template.module.index() != old_modules)
            || self.thread_control != plan.layout.thread_control_policy()
            || plan.bindings().base_plan().parent_identity()
                != Some(self.memory.active_source_identity())
        {
            return Err(invalid("retained TLS layout"));
        }
        if self.initializers.len() != old {
            return Err(invalid("retained TLS initializers"));
        }
        let transitions = plan.bindings().base_plan().fallback_transitions();
        if transitions.len() > 16_384 {
            return Err(invalid("TLS replacement budget"));
        }
        // Only listed initializer bytes change for FUTURE threads. Existing
        // instance bytes, including guest writes, are never copied or replayed.
        let count = transitions.len().checked_mul(8).ok_or_else(allocation)?;
        let mut initializer_writes = Vec::<InitializerWrite>::new();
        initializer_writes
            .try_reserve_exact(count)
            .map_err(|_| allocation())?;
        let mut allocated = initializer_writes
            .capacity()
            .checked_mul(std::mem::size_of::<InitializerWrite>())
            .map(|bytes| bytes as u64)
            .filter(|bytes| *bytes <= MAX_SNAPSHOT_BYTES)
            .ok_or_else(allocation)?;
        for (index, (template, bytes)) in self.templates.iter().zip(&self.initializers).enumerate()
        {
            if bytes.len() as u64 != template.initialized_size {
                return Err(invalid("retained TLS initializer size"));
            }
            let end = template
                .source_address
                .checked_add(template.initialized_size)
                .ok_or(TlsError::AddressOverflow)?;
            for transition in transitions {
                let before = transition.old_patch();
                let after = transition.new_patch();
                let word_end = before
                    .address()
                    .checked_add(8)
                    .ok_or(TlsError::AddressOverflow)?;
                let start = before.address().max(template.source_address);
                let stop = word_end.min(end);
                let expected = before.value().to_le_bytes();
                let replacement = after.value().to_le_bytes();
                for address in start..stop {
                    let offset = (address - template.source_address) as usize;
                    let byte = (address - before.address()) as usize;
                    if bytes[offset] != expected[byte] {
                        return Err(invalid("retained TLS initializer word"));
                    }
                    if initializer_writes.len() == count {
                        return Err(invalid("overlapping TLS initializer sources"));
                    }
                    initializer_writes.push(InitializerWrite {
                        template: index,
                        offset,
                        value: replacement[byte],
                    });
                }
            }
        }
        let mut retained = Vec::new();
        let mut additions = super::threads::allocate_initializers(&templates[old..], &self.limits)?;
        self.initializers
            .try_reserve_exact(additions.len())
            .map_err(|_| allocation())?;
        retained
            .try_reserve_exact(templates.len())
            .map_err(|_| allocation())?;
        retained.extend_from_slice(templates);
        let mut instances = Vec::new();
        instances
            .try_reserve_exact(self.instances.len())
            .map_err(|_| allocation())?;
        let mut snapshots = Vec::new();
        snapshots
            .try_reserve_exact(self.instances.len())
            .map_err(|_| allocation())?;
        let mut total = 0_u64;
        let paused = self.memory.pause().map_err(TlsError::Memory)?;
        for instance in &self.instances {
            if instance.blocks.len() != old {
                return Err(invalid("retained TLS blocks"));
            }
            let mut blocks = Vec::new();
            blocks
                .try_reserve_exact(templates.len())
                .map_err(|_| allocation())?;
            blocks.extend_from_slice(&instance.blocks);
            if let Some(guards) = instance.guard_pages {
                for address in guards {
                    paused
                        .preflight_reserved_unmapped(address, crate::memory::PAGE_SIZE)
                        .map_err(|e| TlsError::Memory(MemoryError::Native(e)))?;
                }
            }
            for template in &templates[old..] {
                let address = instance
                    .thread_pointer
                    .checked_sub(template.tp_offset)
                    .ok_or(TlsError::AddressOverflow)?;
                let end = address
                    .checked_add(template.mem_size)
                    .ok_or(TlsError::AddressOverflow)?;
                if address % template.alignment != template.alignment_bias
                    || end > instance.thread_pointer
                {
                    return Err(invalid("new TLS block alignment"));
                }
                if template.mem_size != 0 {
                    let storage = instance
                        .storage
                        .ok_or(invalid("no reserved TLS capacity"))?;
                    if address < storage.address
                        || end
                            > storage
                                .address
                                .checked_add(storage.size)
                                .ok_or(TlsError::AddressOverflow)?
                    {
                        return Err(invalid("reserved TLS capacity exhausted"));
                    }
                    for block in &instance.blocks {
                        if block.size != 0
                            && address
                                < block
                                    .address
                                    .checked_add(block.size)
                                    .ok_or(TlsError::AddressOverflow)?
                            && block.address < end
                        {
                            return Err(invalid("new TLS overlaps live block"));
                        }
                    }
                    paused
                        .preflight_write(address, template.mem_size)
                        .map_err(|e| TlsError::Memory(MemoryError::Native(e)))?;
                    if !matches!(
                        paused.preflight_fetch_source(address, template.mem_size),
                        Err(NativeMemoryError::PermissionDenied(_))
                    ) {
                        return Err(invalid("TLS storage must remain NX"));
                    }
                    total = total
                        .checked_add(template.mem_size)
                        .filter(|v| *v <= MAX_SNAPSHOT_BYTES)
                        .ok_or(invalid("TLS snapshot budget"))?;
                    let length = usize::try_from(template.mem_size).map_err(|_| allocation())?;
                    let mut bytes = Vec::new();
                    bytes.try_reserve_exact(length).map_err(|_| allocation())?;
                    allocated = allocated
                        .checked_add(bytes.capacity() as u64)
                        .filter(|value| *value <= MAX_SNAPSHOT_BYTES)
                        .ok_or_else(allocation)?;
                    bytes.resize(length, 0);
                    paused
                        .read(address, &mut bytes)
                        .map_err(|e| TlsError::Memory(MemoryError::Native(e)))?;
                    snapshots.push(Snapshot { address, bytes });
                }
                blocks.push(TlsBlock {
                    module: template.module,
                    runtime_id: template.runtime_id,
                    address,
                    size: template.mem_size,
                });
            }
            instances.push(StaticTlsInstance {
                blocks,
                thread_pointer: instance.thread_pointer,
                thread_control_block: instance.thread_control_block,
                storage: instance.storage,
                guard_pages: instance.guard_pages,
            });
        }
        let result = if templates.len() == old && plan.tls_patches().is_empty() {
            self.memory
                .append_source_batch(source, window, limits, before_replacements)
        } else {
            self.memory.append_relocated_source_batch(
                plan,
                window,
                limits,
                |space| {
                    for (template, bytes) in templates[old..].iter().zip(&mut additions) {
                        StagingMemory::read(space, template.source_address, bytes)?;
                    }
                    let mut buffer = [0; 4096];
                    for instance in &instances {
                        for ((template, block), initial) in templates[old..]
                            .iter()
                            .zip(&instance.blocks[old..])
                            .zip(&additions)
                        {
                            let mut offset = 0;
                            while offset < template.mem_size {
                                let count =
                                    (template.mem_size - offset).min(buffer.len() as u64) as usize;
                                space.write(block.address + offset, &buffer[..count])?;
                                offset += count as u64;
                            }
                            offset = 0;
                            while offset < template.initialized_size {
                                let count = (template.initialized_size - offset)
                                    .min(buffer.len() as u64)
                                    as usize;
                                space.write(
                                    block.address + offset,
                                    &initial[offset as usize..offset as usize + count],
                                )?;
                                offset += count as u64;
                            }
                            buffer.fill(0);
                        }
                    }
                    Ok::<_, ProcessError>(())
                },
                before_replacements,
            )
        };
        if let Err(error) = result {
            for snapshot in snapshots {
                if self
                    .memory
                    .space
                    .write(snapshot.address, &snapshot.bytes)
                    .is_err()
                {
                    self.memory.space.abandon();
                    return Err(TlsError::Memory(MemoryError::Native(
                        NativeMemoryError::Poisoned,
                    )));
                }
            }
            return Err(TlsError::Staging(error));
        }
        for write in initializer_writes {
            self.initializers[write.template][write.offset] = write.value;
        }
        self.templates = retained;
        self.instances = instances;
        self.initializers.extend(additions);
        Ok(())
    }
}
