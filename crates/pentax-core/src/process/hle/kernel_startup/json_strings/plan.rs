// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: staged guest-backed String storage. Source pin and
// attribution are retained in the parent module; see NOTICE.

use super::*;
use pentax_kernel::{Failure, RuntimeDataOutputs};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativeRange,
    NativeTransactionLimits,
};
use std::mem::size_of;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StringRef {
    Existing(usize),
    Pending(usize),
}

struct PendingString {
    existing: Option<usize>,
    bytes: Vec<u8>,
    token_offset: usize,
    data_offset: usize,
    wrapper_offset: Option<usize>,
    wrapper_address: u64,
    value: Option<u64>,
    resolved: StringRecord,
}

#[derive(Clone, Copy)]
enum Pointer {
    Zero,
    Token(usize),
}

#[derive(Clone, Copy)]
struct WrapperPatch {
    address: u64,
    pointer: Pointer,
}

pub(super) struct Plan {
    pending: [Option<PendingString>; 2],
    pending_len: usize,
    patches: [Option<WrapperPatch>; 2],
    patch_len: usize,
    retired: Option<usize>,
    retired_data: Option<StringRecord>,
    retired_wrapper: Option<usize>,
    pub(super) value: Option<StagedValue>,
    value_string: Option<usize>,
}

impl Plan {
    pub(super) const fn new() -> Self {
        Self {
            pending: [None, None],
            pending_len: 0,
            patches: [None, None],
            patch_len: 0,
            retired: None,
            retired_data: None,
            retired_wrapper: None,
            value: None,
            value_string: None,
        }
    }

    fn scratch(&self) -> Result<usize, KernelStartupError> {
        self.pending[..self.pending_len]
            .iter()
            .try_fold(0_usize, |sum, entry| {
                sum.checked_add(
                    entry
                        .as_ref()
                        .expect("populated pending entry")
                        .bytes
                        .capacity(),
                )
                .ok_or(KernelStartupError::Budget("JSON scratch"))
            })
    }

    fn scratch_with(&self, extra: usize) -> Result<(), KernelStartupError> {
        if self
            .scratch()?
            .checked_add(extra)
            .is_none_or(|sum| sum > MAX_SCRATCH_BYTES)
        {
            Err(KernelStartupError::Budget("JSON scratch"))
        } else {
            Ok(())
        }
    }

    fn add(
        &mut self,
        existing: Option<usize>,
        bytes: Vec<u8>,
        value: Option<u64>,
    ) -> Result<usize, KernelStartupError> {
        if bytes.len() > MAX_STRING_BYTES || self.pending_len == self.pending.len() {
            return Err(KernelStartupError::Budget("JSON String storage"));
        }
        self.scratch_with(bytes.capacity())?;
        let index = self.pending_len;
        self.pending[index] = Some(PendingString {
            existing,
            bytes,
            token_offset: 0,
            data_offset: 0,
            wrapper_offset: None,
            wrapper_address: 0,
            value,
            resolved: StringRecord {
                token: 0,
                data: 0,
                length: 0,
                live: true,
            },
        });
        self.pending_len += 1;
        Ok(index)
    }

    fn patch(&mut self, address: u64, pointer: Pointer) -> Result<(), KernelStartupError> {
        let end = at(address, 8)?;
        for patch in self.patches[..self.patch_len].iter_mut().flatten() {
            if patch.address == address {
                patch.pointer = pointer;
                return Ok(());
            }
            if address < at(patch.address, 8)? && patch.address < end {
                return Err(KernelStartupError::InvalidJsonString);
            }
        }
        if self.patch_len == self.patches.len() {
            return Err(KernelStartupError::Budget("JSON wrapper outputs"));
        }
        self.patches[self.patch_len] = Some(WrapperPatch { address, pointer });
        self.patch_len += 1;
        Ok(())
    }

    pub(super) fn construct_string(
        &mut self,
        wrapper: u64,
        bytes: Vec<u8>,
    ) -> Result<(), KernelStartupError> {
        let index = self.add(None, bytes, None)?;
        self.patch(wrapper, Pointer::Token(index))
    }

    pub(super) fn value_string(
        &mut self,
        value: u64,
        bytes: Vec<u8>,
    ) -> Result<(), KernelStartupError> {
        let index = self.add(None, bytes, Some(value))?;
        self.value_string = Some(index);
        Ok(())
    }

