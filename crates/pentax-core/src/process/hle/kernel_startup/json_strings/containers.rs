// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned, staged JSON trees and guest iterator views.
// KytyPS5 f186b357 libJson2.cpp, blob d6e968f019fa61e0adc5224cd20be1ecd3261487.
// See NOTICE and docs/upstream-provenance.md.

//! Private container words identify owned guest storage, never host containers.
//! Nodes, keys, pointer cells and strings reside in checked guest mappings.
//! Every mutation stages ownership and bytes before one atomic publication.
//! Retired storage is quarantined for the process lifetime, under the same
//! finite guest-byte budget as existing JSON Strings; addresses are not reused.

use super::*;
use pentax_kernel::{Failure, RuntimeDataOutputs};
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativeRange,
    NativeTransactionLimits,
};
use std::mem::size_of;

pub(super) mod lifecycle;
mod parser;
mod storage;

const MAX_NODES: usize = 4096;
const MAX_DEPTH: usize = 64;
const MAX_TEXT: usize = 256 * 1024;
const MAX_HELD: usize = 64 * 1024 * 1024;
const MAX_CONTAINER_METADATA: usize = 1024 * 1024;
const RELATIVE: u64 = 1 << 63;
const INVALID_ARGUMENT: u64 = 0x8084_8120;
const PARSE_ERROR: u64 = 0x8084_8101;

#[derive(Clone, Copy)]
struct Container {
    token: u64,
    wrapper: u64,
    owner_value: u64,
    kind: u32,
    entries: u64,
    length: usize,
    keys_begin: u64,
    keys_end: u64,
    live: bool,
}
#[derive(Clone, Copy)]
struct OwnedNode {
    address: u64,
    container: u64,
    live: bool,
}
#[derive(Clone, Copy)]
struct Defaults {
    null: u64,
    array: u64,
    object: u64,
}
pub(super) struct Containers {
    records: Vec<Container>,
    nodes: Vec<OwnedNode>,
    defaults: Option<Defaults>,
    ready: bool,
}
impl Containers {
    pub(super) const fn new() -> Self {
        Self {
            records: Vec::new(),
            nodes: Vec::new(),
            defaults: None,
            ready: false,
        }
    }
    #[cfg(test)]
    pub(super) fn readiness_for_test(&self) -> (bool, Option<[u64; 3]>, usize) {
        (
            self.ready,
            self.defaults.map(|d| [d.null, d.array, d.object]),
            self.records.capacity(),
        )
    }
}

enum Node {
    Scalar(u32, u64),
    String(Vec<u8>),
    Array(Vec<Node>),
    Object(Vec<(Vec<u8>, Node)>),
}
impl Node {
    fn kind(&self) -> u32 {
        match self {
            Self::Scalar(kind, _) => *kind,
            Self::String(_) => 5,
            Self::Array(_) => 6,
            Self::Object(_) => 7,
        }
    }
}

struct Admission {
    held: usize,
    work: u64,
    maximum: u64,
    nodes: usize,
}
impl Admission {
    fn spend(&mut self, amount: usize) -> Result<(), KernelStartupError> {
        charge(&mut self.work, amount as u64, self.maximum)
    }
    fn hold(&mut self, amount: usize) -> Result<(), KernelStartupError> {
        self.held = self
            .held
            .checked_add(amount)
            .filter(|n| *n <= MAX_HELD)
            .ok_or(KernelStartupError::Budget("JSON tree staging"))?;
        Ok(())
    }
    fn reserve<T>(
        &mut self,
        vector: &mut Vec<T>,
        additional: usize,
    ) -> Result<(), KernelStartupError> {
        let length = vector
            .len()
            .checked_add(additional)
            .ok_or(KernelStartupError::Budget("JSON tree capacity"))?;
        if length <= vector.capacity() {
            return Ok(());
        }
        let target = length.max(vector.capacity().saturating_mul(2)).max(4);
        let requested = target
            .checked_mul(size_of::<T>())
            .ok_or(KernelStartupError::Budget("JSON tree capacity"))?;
        // The old allocation remains included while its replacement is reserved.
        self.held
            .checked_add(requested)
            .filter(|n| *n <= MAX_HELD)
            .ok_or(KernelStartupError::Budget("JSON tree staging"))?;
        self.spend(requested)?;
        vector
            .try_reserve_exact(target - vector.len())
            .map_err(|_| KernelStartupError::Allocation)?;
        self.hold(
            vector
                .capacity()
                .checked_mul(size_of::<T>())
                .ok_or(KernelStartupError::Budget("JSON tree capacity"))?,
        )
    }
    fn bytes(&mut self, source: &[u8]) -> Result<Vec<u8>, KernelStartupError> {
        if source.len() > MAX_TEXT {
            return Err(KernelStartupError::Budget("JSON text"));
        }
        let mut result = Vec::new();
        self.reserve(&mut result, source.len())?;
        result.extend_from_slice(source);
        Ok(result)
    }
    fn node(&mut self, depth: usize) -> Result<(), KernelStartupError> {
        self.nodes += 1;
        if depth > MAX_DEPTH || self.nodes > MAX_NODES {
            return Err(KernelStartupError::Budget("JSON tree depth/nodes"));
        }
        self.spend(64)
    }
}

