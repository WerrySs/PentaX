// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance 209dffe: ShaderDecoder.cpp `DecodeProgram`
// (S_ENDPGM termination walker), ShaderDecoder.cpp `IsDirectBranch` and
// ShaderCFG.cpp `IsValidTarget`. PentaX modifications: the linear decoder of
// `pentax-shader` supplies the instructions; termination must be the exact end
// of the caller span; branch targets are checked (the source indexes its
// target table unchecked) and resolved to instruction indices.

use pentax_shader::{DecodedSpan, Instruction, Opcode, ShaderStage};

use crate::budget::{Budget, LimitKind};
use crate::error::{ProgramError, ProgramErrorKind};
use crate::{CompileError, Limits};

/// Source `IsConditionalBranch` (debug branches are excluded by the source).
pub(crate) fn is_conditional_branch(opcode: Opcode) -> bool {
    matches!(
        opcode,
        Opcode::SCbranchScc0
            | Opcode::SCbranchScc1
            | Opcode::SCbranchVccz
            | Opcode::SCbranchVccnz
            | Opcode::SCbranchExecz
            | Opcode::SCbranchExecnz
            | Opcode::SSubvectorLoopBegin
            | Opcode::SSubvectorLoopEnd
    )
}

/// Source `IsDirectBranch`.
pub(crate) fn is_direct_branch(opcode: Opcode) -> bool {
    opcode == Opcode::SBranch || is_conditional_branch(opcode)
}

/// A validated direct-branch destination.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchTarget {
    /// The instruction with this index starts at the target PC.
    Instruction(usize),
    /// The target is the program end PC (accepted by the source `IsValidTarget`).
    End,
}

const NO_TARGET: u32 = u32::MAX;
const END_TARGET: u32 = u32::MAX - 1;

/// One exactly delimited, terminated shader program. Every direct branch
/// target is a proven instruction start (or the end PC). The decoded span and
/// the target table are private, so they cannot be associated or forged.
#[derive(Debug)]
pub struct ShaderProgram {
    span: DecodedSpan,
    targets: Vec<u32>,
    end_pc: u32,
    work: u64,
}