    fn changed(&self, reference: StringRef) -> Option<usize> {
        match reference {
            StringRef::Pending(index) => Some(index),
            StringRef::Existing(index) => self.pending[..self.pending_len]
                .iter()
                .position(|pending| pending.as_ref().is_some_and(|p| p.existing == Some(index))),
        }
    }

    pub(super) fn replace(
        &mut self,
        state: &JsonStrings,
        reference: StringRef,
        bytes: Vec<u8>,
    ) -> Result<(), KernelStartupError> {
        if bytes.len() > MAX_STRING_BYTES {
            return Err(KernelStartupError::Budget("JSON String bytes"));
        }
        // Include both old pending contents and the new snapshot before dropping either.
        self.scratch_with(bytes.capacity())?;
        if let Some(index) = self.changed(reference) {
            self.pending[index].as_mut().expect("pending String").bytes = bytes;
        } else {
            let StringRef::Existing(index) = reference else {
                unreachable!("pending reference");
            };
            let pending = self.add(Some(index), bytes, None)?;
            self.pending[pending]
                .as_mut()
                .expect("pending String")
                .resolved = state.records[index];
        }
        Ok(())
    }

    pub(super) fn append(
        &mut self,
        reference: StringRef,
        bytes: &[u8],
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        self.append_held(reference, bytes, 0, work, maximum)
    }