#[derive(Clone, Copy)]
struct Output {
    address: u64,
    length: usize,
    bytes: [u8; 128],
    pointer: bool,
}
struct Transaction {
    admission: Admission,
    containers: Containers,
    strings: Vec<StringRecord>,
    wrappers: Vec<WrapperRecord>,
    blob: Vec<u8>,
    fixups: Vec<usize>,
    outputs: [Output; 3],
    output_count: usize,
    root: Option<StagedValue>,
    root_pointer: Option<u64>,
    retire_depth: usize,
    invalidated: Vec<NativeRange>,
}

fn failure() -> KernelStartupError {
    KernelStartupError::UnavailableJsonValueStorage
}
fn external(address: u64) -> Result<(), KernelStartupError> {
    if address >= 1 << 47 {
        Err(guest(NativeMemoryError::InvalidRange, false))
    } else {
        Ok(())
    }
}
fn kind(memory: &PausedNativeMemory<'_>, object: u64) -> Result<u32, KernelStartupError> {
    if object == 0 {
        return Ok(0);
    }
    let mut bytes = [0; 4];
    memory
        .read(at(object, 28)?, &mut bytes)
        .map_err(|e| guest(e, false))?;
    Ok(u32::from_le_bytes(bytes))
}

pub(super) fn handles(
    memory: &PausedNativeMemory<'_>,
    call: &JsonCall,
    work: &mut u64,
    maximum: u64,
) -> Result<bool, KernelStartupError> {
    use JsonFunction::*;
    if call.function == ValueSetDouble && call.vector_args.is_none() {
        return Err(KernelStartupError::UnavailableVectorArguments);
    }
    if matches!(
        call.function,
        ObjectCtor
            | ObjectCopyCtor
            | ObjectDtor
            | ObjectIndex
            | ArrayCtor
            | ArrayDtor
            | ArrayPushBack
            | ValueObjectCtor
            | ValueArrayCtor
            | ValueSetObject
            | ValueGetObject
            | ValueGetArray
            | ValueIndexString
            | ValueIndexUInt
            | ValueReferValueKey
            | ArrayBegin
            | ArrayEnd
            | ArrayIteratorNotEqual
            | ArrayIteratorDereference
            | ArrayIteratorIncrement
            | ArrayIteratorDtor
            | ParserParse
    ) {
        return Ok(true);
    }
    if matches!(call.function, ValueCopyCtor | ValueAssign) {
        let [object, source, ..] = call.args;
        if object == 0 || source == 0 || source == object {
            return Ok(false);
        }
        charge(work, 512, maximum)?;
        if call.function == ValueAssign && matches!(kind(memory, object)?, 6 | 7) {
            return Ok(true);
        }
        let mut overlay = StagedValue::new(object)?;
        if call.function == ValueCopyCtor {
            overlay.init();
        } else {
            overlay.write(0, &[0; 24]);
            overlay.set_type(0);
        }
        let source_kind = u32::from_le_bytes(overlay.read::<4>(memory, at(source, 28)?)?);
        return Ok(matches!(source_kind, 6 | 7));
    }
    if matches!(call.function, ValueTypeCtor | ValueSetType) && matches!(call.args[1] as u32, 6 | 7)
    {
        return Ok(true);
    }
    if matches!(
        call.function,
        ValueDtor
            | ValueClearMethod
            | ValueSetBool
            | ValueSetInt
            | ValueSetUInt
            | ValueSetDouble
            | ValueSetString
            | ValueSetType
            | ValueToString
            | ValueSerialize
            | ValueCount
    ) {
        // Source Clear/query also reads this field before a union or pointee.
        // Constructors that overwrite their object never take this path.
        charge(work, 64, maximum)?;
        let value_kind = kind(memory, call.args[0])?;
        return Ok(matches!(value_kind, 6 | 7)
            || value_kind == 4 && matches!(call.function, ValueToString | ValueSerialize));
    }
    Ok(false)
}

