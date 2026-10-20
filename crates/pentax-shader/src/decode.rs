// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de:
// ShaderDecoder.cpp, ScalarAluOps.cpp, ExportOps.cpp, VectorAluOps.cpp (VINTRP).
// Vector, memory and image families are dispatched to vector.rs, memory.rs and
// image.rs. PentaX modifications: owned input, fallible capacity/work bounds,
// typed errors for source aborts and SetUnsupported cases, signed branch
// metadata, linear-span API; no aborts or unchecked pointer reads.

use std::fmt;
use std::mem::size_of;

use crate::model::Record;
use crate::opcode::lookup;
use crate::{Branch, DecodedSpan, Export, Family, Opcode, Operand, OperandKind, ShaderStage};

/// Independent limits for owned source, instruction records and bounded work.
/// Reported vector capacities are charged, not hypothetical exact heap usage.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_source_words: usize,
    pub max_instructions: usize,
    pub max_record_bytes: usize,
    /// One unit per copied input word, consumed instruction word, and existing
    /// record potentially moved by a record-vector growth. Opcode lookup and
    /// operand decoding have fixed, allocation-free cost.
    pub max_work: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_source_words: 262_144,
            max_instructions: 131_072,
            max_record_bytes: 32 * 1024 * 1024,
            max_work: 1_048_576,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LimitKind {
    SourceWords,
    Instructions,
    RecordBytes,
    Work,
    /// Byte offsets must fit the pinned frontend's u32 PC representation.
    PcRange,
}

/// A pinned-source `SetUnsupported` case other than an unmapped opcode. The
/// message is the source's exact static reason string.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnsupportedReason {
    Vop1SdwaFloat64,
    Vop1SdwaSelector,
    Vop1SdwaOutputModifiers,
    Vop1SdwaDestinationSelector,
    Vop1SdwaSourceSelector,
    Vop1DppFloat64,
    Vop1DppIntegerSourceModifiers,
    Vop2SdwaSelector,
    Vop2SdwaOutputModifiers,
    Vop2SdwaSourceModifiers,
    Vop2SdwaOpcode,
    Vop2SdwaDestinationSelector,
    Vop2SdwaSourceSelector,
    Vop2DppIntegerSourceModifiers,
    VopcSdwaSelector,
    VopcSdwaOpcode,
    VopcDppOpcode,
    Vop3Modifiers,
    Vop3PermlaneOpSel,
    Vop3pIntegerClamp,
    FlatModifiersOrSegment,
    DsOrderedCountVariant,
    DsSwizzleFft,
    DsLaneOperationGds,
    DsWriteAddtidData1,
    DsReadAddtidData,
    BvhDmask,
    BvhR128,
    BvhTfeLwe,
    BvhNsaAddresses,
    MimgGatherDmask,
    MimgAtomicDmask,
    MimgAtomic64,
    MimgD16,
}

