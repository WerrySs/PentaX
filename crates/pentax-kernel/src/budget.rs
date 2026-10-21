// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded checked guest operations; see NOTICE.

use crate::{Failure, GuestAccess, GuestWrite, MemoryEffect};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_attempts: u64,
    pub max_bytes: u64,
    pub max_work: u64,
    pub max_handles: usize,
    /// Retained per-thread identities, errno cells and specific-value tables.
    pub max_thread_contexts: usize,
    pub max_scratch_bytes: usize,
    pub max_name_bytes: usize,
    pub max_io_bytes: usize,
    /// Aggregate accepted standard-stream bytes awaiting application retrieval.
    pub max_output_bytes: usize,
    /// Aggregate buffer storage retained by live directory descriptors.
    pub max_directory_bytes: u64,
    /// Cached APR path identities, independent of open file descriptors.
    pub max_apr_paths: usize,
    /// Retained APR guest path string capacity; table entries are count-bounded.
    pub max_apr_path_bytes: u64,
    /// Aggregate retained AMPR command records across live command buffers.
    pub max_ampr_commands: usize,
    /// APR completion receipts retained until their matching wait.
    pub max_apr_submissions: usize,
    pub max_mapped_bytes: u64,
    pub direct_memory_bytes: u64,
    /// Immutable guest flexible-pool capacity, independent of live mapped bytes
    /// and the host's backing/commitment allowance. Actual flexible maps share it.
    pub flexible_memory_bytes: u64,
    pub virtual_base: u64,
    pub virtual_bytes: u64,
    /// Optional exclusive search ceiling for nonfixed mappings. The initial
    /// window remains the fixed-address policy. Backing must independently own
    /// every selected address and admit its aggregate reservation/mapping cost.
    pub nonfixed_virtual_end: Option<u64>,
    pub max_sleep_nanoseconds: u64,
    /// Aggregate logical save-memory bytes, including source double buffering.
    pub max_save_memory_bytes: u64,
    /// Aggregate admitted persistent save bytes read and written.
    pub max_save_io_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_attempts: 10_000,
            max_bytes: 16 * 1024 * 1024,
            max_work: 32 * 1024 * 1024,
            max_handles: 256,
            max_thread_contexts: 256,
            // Package-backed services share this actual simultaneous staging
            // ceiling. Ordinary I/O/name limits remain independently bounded.
            max_scratch_bytes: 64 * 1024 * 1024,
            max_name_bytes: 1024,
            max_io_bytes: 1024 * 1024,
            max_output_bytes: 8 * 1024 * 1024,
            max_directory_bytes: 8 * 1024 * 1024,
            max_apr_paths: 262_144,
            max_apr_path_bytes: 64 * 1024 * 1024,
            max_ampr_commands: 65_536,
            max_apr_submissions: 65_536,
            max_mapped_bytes: 64 * 1024 * 1024,
            direct_memory_bytes: 64 * 1024 * 1024,
            flexible_memory_bytes: 64 * 1024 * 1024,
            virtual_base: 0x1_0000_0000,
            virtual_bytes: 256 * 1024 * 1024,
            nonfixed_virtual_end: None,
            max_sleep_nanoseconds: 100_000_000,
            max_save_memory_bytes: 32 * 1024 * 1024,
            max_save_io_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub attempts: u64,
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub work: u64,
    pub handles_live: usize,
    pub scratch_peak: usize,
    pub mapped_bytes: u64,
    pub directory_bytes: u64,
    pub apr_paths: usize,
    pub apr_path_bytes: u64,
    pub ampr_commands: usize,
    pub apr_submissions: usize,
    pub save_memory_bytes: u64,
    pub save_bytes_read: u64,
    pub save_bytes_written: u64,
    /// Actual retained AudioOut2 PCM and stereo-mix capacities, not playback.
    pub audio_bytes: usize,
}

pub(crate) struct Budget {
    pub(crate) limits: Limits,
    usage: Usage,
}

