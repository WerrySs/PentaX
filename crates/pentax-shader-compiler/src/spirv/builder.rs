// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de,
// backend/spirv/SpirvBuilder.{h,cpp}: logical-layout sections, id
// allocation, type/decorated-type/constant interning keys, capability and
// extension sets, ext-inst imports, entry points (1.4+ interface rule),
// deferred phis and Build ordering.
// PentaX modifications: every section growth, id and interned key is charged
// to the compiler budget; failures return typed errors and no module.

use super::spec::*;
use crate::CompileError;
use crate::budget::{Budget, LimitKind};
use crate::manifest::RequiredFeatures;

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
enum Section {
    Capabilities,
    Extensions,
    Imports,
    MemoryModel,
    EntryPoints,
    ExecutionModes,
    Debug,
    Annotations,
    Declarations,
    Functions,
}

const SECTIONS: usize = 10;

/// One annotation of a decorated type (source `TypeAnnotation`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Annotation<'a> {
    pub op: u32,
    pub operands: &'a [u32],
}

/// A phi whose incoming pairs are patched after all blocks are emitted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DeferredPhi {
    offset: usize,
}

pub(crate) struct Builder<'a> {
    budget: &'a mut Budget,
    next_id: u32,
    version: u32,
    required_features: RequiredFeatures,
    sections: [Vec<u32>; SECTIONS],
    words: usize,
    capabilities: Vec<u32>,
    extensions: Vec<&'static str>,
    imports: Vec<(&'static str, u32)>,
    // Flat, capacity-accounted entries. Every compared word is work-charged;
    // no opaque hash-table allocation or nested unaccounted key cloning.
    interned: Vec<(Vec<u32>, u32)>,
    unpatched_phi_incomings: usize,
}

fn string_words(text: &str) -> usize {
    (text.len() + 1).div_ceil(4)
}

impl<'a> Builder<'a> {
    pub fn new(budget: &'a mut Budget, version: u32) -> Self {
        Self {
            budget,
            next_id: 1,
            version,
            required_features: RequiredFeatures {
                spirv_version: version,
                ..RequiredFeatures::default()
            },
            sections: Default::default(),
            words: 0,
            capabilities: Vec::new(),
            extensions: Vec::new(),
            imports: Vec::new(),
            interned: Vec::new(),
            unpatched_phi_incomings: 0,
        }
    }

    pub fn budget(&mut self) -> &mut Budget {
        self.budget
    }

    pub fn budget_ref(&self) -> &Budget {
        self.budget
    }

    pub fn required_features(&self) -> RequiredFeatures {
        self.required_features
    }

    fn word_list(&mut self, prefix: &[u32], tail: &[u32]) -> Result<Vec<u32>, CompileError> {
        let count = prefix
            .len()
            .checked_add(tail.len())
            .ok_or(CompileError::limit(LimitKind::Bytes))?;
        let mut words = Vec::new();
        self.budget
            .reserve(&mut words, count, usize::MAX, LimitKind::Bytes)?;
        self.budget.charge_usize(count)?;
        words.extend_from_slice(prefix);
        words.extend_from_slice(tail);
        Ok(words)
    }

    pub fn alloc_id(&mut self) -> Result<u32, CompileError> {
        let id = self.next_id;
        let next = id
            .checked_add(1)
            .ok_or(CompileError::limit(LimitKind::SpirvIds))?;
        if next > self.budget.limits.max_spirv_ids {
            return Err(CompileError::limit(LimitKind::SpirvIds));
        }
        self.next_id = next;
        Ok(id)
    }

    fn append(&mut self, section: Section, op: u32, operands: &[u32]) -> Result<(), CompileError> {
        let count = operands.len() + 1;
        if count > 0xffff {
            return Err(CompileError::limit(LimitKind::SpirvWords));
        }
        let max = self.budget.limits.max_spirv_words;
        let total = self
            .words
            .checked_add(count)
            .ok_or(CompileError::limit(LimitKind::SpirvWords))?;
        if total > max {
            return Err(CompileError::limit(LimitKind::SpirvWords));
        }
        let words = &mut self.sections[section as usize];
        self.budget
            .reserve(words, count, max, LimitKind::SpirvWords)?;
        self.budget.charge_usize(count)?;
        words.push((count as u32) << WordCountShift | op);
        words.extend_from_slice(operands);
        self.words = total;
        Ok(())
    }