impl UnsupportedReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::Vop1SdwaFloat64 => "FP64 instructions do not support SDWA",
            Self::Vop1SdwaSelector => "VOP1 SDWA selector is invalid",
            Self::Vop1SdwaOutputModifiers => "VOP1 SDWA output modifiers are not supported",
            Self::Vop1SdwaDestinationSelector => "VOP1 SDWA destination selector is not supported",
            Self::Vop1SdwaSourceSelector => "VOP1 SDWA source selector is not supported",
            Self::Vop1DppFloat64 => "FP64 instructions do not support DPP",
            Self::Vop1DppIntegerSourceModifiers => {
                "VOP1 DPP integer source modifiers are not supported"
            }
            Self::Vop2SdwaSelector => "VOP2 SDWA selector is invalid",
            Self::Vop2SdwaOutputModifiers => "VOP2 SDWA output modifiers are not supported",
            Self::Vop2SdwaSourceModifiers => "VOP2 SDWA source modifiers are not supported",
            Self::Vop2SdwaOpcode => "VOP2 SDWA modifier is not supported for opcode",
            Self::Vop2SdwaDestinationSelector => "VOP2 SDWA destination selector is not supported",
            Self::Vop2SdwaSourceSelector => "VOP2 SDWA source selector is not supported",
            Self::Vop2DppIntegerSourceModifiers => {
                "VOP2 DPP integer source modifiers are not supported"
            }
            Self::VopcSdwaSelector => "VOPC SDWA selector is invalid",
            Self::VopcSdwaOpcode => "VOPC SDWA modifier is not supported for opcode",
            Self::VopcDppOpcode => "VOPC DPP modifier is not supported for opcode",
            Self::Vop3Modifiers => "VOP3 source modifiers are not implemented",
            Self::Vop3PermlaneOpSel => "VOP3 perm-lane op_sel bits are not implemented",
            Self::Vop3pIntegerClamp => "VOP3P integer clamp is not implemented",
            Self::FlatModifiersOrSegment => "FLAT modifiers or segment are not implemented",
            Self::DsOrderedCountVariant => "DS ordered count variant is not implemented",
            Self::DsSwizzleFft => "DS swizzle FFT mode is not implemented",
            Self::DsLaneOperationGds => "DS lane operation is available only for LDS",
            Self::DsWriteAddtidData1 => "DS write addtid data1 operand is not implemented",
            Self::DsReadAddtidData => "DS read addtid data operands are not implemented",
            Self::BvhDmask => "BVH intersect requires DMASK=0xf",
            Self::BvhR128 => "BVH intersect requires a 128-bit T# (R128=1)",
            Self::BvhTfeLwe => "BVH intersect does not support TFE or LWE",
            Self::BvhNsaAddresses => "BVH intersect NSA form has too few addresses",
            Self::MimgGatherDmask => "MIMG image gather requires exactly one dmask bit",
            Self::MimgAtomicDmask => "MIMG image atomic has invalid DMASK",
            Self::MimgAtomic64 => "MIMG 64-bit image atomic opcode is not implemented",
            Self::MimgD16 => "MIMG opcode does not support D16 data",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    LimitExceeded {
        at_word: usize,
        kind: LimitKind,
    },
    Allocation,
    UnknownFamily {
        at_word: usize,
    },
    /// Retained for API compatibility. Every family recognized by the pinned
    /// decoder has a ported decoder at this revision, so this is not produced.
    UnsupportedFamily {
        at_word: usize,
        family: Family,
    },
    /// An encoding that no pinned opcode map accepts ("opcode is not implemented").
    UnsupportedOpcode {
        at_word: usize,
        family: Family,
        opcode_id: u16,
    },
    /// A selector for which the pinned operand decoder aborts.
    UnsupportedOperand {
        at_word: usize,
        selector: u16,
        destination: bool,
    },
    /// Another pinned `SetUnsupported` rule (field-dependent restrictions).
    Unsupported {
        at_word: usize,
        family: Family,
        opcode_id: u16,
        reason: UnsupportedReason,
    },
    /// `required_words` is the total length known so far, including the header
    /// and every body/modifier/NSA/literal word that the source would read.
    Truncated {
        at_word: usize,
        family: Family,
        required_words: usize,
        available_words: usize,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LimitExceeded { at_word, kind } => {
                write!(f, "shader decoder {kind:?} limit at word {at_word}")
            }
            Self::Allocation => f.write_str("shader decoder allocation failed"),
            Self::UnknownFamily { at_word } => {
                write!(f, "unknown shader encoding family at word {at_word}")
            }
            Self::UnsupportedFamily { at_word, family } => write!(
                f,
                "shader family {family:?} is not ported at word {at_word}"
            ),
            Self::UnsupportedOpcode {
                at_word,
                family,
                opcode_id,
            } => write!(
                f,
                "unsupported {family:?} opcode {opcode_id} at word {at_word}"
            ),
            Self::UnsupportedOperand {
                at_word,
                selector,
                destination,
            } => write!(
                f,
                "unsupported shader operand {selector} (destination={destination}) at word {at_word}"
            ),
            Self::Unsupported {
                at_word,
                family,
                opcode_id,
                reason,
            } => write!(
                f,
                "unsupported {family:?} opcode {opcode_id} at word {at_word}: {}",
                reason.message()
            ),
            Self::Truncated {
                at_word,
                family,
                required_words,
                available_words,
            } => write!(
                f,
                "truncated {family:?} instruction at word {at_word}: needs {required_words} words, has {available_words}"
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Exact family classifier from the pinned fork. Classification alone never
/// guesses a complete word count; that is determined by the family decoder.
pub fn instruction_family(word: u32) -> Family {
    if word & 0x8000_0000 == 0 {
        return match (word >> 25) & 0x3f {
            0x3e => Family::Vopc,
            0x3f => Family::Vop1,
            _ => Family::Vop2,
        };
    }
    if word & 0xc000_0000 == 0x8000_0000 {
        return match (word >> 23) & 0x7f {
            0x7d => Family::Sop1,
            0x7e => Family::Sopc,
            0x7f => Family::Sopp,
            opcode if opcode >= 0x60 => Family::Sopk,
            _ => Family::Sop2,
        };
    }
    match word >> 26 {
        0x32 => Family::Vintrp,
        0x33 => Family::Vop3p,
        0x35 => Family::Vop3,
        0x36 => Family::Ds,
        0x37 => Family::Flat,
        0x38 => Family::Mubuf,
        0x3a => Family::Mtbuf,
        0x3c => Family::Mimg,
        0x3d => Family::Smem,
        0x3e => Family::Exp,
        _ => Family::Unknown,
    }
}

fn limit(at_word: usize, kind: LimitKind) -> DecodeError {
    DecodeError::LimitExceeded { at_word, kind }
}

/// The owned source words plus the work counter for one decode call.
pub(crate) struct Input<'a> {
    words: &'a [u32],
    limits: &'a Limits,
    work: usize,
}

impl Input<'_> {
    fn charge(&mut self, amount: usize, at: usize) -> Result<(), DecodeError> {
        let next = self
            .work
            .checked_add(amount)
            .ok_or_else(|| limit(at, LimitKind::Work))?;
        if next > self.limits.max_work {
            return Err(limit(at, LimitKind::Work));
        }
        self.work = next;
        Ok(())
    }

    fn truncated(&self, inst: &Record, required_words: usize) -> DecodeError {
        DecodeError::Truncated {
            at_word: inst.word_offset,
            family: inst.family,
            required_words,
            available_words: self.words.len().saturating_sub(inst.word_offset),
        }
    }

    /// Prove that `total` words starting at the instruction exist, then charge
    /// the newly consumed ones. No word beyond the header is read before this.
    pub(crate) fn require(&mut self, inst: &mut Record, total: usize) -> Result<(), DecodeError> {
        if total <= inst.word_count {
            return Ok(());
        }
        let available = self.words.len().saturating_sub(inst.word_offset);
        if total > available {
            return Err(self.truncated(inst, total));
        }
        self.charge(total - inst.word_count, inst.word_offset)?;
        inst.word_count = total;
        Ok(())
    }

    /// Read an already required word of the current instruction.
    pub(crate) fn word(&self, inst: &Record, index: usize) -> Result<u32, DecodeError> {
        if index >= inst.word_count {
            return Err(self.truncated(inst, index + 1));
        }
        self.words
            .get(inst.word_offset + index)
            .copied()
            .ok_or_else(|| self.truncated(inst, index + 1))
    }

    /// The source `ReadLiteralOperands`: one shared word after the current
    /// length if any of the four source slots is a literal.
    pub(crate) fn literal(&mut self, inst: &mut Record) -> Result<(), DecodeError> {
        if inst
            .src
            .iter()
            .any(|src| src.kind == OperandKind::LiteralConstant)
        {
            let index = inst.word_count;
            self.require(inst, index + 1)?;
            let value = self.word(inst, index)?;
            for src in &mut inst.src {
                if src.kind == OperandKind::LiteralConstant {
                    src.value = value;
                    src.signed_value = value as i32;
                }
            }
        }
        Ok(())
    }
}

