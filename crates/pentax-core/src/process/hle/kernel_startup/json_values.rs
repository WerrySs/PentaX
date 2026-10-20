// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted from pinned KytyPS5 libJson2.cpp; see docs/upstream-provenance.md.

//! Primitive 32-byte JsonValue operations over the actual paused guest owner.
//! Fixed staging preserves the source's ordered reads/writes without publishing
//! a partially cleared object. The production String owner implements the same
//! staged cleanup/copy hooks; the legacy test entry point refuses owned storage.

use super::{KernelStartupError, charge, require};
use crate::process::hle::json_provider::JsonFunction;
#[cfg(test)]
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativeTransactionLimits,
};
use pentax_platform::native_memory::{NativeMemoryError, NativeRange, PausedNativeMemory};

const VALUE_BYTES: usize = 32;
const UNION: usize = 16;
const TYPE: usize = 28;
const ALIAS_WORK: u64 = 32;
const MUTATION_WORK: u64 = 512;
const QUERY_WORK: u64 = 64;

/// Exact distinct, immutable guest defaults attested by the provider owner.
#[derive(Clone, Copy)]
pub(super) struct ScalarDefaults {
    pub boolean: u64,
    pub integer: u64,
    pub real: u64,
}

fn guest(error: NativeMemoryError, write: bool) -> KernelStartupError {
    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
        super::super::kernel_adapter::error(error, write),
    ))
}

fn at(base: u64, offset: usize) -> Result<u64, KernelStartupError> {
    base.checked_add(offset as u64)
        .ok_or_else(|| guest(NativeMemoryError::InvalidRange, false))
}

fn read_type(memory: &PausedNativeMemory<'_>, object: u64) -> Result<u32, KernelStartupError> {
    if object == 0 {
        return Ok(0);
    }
    let mut bytes = [0; 4];
    memory
        .read(at(object, TYPE)?, &mut bytes)
        .map_err(|error| guest(error, false))?;
    Ok(u32::from_le_bytes(bytes))
}

pub(super) struct StagedValue {
    pub(super) address: u64,
    bytes: [u8; VALUE_BYTES],
    dirty: [bool; VALUE_BYTES],
}

impl StagedValue {
    pub(super) fn new(address: u64) -> Result<Self, KernelStartupError> {
        at(address, VALUE_BYTES)?;
        Ok(Self {
            address,
            bytes: [0; VALUE_BYTES],
            dirty: [false; VALUE_BYTES],
        })
    }
    pub(super) fn write(&mut self, offset: usize, bytes: &[u8]) {
        self.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        self.dirty[offset..offset + bytes.len()].fill(true);
    }
    pub(super) fn init(&mut self) {
        self.write(0, &[0; VALUE_BYTES]);
    }
    pub(super) fn set_type(&mut self, kind: u32) {
        self.write(TYPE, &kind.to_le_bytes());
    }

    /// Source fields are permission checked even when an earlier staged write
    /// supplies their value. Distinct physical aliases refuse; ordinary virtual
    /// overlap observes all prior writes in their source order.
    pub(super) fn read<const N: usize>(
        &self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
    ) -> Result<[u8; N], KernelStartupError> {
        let mut bytes = [0; N];
        self.read_slice(memory, address, &mut bytes)?;
        Ok(bytes)
    }

