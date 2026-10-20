// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: owned shader metadata and checked in-place creation.
// Pinned agc.cpp/shader.cpp/shader.h; see NOTICE.

use super::driver::Failure;
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryError, NativeMemoryTransaction, NativeMemoryWrite,
    NativeOwnerId, NativeRange, NativeTransactionLimits, PausedNativeMemory,
};

const INVALID_PROGRAM: u64 = 0x8a6c_0005;
const MAX_EDITS: usize = 128;
const MAX_MAPS: usize = 16384;

pub(super) mod fusion;
pub(super) mod interpolants;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderFailure {
    NullArgument,
    Header,
    Version,
    CodeAddress,
    PhysicalAlias,
    Stage,
    Registers,
}
impl ShaderFailure {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NullArgument => "shader-null-argument",
            Self::Header => "shader-header",
            Self::Version => "shader-version",
            Self::CodeAddress => "shader-code-address",
            Self::PhysicalAlias => "shader-physical-alias",
            Self::Stage => "shader-stage",
            Self::Registers => "shader-registers",
        }
    }
}

/// Guest addresses are retained data, never host pointers or access authority.
/// Payload references remain live; later consumers must check the same owner.
#[derive(Clone, Copy)]
pub struct MappedShader {
    pub code: u64,
    pub user_data: u64,
    pub input_semantics: u64,
    pub input_count: u32,
    pub code_bytes: u32,
    pub scratch_dwords: u16,
    pub kind: u8,
}

pub(super) struct State {
    owner: Option<NativeOwnerId>,
    entries: Vec<MappedShader>,
    generation: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            entries: Vec::new(),
            generation: 1,
        }
    }
}