pub(super) fn service(
    state: &mut JsonStrings,
    memory: &mut PausedNativeMemory<'_>,
    kernel: &mut pentax_kernel::Kernel,
    call: JsonCall,
    work: &mut u64,
    maximum: u64,
) -> Result<u64, KernelStartupError> {
    // The fixed charge covers primitive staging's independent fixed counter;
    // all variable traversals/reservations charge the remaining local counter.
    charge(work, 1024, maximum)?;
    let mut transaction = Transaction::new(state, maximum - *work)?;
    let result = transaction
        .stage(state)
        .and_then(|()| transaction.invoke(memory, &call))
        .and_then(|result| transaction.publish(state, memory, kernel, result));
    *work += transaction.admission.work;
    if result.is_ok() {
        state.owner = Some(memory.owner_id());
    }
    result
}

impl Transaction {
    fn invoke(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        call: &JsonCall,
    ) -> Result<u64, KernelStartupError> {
        self.invoke_inner(memory, call)
    }

    fn invoke_inner(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        call: &JsonCall,
    ) -> Result<u64, KernelStartupError> {
        use JsonFunction::*;
        let [object, source, argument, ..] = call.args;
        if call.function == ArrayIteratorDtor {
            return Ok(0);
        }
        if call.function == ParserParse && (object == 0 || source == 0) {
            return Ok(INVALID_ARGUMENT);
        }
        if call.function == ValueAssign && (object == 0 || source == 0 || object == source) {
            return Ok(object);
        }
        if call.function != ParserParse {
            external(object)?;
        }
        // A second GP is a pointer only in these exact roles; scalar indices,
        // payloads and type words must retain all of their original bits.
        if matches!(
            call.function,
            ObjectCopyCtor
                | ValueObjectCtor
                | ValueArrayCtor
                | ValueSetObject
                | ArrayPushBack
                | ValueCopyCtor
                | ValueAssign
        ) && object != 0
            || matches!(
                call.function,
                ObjectIndex
                    | ArrayBegin
                    | ArrayEnd
                    | ArrayIteratorNotEqual
                    | ValueToString
                    | ValueSerialize
            )
        {
            external(source)?;
        }
        match call.function {
            ObjectCtor | ArrayCtor => {
                if object != 0 {
                    let node = if call.function == ArrayCtor {
                        Node::Array(Vec::new())
                    } else {
                        Node::Object(Vec::new())
                    };
                    self.construct(object, &node)?;
                }
                Ok(object)
            }
            ObjectCopyCtor => {
                if object != 0 {
                    let node = self.snapshot_container(memory, None, source, 7, 0)?;
                    self.construct(object, &node)?;
                }
                Ok(object)
            }
            ObjectDtor | ArrayDtor => {
                if object != 0 {
                    let kind = if call.function == ArrayDtor { 6 } else { 7 };
                    self.retire_container(memory, None, object, kind, None, 0)?;
                    self.output(object, &[0; 8], false)?;
                }
                Ok(0)
            }
            ValueObjectCtor | ValueArrayCtor | ValueSetObject => {
                if object == 0 {
                    return Ok(0);
                }
                let mut value = StagedValue::new(object)?;
                if call.function != ValueSetObject {
                    value.init();
                }
                let kind = if call.function == ValueArrayCtor {
                    6
                } else {
                    7
                };
                // SetObject clones BEFORE Clear, preserving self/descendant sources.
                // Constructors Init first, so overlapping source reads see zeroes.
                let tree = self.snapshot_container(memory, Some(&value), source, kind, 0)?;
                if call.function == ValueSetObject {
                    value.clear(memory, self, &mut 0, u64::MAX)?;
                }
                value.set_type(kind);
                self.payload(&mut value, &tree)?;
                self.root = Some(value);
                Ok(if call.function == ValueSetObject {
                    0
                } else {
                    object
                })
            }
            ArrayPushBack => {
                let Some(index) = self.container(memory, None, object, 6, true)? else {
                    return Ok(object);
                };
                let tree = self.snapshot(memory, None, source, 0)?;
                let token = self.containers.records[index].token;
                let child = self.encode(&tree, 0, token)?;
                let mut entries = self.entries(memory, None, index)?;
                self.admission.reserve(&mut entries, 1)?;
                entries.push((Vec::new(), child));
                self.replace_entries(index, &entries)?;
                Ok(object)
            }
            ObjectIndex => {
                let key = self.string(memory, None, source, false)?;
                let Some(index) = self.container(memory, None, object, 7, true)? else {
                    return Ok(self.defaults()?.null);
                };
                let mut entries = self.entries(memory, None, index)?;
                self.admission.spend(entries.len() + key.len())?;
                match entries.binary_search_by(|entry| entry.0.cmp(&key)) {
                    Ok(index) => Ok(entries[index].1),
                    Err(position) => {
                        let child = self.encode(
                            &Node::Scalar(0, 0),
                            0,
                            self.containers.records[index].token,
                        )?;
                        self.admission.reserve(&mut entries, 1)?;
                        entries.insert(position, (key, child));
                        self.replace_entries(index, &entries)?;
                        Ok(child)
                    }
                }
            }
            ValueGetArray | ValueGetObject => {
                let expected = if call.function == ValueGetArray { 6 } else { 7 };
                if object != 0 && self.value_kind(memory, None, object)? == expected {
                    self.word(memory, None, at(object, 16)?)
                } else {
                    let defaults = self.defaults()?;
                    Ok(if expected == 6 {
                        defaults.array
                    } else {
                        defaults.object
                    })
                }
            }
            ValueIndexString | ValueIndexUInt | ValueReferValueKey => {
                let expected = if call.function == ValueIndexUInt {
                    6
                } else {
                    7
                };
                if object == 0 || self.value_kind(memory, None, object)? != expected {
                    return Ok(if call.function == ValueReferValueKey {
                        0
                    } else {
                        self.defaults()?.null
                    });
                }
                let wrapper = self.word(memory, None, at(object, 16)?)?;
                // String index constructs its key before mutable helper lookup;
                // ReferValueKey materializes the object before reading its key.
                let early_key = if call.function == ValueIndexString {
                    Some(self.cstring(memory, source)?)
                } else {
                    None
                };
                let index = self.container(memory, None, wrapper, expected, expected == 7)?;
                let entries = match index {
                    Some(index) => self.entries(memory, None, index)?,
                    None => Vec::new(),
                };
                let found = if expected == 6 {
                    usize::try_from(source)
                        .ok()
                        .and_then(|index| entries.get(index))
                        .map(|entry| entry.1)
                } else {
                    let key = match early_key {
                        Some(key) => key,
                        None => self.string(memory, None, source, false)?,
                    };
                    self.admission.spend(entries.len() + key.len())?;
                    entries
                        .binary_search_by(|entry| entry.0.cmp(&key))
                        .ok()
                        .map(|index| entries[index].1)
                };
                match found {
                    Some(address) => Ok(address),
                    None if call.function == ValueReferValueKey => Ok(0),
                    None => Ok(self.defaults()?.null),
                }
            }
            ArrayBegin | ArrayEnd => {
                let index = self.container(memory, None, source, 6, false)?;
                let position = if let Some(index) = index {
                    let record = self.containers.records[index];
                    // Validate the complete real pointer-cell view, not only an
                    // endpoint selected by an index or an empty shortcut.
                    self.entries(memory, None, index)?;
                    if call.function == ArrayEnd {
                        at(record.entries, record.length * 8)?
                    } else {
                        record.entries
                    }
                } else {
                    0
                };
                self.output(object, &position.to_le_bytes(), true)?;
                Ok(object)
            }
            ArrayIteratorNotEqual => {
                let left = self.word(memory, None, object)?;
                let right = self.word(memory, None, source)?;
                Ok(u64::from(left != right))
            }
            ArrayIteratorDereference | ArrayIteratorIncrement => {
                let position = self.word(memory, None, object)?;
                self.admission.spend(self.containers.records.len())?;
                let index = self
                    .containers
                    .records
                    .iter()
                    .position(|record| {
                        record.live
                            && record.kind == 6
                            && position >= record.entries
                            && (position - record.entries).is_multiple_of(8)
                            && (position - record.entries) / 8 < record.length as u64
                    })
                    .ok_or_else(failure)?;
                let record = self.containers.records[index];
                let entries = self.entries(memory, None, index)?;
                if call.function == ArrayIteratorDereference {
                    Ok(entries[((position - record.entries) / 8) as usize].1)
                } else {
                    self.output(object, &at(position, 8)?.to_le_bytes(), true)?;
                    Ok(object)
                }
            }
            ArrayIteratorDtor => Ok(0), // Exact empty public body, no pointer read.
            ParserParse => {
                if object == 0 || source == 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                let length = usize::try_from(argument)
                    .ok()
                    .filter(|n| *n <= MAX_TEXT)
                    .ok_or(KernelStartupError::Budget("JSON parser input"))?;
                let mut input = Vec::new();
                self.admission.reserve(&mut input, length)?;
                input.resize(length, 0);
                if length != 0 {
                    external(source)?;
                    self.read(memory, None, source, &mut input)?;
                }
                let Some(tree) = parser::parse(&input, &mut self.admission)? else {
                    return Ok(PARSE_ERROR);
                };
                // Parsing completes before Clear, including overlapping input.
                external(object)?;
                let mut value = StagedValue::new(object)?;
                value.clear(memory, self, &mut 0, u64::MAX)?;
                value.set_type(tree.kind());
                self.payload(&mut value, &tree)?;
                self.root = Some(value);
                Ok(0)
            }
            ValueSetString => {
                if object == 0 {
                    return Ok(0);
                }
                let mut value = StagedValue::new(object)?;
                value.clear(memory, self, &mut 0, u64::MAX)?;
                let bytes = self.string(memory, Some(&value), source, false)?;
                value.set_type(5);
                self.payload(&mut value, &Node::String(bytes))?;
                self.root = Some(value);
                Ok(0)
            }
            ValueToString | ValueSerialize => {
                if source == 0 {
                    return Ok(0);
                }
                self.string(memory, None, source, true)?; // Actual mutable target admission first.
                self.replace_string(memory, source, b"")?;
                let node = self.snapshot(memory, None, object, 0)?;
                let mut bytes = Vec::new();
                self.serialize(&node, &mut bytes)?;
                self.replace_string(memory, source, &bytes)?;
                Ok(0)
            }
            _ => {
                let action = json_values::prepare(
                    memory,
                    call.function,
                    call.args,
                    call.vector_args.as_ref(),
                    call.defaults,
                    &mut 0,
                    1024,
                    self,
                )?;
                match action {
                    ValueAction::Return(result) => Ok(result),
                    ValueAction::Write { value, result } => {
                        self.root = Some(value);
                        Ok(result)
                    }
                }
            }
        }
    }