impl ShaderProgram {
    /// Decode `words` and prove the program boundary under the source rules:
    /// nonempty input of a translatable stage, a terminating S_ENDPGM found by
    /// the `DecodeProgram` walk (an S_ENDPGM directly followed by an earlier
    /// forward branch target does not terminate), no words after it, and
    /// branch targets on instruction starts within `[0, end]`.
    pub fn decode(
        words: &[u32],
        stage: ShaderStage,
        limits: &Limits,
    ) -> Result<Self, CompileError> {
        if matches!(stage, ShaderStage::Unknown | ShaderStage::Fetch) {
            return Err(CompileError::Program(ProgramError {
                pc: 0,
                kind: ProgramErrorKind::InvalidStage,
            }));
        }
        if words.is_empty() {
            return Err(CompileError::Program(ProgramError {
                pc: 0,
                kind: ProgramErrorKind::Empty,
            }));
        }
        let span = pentax_shader::decode(words, stage, &limits.decode)?;
        let mut budget = Budget::new(limits);
        budget.charge_usize(span.work_units())?;
        let word_count = span.source_words().len();
        // Word index -> instruction index (+1), for target validation.
        let mut starts = budget.table(word_count + 1, 0u32, usize::MAX, LimitKind::Bytes)?;
        // Forward branch targets seen so far, as in the source walker.
        let mut marked = budget.table(word_count + 1, false, usize::MAX, LimitKind::Bytes)?;
        let mut terminated_at = None;
        for (index, inst) in span.instructions().enumerate() {
            budget.charge(1)?;
            let start = starts
                .get_mut(inst.word_offset())
                .ok_or(CompileError::Internal("instruction outside span"))?;
            *start = u32::try_from(index + 1).map_err(|_| CompileError::limit(LimitKind::Work))?;
            if is_direct_branch(inst.opcode()) {
                let branch = inst.branch().ok_or(CompileError::Internal(
                    "direct branch without branch metadata",
                ))?;
                let target = branch.target_byte_offset();
                let end = i64::try_from(word_count)
                    .ok()
                    .and_then(|words| words.checked_mul(4))
                    .ok_or(CompileError::limit(LimitKind::Work))?;
                if target < 0 || target > end {
                    return Err(CompileError::Program(ProgramError {
                        pc: inst.byte_offset(),
                        kind: ProgramErrorKind::BranchOutsideProgram { target },
                    }));
                }
                let word = usize::try_from(target / 4)
                    .map_err(|_| CompileError::Internal("negative target"))?;
                if let Some(mark) = marked.get_mut(word) {
                    *mark = true;
                }
            }
            if inst.opcode() == Opcode::SEndpgm {
                let next = inst.word_offset() + inst.raw_words().len();
                if next >= word_count || !marked.get(next).copied().unwrap_or(false) {
                    terminated_at = Some((index, next));
                    break;
                }
            }
        }
        let Some((last, next_word)) = terminated_at else {
            return Err(CompileError::Program(ProgramError {
                pc: u32::try_from(word_count * 4).unwrap_or(u32::MAX),
                kind: ProgramErrorKind::MissingTermination,
            }));
        };
        if next_word != word_count || last + 1 != span.instruction_count() {
            return Err(CompileError::Program(ProgramError {
                pc: u32::try_from(next_word * 4).unwrap_or(u32::MAX),
                kind: ProgramErrorKind::TrailingWords {
                    words: word_count - next_word.min(word_count),
                },
            }));
        }
        let end_pc =
            u32::try_from(word_count * 4).map_err(|_| CompileError::limit(LimitKind::Work))?;
        let mut targets = budget.table(
            span.instruction_count(),
            NO_TARGET,
            usize::MAX,
            LimitKind::Bytes,
        )?;
        for (index, inst) in span.instructions().enumerate() {
            budget.charge(1)?;
            if !is_direct_branch(inst.opcode()) {
                continue;
            }
            let target = inst
                .branch()
                .ok_or(CompileError::Internal(
                    "direct branch without branch metadata",
                ))?
                .target_byte_offset();
            let word = usize::try_from(target / 4)
                .map_err(|_| CompileError::Internal("negative target"))?;
            let resolved = if word == word_count {
                END_TARGET
            } else {
                match starts.get(word).copied() {
                    Some(start) if start != 0 => start - 1,
                    _ => {
                        return Err(CompileError::Program(ProgramError {
                            pc: inst.byte_offset(),
                            kind: ProgramErrorKind::BranchIntoInstruction {
                                target: u32::try_from(target).unwrap_or(u32::MAX),
                            },
                        }));
                    }
                }
            };
            if let Some(slot) = targets.get_mut(index) {
                *slot = resolved;
            }
        }
        Ok(Self {
            span,
            targets,
            end_pc,
            work: budget.work(),
        })
    }

    pub fn stage(&self) -> ShaderStage {
        self.span.stage()
    }

    pub fn instruction_count(&self) -> usize {
        self.span.instruction_count()
    }

    pub fn instruction(&self, index: usize) -> Option<Instruction<'_>> {
        self.span.instruction(index)
    }

    pub fn instructions(&self) -> impl ExactSizeIterator<Item = Instruction<'_>> {
        self.span.instructions()
    }

    /// Byte PC one past the terminating S_ENDPGM.
    pub fn end_pc(&self) -> u32 {
        self.end_pc
    }

    pub fn source_words(&self) -> &[u32] {
        self.span.source_words()
    }

    /// Validation work units (decoder work included).
    pub fn work_units(&self) -> u64 {
        self.work
    }

    /// The validated destination of a direct branch instruction.
    pub fn branch_target(&self, index: usize) -> Option<BranchTarget> {
        match self.targets.get(index).copied()? {
            NO_TARGET => None,
            END_TARGET => Some(BranchTarget::End),
            target => usize::try_from(target).ok().map(BranchTarget::Instruction),
        }
    }

    /// Index of the first instruction whose PC is at least `pc`.
    pub(crate) fn lower_bound(&self, budget: &Budget, pc: u32) -> Result<usize, CompileError> {
        let mut low = 0usize;
        let mut high = self.instruction_count();
        while low < high {
            budget.charge(1)?;
            let mid = low + (high - low) / 2;
            match self.instruction(mid) {
                Some(inst) if inst.byte_offset() < pc => low = mid + 1,
                _ => high = mid,
            }
        }
        Ok(low)
    }
}