/// Decode a caller-delimited linear instruction span. This is not the fork's
/// DecodeProgram termination/CFG walker: every supplied word must belong to a
/// supported instruction, including words after ENDPGM. Empty input is valid.
///
/// Literal, modifier, body and NSA words are consumed as data, never
/// reclassified as instructions. Stage is preserved metadata only. On any
/// failure, all temporary owned input and records are dropped and no partial
/// decoded span is returned.
pub fn decode(
    words: &[u32],
    stage: ShaderStage,
    limits: &Limits,
) -> Result<DecodedSpan, DecodeError> {
    if words.len() > limits.max_source_words {
        return Err(limit(0, LimitKind::SourceWords));
    }
    if words.len() > (u32::MAX as usize) / 4 {
        return Err(limit(0, LimitKind::PcRange));
    }
    let mut work = 0;
    if words.len() > limits.max_work {
        return Err(limit(0, LimitKind::Work));
    }
    work += words.len();
    let mut source = Vec::new();
    source
        .try_reserve_exact(words.len())
        .map_err(|_| DecodeError::Allocation)?;
    if source.capacity() > limits.max_source_words {
        return Err(limit(0, LimitKind::SourceWords));
    }
    source.extend_from_slice(words);
    let mut input = Input {
        words: &source,
        limits,
        work,
    };
    let mut records = Vec::<Record>::new();
    let mut at = 0;
    while at < input.words.len() {
        if records.len() >= limits.max_instructions {
            return Err(limit(at, LimitKind::Instructions));
        }
        input.charge(1, at)?;
        let record = decode_instruction(&mut input, at)?;
        let next_count = records
            .len()
            .checked_add(1)
            .ok_or_else(|| limit(at, LimitKind::RecordBytes))?;
        let required = next_count
            .checked_mul(size_of::<Record>())
            .ok_or_else(|| limit(at, LimitKind::RecordBytes))?;
        if required > limits.max_record_bytes {
            return Err(limit(at, LimitKind::RecordBytes));
        }
        if records.len() == records.capacity() {
            // Geometric growth avoids quadratic record copying. Clamp to both
            // configured record bounds and charge any existing records that a
            // reallocation could move before requesting that allocation.
            let max_records = limits
                .max_instructions
                .min(limits.max_record_bytes / size_of::<Record>());
            let next_capacity = records.capacity().saturating_mul(2).max(4).min(max_records);
            input.charge(records.len(), at)?;
            records
                .try_reserve_exact(next_capacity - records.len())
                .map_err(|_| DecodeError::Allocation)?;
            let capacity_bytes = records
                .capacity()
                .checked_mul(size_of::<Record>())
                .ok_or_else(|| limit(at, LimitKind::RecordBytes))?;
            if capacity_bytes > limits.max_record_bytes {
                return Err(limit(at, LimitKind::RecordBytes));
            }
        }
        at += record.word_count; // 1 to 5, already proven present by decoding.
        records.push(record);
    }
    let work = input.work;
    Ok(DecodedSpan {
        source,
        records,
        stage,
        work,
    })
}

