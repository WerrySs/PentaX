// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded guest-backed String ownership and atomic
// publication, adapted from KytyPS5 f186b35706ddbc3faf5513f326e2335d224802f7
// libJson2.cpp (blob d6e968f019fa61e0adc5224cd20be1ecd3261487). See NOTICE.

//! The pinned eight-byte String wrapper and 32-byte Value profile. Impl words
//! are opaque, owned guest addresses, never Rust/C++ container pointers. String
//! bytes are read from current guest backing. Retired storage is quarantined
//! until session destruction and remains charged by the kernel mapping owner.
//! Partial overlaps requiring a not-yet-selected allocation address refuse;
//! exact String aliases and the existing Value overlay remain supported.
//! Raw aliases into a retired or logically replaced String buffer also refuse
//! instead of reading a detached pre-mutation snapshot. Real/container output
//! formatting is explicitly unavailable in this slice.

use super::json_values::{self, ScalarDefaults, StagedValue, ValueAction, ValueStorage};
use super::{KernelStartupError, charge};
use crate::process::hle::json_provider::JsonFunction;
use pentax_platform::native_memory::{NativeMemoryError, NativeOwnerId, PausedNativeMemory};

mod containers;
mod plan;
use plan::{Plan, StringRef};

const MAX_STRING_BYTES: usize = 256 * 1024;
const MAX_RECORDS: usize = 4096;
const MAX_METADATA_BYTES: usize = 1024 * 1024;
const MAX_SCRATCH_BYTES: usize = 1024 * 1024;
const MAX_GUEST_BYTES: u64 = 64 * 1024 * 1024;
const CALL_WORK: u64 = 512;

#[derive(Clone, Copy)]
struct StringRecord {
    token: u64,
    data: u64,
    length: usize,
    live: bool,
}

#[derive(Clone, Copy)]
struct WrapperRecord {
    address: u64,
    value: u64,
    live: bool,
}

pub(super) struct JsonStrings {
    owner: Option<NativeOwnerId>,
    records: Vec<StringRecord>,
    wrappers: Vec<WrapperRecord>,
    guest_bytes: u64,
    containers: containers::Containers,
}

pub(super) struct JsonCall {
    pub(super) function: JsonFunction,
    pub(super) args: [u64; 6],
    pub(super) vector_args: Option<[[u8; 16]; 8]>,
    pub(super) defaults: ScalarDefaults,
    pub(super) empty_string: u64,
    pub(super) empty_cstr: u64,
}

fn guest(error: NativeMemoryError, write: bool) -> KernelStartupError {
    KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
        super::super::kernel_adapter::error(error, write),
    ))
}

fn at(address: u64, offset: usize) -> Result<u64, KernelStartupError> {
    address
        .checked_add(offset as u64)
        .ok_or_else(|| guest(NativeMemoryError::InvalidRange, false))
}

impl JsonStrings {
    pub(super) const fn new() -> Self {
        Self {
            owner: None,
            records: Vec::new(),
            wrappers: Vec::new(),
            guest_bytes: 0,
            containers: containers::Containers::new(),
        }
    }

    pub(super) fn service(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        call: JsonCall,
        work: &mut u64,
        maximum: u64,
    ) -> Result<u64, KernelStartupError> {
        if self.owner.is_some_and(|owner| owner != memory.owner_id()) {
            return Err(KernelStartupError::NativeOwnerMismatch);
        }
        if containers::handles(memory, &call, work, maximum)? {
            return containers::service(self, memory, kernel, call, work, maximum);
        }
        self.owner = Some(memory.owner_id());
        charge(work, CALL_WORK, maximum)?;
        let mut plan = Plan::new();
        let result = self.prepare(memory, &call, &mut plan, work, maximum)?;
        plan.publish(self, memory, kernel, work, maximum)?;
        Ok(result)
    }