impl Budget {
    pub(crate) fn retained_audio_bytes(&mut self, bytes: usize) {
        self.usage.audio_bytes = bytes;
    }
    pub(crate) fn new(limits: Limits) -> Result<Self, Failure> {
        if limits.max_handles > 65_536
            || limits.max_apr_paths > 1_048_576
            || limits.max_apr_path_bytes > 512 * 1024 * 1024
            || limits.max_ampr_commands > 1_048_576
            || limits.max_apr_submissions > 1_048_576
            || limits.max_save_memory_bytes > 32 * 1024 * 1024
            || limits.max_thread_contexts == 0
            || limits.max_thread_contexts > 4096
            || limits.max_name_bytes == 0
            || limits.max_name_bytes > limits.max_scratch_bytes
            || limits.max_io_bytes > limits.max_scratch_bytes
            || limits
                .virtual_base
                .checked_add(limits.virtual_bytes)
                .is_none()
            || limits.virtual_base & 0x3fff != 0
            || limits.virtual_bytes & 0x3fff != 0
            || limits.direct_memory_bytes & 0x3fff != 0
            || limits.direct_memory_bytes > i64::MAX as u64
            || limits.flexible_memory_bytes == 0
            || limits.flexible_memory_bytes & 0x3fff != 0
            || limits.flexible_memory_bytes >= 13_824 * 1024 * 1024
            || limits.virtual_base == 0
            || limits.nonfixed_virtual_end.is_some_and(|end| {
                end & 0x3fff != 0
                    || limits
                        .virtual_base
                        .checked_add(limits.virtual_bytes)
                        .is_none_or(|initial_end| end < initial_end)
            })
        {
            return Err(Failure::Limit("invalid resource policy"));
        }
        Ok(Self {
            limits,
            usage: Usage::default(),
        })
    }
    pub(crate) const fn usage(&self) -> Usage {
        self.usage
    }
    pub(crate) fn admit_save_memory(&self, bytes: u64) -> Result<(), Failure> {
        if bytes > self.limits.max_save_memory_bytes {
            Err(Failure::Limit("save memory"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn retained_save_memory(&mut self, bytes: u64) {
        self.usage.save_memory_bytes = bytes;
    }
    pub(crate) fn save_io(&mut self, bytes: u64, write: bool) -> Result<(), Failure> {
        let total = self
            .usage
            .save_bytes_read
            .checked_add(self.usage.save_bytes_written)
            .and_then(|value| value.checked_add(bytes))
            .ok_or(Failure::Limit("save I/O"))?;
        if total > self.limits.max_save_io_bytes {
            return Err(Failure::Limit("save I/O"));
        }
        self.work(bytes)?;
        if write {
            self.usage.save_bytes_written += bytes;
        } else {
            self.usage.save_bytes_read += bytes;
        }
        Ok(())
    }
    pub(crate) fn attempt(&mut self) -> Result<(), Failure> {
        let next = self
            .usage
            .attempts
            .checked_add(1)
            .ok_or(Failure::Limit("attempts"))?;
        if next > self.limits.max_attempts {
            return Err(Failure::Limit("attempts"));
        }
        self.usage.attempts = next;
        self.work(1)
    }
    pub(crate) fn work(&mut self, count: u64) -> Result<(), Failure> {
        let total = self
            .usage
            .work
            .checked_add(count)
            .ok_or(Failure::Limit("work"))?;
        if total > self.limits.max_work {
            return Err(Failure::Limit("work"));
        }
        self.usage.work = total;
        Ok(())
    }
    fn bytes(&mut self, count: u64, write: bool) -> Result<(), Failure> {
        let total = self
            .usage
            .bytes_read
            .checked_add(self.usage.bytes_written)
            .and_then(|v| v.checked_add(count))
            .ok_or(Failure::Limit("guest bytes"))?;
        if total > self.limits.max_bytes {
            return Err(Failure::Limit("guest bytes"));
        }
        self.work(count)?;
        if write {
            self.usage.bytes_written += count;
        } else {
            self.usage.bytes_read += count;
        }
        Ok(())
    }
    pub(crate) fn precharge_read(&mut self, count: u64) -> Result<(), Failure> {
        self.bytes(count, false)
    }
    pub(crate) fn read(
        &mut self,
        guest: &impl GuestAccess,
        address: u64,
        output: &mut [u8],
    ) -> Result<(), Failure> {
        address
            .checked_add(output.len() as u64)
            .ok_or(crate::GuestError::AddressOverflow)?;
        self.bytes(output.len() as u64, false)?;
        guest.read(address, output).map_err(Failure::Guest)
    }
    pub(crate) fn u64(&mut self, guest: &impl GuestAccess, address: u64) -> Result<u64, Failure> {
        let mut bytes = [0; 8];
        self.read(guest, address, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }
    /// Bounded source-order reads after privately staged guest stores. The
    /// native adapter proves whole-write physical uniqueness and checks each
    /// non-overlapping read fragment before applying virtual-byte overlays.
    pub(crate) fn read_staged(
        &mut self,
        guest: &impl GuestAccess,
        address: u64,
        output: &mut [u8],
        writes: &[GuestWrite<'_>],
    ) -> Result<(), Failure> {
        let total = writes
            .iter()
            .try_fold(0usize, |total, write| total.checked_add(write.bytes.len()));
        if output.len() > 128 || writes.len() > 16 || total.is_none_or(|n| n > 1024 * 1024) {
            return Err(Failure::Limit("staged guest read"));
        }
        address
            .checked_add(output.len() as u64)
            .ok_or(crate::GuestError::AddressOverflow)?;
        self.work(16_384 * writes.len() as u64 + 1)?;
        self.precharge_read(output.len() as u64)?;
        guest
            .read_after_writes(address, output, writes)
            .map_err(Failure::Guest)
    }
    pub(crate) fn u32(&mut self, guest: &impl GuestAccess, address: u64) -> Result<u32, Failure> {
        let mut bytes = [0; 4];
        self.read(guest, address, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }
    pub(crate) fn write(
        &mut self,
        guest: &mut impl GuestAccess,
        writes: &[GuestWrite<'_>],
    ) -> Result<(), Failure> {
        self.apply(guest, MemoryEffect::None, writes)
    }
    /// Admit an adapter-owned transactional output before its state mutation.
    pub(crate) fn reserve_write(&mut self, address: u64, length: u64) -> Result<(), Failure> {
        address
            .checked_add(length)
            .ok_or(crate::GuestError::AddressOverflow)?;
        self.bytes(length, true)
    }
    pub(crate) fn apply(
        &mut self,
        guest: &mut impl GuestAccess,
        effect: MemoryEffect,
        writes: &[GuestWrite<'_>],
    ) -> Result<(), Failure> {
        let mut bytes = 0_u64;
        for (index, write) in writes.iter().enumerate() {
            let end = write
                .address
                .checked_add(write.bytes.len() as u64)
                .ok_or(crate::GuestError::AddressOverflow)?;
            bytes = bytes
                .checked_add(write.bytes.len() as u64)
                .ok_or(Failure::Limit("guest bytes"))?;
            for prior in &writes[..index] {
                self.work(1)?;
                let prior_end = prior
                    .address
                    .checked_add(prior.bytes.len() as u64)
                    .ok_or(crate::GuestError::AddressOverflow)?;
                if !write.bytes.is_empty()
                    && !prior.bytes.is_empty()
                    && write.address < prior_end
                    && prior.address < end
                {
                    return Err(Failure::Unsupported("overlapping guest outputs"));
                }
            }
        }
        self.bytes(bytes, true)?;
        guest.apply(effect, writes).map_err(Failure::Guest)
    }
    pub(crate) fn buffer(&mut self, length: usize) -> Result<Vec<u8>, Failure> {
        self.scratch(length)?;
        self.work(length as u64)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| Failure::Allocation)?;
        bytes.resize(length, 0);
        self.record_scratch(length);
        Ok(bytes)
    }
    /// Fallible byte staging including other simultaneously retained staging.
    /// Account the allocator's actual capacity before initializing its payload.
    pub(crate) fn buffer_held(&mut self, length: usize, held: usize) -> Result<Vec<u8>, Failure> {
        let total = held.checked_add(length).ok_or(Failure::Limit("scratch"))?;
        self.scratch(total)?;
        self.work(length as u64)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| Failure::Allocation)?;
        let actual = held
            .checked_add(bytes.capacity())
            .ok_or(Failure::Limit("scratch"))?;
        self.scratch(actual)?;
        self.record_scratch(actual);
        bytes.resize(length, 0);
        Ok(bytes)
    }
    pub(crate) fn scratch(&self, length: usize) -> Result<(), Failure> {
        if length > self.limits.max_scratch_bytes {
            return Err(Failure::Limit("scratch"));
        }
        Ok(())
    }
    pub(crate) fn record_scratch(&mut self, length: usize) {
        self.usage.scratch_peak = self.usage.scratch_peak.max(length);
    }
    pub(crate) fn can_retain_directory(&self, length: u64) -> Result<(), Failure> {
        if self
            .usage
            .directory_bytes
            .checked_add(length)
            .is_none_or(|total| total > self.limits.max_directory_bytes)
        {
            return Err(Failure::Limit("directory bytes"));
        }
        Ok(())
    }
    pub(crate) fn retained_directory(&mut self, length: u64) {
        self.usage.directory_bytes += length;
    }
    pub(crate) fn released_directory(&mut self, length: u64) {
        self.usage.directory_bytes -= length;
    }
    pub(crate) fn name(
        &mut self,
        guest: &impl GuestAccess,
        address: u64,
    ) -> Result<String, Failure> {
        let mut bytes = self.buffer(self.limits.max_name_bytes)?;
        for index in 0..bytes.len() {
            self.read(
                guest,
                address
                    .checked_add(index as u64)
                    .ok_or(crate::GuestError::AddressOverflow)?,
                &mut bytes[index..index + 1],
            )?;
            if bytes[index] == 0 {
                bytes.truncate(index);
                return String::from_utf8(bytes)
                    .map_err(|_| Failure::Unsupported("non-UTF8 guest name"));
            }
        }
        Err(Failure::Limit("name bytes"))
    }
    pub(crate) fn can_add_handle(&self) -> Result<(), Failure> {
        if self.usage.handles_live >= self.limits.max_handles {
            Err(Failure::Limit("handles"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn can_retain_apr_path(&self, bytes: usize) -> Result<(), Failure> {
        if self.usage.apr_paths >= self.limits.max_apr_paths
            || self
                .usage
                .apr_path_bytes
                .checked_add(bytes as u64)
                .is_none_or(|total| total > self.limits.max_apr_path_bytes)
        {
            return Err(Failure::Limit("APR path cache"));
        }
        Ok(())
    }
    pub(crate) fn retained_apr_path(&mut self, bytes: usize) {
        self.usage.apr_paths += 1;
        self.usage.apr_path_bytes += bytes as u64;
    }
    pub(crate) fn can_add_ampr_command(&self) -> Result<(), Failure> {
        if self.usage.ampr_commands >= self.limits.max_ampr_commands {
            return Err(Failure::Limit("AMPR command records"));
        }
        Ok(())
    }
    pub(crate) fn added_ampr_command(&mut self) {
        self.usage.ampr_commands += 1;
    }
    pub(crate) fn released_ampr_commands(&mut self, count: usize) {
        self.usage.ampr_commands -= count;
    }
    pub(crate) fn can_add_apr_submission(&self) -> Result<(), Failure> {
        if self.usage.apr_submissions >= self.limits.max_apr_submissions {
            return Err(Failure::Limit("APR submission receipts"));
        }
        Ok(())
    }
    pub(crate) fn added_apr_submission(&mut self) {
        self.usage.apr_submissions += 1;
    }
    pub(crate) fn removed_apr_submission(&mut self) {
        self.usage.apr_submissions -= 1;
    }
    pub(crate) fn added_handle(&mut self) {
        self.usage.handles_live += 1;
    }
    pub(crate) fn removed_handle(&mut self) {
        self.usage.handles_live -= 1;
    }
    /// Admit metadata splitting/coalescing while preserving every other live
    /// resource. The returned total is published only after the backing effect.
    pub(crate) fn replacement_handles(&self, old: usize, new: usize) -> Result<usize, Failure> {
        self.usage
            .handles_live
            .checked_sub(old)
            .and_then(|total| total.checked_add(new))
            .filter(|total| *total <= self.limits.max_handles)
            .ok_or(Failure::Limit("mapping handles"))
    }
    pub(crate) fn replaced_handles(&mut self, total: usize) {
        debug_assert!(total <= self.limits.max_handles);
        self.usage.handles_live = total;
    }
    pub(crate) fn can_map(&self, length: u64) -> Result<(), Failure> {
        if self
            .usage
            .mapped_bytes
            .checked_add(length)
            .is_none_or(|v| v > self.limits.max_mapped_bytes)
        {
            Err(Failure::Limit("mapped bytes"))
        } else {
            Ok(())
        }
    }
    pub(crate) fn mapped(&mut self, length: u64) {
        self.usage.mapped_bytes += length;
    }
    pub(crate) fn unmapped(&mut self, length: u64) {
        self.usage.mapped_bytes -= length;
    }
}