pub(crate) fn unsupported_opcode(at: usize, family: Family, id: u16) -> DecodeError {
    DecodeError::UnsupportedOpcode {
        at_word: at,
        family,
        opcode_id: id,
    }
}

pub(crate) fn unsupported(
    at: usize,
    family: Family,
    id: u16,
    reason: UnsupportedReason,
) -> DecodeError {
    DecodeError::Unsupported {
        at_word: at,
        family,
        opcode_id: id,
        reason,
    }
}

fn opcode(family: Family, id: u16, at: usize) -> Result<Opcode, DecodeError> {
    lookup(family, id).ok_or(unsupported_opcode(at, family, id))
}

pub(crate) fn record(at: usize, family: Family, id: u16, opcode: Opcode) -> Record {
    Record {
        word_offset: at,
        word_count: 1,
        family,
        opcode_id: id,
        opcode,
        dst: Operand::UNKNOWN,
        dst2: Operand::UNKNOWN,
        src: [Operand::UNKNOWN; 4],
        src_count: 0,
        branch: None,
        export: None,
        memory: None,
        image: None,
    }
}

pub(crate) fn scalar_source(selector: u16, at: usize) -> Result<Operand, DecodeError> {
    let special = |kind| Ok(Operand::special(kind));
    match selector {
        0..=105 => Ok(Operand {
            register: selector,
            ..Operand::special(OperandKind::Sgpr)
        }),
        128..=192 => Ok(Operand::integer(i32::from(selector) - 128)),
        193..=208 => Ok(Operand::integer(192 - i32::from(selector))),
        240..=247 => {
            // Exact IEEE-754 patterns of the pinned source's +/-0.5, 1, 2, 4.
            const BITS: [u32; 8] = [
                0x3f00_0000,
                0xbf00_0000,
                0x3f80_0000,
                0xbf80_0000,
                0x4000_0000,
                0xc000_0000,
                0x4080_0000,
                0xc080_0000,
            ];
            Ok(Operand {
                value: BITS[usize::from(selector - 240)],
                ..Operand::special(OperandKind::FloatInlineConstant)
            })
        }
        256..=511 => Ok(Operand::vgpr(selector - 256)),
        106 => special(OperandKind::VccLo),
        107 => special(OperandKind::VccHi),
        124 => special(OperandKind::M0),
        125 => special(OperandKind::Null),
        126 => special(OperandKind::ExecLo),
        127 => special(OperandKind::ExecHi),
        239 => special(OperandKind::PopsExitingWaveId),
        248 => Ok(Operand {
            value: 0x3e22_f983,
            ..Operand::special(OperandKind::FloatInlineConstant)
        }),
        251 => special(OperandKind::VccZ),
        252 => special(OperandKind::ExecZ),
        253 => special(OperandKind::Scc),
        255 => special(OperandKind::LiteralConstant),
        _ => Err(DecodeError::UnsupportedOperand {
            at_word: at,
            selector,
            destination: false,
        }),
    }
}