    /// Reads are deliberately small so the existing fixed alias-work bound
    /// remains valid. Dynamic String snapshots call this in <=32-byte pieces.
    pub(super) fn read_slice(
        &self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
        bytes: &mut [u8],
    ) -> Result<(), KernelStartupError> {
        require(bytes.len() <= VALUE_BYTES, "JSON staged read size")?;
        memory
            .read(address, bytes)
            .map_err(|error| guest(error, false))?;
        if self.dirty.iter().any(|&dirty| dirty) {
            let end = at(address, bytes.len())?;
            let object_end = at(self.address, VALUE_BYTES)?;
            if address < object_end && self.address < end {
                let start = address.min(self.address);
                let size = end.max(object_end) - start;
                memory
                    .preflight_distinct_backing(start, size, ALIAS_WORK)
                    .map_err(|error| guest(error, false))?;
            } else {
                memory
                    .preflight_distinct_backing(address, bytes.len() as u64, ALIAS_WORK)
                    .map_err(|error| guest(error, false))?;
                memory
                    .preflight_disjoint_backing(
                        NativeRange {
                            address: self.address,
                            size: VALUE_BYTES as u64,
                        },
                        NativeRange {
                            address,
                            size: bytes.len() as u64,
                        },
                        ALIAS_WORK,
                    )
                    .map_err(|error| guest(error, false))?;
            }
            for (index, byte) in bytes.iter_mut().enumerate() {
                let source = address + index as u64;
                if let Some(offset) = source.checked_sub(self.address)
                    && offset < VALUE_BYTES as u64
                    && self.dirty[offset as usize]
                {
                    *byte = self.bytes[offset as usize];
                }
            }
        }
        Ok(())
    }

    pub(super) fn clear(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        storage: &mut impl ValueStorage,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        let kind = u32::from_le_bytes(self.read(memory, at(self.address, TYPE)?)?);
        if matches!(kind, 5..=7) {
            let pointer = u64::from_le_bytes(self.read(memory, at(self.address, UNION)?)?);
            if pointer != 0 {
                if kind == 5 {
                    storage.clear_string(memory, self, pointer, work, maximum)?;
                } else {
                    return Err(KernelStartupError::UnavailableJsonValueStorage);
                }
            }
        }
        self.write(0, &[0; 24]);
        self.set_type(0); // The four padding bytes at24..28 survive Clear.
        Ok(())
    }

    fn copy(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        source: u64,
        storage: &mut impl ValueStorage,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        if source == 0 || source == self.address {
            return Ok(());
        }
        self.clear(memory, storage, work, maximum)?;
        let first_type = u32::from_le_bytes(self.read(memory, at(source, TYPE)?)?);
        self.set_type(first_type);
        // The assignment above can change src->type under virtual overlap.
        let switch_type = u32::from_le_bytes(self.read(memory, at(source, TYPE)?)?);
        match switch_type {
            1 => self.write(UNION, &self.read::<1>(memory, at(source, UNION)?)?),
            2..=4 => self.write(UNION, &self.read::<8>(memory, at(source, UNION)?)?),
            5 => {
                let pointer = u64::from_le_bytes(self.read(memory, at(source, UNION)?)?);
                storage.copy_string(memory, self, pointer, work, maximum)?;
            }
            6 | 7 => return Err(KernelStartupError::UnavailableJsonValueStorage),
            _ => self.write(UNION, &[0; 8]),
        }
        Ok(())
    }

    pub(super) fn outputs(&self, mut write: impl FnMut(u64, &[u8])) {
        if self.dirty.iter().all(|&dirty| dirty) {
            write(self.address, &self.bytes);
        } else {
            write(self.address, &self.bytes[..24]);
            // new() has already checked address+32.
            write(self.address + TYPE as u64, &self.bytes[TYPE..]);
        }
    }

    #[cfg(test)]
    pub(super) fn publish(
        self,
        memory: &mut PausedNativeMemory<'_>,
    ) -> Result<(), KernelStartupError> {
        memory
            .preflight_distinct_backing(self.address, VALUE_BYTES as u64, ALIAS_WORK)
            .map_err(|error| guest(error, true))?;
        let limits = NativeTransactionLimits {
            max_outputs: 2,
            max_output_bytes: VALUE_BYTES as u64,
            max_preflight_steps: 64,
            max_scratch_bytes: 0,
        };
        if self.dirty.iter().all(|&dirty| dirty) {
            memory
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::None,
                        writes: &[NativeMemoryWrite {
                            address: self.address,
                            bytes: &self.bytes,
                        }],
                    },
                    &limits,
                )
                .map_err(|error| guest(error, true))
        } else {
            // Clear, setters and distinct assignment touch exactly these spans.
            memory
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::None,
                        writes: &[
                            NativeMemoryWrite {
                                address: self.address,
                                bytes: &self.bytes[..24],
                            },
                            NativeMemoryWrite {
                                address: at(self.address, TYPE)?,
                                bytes: &self.bytes[TYPE..],
                            },
                        ],
                    },
                    &limits,
                )
                .map_err(|error| guest(error, true))
        }
    }
}

