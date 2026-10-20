// SPDX-License-Identifier: GPL-2.0-only
//! Transactional storage for the pinned JSON wrapper/Value representation.
use super::*;

fn clone_records<T: Copy>(
    old: &[T],
    admission: &mut Admission,
) -> Result<Vec<T>, KernelStartupError> {
    let mut next = Vec::new();
    admission.spend(old.len())?;
    admission.reserve(&mut next, old.len())?;
    next.extend_from_slice(old);
    Ok(next)
}
fn metadata(state: &JsonStrings) -> Result<usize, KernelStartupError> {
    [
        state
            .records
            .capacity()
            .checked_mul(size_of::<StringRecord>()),
        state
            .wrappers
            .capacity()
            .checked_mul(size_of::<WrapperRecord>()),
        state
            .containers
            .records
            .capacity()
            .checked_mul(size_of::<Container>()),
        state
            .containers
            .nodes
            .capacity()
            .checked_mul(size_of::<OwnedNode>()),
    ]
    .into_iter()
    .try_fold(0usize, |sum, value| {
        sum.checked_add(value.ok_or(KernelStartupError::Budget("JSON metadata"))?)
            .ok_or(KernelStartupError::Budget("JSON metadata"))
    })
}
fn resolved(address: u64, base: u64) -> Result<u64, Failure> {
    if address & RELATIVE == 0 {
        Ok(address)
    } else {
        base.checked_add(address & !RELATIVE)
            .ok_or(Failure::Limit("JSON address"))
    }
}