pub(crate) fn scalar_destination(selector: u16, at: usize) -> Result<Operand, DecodeError> {
    match selector {
        0..=105 => Ok(Operand {
            register: selector,
            ..Operand::special(OperandKind::Sgpr)
        }),
        106 | 107 | 124..=127 => scalar_source(selector, at),
        _ => Err(DecodeError::UnsupportedOperand {
            at_word: at,
            selector,
            destination: true,
        }),
    }
}

fn branch(at: usize, immediate: u16) -> Branch {
    let displacement_words = immediate as i16;
    Branch {
        displacement_words,
        target_byte_offset: (at as i64) * 4 + 4 + i64::from(displacement_words) * 4,
    }
}

fn decode_instruction(input: &mut Input<'_>, at: usize) -> Result<Record, DecodeError> {
    let word = input
        .words
        .get(at)
        .copied()
        .ok_or(DecodeError::UnknownFamily { at_word: at })?; // caller proves presence
    let family = instruction_family(word);
    let low = (word & 0xff) as u16;
    let high = ((word >> 8) & 0xff) as u16;
    let sdst = ((word >> 16) & 0x7f) as u16;
    match family {
        Family::Sop1 => {
            let id = high;
            let op = if id == 0x21 && sdst == 125 {
                Opcode::SSetpcB64
            } else {
                opcode(family, id, at)?
            };
            let mut inst = record(at, family, id, op);
            if op == Opcode::SGetpcB64 {
                inst.dst = scalar_destination(sdst, at)?;
                return Ok(inst);
            }
            // Ordinary SOP1 validates its source before its destination in the
            // pinned decoder. Keep that diagnostic order when both are invalid.
            inst.src[0] = scalar_source(low, at)?;
            inst.dst = if op == Opcode::SSetpcB64 {
                Operand::special(OperandKind::Null)
            } else {
                scalar_destination(sdst, at)?
            };
            inst.src_count = 1;
            input.literal(&mut inst)?;
            Ok(inst)
        }
        Family::Sop2 | Family::Sopc => {
            let id = if family == Family::Sop2 {
                ((word >> 23) & 0x7f) as u16
            } else {
                sdst
            };
            let mut inst = record(at, family, id, opcode(family, id, at)?);
            inst.dst = if family == Family::Sopc {
                Operand::special(OperandKind::Scc)
            } else {
                scalar_destination(sdst, at)?
            };
            inst.src[0] = scalar_source(low, at)?;
            inst.src[1] = scalar_source(high, at)?;
            inst.src_count = 2;
            input.literal(&mut inst)?;
            Ok(inst)
        }
        Family::Sopk => decode_sopk(word, at),
        Family::Sopp => decode_sopp(word, at),
        Family::Exp => {
            let id = ((word >> 4) & 0x3f) as u16;
            let mut inst = record(at, family, id, Opcode::Exp);
            input.require(&mut inst, 2)?;
            let body = input.word(&inst, 1)?;
            let exp = Export {
                target: id as u8,
                enabled: (word & 0xf) as u8,
                compressed: word & (1 << 10) != 0,
                done: word & (1 << 11) != 0,
                valid_mask: word & (1 << 12) != 0,
            };
            for (index, src) in inst.src.iter_mut().enumerate() {
                *src = Operand::vgpr(((body >> (index * 8)) & 0xff) as u16);
            }
            inst.src_count = if exp.enabled == 0 {
                0
            } else if exp.target == 0x14 && exp.done && exp.enabled == 1 {
                1
            } else if exp.compressed {
                2
            } else {
                4
            };
            inst.export = Some(exp);
            Ok(inst)
        }
        Family::Vintrp => {
            let id = ((word >> 16) & 3) as u16;
            let op = opcode(family, id, at)?;
            let mut inst = record(at, family, id, op);
            inst.dst = Operand::vgpr(((word >> 18) & 0xff) as u16);
            inst.src[0] = if op == Opcode::VInterpMovF32 {
                Operand::integer(i32::from(low & 3))
            } else {
                Operand::vgpr(low)
            };
            inst.src[1] = Operand::integer(((word >> 10) & 0x3f) as i32);
            inst.src[2] = Operand::integer(((word >> 8) & 3) as i32);
            inst.src_count = 3;
            Ok(inst)
        }
        Family::Vop1 => crate::vector::decode_vop1(input, word, at),
        Family::Vop2 => crate::vector::decode_vop2(input, word, at),
        Family::Vopc => crate::vector::decode_vopc(input, word, at),
        Family::Vop3 => crate::vector::decode_vop3(input, word, at),
        Family::Vop3p => crate::vector::decode_vop3p(input, word, at),
        Family::Smem => crate::memory::decode_smem(input, at),
        Family::Mubuf => crate::memory::decode_mubuf(input, at),
        Family::Mtbuf => crate::memory::decode_mtbuf(input, at),
        Family::Flat => crate::memory::decode_flat(input, at),
        Family::Ds => crate::memory::decode_ds(input, at),
        Family::Mimg => crate::image::decode_mimg(input, word, at),
        Family::Unknown => Err(DecodeError::UnknownFamily { at_word: at }),
    }
}