    fn prepare(
        &self,
        memory: &PausedNativeMemory<'_>,
        call: &JsonCall,
        plan: &mut Plan,
        work: &mut u64,
        maximum: u64,
    ) -> Result<u64, KernelStartupError> {
        use JsonFunction::*;
        let [object, source, ..] = call.args;
        match call.function {
            StringCtor | StringCStringCtor => {
                if object != 0 {
                    let bytes = if call.function == StringCStringCtor {
                        plan.cstring(memory, None, source, work, maximum)?
                    } else {
                        Vec::new()
                    };
                    plan.construct_string(object, bytes)?;
                }
                Ok(object)
            }
            StringAssign => {
                if object != 0 {
                    // Source: mutable destination helper precedes const source.
                    let target = plan.mutable(self, memory, None, object, work, maximum)?;
                    let bytes = plan.constant(self, memory, None, source, work, maximum)?;
                    plan.replace(self, target, bytes)?;
                }
                Ok(object)
            }
            StringDtor => {
                if object != 0 {
                    plan.destroy_string(self, memory, None, object, work, maximum)?;
                }
                Ok(0)
            }
            StringCStr | StringLength => {
                if object == 0 {
                    return Ok(if call.function == StringCStr {
                        call.empty_cstr
                    } else {
                        0
                    });
                }
                let reference = plan.lookup(self, memory, None, object, work, maximum)?;
                match reference {
                    None => Ok(if call.function == StringCStr {
                        call.empty_cstr
                    } else {
                        0
                    }),
                    Some(StringRef::Existing(index)) => {
                        let record = self.records[index];
                        Ok(if call.function == StringCStr {
                            record.data
                        } else {
                            record.length as u64
                        })
                    }
                    Some(StringRef::Pending(_)) => unreachable!("query has no pending changes"),
                }
            }
            ValueGetString => {
                let kind = plan.kind(memory, None, object, work, maximum)?;
                if object != 0 && kind == 5 {
                    plan.word(memory, None, at(object, 16)?, work, maximum)
                } else {
                    Ok(call.empty_string)
                }
            }
            ValueCStringCtor | ValueStringCtor | ValueSetString => {
                if object == 0 {
                    return Ok(if call.function == ValueSetString {
                        0
                    } else {
                        object
                    });
                }
                let mut value = StagedValue::new(object)?;
                if call.function == ValueSetString {
                    value.clear(
                        memory,
                        &mut StringValues { state: self, plan },
                        work,
                        maximum,
                    )?;
                } else {
                    value.init();
                }
                value.set_type(5);
                let bytes = if call.function == ValueCStringCtor {
                    plan.cstring(memory, Some(&value), source, work, maximum)?
                } else {
                    plan.constant(self, memory, Some(&value), source, work, maximum)?
                };
                plan.value_string(object, bytes)?;
                plan.value = Some(value);
                Ok(if call.function == ValueSetString {
                    0
                } else {
                    object
                })
            }
            ValueToString | ValueSerialize => {
                if source == 0 {
                    return Ok(0);
                }
                let target = plan.mutable(self, memory, None, source, work, maximum)?;
                let kind = if call.function == ValueToString {
                    plan.kind(memory, None, object, work, maximum)?
                } else {
                    0
                };
                if call.function == ValueToString && object != 0 && kind == 5 {
                    let wrapper = plan.word(memory, None, at(object, 16)?, work, maximum)?;
                    let from = plan.mutable(self, memory, None, wrapper, work, maximum)?;
                    let bytes = plan.snapshot(self, memory, None, from, work, maximum)?;
                    plan.replace(self, target, bytes)?;
                } else {
                    // Serialize clears first, including when source String aliases dst.
                    plan.replace(self, target, Vec::new())?;
                    self.serialize(memory, object, target, plan, work, maximum)?;
                }
                Ok(0)
            }
            _ => {
                let action = json_values::prepare(
                    memory,
                    call.function,
                    call.args,
                    call.vector_args.as_ref(),
                    call.defaults,
                    work,
                    maximum,
                    &mut StringValues { state: self, plan },
                )?;
                match action {
                    ValueAction::Return(result) => Ok(result),
                    ValueAction::Write { value, result } => {
                        plan.value = Some(value);
                        Ok(result)
                    }
                }
            }
        }
    }

    fn serialize(
        &self,
        memory: &PausedNativeMemory<'_>,
        object: u64,
        target: StringRef,
        plan: &mut Plan,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        let kind = plan.kind(memory, None, object, work, maximum)?;
        match kind {
            1 => {
                let byte = plan.byte(memory, None, at(object, 16)?, work, maximum)?;
                plan.append(
                    target,
                    if byte == 0 { b"false" } else { b"true" },
                    work,
                    maximum,
                )
            }
            2 | 3 => {
                let raw = plan.word(memory, None, at(object, 16)?, work, maximum)?;
                let negative = kind == 2 && (raw as i64) < 0;
                let mut value = if negative {
                    (raw as i64).unsigned_abs()
                } else {
                    raw
                };
                let mut digits = [0; 21];
                let mut start = digits.len();
                loop {
                    start -= 1;
                    digits[start] = b'0' + (value % 10) as u8;
                    value /= 10;
                    if value == 0 {
                        break;
                    }
                }
                if negative {
                    start -= 1;
                    digits[start] = b'-';
                }
                plan.append(target, &digits[start..], work, maximum)
            }
            4 | 6 | 7 => Err(KernelStartupError::UnavailableJsonFormat),
            5 => {
                plan.append(target, b"\"", work, maximum)?;
                let wrapper = plan.word(memory, None, at(object, 16)?, work, maximum)?;
                let from = plan.mutable(self, memory, None, wrapper, work, maximum)?;
                let bytes = plan.snapshot(self, memory, None, from, work, maximum)?;
                // Pinned source deliberately does not escape String contents.
                plan.append_held(target, &bytes, bytes.capacity(), work, maximum)?;
                plan.append(target, b"\"", work, maximum)
            }
            _ => plan.append(target, b"null", work, maximum),
        }
    }
}

struct StringValues<'a> {
    state: &'a JsonStrings,
    plan: &'a mut Plan,
}
impl ValueStorage for StringValues<'_> {
    fn clear_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &StagedValue,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        self.plan
            .clear_value(self.state, memory, value, wrapper, work, maximum)
    }
    fn copy_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &mut StagedValue,
        wrapper: u64,
        work: &mut u64,
        maximum: u64,
    ) -> Result<(), KernelStartupError> {
        let from = self
            .plan
            .mutable(self.state, memory, Some(value), wrapper, work, maximum)?;
        let bytes = self
            .plan
            .snapshot(self.state, memory, Some(value), from, work, maximum)?;
        self.plan.value_string(value.address, bytes)
    }
    fn empty_string(&mut self, value: &mut StagedValue) -> Result<(), KernelStartupError> {
        self.plan.value_string(value.address, Vec::new())
    }
}

#[cfg(test)]
mod tests;