impl Transaction {
    pub(super) fn new(state: &JsonStrings, maximum: u64) -> Result<Self, KernelStartupError> {
        let admission = Admission {
            held: metadata(state)?,
            work: 0,
            maximum,
            nodes: 0,
        };
        Ok(Self {
            admission,
            strings: Vec::new(),
            wrappers: Vec::new(),
            containers: Containers {
                records: Vec::new(),
                nodes: Vec::new(),
                defaults: state.containers.defaults,
            },
            blob: Vec::new(),
            fixups: Vec::new(),
            root: None,
            root_pointer: None,
            retire_depth: 0,
            outputs: [Output {
                address: 0,
                length: 0,
                bytes: [0; 128],
                pointer: false,
            }; 3],
            output_count: 0,
            invalidated: Vec::new(),
        })
    }
    pub(super) fn stage(&mut self, state: &JsonStrings) -> Result<(), KernelStartupError> {
        self.strings = clone_records(&state.records, &mut self.admission)?;
        self.wrappers = clone_records(&state.wrappers, &mut self.admission)?;
        self.containers.records = clone_records(&state.containers.records, &mut self.admission)?;
        self.containers.nodes = clone_records(&state.containers.nodes, &mut self.admission)?;
        Ok(())
    }
    fn record_limits(&self) -> Result<(), KernelStartupError> {
        if self.strings.len() > MAX_RECORDS
            || self.wrappers.len() > MAX_RECORDS
            || self.containers.records.len() > MAX_NODES
            || self.containers.nodes.len() > MAX_NODES
        {
            return Err(KernelStartupError::Budget("JSON retained records"));
        }
        let bytes = self
            .containers
            .records
            .capacity()
            .checked_mul(size_of::<Container>())
            .and_then(|n| n.checked_add(self.containers.nodes.capacity() * size_of::<OwnedNode>()))
            .ok_or(KernelStartupError::Budget("JSON metadata"))?;
        let strings = self
            .strings
            .capacity()
            .checked_mul(size_of::<StringRecord>())
            .and_then(|n| n.checked_add(self.wrappers.capacity() * size_of::<WrapperRecord>()))
            .ok_or(KernelStartupError::Budget("JSON metadata"))?;
        if bytes > MAX_CONTAINER_METADATA || strings > MAX_METADATA_BYTES {
            return Err(KernelStartupError::Budget("JSON metadata"));
        }
        Ok(())
    }
    pub(super) fn allocate(&mut self, length: usize) -> Result<u64, KernelStartupError> {
        let start = self
            .blob
            .len()
            .checked_add(7)
            .map(|n| n & !7)
            .ok_or(KernelStartupError::Budget("JSON blob"))?;
        let end = start
            .checked_add(length)
            .ok_or(KernelStartupError::Budget("JSON blob"))?;
        self.admission.spend(length)?;
        let extra = end - self.blob.len();
        self.admission.reserve(&mut self.blob, extra)?;
        self.blob.resize(end, 0);
        Ok(RELATIVE | start as u64)
    }
    fn put(&mut self, address: u64, bytes: &[u8]) -> Result<(), KernelStartupError> {
        let start = usize::try_from(address & !RELATIVE).map_err(|_| failure())?;
        let end = start.checked_add(bytes.len()).ok_or_else(failure)?;
        self.blob
            .get_mut(start..end)
            .ok_or_else(failure)?
            .copy_from_slice(bytes);
        Ok(())
    }
    fn pointer(&mut self, address: u64, value: u64) -> Result<(), KernelStartupError> {
        self.put(address, &value.to_le_bytes())?;
        if value & RELATIVE != 0 {
            self.admission.reserve(&mut self.fixups, 1)?;
            self.fixups.push((address & !RELATIVE) as usize);
        }
        Ok(())
    }
    pub(super) fn output(
        &mut self,
        address: u64,
        bytes: &[u8],
        pointer: bool,
    ) -> Result<(), KernelStartupError> {
        at(address, bytes.len())?;
        if address & RELATIVE != 0 {
            return if pointer {
                self.pointer(
                    address,
                    u64::from_le_bytes(bytes.try_into().map_err(|_| failure())?),
                )
            } else {
                self.put(address, bytes)
            };
        }
        for old in &mut self.outputs[..self.output_count] {
            if old.address == address && old.length == bytes.len() {
                old.bytes[..bytes.len()].copy_from_slice(bytes);
                old.pointer = pointer;
                return Ok(());
            }
            if address < at(old.address, old.length)? && old.address < at(address, bytes.len())? {
                // Partial wrapper overlaps can require the not-yet-selected
                // allocation address. Exact aliases retain ordered last writes.
                return Err(failure());
            }
        }
        if self.output_count == 3 || bytes.len() > 128 {
            return Err(failure());
        }
        self.outputs[self.output_count] = Output {
            address,
            length: bytes.len(),
            bytes: [0; 128],
            pointer,
        };
        self.outputs[self.output_count].bytes[..bytes.len()].copy_from_slice(bytes);
        self.output_count += 1;
        Ok(())
    }
    pub(super) fn read(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        address: u64,
        bytes: &mut [u8],
    ) -> Result<(), KernelStartupError> {
        self.admission
            .spend(bytes.len().div_ceil(32).max(1) * 256 + bytes.len())?;
        if address & RELATIVE != 0 {
            let offset = (address & !RELATIVE) as usize;
            bytes.copy_from_slice(
                self.blob
                    .get(offset..offset + bytes.len())
                    .ok_or_else(failure)?,
            );
            return Ok(());
        }
        let end = at(address, bytes.len())?;
        for range in &self.invalidated {
            if address < range.address + range.size && range.address < end {
                return Err(failure());
            }
        }
        for (index, chunk) in bytes.chunks_mut(32).enumerate() {
            let from = at(address, index * 32)?;
            if let Some(value) = overlay {
                value.read_slice(memory, from, chunk)?;
            } else {
                memory.read(from, chunk).map_err(|e| guest(e, false))?;
            }
        }
        for output in &self.outputs[..self.output_count] {
            let other_end = at(output.address, output.length)?;
            if address < other_end && output.address < end {
                memory
                    .preflight_distinct_backing(
                        address.min(output.address),
                        end.max(other_end) - address.min(output.address),
                        32,
                    )
                    .map_err(|e| guest(e, false))?;
                if output.pointer && (address != output.address || bytes.len() != 8) {
                    return Err(failure());
                }
                for (index, byte) in bytes.iter_mut().enumerate() {
                    if let Some(offset) = (address + index as u64).checked_sub(output.address)
                        && offset < output.length as u64
                    {
                        *byte = output.bytes[offset as usize];
                    }
                }
            } else {
                memory
                    .preflight_disjoint_backing(
                        NativeRange {
                            address,
                            size: bytes.len() as u64,
                        },
                        NativeRange {
                            address: output.address,
                            size: output.length as u64,
                        },
                        32,
                    )
                    .map_err(|e| guest(e, false))?;
            }
        }
        Ok(())
    }
    pub(super) fn word(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        address: u64,
    ) -> Result<u64, KernelStartupError> {
        let mut bytes = [0; 8];
        self.read(memory, overlay, address, &mut bytes)?;
        Ok(u64::from_le_bytes(bytes))
    }
    fn pointer_word(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        address: u64,
    ) -> Result<u64, KernelStartupError> {
        let word = self.word(memory, overlay, address)?;
        // Relative addresses are an internal staging representation. A guest
        // word may never mint one merely by choosing matching numeric bits.
        if word & RELATIVE != 0
            && address & RELATIVE == 0
            && !self.outputs[..self.output_count]
                .iter()
                .any(|output| output.pointer && output.address == address && output.length == 8)
        {
            return Err(failure());
        }
        Ok(word)
    }
    pub(super) fn value_kind(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        address: u64,
    ) -> Result<u32, KernelStartupError> {
        if address == 0 {
            return Ok(0);
        }
        self.live_node(address)?;
        let mut bytes = [0; 4];
        self.read(memory, overlay, at(address, 28)?, &mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }
    fn live_node(&mut self, address: u64) -> Result<(), KernelStartupError> {
        self.admission.spend(self.containers.nodes.len())?;
        if self
            .containers
            .nodes
            .iter()
            .any(|node| node.address == address && !node.live)
        {
            return Err(failure());
        }
        Ok(())
    }
    pub(super) fn string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        wrapper: u64,
        mutable: bool,
    ) -> Result<Vec<u8>, KernelStartupError> {
        if wrapper == 0 {
            return if mutable {
                Err(failure())
            } else {
                Ok(Vec::new())
            };
        }
        external(wrapper)?;
        self.admission.spend(self.wrappers.len())?;
        if self
            .wrappers
            .iter()
            .any(|r| r.address == wrapper && !r.live)
        {
            return Err(failure());
        }
        let token = self.pointer_word(memory, overlay, wrapper)?;
        if token == 0 {
            if mutable {
                let record = self.new_string(None, wrapper, b"")?;
                self.output(wrapper, &record.token.to_le_bytes(), true)?;
            }
            return Ok(Vec::new());
        }
        self.admission.spend(self.strings.len())?;
        let record = *self
            .strings
            .iter()
            .find(|r| r.token == token && r.live)
            .ok_or_else(failure)?;
        let mut bytes = Vec::new();
        self.admission.reserve(&mut bytes, record.length)?;
        bytes.resize(record.length, 0);
        self.read(memory, overlay, record.data, &mut bytes)?;
        Ok(bytes)
    }
    pub(super) fn cstring(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        address: u64,
    ) -> Result<Vec<u8>, KernelStartupError> {
        if address == 0 {
            return Ok(Vec::new());
        }
        external(address)?;
        let mut result = Vec::new();
        for offset in 0..=MAX_TEXT {
            let mut byte = [0];
            self.read(memory, None, at(address, offset)?, &mut byte)?;
            if byte[0] == 0 {
                return Ok(result);
            }
            if offset == MAX_TEXT {
                return Err(KernelStartupError::Budget("JSON key"));
            }
            self.admission.reserve(&mut result, 1)?;
            result.push(byte[0]);
        }
        unreachable!("bounded CString")
    }
    pub(super) fn container(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        wrapper: u64,
        kind: u32,
        mutable: bool,
    ) -> Result<Option<usize>, KernelStartupError> {
        if wrapper == 0 {
            return Ok(None);
        }
        external(wrapper)?;
        let token = self.pointer_word(memory, overlay, wrapper)?;
        if token == 0 {
            if !mutable {
                return Ok(None);
            }
            let index = self.new_container(wrapper, 0, kind, &[])?;
            self.output(
                wrapper,
                &self.containers.records[index].token.to_le_bytes(),
                true,
            )?;
            return Ok(Some(index));
        }
        self.admission.spend(self.containers.records.len())?;
        let index = self
            .containers
            .records
            .iter()
            .position(|r| r.token == token)
            .ok_or_else(failure)?;
        let record = self.containers.records[index];
        if !record.live || record.kind != kind {
            return Err(failure());
        }
        // A token never grants access after backing or mapping permissions change.
        let mut proof = [0; 8];
        self.read(memory, overlay, token, &mut proof)?;
        Ok(Some(index))
    }
    pub(super) fn entries(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        index: usize,
    ) -> Result<Vec<(Vec<u8>, u64)>, KernelStartupError> {
        let record = self.containers.records[index];
        if !record.live || record.length > MAX_NODES {
            return Err(failure());
        }
        let mut entries = Vec::new();
        self.admission.reserve(&mut entries, record.length)?;
        for entry in 0..record.length {
            let width = if record.kind == 6 { 8 } else { 24 };
            let at = at(record.entries, entry * width)?;
            let (key, address) = if record.kind == 7 {
                let data = self.pointer_word(memory, overlay, at)?;
                let length =
                    usize::try_from(self.word(memory, overlay, at + 8)?).map_err(|_| failure())?;
                if length > MAX_TEXT
                    || data < record.keys_begin
                    || data
                        .checked_add(length as u64)
                        .is_none_or(|end| end > record.keys_end)
                {
                    return Err(failure());
                }
                let mut key = Vec::new();
                self.admission.reserve(&mut key, length)?;
                key.resize(length, 0);
                self.read(memory, overlay, data, &mut key)?;
                if entries
                    .last()
                    .is_some_and(|(old, _): &(Vec<u8>, u64)| old >= &key)
                {
                    return Err(failure());
                }
                (key, self.pointer_word(memory, overlay, at + 16)?)
            } else {
                (Vec::new(), self.pointer_word(memory, overlay, at)?)
            };
            self.admission.spend(self.containers.nodes.len())?;
            if !self
                .containers
                .nodes
                .iter()
                .any(|node| node.address == address && node.container == record.token && node.live)
            {
                return Err(failure());
            }
            entries.push((key, address));
        }
        Ok(entries)
    }
    pub(super) fn snapshot(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        address: u64,
        depth: usize,
    ) -> Result<Node, KernelStartupError> {
        self.admission.node(depth)?;
        let kind = self.value_kind(memory, overlay, address)?;
        if address == 0 || kind == 0 {
            return Ok(Node::Scalar(0, 0));
        }
        match kind {
            1 => {
                let mut byte = [0];
                self.read(memory, overlay, at(address, 16)?, &mut byte)?;
                Ok(Node::Scalar(1, u64::from(byte[0])))
            }
            2..=4 => Ok(Node::Scalar(
                kind,
                self.word(memory, overlay, at(address, 16)?)?,
            )),
            5 => {
                let wrapper = self.pointer_word(memory, overlay, at(address, 16)?)?;
                Ok(Node::String(self.string(memory, overlay, wrapper, true)?))
            }
            6 | 7 => {
                let wrapper = self.pointer_word(memory, overlay, at(address, 16)?)?;
                self.snapshot_container(memory, overlay, wrapper, kind, depth)
            }
            _ => Ok(Node::Scalar(kind, 0)),
        }
    }
    pub(super) fn snapshot_container(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        wrapper: u64,
        kind: u32,
        depth: usize,
    ) -> Result<Node, KernelStartupError> {
        let entries = match self.container(memory, overlay, wrapper, kind, false)? {
            Some(index) => self.entries(memory, overlay, index)?,
            None => Vec::new(),
        };
        if kind == 6 {
            let mut nodes = Vec::new();
            self.admission.reserve(&mut nodes, entries.len())?;
            for (_, child) in entries {
                nodes.push(self.snapshot(memory, overlay, child, depth + 1)?);
            }
            Ok(Node::Array(nodes))
        } else {
            let mut nodes = Vec::new();
            self.admission.reserve(&mut nodes, entries.len())?;
            for (key, child) in entries {
                nodes.push((key, self.snapshot(memory, overlay, child, depth + 1)?));
            }
            Ok(Node::Object(nodes))
        }
    }
    fn new_string(
        &mut self,
        value: Option<u64>,
        wrapper: u64,
        bytes: &[u8],
    ) -> Result<StringRecord, KernelStartupError> {
        if bytes.len() > MAX_TEXT {
            return Err(KernelStartupError::Budget("JSON String bytes"));
        }
        let token = self.allocate(8)?;
        let data = self.allocate(bytes.len() + 1)?;
        self.put(data, bytes)?;
        let record = StringRecord {
            token,
            data,
            length: bytes.len(),
            live: true,
        };
        self.admission.reserve(&mut self.strings, 1)?;
        self.strings.push(record);
        if let Some(value) = value {
            self.admission.reserve(&mut self.wrappers, 1)?;
            self.wrappers.push(WrapperRecord {
                address: wrapper,
                value,
                live: true,
            });
        }
        self.record_limits()?;
        Ok(record)
    }
    pub(super) fn encode(
        &mut self,
        node: &Node,
        parent: u64,
        owner: u64,
    ) -> Result<u64, KernelStartupError> {
        let address = self.allocate(32)?;
        self.pointer(address, parent)?;
        self.encode_payload(node, address, address)?;
        self.admission.reserve(&mut self.containers.nodes, 1)?;
        self.containers.nodes.push(OwnedNode {
            address,
            container: owner,
            live: true,
        });
        self.record_limits()?;
        Ok(address)
    }
    pub(super) fn encode_payload(
        &mut self,
        node: &Node,
        address: u64,
        owner_value: u64,
    ) -> Result<u64, KernelStartupError> {
        let (kind, pointer) = match node {
            Node::Scalar(kind, payload) => {
                self.put(address + 28, &kind.to_le_bytes())?;
                self.put(address + 16, &payload.to_le_bytes())?;
                return Ok(0);
            }
            Node::String(bytes) => {
                let wrapper = self.allocate(8)?;
                let record = self.new_string(Some(owner_value), wrapper, bytes)?;
                self.pointer(wrapper, record.token)?;
                (5u32, wrapper)
            }
            Node::Array(_) => (6, self.encode_container(owner_value, 6, node)?),
            Node::Object(_) => (7, self.encode_container(owner_value, 7, node)?),
        };
        self.put(address + 28, &kind.to_le_bytes())?;
        self.pointer(address + 16, pointer)?;
        Ok(pointer)
    }
    pub(super) fn payload(
        &mut self,
        value: &mut StagedValue,
        node: &Node,
    ) -> Result<(), KernelStartupError> {
        let temporary = self.allocate(32)?;
        let pointer = self.encode_payload(node, temporary, value.address)?;
        let offset = (temporary & !RELATIVE) as usize;
        value.write(16, &self.blob[offset + 16..offset + 24]);
        self.root_pointer = if pointer != 0 { Some(pointer) } else { None };
        Ok(())
    }
    pub(super) fn construct(
        &mut self,
        wrapper: u64,
        node: &Node,
    ) -> Result<(), KernelStartupError> {
        let kind = match node {
            Node::Array(_) => 6,
            Node::Object(_) => 7,
            _ => return Err(failure()),
        };
        let temporary = self.encode_container(0, kind, node)?;
        let record = self
            .containers
            .records
            .iter_mut()
            .find(|r| r.wrapper == temporary)
            .ok_or_else(failure)?;
        record.wrapper = wrapper;
        let token = record.token;
        self.output(wrapper, &token.to_le_bytes(), true)
    }
    fn encode_container(
        &mut self,
        value: u64,
        kind: u32,
        node: &Node,
    ) -> Result<u64, KernelStartupError> {
        let count = match node {
            Node::Array(children) => children.len(),
            Node::Object(children) => children.len(),
            _ => return Err(failure()),
        };
        let wrapper = self.allocate(8)?;
        let token = self.allocate(8)?;
        let entries = self.allocate(
            count
                .checked_mul(if kind == 6 { 8 } else { 24 })
                .ok_or_else(failure)?,
        )?;
        let keys_begin = self.allocate(0)?;
        let mut keys = Vec::new();
        self.admission.reserve(&mut keys, count)?;
        for index in 0..count {
            let key = match node {
                Node::Object(children) => children[index].0.as_slice(),
                _ => b"",
            };
            let address = self.allocate(key.len())?;
            self.put(address, key)?;
            keys.push(address);
        }
        let keys_end = self.allocate(0)?;
        for (index, key_address) in keys.into_iter().enumerate() {
            let (key, child) = match node {
                Node::Array(children) => (b"".as_slice(), &children[index]),
                Node::Object(children) => (children[index].0.as_slice(), &children[index].1),
                _ => unreachable!("checked container node"),
            };
            let child = self.encode(child, value, token)?;
            let at = entries + index as u64 * if kind == 6 { 8 } else { 24 };
            if kind == 7 {
                self.pointer(at, key_address)?;
                self.put(at + 8, &(key.len() as u64).to_le_bytes())?;
                self.pointer(at + 16, child)?;
            } else {
                self.pointer(at, child)?;
            }
        }
        self.admission.reserve(&mut self.containers.records, 1)?;
        self.containers.records.push(Container {
            token,
            wrapper,
            owner_value: value,
            kind,
            entries,
            length: count,
            keys_begin,
            keys_end,
            live: true,
        });
        self.pointer(wrapper, token)?;
        self.record_limits()?;
        Ok(wrapper)
    }
    pub(super) fn new_container(
        &mut self,
        wrapper: u64,
        value: u64,
        kind: u32,
        entries: &[(Vec<u8>, u64)],
    ) -> Result<usize, KernelStartupError> {
        let token = self.allocate(8)?;
        let index = self.containers.records.len();
        self.admission.reserve(&mut self.containers.records, 1)?;
        self.containers.records.push(Container {
            token,
            wrapper,
            owner_value: value,
            kind,
            entries: 0,
            length: 0,
            keys_begin: 0,
            keys_end: 0,
            live: true,
        });
        self.replace_entries(index, entries)?;
        self.record_limits()?;
        Ok(index)
    }
    pub(super) fn replace_entries(
        &mut self,
        index: usize,
        entries: &[(Vec<u8>, u64)],
    ) -> Result<(), KernelStartupError> {
        let kind = self.containers.records[index].kind;
        let table = self.allocate(
            entries
                .len()
                .checked_mul(if kind == 6 { 8 } else { 24 })
                .ok_or_else(failure)?,
        )?;
        let keys_begin = self.allocate(0)?;
        for (offset, (key, child)) in entries.iter().enumerate() {
            let at = table + offset as u64 * if kind == 6 { 8 } else { 24 };
            if kind == 7 {
                let data = self.allocate(key.len())?;
                self.put(data, key)?;
                self.pointer(at, data)?;
                self.put(at + 8, &(key.len() as u64).to_le_bytes())?;
                self.pointer(at + 16, *child)?;
            } else {
                self.pointer(at, *child)?;
            }
        }
        let keys_end = self.allocate(0)?;
        let record = &mut self.containers.records[index];
        record.entries = table;
        record.length = entries.len();
        record.keys_begin = keys_begin;
        record.keys_end = keys_end;
        Ok(())
    }
    pub(super) fn defaults(&mut self) -> Result<Defaults, KernelStartupError> {
        if let Some(defaults) = self.containers.defaults {
            return Ok(defaults);
        }
        let null = self.allocate(32)?;
        let array = self.allocate(8)?;
        let object = self.allocate(8)?;
        for (wrapper, kind) in [(array, 6), (object, 7)] {
            let index = self.new_container(wrapper, 0, kind, &[])?;
            self.pointer(wrapper, self.containers.records[index].token)?;
        }
        let defaults = Defaults {
            null,
            array,
            object,
        };
        self.containers.defaults = Some(defaults);
        Ok(defaults)
    }
    pub(super) fn retire_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: &StagedValue,
        wrapper: u64,
    ) -> Result<(), KernelStartupError> {
        self.admission.spend(self.wrappers.len())?;
        let owner = self
            .wrappers
            .iter()
            .position(|r| r.address == wrapper && r.value == overlay.address && r.live)
            .ok_or_else(failure)?;
        let token = self.word(memory, Some(overlay), wrapper)?;
        if token != 0 {
            self.admission.spend(self.strings.len())?;
            let record = self
                .strings
                .iter_mut()
                .find(|r| r.token == token && r.live)
                .ok_or_else(failure)?;
            self.admission.reserve(&mut self.invalidated, 1)?;
            self.invalidated.push(NativeRange {
                address: record.data,
                size: record.length as u64 + 1,
            });
            record.live = false;
        }
        self.wrappers[owner].live = false;
        Ok(())
    }
    pub(super) fn retire_container(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        overlay: Option<&StagedValue>,
        wrapper: u64,
        kind: u32,
        owner: Option<u64>,
        depth: usize,
    ) -> Result<(), KernelStartupError> {
        self.admission.node(depth)?;
        let Some(index) = self.container(memory, overlay, wrapper, kind, false)? else {
            return Ok(());
        };
        let record = self.containers.records[index];
        if record.wrapper != wrapper || owner.is_some_and(|value| record.owner_value != value) {
            return Err(failure());
        }
        let entries = self.entries(memory, overlay, index)?;
        // Mark in the staged owner first: a cycle or repeated owning child is
        // refused rather than recursing through freed host/guest storage.
        self.containers.records[index].live = false;
        for (_, child) in entries {
            let mut value = StagedValue::new(child)?;
            let mut ignored = 0;
            value.clear(memory, self, &mut ignored, u64::MAX)?;
            self.admission.spend(self.containers.nodes.len())?;
            self.containers
                .nodes
                .iter_mut()
                .find(|node| node.address == child && node.live)
                .ok_or_else(failure)?
                .live = false;
        }
        Ok(())
    }
    pub(super) fn replace_string(
        &mut self,
        memory: &PausedNativeMemory<'_>,
        wrapper: u64,
        bytes: &[u8],
    ) -> Result<(), KernelStartupError> {
        let token = self.word(memory, None, wrapper)?;
        if token == 0 {
            let record = self.new_string(None, wrapper, bytes)?;
            return self.output(wrapper, &record.token.to_le_bytes(), true);
        }
        self.admission.spend(self.strings.len())?;
        let index = self
            .strings
            .iter()
            .position(|r| r.token == token && r.live)
            .ok_or_else(failure)?;
        let old = self.strings[index];
        let data = self.allocate(bytes.len() + 1)?;
        self.put(data, bytes)?;
        if old.data & RELATIVE == 0 {
            self.admission.reserve(&mut self.invalidated, 1)?;
            self.invalidated.push(NativeRange {
                address: old.data,
                size: old.length as u64 + 1,
            });
        }
        self.strings[index].data = data;
        self.strings[index].length = bytes.len();
        Ok(())
    }
    pub(super) fn publish(
        &mut self,
        state: &mut JsonStrings,
        memory: &mut PausedNativeMemory<'_>,
        kernel: &mut pentax_kernel::Kernel,
        result: u64,
    ) -> Result<u64, KernelStartupError> {
        if self.blob.is_empty()
            && self.output_count == 0
            && self.root.is_none()
            && self.invalidated.is_empty()
        {
            // Read-only queries and parser source errors discard all temporary
            // clones; they do not publish enlarged metadata or guest storage.
            return Ok(result);
        }
        self.record_limits()?;
        let mapped = (self.blob.len() as u64)
            .checked_add(0x3fff)
            .map(|n| n & !0x3fff)
            .ok_or(KernelStartupError::Budget("JSON guest bytes"))?;
        let next_guest = state
            .guest_bytes
            .checked_add(mapped)
            .filter(|n| *n <= MAX_GUEST_BYTES)
            .ok_or(KernelStartupError::Budget("JSON guest bytes"))?;
        self.admission.spend(
            self.blob.len()
                + self.fixups.len() * 8
                + self.strings.len()
                + self.wrappers.len()
                + self.containers.records.len()
                + self.containers.nodes.len()
                + 1024,
        )?;
        if let Some(root) = &self.root {
            memory
                .preflight_distinct_backing(root.address, 32, 32)
                .map_err(|e| guest(e, true))?;
            for output in &self.outputs[..self.output_count] {
                if root.address < at(output.address, output.length)?
                    && output.address < at(root.address, 32)?
                {
                    return Err(failure());
                }
            }
        }
        let mut final_result = result;
        if self.blob.is_empty() {
            let outputs = self
                .finish(0, &mut [], result, &mut final_result)
                .map_err(KernelStartupError::Kernel)?;
            let mut writes = [NativeMemoryWrite {
                address: 0,
                bytes: &[],
            }; 3];
            // RuntimeDataOutputs intentionally keeps its fields opaque. The
            // fixed staging below is rebuilt from our already resolved fields.
            let _ = outputs;
            let mut bytes = [[0; 128]; 3];
            let mut addresses = [0; 3];
            let mut lengths = [0; 3];
            let mut count = 0;
            for output in &self.outputs[..self.output_count] {
                bytes[count] = output.bytes;
                addresses[count] = output.address;
                lengths[count] = output.length;
                count += 1;
            }
            if let Some(root) = &self.root {
                root.outputs(|address, source| {
                    addresses[count] = address;
                    lengths[count] = source.len();
                    bytes[count][..source.len()].copy_from_slice(source);
                    count += 1;
                });
            }
            for index in 0..count {
                writes[index] = NativeMemoryWrite {
                    address: addresses[index],
                    bytes: &bytes[index][..lengths[index]],
                };
            }
            if count != 0 {
                memory
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::None,
                            writes: &writes[..count],
                        },
                        &NativeTransactionLimits {
                            max_outputs: 3,
                            max_output_bytes: 384,
                            max_preflight_steps: 256,
                            max_scratch_bytes: 0,
                        },
                    )
                    .map_err(|e| guest(e, true))?;
            }
        } else {
            let mut adapter = super::super::super::super::kernel_adapter::KernelMemory {
                paused: memory,
                source: None,
                supplied_modules: 0,
                lookup_limits: None,
                lookup_work: None,
                loaded_modules: None,
                thread_tls: None,
                initial_entry: None,
            };
            kernel
                .publish_runtime_data(
                    self.blob.len(),
                    self.admission.held,
                    &mut adapter,
                    |base, bytes| self.finish(base, bytes, result, &mut final_result),
                )
                .map_err(KernelStartupError::Kernel)?;
        }
        std::mem::swap(&mut state.records, &mut self.strings);
        std::mem::swap(&mut state.wrappers, &mut self.wrappers);
        std::mem::swap(&mut state.containers, &mut self.containers);
        state.guest_bytes = next_guest;
        Ok(final_result)
    }
    fn finish(
        &mut self,
        base: u64,
        bytes: &mut [u8],
        result: u64,
        final_result: &mut u64,
    ) -> Result<RuntimeDataOutputs, Failure> {
        if !self.blob.is_empty() {
            bytes.copy_from_slice(&self.blob);
            for offset in &self.fixups {
                let pointer = u64::from_le_bytes(bytes[*offset..*offset + 8].try_into().unwrap());
                bytes[*offset..*offset + 8]
                    .copy_from_slice(&resolved(pointer, base)?.to_le_bytes());
            }
        }
        let mut outputs = RuntimeDataOutputs::new();
        for output in &mut self.outputs[..self.output_count] {
            if output.pointer {
                let pointer = u64::from_le_bytes(output.bytes[..8].try_into().unwrap());
                output.bytes[..8].copy_from_slice(&resolved(pointer, base)?.to_le_bytes());
            }
            outputs.push(output.address, &output.bytes[..output.length])?;
        }
        if let Some(root) = &mut self.root {
            if let Some(pointer) = self.root_pointer {
                root.write(16, &resolved(pointer, base)?.to_le_bytes());
            }
            let mut status = Ok(());
            root.outputs(|address, source| {
                if status.is_ok() {
                    status = outputs.push(address, source);
                }
            });
            status?;
        }
        for string in &mut self.strings {
            string.token = resolved(string.token, base)?;
            string.data = resolved(string.data, base)?;
        }
        for wrapper in &mut self.wrappers {
            wrapper.address = resolved(wrapper.address, base)?;
            wrapper.value = resolved(wrapper.value, base)?;
        }
        for record in &mut self.containers.records {
            record.token = resolved(record.token, base)?;
            record.wrapper = resolved(record.wrapper, base)?;
            record.owner_value = resolved(record.owner_value, base)?;
            record.entries = resolved(record.entries, base)?;
            record.keys_begin = resolved(record.keys_begin, base)?;
            record.keys_end = resolved(record.keys_end, base)?;
        }
        for node in &mut self.containers.nodes {
            node.address = resolved(node.address, base)?;
            node.container = resolved(node.container, base)?;
        }
        if let Some(defaults) = &mut self.containers.defaults {
            defaults.null = resolved(defaults.null, base)?;
            defaults.array = resolved(defaults.array, base)?;
            defaults.object = resolved(defaults.object, base)?;
        }
        *final_result = if !self.blob.is_empty()
            && result & RELATIVE != 0
            && result & !RELATIVE <= self.blob.len() as u64
        {
            resolved(result, base)?
        } else {
            result
        };
        Ok(outputs)
    }
}