struct Work<'a> {
    used: &'a mut u64,
    maximum: u64,
}
impl Work<'_> {
    fn charge(&mut self, n: u64) -> Result<(), Failure> {
        *self.used = self
            .used
            .checked_add(n)
            .filter(|n| *n <= self.maximum)
            .ok_or(Failure::Limit)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct ByteEdit {
    address: u64,
    value: u8,
}
struct Edits<const CAP: usize = MAX_EDITS> {
    bytes: [ByteEdit; CAP],
    used: usize,
}
impl<const CAP: usize> Edits<CAP> {
    fn new() -> Self {
        Self {
            bytes: [ByteEdit {
                address: 0,
                value: 0,
            }; CAP],
            used: 0,
        }
    }

    fn aliases(
        &self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
        length: usize,
        work: &mut Work<'_>,
    ) -> Result<(), Failure> {
        let end = address.checked_add(length as u64).ok_or(Failure::Memory)?;
        if length > 8 {
            return Err(Failure::Limit);
        }
        work.charge(self.used as u64 + length as u64 + 1)?;
        for edit in &self.bytes[..self.used] {
            // Same-VA overlap is modeled in source order. Distinct virtual
            // views of identical physical bytes need an ordered-backing API;
            // refuse explicitly instead of reading an unstaged alias value.
            if address <= edit.address && edit.address < end {
                continue;
            }
            // Admit the complete two-page/one-byte metadata visit bound first.
            work.charge(16)?;
            match memory.preflight_disjoint_backing(
                NativeRange {
                    address,
                    size: length as u64,
                },
                NativeRange {
                    address: edit.address,
                    size: 1,
                },
                16,
            ) {
                Ok(_) => {}
                Err(NativeMemoryError::Overlap) => {
                    return Err(Failure::Shader(ShaderFailure::PhysicalAlias));
                }
                Err(NativeMemoryError::BudgetExceeded) => return Err(Failure::Limit),
                Err(_) => return Err(Failure::Memory),
            }
        }
        Ok(())
    }
    fn read<const N: usize>(
        &self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
        work: &mut Work<'_>,
    ) -> Result<[u8; N], Failure> {
        let mut output = [0; N];
        self.aliases(memory, address, N, work)?;
        memory
            .read(address, &mut output)
            .map_err(|_| Failure::Memory)?;
        for edit in &self.bytes[..self.used] {
            if let Some(i) = edit
                .address
                .checked_sub(address)
                .and_then(|n| usize::try_from(n).ok())
                .filter(|i| *i < N)
            {
                output[i] = edit.value;
            }
        }
        Ok(output)
    }
    fn write(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
        input: &[u8],
        work: &mut Work<'_>,
    ) -> Result<(), Failure> {
        self.aliases(memory, address, input.len(), work)?;
        memory
            .preflight_write(address, input.len() as u64)
            .map_err(|_| Failure::Memory)?;
        for (i, value) in input.iter().copied().enumerate() {
            let at = address.checked_add(i as u64).ok_or(Failure::Memory)?;
            if let Some(edit) = self.bytes[..self.used].iter_mut().find(|e| e.address == at) {
                edit.value = value;
            } else {
                let slot = self.bytes.get_mut(self.used).ok_or(Failure::Limit)?;
                *slot = ByteEdit { address: at, value };
                self.used += 1;
            }
        }
        Ok(())
    }
    fn relocate(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        field: u64,
        work: &mut Work<'_>,
    ) -> Result<(), Failure> {
        let offset = u64::from_le_bytes(self.read(memory, field, work)?);
        if offset != 0 {
            self.write(
                memory,
                field,
                &offset.wrapping_add(field).to_le_bytes(),
                work,
            )?;
        }
        Ok(())
    }
    fn publish(
        &self,
        memory: &mut PausedNativeMemory<'_>,
        work: &mut Work<'_>,
    ) -> Result<(), Failure> {
        work.charge(self.used as u64 + 1)?;
        let mut data = [0; CAP];
        for (target, edit) in data.iter_mut().zip(&self.bytes[..self.used]) {
            *target = edit.value;
        }
        let mut writes = [NativeMemoryWrite {
            address: 0,
            bytes: &[],
        }; CAP];
        let mut count = 0;
        let mut start = 0;
        while start < self.used {
            let mut stop = start + 1;
            while stop < self.used
                && self.bytes[stop - 1].address.checked_add(1) == Some(self.bytes[stop].address)
            {
                stop += 1;
            }
            writes[count] = NativeMemoryWrite {
                address: self.bytes[start].address,
                bytes: &data[start..stop],
            };
            count += 1;
            start = stop;
        }
        work.charge((count as u64 + 1).pow(2) * 4)?;
        memory
            .apply_transaction(
                NativeMemoryTransaction {
                    effect: NativeMemoryEffect::None,
                    writes: &writes[..count],
                },
                &NativeTransactionLimits {
                    max_outputs: CAP,
                    max_output_bytes: CAP as u64,
                    ..Default::default()
                },
            )
            .map_err(|e| match e {
                NativeMemoryError::BudgetExceeded => Failure::Limit,
                NativeMemoryError::Overlap => Failure::Shader(ShaderFailure::PhysicalAlias),
                _ => Failure::Memory,
            })
    }
}

fn at(base: u64, offset: u64) -> Result<u64, Failure> {
    base.checked_add(offset).ok_or(Failure::Memory)
}

fn program_register(kind: u8) -> Option<u32> {
    Some(match kind {
        0 => 0x20c,
        1 => 0x8,
        2 => 0xc8,
        3 => 0x148,
        6 => 0x88,
        7 => 0x108,
        _ => return None,
    })
}

impl State {
    pub(super) fn lookup_work(&self) -> u64 {
        self.entries.len() as u64 + 1
    }
    pub(super) fn mapping(&self, owner: NativeOwnerId, address: u64) -> Option<MappedShader> {
        if self.owner != Some(owner) {
            return None;
        }
        self.entries
            .iter()
            .find(|entry| entry.code == address)
            .copied()
    }

    pub(super) fn create(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
        max_maps: usize,
        used_work: &mut u64,
        maximum_work: u64,
    ) -> Result<u64, Failure> {
        let [destination, header, code, ..] = args;
        if destination == 0 || header == 0 || code == 0 {
            return Err(Failure::Shader(ShaderFailure::NullArgument));
        }
        if max_maps > MAX_MAPS {
            return Err(Failure::Limit);
        }
        let owner = memory.owner_id();
        if self.owner.is_some_and(|id| id != owner) {
            return Err(Failure::Memory);
        }
        let mut work = Work {
            used: used_work,
            maximum: maximum_work,
        };
        let mut edits = Edits::new();
        for offset in [24, 32, 8, 40, 48, 56] {
            edits.relocate(memory, at(header, offset)?, &mut work)?;
        }
        let user = u64::from_le_bytes(edits.read(memory, at(header, 8)?, &mut work)?);
        if user != 0 {
            for offset in [0, 8, 16, 24, 32] {
                edits.relocate(memory, at(user, offset)?, &mut work)?;
            }
        }
        edits.write(memory, at(header, 16)?, &code.to_le_bytes(), &mut work)?;
        if u32::from_le_bytes(edits.read(memory, header, &mut work)?) != 0x3433_3231 {
            return Err(Failure::Shader(ShaderFailure::Header));
        }
        if u32::from_le_bytes(edits.read(memory, at(header, 4)?, &mut work)?) != 0x18 {
            return Err(Failure::Shader(ShaderFailure::Version));
        }
        let entry = MappedShader {
            code,
            user_data: u64::from_le_bytes(edits.read(memory, at(header, 8)?, &mut work)?),
            input_semantics: u64::from_le_bytes(edits.read(memory, at(header, 48)?, &mut work)?),
            input_count: u32::from_le_bytes(edits.read(memory, at(header, 80)?, &mut work)?),
            code_bytes: u32::from_le_bytes(edits.read(memory, at(header, 68)?, &mut work)?),
            scratch_dwords: u16::from_le_bytes(edits.read(memory, at(header, 84)?, &mut work)?),
            kind: edits.read::<1>(memory, at(header, 90)?, &mut work)?[0],
        };
        if code & 0xffff_0000_0000_00ff != 0 {
            return Err(Failure::Shader(ShaderFailure::CodeAddress));
        }
        work.charge(self.entries.len() as u64 + 1)?;
        let existing = self.entries.iter().position(|e| e.code == code);
        if existing.is_none() && self.entries.len() >= max_maps {
            return Err(Failure::Limit);
        }
        let next_generation = self.generation.checked_add(1).ok_or(Failure::Limit)?;
        if existing.is_none() && self.entries.len() == self.entries.capacity() {
            work.charge((max_maps * std::mem::size_of::<MappedShader>()) as u64)?;
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(max_maps)
                .map_err(|_| Failure::Allocation)?;
            if entries.capacity() > max_maps {
                return Err(Failure::Limit);
            }
            entries.extend_from_slice(&self.entries);
            self.entries = entries;
        }
        let result = patch_program(&mut edits, memory, header, code, &mut work)?;
        if result == 0 {
            edits.write(memory, destination, &header.to_le_bytes(), &mut work)?;
        }
        edits.publish(memory, &mut work)?;
        // Source maps metadata even when program-register validation returns
        // its guest error. Host/admission failures above publish neither part.
        if let Some(i) = existing {
            self.entries[i] = entry;
        } else {
            self.entries.push(entry);
        }
        self.owner = Some(owner);
        self.generation = next_generation;
        Ok(result)
    }
}

fn patch_program(
    edits: &mut Edits,
    memory: &PausedNativeMemory<'_>,
    header: u64,
    code: u64,
    work: &mut Work<'_>,
) -> Result<u64, Failure> {
    let kind = edits.read::<1>(memory, at(header, 90)?, work)?[0];
    let Some(expected) = program_register(kind) else {
        return Ok(0);
    };
    let registers = u64::from_le_bytes(edits.read(memory, at(header, 32)?, work)?);
    let count = edits.read::<1>(memory, at(header, 92)?, work)?[0];
    if registers == 0 || count == 0 {
        return Ok(INVALID_PROGRAM);
    }
    for i in 0..u64::from(count) {
        let low = at(registers, i * 8)?;
        if u32::from_le_bytes(edits.read(memory, low, work)?) != expected {
            continue;
        }
        if i + 1 >= u64::from(count) {
            return Ok(INVALID_PROGRAM);
        }
        let high = at(low, 8)?;
        if u32::from_le_bytes(edits.read(memory, high, work)?) != expected + 1 {
            return Ok(INVALID_PROGRAM);
        }
        let lo = u32::from_le_bytes(edits.read(memory, at(low, 4)?, work)?);
        let hi = u32::from_le_bytes(edits.read(memory, at(high, 4)?, work)?);
        let offset = (u64::from(lo) << 8) | ((u64::from(hi) & 255) << 40);
        let address = code.wrapping_add(offset);
        edits.write(
            memory,
            at(low, 4)?,
            &((address >> 8) as u32).to_le_bytes(),
            work,
        )?;
        let high_value = (hi & 0xffff_ff00) | ((address >> 40) as u32 & 255);
        edits.write(memory, at(high, 4)?, &high_value.to_le_bytes(), work)?;
        return Ok(0);
    }
    Ok(INVALID_PROGRAM)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod fusion_tests;