    pub(super) fn append_held(
        &mut self,
        reference: StringRef,
        bytes: &[u8],
        held: usize,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        let index = self
            .changed(reference)
            .ok_or(KernelStartupError::InvalidJsonString)?;
        let old = &self.pending[index].as_ref().expect("pending String").bytes;
        let length = old
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_STRING_BYTES)
            .ok_or(KernelStartupError::Budget("JSON String bytes"))?;
        charge(work, length as u64, maximum)?;
        let mut next = Vec::new();
        next.try_reserve_exact(length)
            .map_err(|_| KernelStartupError::Allocation)?;
        self.scratch_with(
            next.capacity()
                .checked_add(held)
                .ok_or(KernelStartupError::Budget("JSON scratch"))?,
        )?;
        next.extend_from_slice(old);
        next.extend_from_slice(bytes);
        self.pending[index].as_mut().expect("pending String").bytes = next;
        Ok(())
    }

    fn check_wrapper(
        &self,
        state: &JsonStrings,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        charge(work, state.wrappers.len() as u64, maximum)?;
        for (index, record) in state.wrappers.iter().enumerate() {
            if record.address == wrapper && (!record.live || self.retired_wrapper == Some(index)) {
                return Err(KernelStartupError::InvalidJsonString);
            }
        }
        Ok(())
    }

    fn alias_read(
        &self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
        length: usize,
        exact: Option<u64>,
    ) -> Result<(), KernelStartupError> {
        let end = at(address, length)?;
        for patch in self.patches[..self.patch_len].iter().flatten() {
            if exact == Some(patch.address) {
                continue;
            }
            let other_end = at(patch.address, 8)?;
            if address < other_end && patch.address < end {
                // A new token's numeric bytes are not selected until publication.
                if matches!(patch.pointer, Pointer::Token(_)) {
                    return Err(KernelStartupError::InvalidJsonString);
                }
                memory
                    .preflight_distinct_backing(
                        address.min(patch.address),
                        end.max(other_end) - address.min(patch.address),
                        32,
                    )
                    .map_err(|e| guest(e, false))?;
            } else {
                memory
                    .preflight_disjoint_backing(
                        NativeRange {
                            address,
                            size: length as u64,
                        },
                        NativeRange {
                            address: patch.address,
                            size: 8,
                        },
                        32,
                    )
                    .map_err(|e| guest(e, false))?;
            }
        }
        Ok(())
    }

    fn read(
        &self,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        address: u64,
        bytes: &mut [u8],
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        // Each small read includes the <=2-page Value and wrapper alias checks.
        charge(work, 256 + bytes.len() as u64, maximum)?;
        let end = at(address, bytes.len())?;
        if let Some(record) = self.retired_data
            && address < at(record.data, record.length + 1)?
            && record.data < end
        {
            return Err(KernelStartupError::InvalidJsonString);
        }
        for pending in self.pending[..self.pending_len].iter().flatten() {
            if pending.existing.is_some() && pending.resolved.data != 0 {
                let old_end = at(pending.resolved.data, pending.resolved.length + 1)?;
                if address < old_end && pending.resolved.data < end {
                    // A raw alias into a logically changed std::string buffer is
                    // not replaced with a detached pre-mutation snapshot.
                    return Err(KernelStartupError::InvalidJsonString);
                }
            }
        }
        if let Some(value) = value {
            value.read_slice(memory, address, bytes)?;
        } else {
            memory.read(address, bytes).map_err(|e| guest(e, false))?;
        }
        self.alias_read(memory, address, bytes.len(), None)?;
        for patch in self.patches[..self.patch_len].iter().flatten() {
            if matches!(patch.pointer, Pointer::Zero) {
                for (index, byte) in bytes.iter_mut().enumerate() {
                    if (patch.address..patch.address + 8).contains(&(address + index as u64)) {
                        *byte = 0;
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn word(
        &self,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        address: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<u64, KernelStartupError> {
        let mut bytes = [0; 8];
        self.read(memory, value, address, &mut bytes, work, maximum)?;
        Ok(u64::from_le_bytes(bytes))
    }

    pub(super) fn byte(
        &self,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        address: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<u8, KernelStartupError> {
        let mut bytes = [0];
        self.read(memory, value, address, &mut bytes, work, maximum)?;
        Ok(bytes[0])
    }

    pub(super) fn kind(
        &self,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        object: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<u32, KernelStartupError> {
        if object == 0 {
            return Ok(0);
        }
        let mut bytes = [0; 4];
        self.read(memory, value, at(object, 28)?, &mut bytes, work, maximum)?;
        Ok(u32::from_le_bytes(bytes))
    }

    pub(super) fn lookup(
        &self,
        state: &JsonStrings,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<Option<StringRef>, KernelStartupError> {
        if wrapper == 0 {
            return Ok(None);
        }
        self.check_wrapper(state, wrapper, work, maximum)?;
        if let Some(patch) = self.patches[..self.patch_len]
            .iter()
            .flatten()
            .find(|patch| patch.address == wrapper)
        {
            charge(work, 256, maximum)?;
            let mut proof = [0; 8];
            if let Some(value) = value {
                value.read_slice(memory, wrapper, &mut proof)?;
            } else {
                memory
                    .read(wrapper, &mut proof)
                    .map_err(|e| guest(e, false))?;
            }
            memory
                .preflight_distinct_backing(wrapper, 8, 32)
                .map_err(|e| guest(e, false))?;
            self.alias_read(memory, wrapper, 8, Some(wrapper))?;
            return Ok(match patch.pointer {
                Pointer::Zero => None,
                Pointer::Token(index) => Some(StringRef::Pending(index)),
            });
        }
        let token = self.word(memory, value, wrapper, work, maximum)?;
        if token == 0 {
            return Ok(None);
        }
        charge(work, state.records.len() as u64, maximum)?;
        let (index, record) = state
            .records
            .iter()
            .enumerate()
            .find(|(_, record)| record.token == token)
            .ok_or(KernelStartupError::InvalidJsonString)?;
        if !record.live || self.retired == Some(index) {
            return Err(KernelStartupError::InvalidJsonString);
        }
        Ok(Some(
            self.changed(StringRef::Existing(index))
                .map_or(StringRef::Existing(index), StringRef::Pending),
        ))
    }

    pub(super) fn mutable(
        &mut self,
        state: &JsonStrings,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<StringRef, KernelStartupError> {
        if wrapper == 0 {
            return Err(KernelStartupError::InvalidJsonString);
        }
        if let Some(reference) = self.lookup(state, memory, value, wrapper, work, maximum)? {
            return Ok(reference);
        }
        if let Some(value) = value
            && wrapper < at(value.address, 32)?
            && value.address < at(wrapper, 8)?
        {
            return Err(KernelStartupError::InvalidJsonString);
        }
        let index = self.add(None, Vec::new(), None)?;
        self.patch(wrapper, Pointer::Token(index))?;
        Ok(StringRef::Pending(index))
    }

    pub(super) fn constant(
        &self,
        state: &JsonStrings,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<Vec<u8>, KernelStartupError> {
        match self.lookup(state, memory, value, wrapper, work, maximum)? {
            Some(reference) => self.snapshot(state, memory, value, reference, work, maximum),
            None => Ok(Vec::new()),
        }
    }

    pub(super) fn snapshot(
        &self,
        state: &JsonStrings,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        reference: StringRef,
        work: &mut u64,
        maximum: u64,
    ) -> Result<Vec<u8>, KernelStartupError> {
        if let Some(index) = self.changed(reference) {
            let bytes = &self.pending[index].as_ref().expect("pending String").bytes;
            charge(work, bytes.len() as u64, maximum)?;
            let mut copy = Vec::new();
            copy.try_reserve_exact(bytes.len())
                .map_err(|_| KernelStartupError::Allocation)?;
            self.scratch_with(copy.capacity())?;
            copy.extend_from_slice(bytes);
            return Ok(copy);
        }
        let StringRef::Existing(index) = reference else {
            unreachable!("pending String reference");
        };
        let record = state.records[index];
        if record.length > MAX_STRING_BYTES {
            return Err(KernelStartupError::InvalidJsonString);
        }
        charge(work, record.length as u64, maximum)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(record.length)
            .map_err(|_| KernelStartupError::Allocation)?;
        self.scratch_with(bytes.capacity())?;
        bytes.resize(record.length, 0);
        for (index, chunk) in bytes.chunks_mut(32).enumerate() {
            self.read(
                memory,
                value,
                at(record.data, index * 32)?,
                chunk,
                work,
                maximum,
            )?;
        }
        Ok(bytes)
    }

    pub(super) fn cstring(
        &self,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        address: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<Vec<u8>, KernelStartupError> {
        if address == 0 {
            return Ok(Vec::new());
        }
        let mut bytes = Vec::new();
        for index in 0..=MAX_STRING_BYTES {
            let byte = self.byte(memory, value, at(address, index)?, work, maximum)?;
            if byte == 0 {
                return Ok(bytes);
            }
            if index == MAX_STRING_BYTES {
                return Err(KernelStartupError::Budget("JSON String bytes"));
            }
            if bytes.len() == bytes.capacity() {
                let next = (bytes.capacity().max(32) * 2).min(MAX_STRING_BYTES);
                bytes
                    .try_reserve_exact(next - bytes.len())
                    .map_err(|_| KernelStartupError::Allocation)?;
                self.scratch_with(bytes.capacity())?;
            }
            bytes.push(byte);
        }
        unreachable!("bounded CString loop")
    }

    pub(super) fn destroy_string(
        &mut self,
        state: &JsonStrings,
        memory: &PausedNativeMemory<'_>,
        value: Option<&StagedValue>,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        if let Some(reference) = self.lookup(state, memory, value, wrapper, work, maximum)? {
            let StringRef::Existing(index) = reference else {
                return Err(KernelStartupError::InvalidJsonString);
            };
            self.retired = Some(index);
            self.retired_data = Some(state.records[index]);
        }
        self.patch(wrapper, Pointer::Zero)
    }

    pub(super) fn clear_value(
        &mut self,
        state: &JsonStrings,
        memory: &PausedNativeMemory<'_>,
        value: &StagedValue,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        charge(work, state.wrappers.len() as u64, maximum)?;
        let index = state
            .wrappers
            .iter()
            .position(|record| {
                record.address == wrapper && record.value == value.address && record.live
            })
            .ok_or(KernelStartupError::InvalidJsonString)?;
        if self.retired_wrapper == Some(index) {
            return Err(KernelStartupError::InvalidJsonString);
        }
        if let Some(reference) = self.lookup(state, memory, Some(value), wrapper, work, maximum)? {
            let StringRef::Existing(record) = reference else {
                return Err(KernelStartupError::InvalidJsonString);
            };
            self.retired = Some(record);
            self.retired_data = Some(state.records[record]);
        }
        self.retired_wrapper = Some(index);
        Ok(())
    }

    fn layout(&mut self) -> Result<usize, KernelStartupError> {
        let mut size = 0_usize;
        for pending in self.pending[..self.pending_len].iter_mut().flatten() {
            size = size
                .checked_add(7)
                .map(|n| n & !7)
                .ok_or(KernelStartupError::Budget("JSON blob"))?;
            if pending.value.is_some() {
                pending.wrapper_offset = Some(size);
                size = size
                    .checked_add(8)
                    .ok_or(KernelStartupError::Budget("JSON blob"))?;
            }
            if pending.existing.is_none() {
                pending.token_offset = size;
                size = size
                    .checked_add(8)
                    .ok_or(KernelStartupError::Budget("JSON blob"))?;
            }
            pending.data_offset = size;
            size = size
                .checked_add(pending.bytes.len())
                .and_then(|n| n.checked_add(1))
                .ok_or(KernelStartupError::Budget("JSON blob"))?;
        }
        Ok(size)
    }

    fn finish(
        &mut self,
        base: u64,
        bytes: &mut [u8],
        state: &JsonStrings,
    ) -> Result<RuntimeDataOutputs, Failure> {
        for pending in self.pending[..self.pending_len].iter_mut().flatten() {
            let token = if let Some(index) = pending.existing {
                state.records[index].token
            } else {
                base.checked_add(pending.token_offset as u64)
                    .ok_or(Failure::Limit("JSON token address"))?
            };
            let data = base
                .checked_add(pending.data_offset as u64)
                .ok_or(Failure::Limit("JSON data address"))?;
            pending.resolved = StringRecord {
                token,
                data,
                length: pending.bytes.len(),
                live: true,
            };
            bytes[pending.data_offset..pending.data_offset + pending.bytes.len()]
                .copy_from_slice(&pending.bytes);
            if let Some(offset) = pending.wrapper_offset {
                pending.wrapper_address = base
                    .checked_add(offset as u64)
                    .ok_or(Failure::Limit("JSON wrapper address"))?;
                bytes[offset..offset + 8].copy_from_slice(&token.to_le_bytes());
            }
        }
        if let Some(index) = self.value_string {
            let wrapper = self.pending[index]
                .as_ref()
                .expect("Value String")
                .wrapper_address;
            self.value
                .as_mut()
                .expect("staged Value")
                .write(16, &wrapper.to_le_bytes());
        }
        let mut outputs = RuntimeDataOutputs::new();
        let mut result = Ok(());
        if let Some(value) = &self.value {
            value.outputs(|address, bytes| {
                if result.is_ok() {
                    result = outputs.push(address, bytes);
                }
            });
        }
        result?;
        for patch in self.patches[..self.patch_len].iter().flatten() {
            let pointer = match patch.pointer {
                Pointer::Zero => 0,
                Pointer::Token(index) => {
                    self.pending[index]
                        .as_ref()
                        .expect("String token")
                        .resolved
                        .token
                }
            };
            outputs.push(patch.address, &pointer.to_le_bytes())?;
        }
        Ok(outputs)
    }

    pub(super) fn publish(
        mut self,
        state: &mut JsonStrings,
        memory: &mut PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        let size = self.layout()?;
        let new_records = self.pending[..self.pending_len]
            .iter()
            .flatten()
            .filter(|p| p.existing.is_none())
            .count();
        let new_wrappers = usize::from(self.value_string.is_some());
        let mut records = growth(&state.records, new_records, work, maximum)?;
        let mut wrappers = growth(&state.wrappers, new_wrappers, work, maximum)?;
        let retained = metadata_bytes(
            records.as_ref().unwrap_or(&state.records),
            wrappers.as_ref().unwrap_or(&state.wrappers),
        )?;
        if retained > MAX_METADATA_BYTES {
            return Err(KernelStartupError::Budget("JSON metadata"));
        }
        let growth_scratch = records
            .as_ref()
            .map_or(0, |v| v.capacity() * size_of::<StringRecord>())
            + wrappers
                .as_ref()
                .map_or(0, |v| v.capacity() * size_of::<WrapperRecord>());
        self.scratch_with(growth_scratch)?;
        if let Some(value) = &self.value {
            memory
                .preflight_distinct_backing(value.address, 32, 32)
                .map_err(|e| guest(e, true))?;
            for patch in self.patches[..self.patch_len].iter().flatten() {
                if patch.address < at(value.address, 32)? && value.address < at(patch.address, 8)? {
                    return Err(KernelStartupError::InvalidJsonString);
                }
            }
        }
        let mapped = (size as u64)
            .checked_add(0x3fff)
            .map(|n| n & !0x3fff)
            .ok_or(KernelStartupError::Budget("JSON guest storage"))?;
        let guest_bytes = state
            .guest_bytes
            .checked_add(mapped)
            .filter(|n| *n <= MAX_GUEST_BYTES)
            .ok_or(KernelStartupError::Budget("JSON guest storage"))?;
        if size != 0 {
            charge(
                work,
                size as u64 + growth_scratch as u64 + self.scratch()? as u64,
                maximum,
            )?;
            let mut guest = super::super::super::kernel_adapter::KernelMemory {
                paused: memory,
                source: None,
                supplied_modules: 0,
                lookup_limits: None,
                lookup_work: None,
                loaded_modules: None,
                thread_tls: None,
                initial_entry: None,
            };
            let held = self
                .scratch()?
                .checked_add(growth_scratch)
                .ok_or(KernelStartupError::Budget("JSON scratch"))?;
            kernel
                .publish_runtime_data(size, held, &mut guest, |base, bytes| {
                    self.finish(base, bytes, state)
                })
                .map_err(KernelStartupError::Kernel)?;
        } else {
            let mut prospective = self.patch_len;
            if let Some(value) = &self.value {
                value.outputs(|_, _| prospective += 1);
            }
            if prospective > 3 {
                return Err(KernelStartupError::Budget("JSON outputs"));
            }
            let mut storage = [[0_u8; 32]; 3];
            let mut addresses = [0; 3];
            let mut lengths = [0; 3];
            let mut count = 0;
            if let Some(value) = &self.value {
                value.outputs(|address, bytes| {
                    addresses[count] = address;
                    lengths[count] = bytes.len();
                    storage[count][..bytes.len()].copy_from_slice(bytes);
                    count += 1;
                });
            }
            for patch in self.patches[..self.patch_len].iter().flatten() {
                if !matches!(patch.pointer, Pointer::Zero) {
                    return Err(KernelStartupError::InvalidJsonString);
                }
                addresses[count] = patch.address;
                lengths[count] = 8;
                count += 1;
            }
            if count != 0 {
                let mut outputs = [NativeMemoryWrite {
                    address: 0,
                    bytes: &[],
                }; 3];
                for index in 0..count {
                    outputs[index] = NativeMemoryWrite {
                        address: addresses[index],
                        bytes: &storage[index][..lengths[index]],
                    };
                }
                memory
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::None,
                            writes: &outputs[..count],
                        },
                        &NativeTransactionLimits {
                            max_outputs: 3,
                            max_output_bytes: 96,
                            max_preflight_steps: 256,
                            max_scratch_bytes: 0,
                        },
                    )
                    .map_err(|e| guest(e, true))?;
            }
        }
        // Every allocation, checked calculation and guest effect has succeeded.
        // The following bounded Copy-record swaps and pushes cannot allocate.
        if let Some(ref mut next) = records {
            std::mem::swap(&mut state.records, next);
        }
        if let Some(ref mut next) = wrappers {
            std::mem::swap(&mut state.wrappers, next);
        }
        if let Some(index) = self.retired {
            state.records[index].live = false;
        }
        if let Some(index) = self.retired_wrapper {
            state.wrappers[index].live = false;
        }
        for pending in self.pending[..self.pending_len].iter().flatten() {
            if let Some(index) = pending.existing {
                state.records[index] = pending.resolved;
            } else {
                state.records.push(pending.resolved);
            }
            if let Some(value) = pending.value {
                state.wrappers.push(WrapperRecord {
                    address: pending.wrapper_address,
                    value,
                    live: true,
                });
            }
        }
        state.guest_bytes = guest_bytes;
        Ok(())
    }
}

fn metadata_bytes(
    records: &Vec<StringRecord>,
    wrappers: &Vec<WrapperRecord>,
) -> Result<usize, KernelStartupError> {
    records
        .capacity()
        .checked_mul(size_of::<StringRecord>())
        .and_then(|n| {
            n.checked_add(
                wrappers
                    .capacity()
                    .checked_mul(size_of::<WrapperRecord>())?,
            )
        })
        .ok_or(KernelStartupError::Budget("JSON metadata"))
}

fn growth<T: Copy>(
    old: &Vec<T>,
    count: usize,
    work: &mut u64,
    maximum: u64,
) -> Result<Option<Vec<T>>, KernelStartupError> {
    let length = old
        .len()
        .checked_add(count)
        .filter(|n| *n <= MAX_RECORDS)
        .ok_or(KernelStartupError::Budget("JSON records"))?;
    if length <= old.capacity() {
        return Ok(None);
    }
    charge(work, old.len() as u64, maximum)?;
    let requested = length.max(old.capacity().saturating_mul(2).min(MAX_RECORDS));
    let mut next = Vec::new();
    next.try_reserve_exact(requested)
        .map_err(|_| KernelStartupError::Allocation)?;
    if next
        .capacity()
        .checked_mul(size_of::<T>())
        .is_none_or(|n| n > MAX_METADATA_BYTES)
    {
        return Err(KernelStartupError::Budget("JSON metadata"));
    }
    next.extend_from_slice(old);
    Ok(Some(next))
}