    fn append(&mut self, output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), KernelStartupError> {
        if output
            .len()
            .checked_add(bytes.len())
            .is_none_or(|n| n > MAX_TEXT)
        {
            return Err(KernelStartupError::Budget("JSON serialized text"));
        }
        self.admission.spend(bytes.len())?;
        self.admission.reserve(output, bytes.len())?;
        output.extend_from_slice(bytes);
        Ok(())
    }
    fn serialize(&mut self, node: &Node, output: &mut Vec<u8>) -> Result<(), KernelStartupError> {
        use std::fmt::Write;
        match node {
            Node::Scalar(1, value) => {
                self.append(output, if *value == 0 { b"false" } else { b"true" })
            }
            Node::Scalar(kind @ (2 | 3), value) => {
                let mut text = NumberText::new();
                if *kind == 2 {
                    write!(&mut text, "{}", *value as i64)
                } else {
                    write!(&mut text, "{value}")
                }
                .map_err(|_| failure())?;
                self.append(output, &text.bytes[..text.length])
            }
            Node::Scalar(4, value) => {
                let mut text = NumberText::new();
                let real = f64::from_bits(*value);
                if real.is_nan() {
                    text.write_str(if value >> 63 == 0 { "nan" } else { "-nan" })
                } else if real.is_infinite() {
                    text.write_str(if real.is_sign_negative() {
                        "-inf"
                    } else {
                        "inf"
                    })
                } else {
                    write!(&mut text, "{real:.6}")
                }
                .map_err(|_| failure())?;
                self.append(output, &text.bytes[..text.length])
            }
            Node::Scalar(_, _) => self.append(output, b"null"),
            Node::String(bytes) => {
                self.append(output, b"\"")?;
                self.append(output, bytes)?;
                self.append(output, b"\"")
            }
            Node::Array(children) => {
                self.append(output, b"[")?;
                for (index, child) in children.iter().enumerate() {
                    if index != 0 {
                        self.append(output, b",")?;
                    }
                    self.serialize(child, output)?;
                }
                self.append(output, b"]")
            }
            Node::Object(children) => {
                self.append(output, b"{")?;
                for (index, (key, child)) in children.iter().enumerate() {
                    if index != 0 {
                        self.append(output, b",")?;
                    }
                    self.append(output, b"\"")?;
                    self.append(output, key)?;
                    self.append(output, b"\":")?;
                    self.serialize(child, output)?;
                }
                self.append(output, b"}")
            }
        }
    }
}