    fn append_string(&mut self, out: &mut Vec<u32>, text: &str) -> Result<(), CompileError> {
        let words = string_words(text);
        self.budget
            .reserve(out, words, usize::MAX, LimitKind::SpirvWords)?;
        self.budget.charge_usize(words)?;
        let bytes = text.as_bytes();
        for index in 0..words {
            let mut word = 0u32;
            for byte in 0..4 {
                if let Some(&b) = bytes.get(index * 4 + byte) {
                    word |= u32::from(b) << (byte * 8);
                }
            }
            out.push(word);
        }
        Ok(())
    }

    pub fn require_capability(&mut self, capability: u32) -> Result<(), CompileError> {
        match capability {
            value if value == CapabilityShader => self.required_features.shader = true,
            value if value == CapabilitySignedZeroInfNanPreserve => {
                self.required_features
                    .signed_zero_inf_nan_preserve_capability = true
            }
            value if value == CapabilityInt64 => self.required_features.shader_int64 = true,
            value if value == CapabilityDrawParameters => {
                self.required_features.draw_parameters = true
            }
            value if value == CapabilityGroupNonUniform => {
                self.required_features.subgroup_basic = true
            }
            value if value == CapabilityGroupNonUniformBallot => {
                self.required_features.subgroup_ballot = true
            }
            _ => {
                return Err(CompileError::Internal(
                    "capability without host feature contract",
                ));
            }
        }
        self.budget.charge_usize(self.capabilities.len())?;
        if self.capabilities.contains(&capability) {
            return Ok(());
        }
        self.budget.push(
            &mut self.capabilities,
            capability,
            256,
            LimitKind::SourceTable,
        )?;
        self.append(Section::Capabilities, OpCapability, &[capability])
    }

    pub fn require_extension(&mut self, name: &'static str) -> Result<(), CompileError> {
        match name {
            "SPV_KHR_float_controls" => self.required_features.float_controls_extension = true,
            _ => {
                return Err(CompileError::Internal(
                    "extension without host feature contract",
                ));
            }
        }
        self.budget.charge_usize(self.extensions.len())?;
        if self.extensions.contains(&name) {
            return Ok(());
        }
        self.budget
            .push(&mut self.extensions, name, 64, LimitKind::SourceTable)?;
        let mut operands = Vec::new();
        self.append_string(&mut operands, name)?;
        self.append(Section::Extensions, OpExtension, &operands)
    }

    pub fn import(&mut self, name: &'static str) -> Result<u32, CompileError> {
        self.budget.charge_usize(self.imports.len())?;
        if let Some(&(_, id)) = self.imports.iter().find(|(n, _)| *n == name) {
            return Ok(id);
        }
        let id = self.alloc_id()?;
        self.budget
            .push(&mut self.imports, (name, id), 16, LimitKind::SourceTable)?;
        let mut operands = self.word_list(&[id], &[])?;
        self.append_string(&mut operands, name)?;
        self.append(Section::Imports, OpExtInstImport, &operands)?;
        Ok(id)
    }

    fn intern(&mut self, key: Vec<u32>) -> Result<Result<u32, Vec<u32>>, CompileError> {
        for (existing, id) in &self.interned {
            self.budget
                .charge_usize(existing.len().min(key.len()) + 1)?;
            if existing == &key {
                return Ok(Ok(*id));
            }
        }
        self.budget.reserve(
            &mut self.interned,
            1,
            self.budget.limits.max_spirv_ids as usize,
            LimitKind::SpirvIds,
        )?;
        Ok(Err(key))
    }

    fn remember(&mut self, key: Vec<u32>, id: u32) -> Result<(), CompileError> {
        // The key capacity was charged when it was constructed, and the
        // entry slot was reserved by intern. Moving both cannot allocate.
        self.interned.push((key, id));
        Ok(())
    }