pub(super) trait ValueStorage {
    fn clear_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &StagedValue,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError>;
    fn copy_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &mut StagedValue,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError>;
    fn empty_string(&mut self, value: &mut StagedValue) -> Result<(), KernelStartupError>;
}

#[cfg(test)]
struct NoStorage;
#[cfg(test)]
impl ValueStorage for NoStorage {
    fn clear_string(
        &mut self,
        _: &PausedNativeMemory<'_>,
        _: &StagedValue,
        _: u64,
        _: &mut u64,
        _: u64,
    ) -> Result<(), KernelStartupError> {
        Err(KernelStartupError::UnavailableJsonValueStorage)
    }
    fn copy_string(
        &mut self,
        _: &PausedNativeMemory<'_>,
        _: &mut StagedValue,
        _: u64,
        _: &mut u64,
        _: u64,
    ) -> Result<(), KernelStartupError> {
        Err(KernelStartupError::UnavailableJsonValueStorage)
    }
    fn empty_string(&mut self, _: &mut StagedValue) -> Result<(), KernelStartupError> {
        Err(KernelStartupError::UnavailableJsonValueStorage)
    }
}

pub(super) enum ValueAction {
    Return(u64),
    Write { value: StagedValue, result: u64 },
}

#[cfg(test)]
pub(super) fn service(
    memory: &mut PausedNativeMemory<'_>,
    function: JsonFunction,
    args: [u64; 6],
    vector_args: Option<&[[u8; 16]; 8]>,
    defaults: ScalarDefaults,
    work: &mut u64,
    maximum: u64,
) -> Result<u64, KernelStartupError> {
    match prepare(
        memory,
        function,
        args,
        vector_args,
        defaults,
        work,
        maximum,
        &mut NoStorage,
    )? {
        ValueAction::Return(result) => Ok(result),
        ValueAction::Write { value, result } => {
            value.publish(memory)?;
            Ok(result)
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "source-bound staged Value arguments"
)]
pub(super) fn prepare(
    memory: &PausedNativeMemory<'_>,
    function: JsonFunction,
    args: [u64; 6],
    vector_args: Option<&[[u8; 16]; 8]>,
    defaults: ScalarDefaults,
    work: &mut u64,
    maximum: u64,
    storage: &mut impl ValueStorage,
) -> Result<ValueAction, KernelStartupError> {
    use JsonFunction::*;
    let query = matches!(
        function,
        ValueGetType | ValueGetBoolean | ValueGetInteger | ValueGetReal | ValueCount
    );
    require(
        query
            || matches!(
                function,
                ValueBoolCtor
                    | ValueIntCtor
                    | ValueDoubleCtor
                    | ValueTypeCtor
                    | ValueSetBool
                    | ValueSetInt
                    | ValueSetUInt
                    | ValueSetDouble
                    | ValueSetType
                    | ValueCopyCtor
                    | ValueAssign
                    | ValueDtor
                    | ValueClearMethod
            ),
        "JSON value function",
    )?;
    // Fixed conservative bounds include field reads,32-byte overlay passes,
    // <=2-page physical-alias queries and the complete None-effect transaction.
    charge(
        work,
        if query { QUERY_WORK } else { MUTATION_WORK },
        maximum,
    )?;
    // SysV passes the real payload in XMM0, independently of integer args.
    // Missing capture must refuse even a null destination; never manufacture
    // a default from RSI or convert through a Rust floating-point value.
    let mut real_payload = [0; 8];
    if matches!(function, ValueDoubleCtor | ValueSetDouble) {
        let vectors = vector_args.ok_or(KernelStartupError::UnavailableVectorArguments)?;
        real_payload.copy_from_slice(&vectors[0][..8]);
    }
    let object = args[0];
    if query {
        let kind = read_type(memory, object)?;
        return match function {
            ValueGetType => Ok(u64::from(kind)),
            ValueCount if matches!(kind, 6 | 7) => {
                let mut pointer = [0; 8];
                memory
                    .read(at(object, UNION)?, &mut pointer)
                    .map_err(|error| guest(error, false))?;
                if u64::from_le_bytes(pointer) == 0 {
                    Ok(0)
                } else {
                    Err(KernelStartupError::UnavailableJsonValueStorage)
                }
            }
            ValueCount => Ok(0),
            ValueGetBoolean if object != 0 && kind == 1 => at(object, UNION),
            ValueGetInteger if object != 0 && matches!(kind, 2 | 3) => at(object, UNION),
            ValueGetReal if object != 0 && kind == 4 => at(object, UNION),
            ValueGetBoolean => Ok(defaults.boolean),
            ValueGetInteger => Ok(defaults.integer),
            ValueGetReal => Ok(defaults.real),
            _ => unreachable!("checked JSON query"),
        }
        .map(ValueAction::Return);
    }
    let returns_self = matches!(
        function,
        ValueBoolCtor
            | ValueIntCtor
            | ValueDoubleCtor
            | ValueTypeCtor
            | ValueCopyCtor
            | ValueAssign
    );
    let result = if returns_self { object } else { 0 };
    if object == 0 {
        return Ok(ValueAction::Return(result));
    }
    if function == ValueAssign && (args[1] == 0 || args[1] == object) {
        return Ok(ValueAction::Return(result));
    }
    if function == ValueTypeCtor && matches!(args[1] as u32, 6 | 7) {
        return Err(KernelStartupError::UnavailableJsonValueStorage);
    }
    let mut staged = StagedValue::new(object)?;
    match function {
        ValueBoolCtor | ValueIntCtor | ValueDoubleCtor | ValueTypeCtor | ValueCopyCtor => {
            staged.init()
        }
        ValueClearMethod => {
            let links = staged.read::<16>(memory, object)?;
            staged.clear(memory, storage, work, maximum)?;
            staged.write(0, &links);
        }
        ValueAssign => {} // Copy performs its one source-ordered Clear below.
        _ => staged.clear(memory, storage, work, maximum)?,
    }
    match function {
        ValueBoolCtor | ValueSetBool => {
            staged.set_type(1);
            staged.write(UNION, &[args[1] as u8]);
        }
        ValueIntCtor | ValueSetInt => {
            staged.set_type(2);
            staged.write(UNION, &args[1].to_le_bytes());
        }
        ValueSetUInt => {
            staged.set_type(3);
            staged.write(UNION, &args[1].to_le_bytes());
        }
        ValueDoubleCtor | ValueSetDouble => {
            staged.set_type(4);
            staged.write(UNION, &real_payload);
        }
        ValueTypeCtor | ValueSetType => {
            if matches!(args[1] as u32, 6 | 7) {
                return Err(KernelStartupError::UnavailableJsonValueStorage);
            }
            staged.set_type(args[1] as u32);
            staged.write(UNION, &[0; 8]);
            if args[1] as u32 == 5 {
                storage.empty_string(&mut staged)?;
            }
        }
        ValueCopyCtor | ValueAssign => staged.copy(memory, args[1], storage, work, maximum)?,
        ValueDtor | ValueClearMethod => {}
        _ => unreachable!("checked JSON mutation"),
    }
    Ok(ValueAction::Write {
        value: staged,
        result,
    })
}

#[cfg(test)]
mod tests;