struct NumberText {
    bytes: [u8; 512],
    length: usize,
}
impl NumberText {
    fn new() -> Self {
        Self {
            bytes: [0; 512],
            length: 0,
        }
    }
}
impl std::fmt::Write for NumberText {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let end = self
            .length
            .checked_add(value.len())
            .filter(|n| *n <= self.bytes.len())
            .ok_or(std::fmt::Error)?;
        self.bytes[self.length..end].copy_from_slice(value.as_bytes());
        self.length = end;
        Ok(())
    }
}

impl ValueStorage for Transaction {
    fn clear_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &StagedValue,
        wrapper: u64,
        _: &mut u64,
        _: u64,
    ) -> Result<(), KernelStartupError> {
        self.retire_string(memory, value, wrapper)
    }
    fn copy_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &mut StagedValue,
        wrapper: u64,
        _: &mut u64,
        _: u64,
    ) -> Result<(), KernelStartupError> {
        let text = self.string(memory, Some(value), wrapper, true)?;
        self.payload(value, &Node::String(text))
    }
    fn empty_string(&mut self, value: &mut StagedValue) -> Result<(), KernelStartupError> {
        self.payload(value, &Node::String(Vec::new()))
    }
    fn clear_container(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &StagedValue,
        kind: u32,
        wrapper: u64,
    ) -> Result<(), KernelStartupError> {
        self.retire_depth += 1;
        if self.retire_depth > MAX_DEPTH {
            return Err(KernelStartupError::Budget("JSON destruction depth"));
        }
        let result = self.retire_container(
            memory,
            Some(value),
            wrapper,
            kind,
            Some(value.address),
            self.retire_depth,
        );
        self.retire_depth -= 1;
        result
    }
    fn copy_container(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        value: &mut StagedValue,
        kind: u32,
        wrapper: u64,
    ) -> Result<(), KernelStartupError> {
        let tree = self.snapshot_container(memory, Some(value), wrapper, kind, 0)?;
        self.payload(value, &tree)
    }
    fn empty_container(
        &mut self,
        value: &mut StagedValue,
        kind: u32,
    ) -> Result<(), KernelStartupError> {
        self.payload(
            value,
            &if kind == 6 {
                Node::Array(Vec::new())
            } else {
                Node::Object(Vec::new())
            },
        )
    }
    fn count_container(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        kind: u32,
        wrapper: u64,
    ) -> Result<u64, KernelStartupError> {
        match self.container(memory, None, wrapper, kind, false)? {
            Some(index) => {
                self.entries(memory, None, index)?;
                Ok(self.containers.records[index].length as u64)
            }
            None => Ok(0),
        }
    }
}