    /// Source `Builder::Type` (key: opcode, operand count, operands).
    pub fn ty(&mut self, op: u32, operands: &[u32]) -> Result<u32, CompileError> {
        let mut key = Vec::new();
        self.budget
            .reserve(&mut key, operands.len() + 2, usize::MAX, LimitKind::Bytes)?;
        key.push(op);
        key.push(operands.len() as u32);
        key.extend_from_slice(operands);
        match self.intern(key)? {
            Ok(id) => Ok(id),
            Err(key) => {
                let id = self.alloc_id()?;
                let mut words = Vec::new();
                self.budget.reserve(
                    &mut words,
                    operands.len() + 1,
                    usize::MAX,
                    LimitKind::Bytes,
                )?;
                words.push(id);
                words.extend_from_slice(operands);
                self.append(Section::Declarations, op, &words)?;
                self.remember(key, id)?;
                Ok(id)
            }
        }
    }

    /// Source `Builder::DecoratedType`.
    pub fn decorated_ty(
        &mut self,
        op: u32,
        annotations: &[Annotation<'_>],
        operands: &[u32],
    ) -> Result<u32, CompileError> {
        if annotations.is_empty() {
            return self.ty(op, operands);
        }
        let mut key = Vec::new();
        let extra: usize = annotations.iter().map(|a| a.operands.len() + 2).sum();
        self.budget.reserve(
            &mut key,
            operands.len() + 3 + extra,
            usize::MAX,
            LimitKind::Bytes,
        )?;
        key.push(op);
        key.push(operands.len() as u32);
        key.extend_from_slice(operands);
        key.push(annotations.len() as u32);
        for annotation in annotations {
            key.push(annotation.op);
            key.push(annotation.operands.len() as u32);
            key.extend_from_slice(annotation.operands);
        }
        match self.intern(key)? {
            Ok(id) => Ok(id),
            Err(key) => {
                let id = self.alloc_id()?;
                let words = self.word_list(&[id], operands)?;
                self.append(Section::Declarations, op, &words)?;
                for annotation in annotations {
                    let words = self.word_list(&[id], annotation.operands)?;
                    self.append(Section::Annotations, annotation.op, &words)?;
                }
                self.remember(key, id)?;
                Ok(id)
            }
        }
    }

    /// Source `Builder::Constant` (key: opcode, type, operands).
    pub fn constant(&mut self, op: u32, ty: u32, operands: &[u32]) -> Result<u32, CompileError> {
        let mut key = Vec::new();
        self.budget
            .reserve(&mut key, operands.len() + 2, usize::MAX, LimitKind::Bytes)?;
        key.push(op);
        key.push(ty);
        key.extend_from_slice(operands);
        match self.intern(key)? {
            Ok(id) => Ok(id),
            Err(key) => {
                let id = self.alloc_id()?;
                let words = self.word_list(&[ty, id], operands)?;
                self.append(Section::Declarations, op, &words)?;
                self.remember(key, id)?;
                Ok(id)
            }
        }
    }

    pub fn global_variable(&mut self, pointer_ty: u32, storage: u32) -> Result<u32, CompileError> {
        let id = self.alloc_id()?;
        self.append(
            Section::Declarations,
            OpVariable,
            &[pointer_ty, id, storage],
        )?;
        Ok(id)
    }

    pub fn memory_model(&mut self, addressing: u32, memory: u32) -> Result<(), CompileError> {
        self.append(Section::MemoryModel, OpMemoryModel, &[addressing, memory])
    }

    /// Source `AddEntryPoint` (SPIR-V 1.4+ lists every global variable).
    pub fn entry_point(
        &mut self,
        model: u32,
        function: u32,
        name: &str,
        interfaces: &[u32],
    ) -> Result<(), CompileError> {
        let mut operands = self.word_list(&[model, function], &[])?;
        self.append_string(&mut operands, name)?;
        self.budget.reserve(
            &mut operands,
            interfaces.len(),
            usize::MAX,
            LimitKind::Bytes,
        )?;
        self.budget.charge_usize(interfaces.len())?;
        operands.extend_from_slice(interfaces);
        if self.version >= 0x0001_0400 {
            let declarations = &self.sections[Section::Declarations as usize];
            let mut offset = 0;
            let mut extra = Vec::new();
            while offset < declarations.len() {
                self.budget.charge_usize(1 + interfaces.len())?;
                let word = declarations[offset];
                let count = (word >> WordCountShift) as usize;
                if word & OpCodeMask == OpVariable
                    && let Some(&id) = declarations.get(offset + 2)
                    && !interfaces.contains(&id)
                {
                    self.budget
                        .push(&mut extra, id, usize::MAX, LimitKind::SpirvIds)?;
                }
                offset += count.max(1);
            }
            self.budget.charge_usize(declarations.len())?;
            self.budget
                .reserve(&mut operands, extra.len(), usize::MAX, LimitKind::Bytes)?;
            operands.extend_from_slice(&extra);
        }
        self.append(Section::EntryPoints, OpEntryPoint, &operands)
    }

    pub fn execution_mode(
        &mut self,
        function: u32,
        mode: u32,
        operands: &[u32],
    ) -> Result<(), CompileError> {
        match mode {
            value if value == ExecutionModeSignedZeroInfNanPreserve && operands == [32] => {
                self.required_features.float32_signed_zero_inf_nan_preserve = true;
            }
            value if value == ExecutionModeLocalSize || value == ExecutionModeOriginUpperLeft => {}
            _ => {
                return Err(CompileError::Internal(
                    "execution mode without host feature contract",
                ));
            }
        }
        let words = self.word_list(&[function, mode], operands)?;
        self.append(Section::ExecutionModes, OpExecutionMode, &words)
    }

    pub fn name(&mut self, target: u32, name: &str) -> Result<(), CompileError> {
        let mut operands = self.word_list(&[target], &[])?;
        self.append_string(&mut operands, name)?;
        self.append(Section::Debug, OpName, &operands)
    }

    pub fn annotate(&mut self, op: u32, operands: &[u32]) -> Result<(), CompileError> {
        self.append(Section::Annotations, op, operands)
    }

    pub fn func(&mut self, op: u32, operands: &[u32]) -> Result<(), CompileError> {
        self.append(Section::Functions, op, operands)
    }

    /// Source `AddDeferredPhi`.
    pub fn deferred_phi(
        &mut self,
        ty: u32,
        result: u32,
        incoming: usize,
    ) -> Result<DeferredPhi, CompileError> {
        let offset = self.sections[Section::Functions as usize].len();
        let mut operands = self.word_list(&[ty, result], &[])?;
        let pairs = incoming
            .checked_mul(2)
            .ok_or(CompileError::limit(LimitKind::SpirvWords))?;
        self.budget
            .reserve(&mut operands, pairs, usize::MAX, LimitKind::Bytes)?;
        self.budget.charge_usize(pairs)?;
        operands.resize(2 + pairs, 0);
        self.append(Section::Functions, OpPhi, &operands)?;
        self.unpatched_phi_incomings += incoming;
        Ok(DeferredPhi { offset })
    }

    /// Source `PatchDeferredPhi`.
    pub fn patch_phi(
        &mut self,
        phi: DeferredPhi,
        incoming: usize,
        value: u32,
        parent: u32,
    ) -> Result<(), CompileError> {
        let functions = &mut self.sections[Section::Functions as usize];
        let header = *functions
            .get(phi.offset)
            .ok_or(CompileError::Internal("deferred phi offset"))?;
        let count = ((header >> WordCountShift) as usize).saturating_sub(3) / 2;
        if incoming >= count || value == 0 || parent == 0 {
            return Err(CompileError::Internal("deferred phi operand"));
        }
        let value_word = phi.offset + 3 + incoming * 2;
        if functions.get(value_word) != Some(&0) || functions.get(value_word + 1) != Some(&0) {
            return Err(CompileError::Internal("deferred phi patched twice"));
        }
        functions[value_word] = value;
        functions[value_word + 1] = parent;
        self.unpatched_phi_incomings -= 1;
        Ok(())
    }

    /// Source `Build`: header, then the sections in logical-layout order.
    pub fn build(self) -> Result<Vec<u32>, CompileError> {
        if self.unpatched_phi_incomings != 0 {
            return Err(CompileError::Internal("unpatched phi incomings"));
        }
        let mut module = Vec::new();
        self.budget.reserve(
            &mut module,
            self.words + 5,
            self.budget.limits.max_spirv_words,
            LimitKind::SpirvWords,
        )?;
        self.budget.charge_usize(self.words + 5)?;
        module.extend_from_slice(&[MagicNumber, self.version, 0, self.next_id, 0]);
        for section in &self.sections {
            module.extend_from_slice(section);
        }
        Ok(module)
    }
}