fn decode_sopk(word: u32, at: usize) -> Result<Record, DecodeError> {
    let family = Family::Sopk;
    let id = ((word >> 23) & 0x1f) as u16;
    let sdst = ((word >> 16) & 0x7f) as u16;
    let raw = (word & 0xffff) as u16;
    let imm = if (0x09..=0x0e).contains(&id) {
        i32::from(raw)
    } else {
        i32::from(raw as i16)
    };
    let op = opcode(family, id, at)?;
    let mut inst = record(at, family, id, op);
    inst.src[0] = Operand::integer(imm);
    inst.src_count = 1;
    match op {
        Opcode::SMovkI32 | Opcode::SCmovkI32 => inst.dst = scalar_destination(sdst, at)?,
        Opcode::SSubvectorLoopBegin | Opcode::SSubvectorLoopEnd => {
            inst.dst = scalar_destination(sdst, at)?;
            inst.branch = Some(branch(at, raw));
        }
        Opcode::SWaitcnt => {
            inst.dst = Operand::special(OperandKind::Null);
            inst.src[0] = Operand::integer(i32::from(raw));
        }
        Opcode::SWaitcntVscnt => {
            inst.dst = Operand::special(OperandKind::Null);
            inst.src[1] = Operand::integer(i32::from(raw));
            inst.src[0] = scalar_source(sdst, at)?;
            inst.src_count = 2;
        }
        Opcode::SSetregB32 => {
            inst.dst = Operand::special(OperandKind::Null);
            // This is an embedded immediate, not an additional literal word.
            inst.src[1] = Operand {
                value: u32::from(raw),
                signed_value: imm,
                ..Operand::special(OperandKind::LiteralConstant)
            };
            inst.src[0] = scalar_source(sdst, at)?;
            inst.src_count = 2;
        }
        _ => {
            inst.src[1] = inst.src[0];
            inst.src[0] = scalar_source(sdst, at)?;
            inst.src_count = 2;
            inst.dst = if matches!(op, Opcode::SAddI32 | Opcode::SMulkI32) {
                scalar_destination(sdst, at)?
            } else {
                Operand::special(OperandKind::Scc)
            };
        }
    }
    Ok(inst)
}

fn decode_sopp(word: u32, at: usize) -> Result<Record, DecodeError> {
    let family = Family::Sopp;
    let id = ((word >> 16) & 0x7f) as u16;
    let op = opcode(family, id, at)?;
    let raw = (word & 0xffff) as u16;
    let mut inst = record(at, family, id, op);
    inst.src[0] = Operand {
        value: u32::from(if op == Opcode::STrap { raw & 0xff } else { raw }),
        signed_value: if op == Opcode::STrap {
            i32::from(raw & 0xff)
        } else {
            i32::from(raw as i16)
        },
        ..Operand::special(OperandKind::LiteralConstant)
    };
    inst.src_count = usize::from(matches!(
        op,
        Opcode::SNop
            | Opcode::SWaitcnt
            | Opcode::SWaitcntDepctr
            | Opcode::SSleep
            | Opcode::SSetprio
            | Opcode::SSendmsg
            | Opcode::STrap
            | Opcode::STtracedata
            | Opcode::SInstPrefetch
            | Opcode::SClause
    ));
    if matches!(
        op,
        Opcode::SBranch
            | Opcode::SCbranchScc0
            | Opcode::SCbranchScc1
            | Opcode::SCbranchVccz
            | Opcode::SCbranchVccnz
            | Opcode::SCbranchExecz
            | Opcode::SCbranchExecnz
            | Opcode::SCbranchCdbgsys
            | Opcode::SCbranchCdbgsysOrUser
    ) {
        inst.branch = Some(branch(at, raw));
    }
    Ok(inst)
}
