// SPDX-License-Identifier: GPL-2.0-only
// Direct Rust adaptation of KytyPS5-Performance at
// 209dffe9032d72749257ce18680d506b891284de, MemoryOps.cpp:
// SMEM, MUBUF, MTBUF, FLAT and DS decoding with the source opcode tables.
// PentaX modifications: typed errors instead of SetUnsupported/abort and
// checked two-word reads. Where the source keeps decoding after SetUnsupported
// (SMEM/MUBUF/MTBUF), operand aborts still take precedence over that reason.

use crate::decode::{
    Input, record, scalar_destination, scalar_source, unsupported, unsupported_opcode,
};
use crate::model::{MemoryAccess, Record, memory_flag as mf};
use crate::opcode::{self, MemoryOpcode};
use crate::{
    DecodeError, Family, Opcode as O, Operand, OperandKind, SdwaSelect, UnsupportedReason as R,
};

fn bit(value: u32, shift: u32) -> bool {
    (value >> shift) & 1 != 0
}

fn sign_extend(value: u32, bits: u32) -> u32 {
    let sign = 1 << (bits - 1);
    (value ^ sign).wrapping_sub(sign)
}

/// Source `ApplyMemoryInfo`; a missing table entry keeps the source defaults.
fn memory(info: Option<MemoryOpcode>) -> MemoryAccess {
    let mut memory = MemoryAccess::new(1, 32);
    if let Some(info) = info {
        memory = MemoryAccess::new(info.data_dwords, info.data_bits);
        memory.set(mf::DATA_SIGNED, info.data_signed);
        memory.set(mf::TYPED, info.typed);
        memory.set(mf::FORMATTED, info.formatted);
    }
    memory
}

/// Start a two-word memory instruction and return both words. All five
/// families read the second word unconditionally before using the opcode.
/// The placeholder opcode is replaced by `finish` or an error is returned.
fn start(
    input: &mut Input<'_>,
    family: Family,
    at: usize,
) -> Result<(Record, u32, u32), DecodeError> {
    let mut inst = record(at, family, 0, O::SNop);
    input.require(&mut inst, 2)?;
    let w0 = input.word(&inst, 0)?;
    let w1 = input.word(&inst, 1)?;
    Ok((inst, w0, w1))
}

/// Finish a record whose opcode lookup may have failed after the source
/// continued decoding operands: the operand outcome (abort) is checked first.
fn finish(mut inst: Record, id: u16, info: Option<MemoryOpcode>) -> Result<Record, DecodeError> {
    let info = info.ok_or(unsupported_opcode(inst.word_offset, inst.family, id))?;
    inst.opcode_id = id;
    inst.opcode = info.opcode;
    Ok(inst)
}

pub(crate) fn decode_smem(input: &mut Input<'_>, at: usize) -> Result<Record, DecodeError> {
    let (mut inst, w0, w1) = start(input, Family::Smem, at)?;
    let id = ((w0 >> 18) & 0xff) as u16;
    let info = opcode::smem(id);
    let op = info.map(|info| info.opcode);
    let mut access = memory(info);
    access.offset = sign_extend(w1 & 0x1f_ffff, 21);
    access.set(mf::GLC, bit(w0, 16));
    inst.memory = Some(access);
    inst.dst = scalar_destination(((w0 >> 6) & 0x7f) as u16, at)?;
    match op {
        Some(O::SMemrealtime) => {}
        Some(O::SDcacheInv | O::SGl1Inv) => {
            // Cache invalidations have no operands.
            inst.dst = Operand::special(OperandKind::Null);
        }
        _ => {
            // SBASE is an SGPR-pair index; buffer loads read four SGPRs from it.
            inst.src[0] = scalar_source(((w0 & 0x3f) * 2) as u16, at)?;
            inst.src[1] = scalar_source(((w1 >> 25) & 0x7f) as u16, at)?;
            inst.src_count = 2;
        }
    }
    finish(inst, id, info)
}

/// Shared MUBUF/MTBUF fields, set by the source before any operand decoding.
fn buffer_fields(access: &mut MemoryAccess, w0: u32, w1: u32) {
    access.offset = w0 & 0xfff;
    access.set(mf::IDXEN, bit(w0, 13));
    access.set(mf::OFFEN, bit(w0, 12));
    access.set(mf::GLC, bit(w0, 14));
    access.set(mf::DLC, bit(w0, 15));
    access.set(mf::SLC, bit(w1, 22));
    access.set(mf::TFE, bit(w1, 23));
}

/// Shared MUBUF/MTBUF operands: VDATA, VADDR, SRSRC quad and SOFFSET.
fn buffer_operands(inst: &mut Record, w1: u32, at: usize) -> Result<(), DecodeError> {
    inst.dst = Operand::vgpr(((w1 >> 8) & 0xff) as u16);
    inst.src[0] = Operand::vgpr((w1 & 0xff) as u16);
    inst.src[1] = scalar_source((((w1 >> 16) & 0x1f) * 4) as u16, at)?;
    // Selector 255 stays an unresolved literal kind: no literal word is read.
    inst.src[2] = scalar_source(((w1 >> 24) & 0xff) as u16, at)?;
    inst.src_count = 3;
    Ok(())
}

pub(crate) fn decode_mubuf(input: &mut Input<'_>, at: usize) -> Result<Record, DecodeError> {
    let (mut inst, w0, w1) = start(input, Family::Mubuf, at)?;
    let id = (((w0 >> 18) & 0x7f) | (((w0 >> 25) & 1) << 7)) as u16;
    let info = opcode::mubuf(id);
    let op = info.map(|info| info.opcode);
    let mut access = memory(info);
    buffer_fields(&mut access, w0, w1);
    inst.memory = Some(access);
    if matches!(op, Some(O::BufferGl0Inv | O::BufferGl1Inv)) {
        // Cache invalidations ignore every operand field.
        inst.dst = Operand::special(OperandKind::Null);
        return finish(inst, id, info);
    }
    buffer_operands(&mut inst, w1, at)?;
    if op == Some(O::BufferLoadFormatD16X) {
        inst.dst.sdwa_select = SdwaSelect::Word0;
    }
    finish(inst, id, info)
}

pub(crate) fn decode_mtbuf(input: &mut Input<'_>, at: usize) -> Result<Record, DecodeError> {
    let (mut inst, w0, w1) = start(input, Family::Mtbuf, at)?;
    let id = (((w0 >> 16) & 7) | (((w1 >> 21) & 1) << 3)) as u16;
    let info = opcode::mtbuf(id);
    let mut access = memory(info);
    access.data_format = ((w0 >> 19) & 0xf) as u8;
    access.number_format = ((w0 >> 23) & 7) as u8;
    buffer_fields(&mut access, w0, w1);
    inst.memory = Some(access);
    buffer_operands(&mut inst, w1, at)?;
    finish(inst, id, info)
}

pub(crate) fn decode_flat(input: &mut Input<'_>, at: usize) -> Result<Record, DecodeError> {
    let (mut inst, w0, w1) = start(input, Family::Flat, at)?;
    let id = ((w0 >> 18) & 0x7f) as u16;
    let raw_offset = w0 & 0xfff;
    let segment = (w0 >> 14) & 3;
    let saddr = ((w1 >> 16) & 0x7f) as u16;
    let addr = (w1 & 0xff) as u16;
    if bit(w0, 13) || segment == 3 {
        return Err(unsupported(at, Family::Flat, id, R::FlatModifiersOrSegment));
    }
    let info = opcode::flat(id).ok_or(unsupported_opcode(at, Family::Flat, id))?;
    let mut access = memory(Some(info));
    access.offset = if segment == 0 {
        raw_offset & 0x7ff
    } else {
        sign_extend(raw_offset, 12)
    };
    access.set(mf::DLC, bit(w0, 12));
    access.set(mf::GLC, bit(w0, 16));
    access.set(mf::SLC, bit(w0, 17));
    access.segment = segment as u8;
    inst.memory = Some(access);
    let store = matches!(
        info.opcode,
        O::FlatStoreByte
            | O::FlatStoreShort
            | O::FlatStoreDword
            | O::FlatStoreDwordx2
            | O::FlatStoreDwordx3
            | O::FlatStoreDwordx4
    );
    let data = (w1 >> 8) & 0xff;
    let vdst = (w1 >> 24) & 0xff;
    inst.dst = Operand::vgpr((if store { data } else { vdst }) as u16);
    if info.opcode == O::FlatLoadShortD16 {
        // D16 loads reuse partial destinations to preserve the untouched high half.
        inst.dst.sdwa_select = SdwaSelect::Word0;
    }
    inst.src[0] = Operand::vgpr(addr);
    inst.src[1] = if segment == 0 || saddr == 0x7d || saddr == 0x7f {
        // The source's unchecked pair continuation; VGPR 255 yields 256.
        Operand::vgpr(addr + 1)
    } else {
        scalar_source(saddr, at)?
    };
    inst.src_count = 2;
    finish(inst, id, Some(info))
}

fn is_ds_write(op: O) -> bool {
    matches!(
        op,
        O::DsWriteB8
            | O::DsWriteB16
            | O::DsWriteB8D16Hi
            | O::DsWriteB16D16Hi
            | O::DsWrite2B32
            | O::DsWrite2st64B32
            | O::DsWrite2B64
            | O::DsWrite2st64B64
            | O::DsWriteB32
            | O::DsWriteB64
            | O::DsWriteB96
            | O::DsWriteB128
    )
}

fn is_ds_atomic(op: O) -> bool {
    matches!(
        op,
        O::DsAddU32
            | O::DsAddU64
            | O::DsAddRtnU32
            | O::DsSubU32
            | O::DsSubRtnU32
            | O::DsIncU32
            | O::DsIncRtnU32
            | O::DsDecU32
            | O::DsDecRtnU32
            | O::DsMinI32
            | O::DsMinRtnI32
            | O::DsMaxI32
            | O::DsMaxRtnI32
            | O::DsMinU32
            | O::DsMinRtnU32
            | O::DsMaxU32
            | O::DsMaxRtnU32
            | O::DsAndB32
            | O::DsAndRtnB32
            | O::DsOrB32
            | O::DsOrB64
            | O::DsOrRtnB32
            | O::DsXorB32
            | O::DsXorRtnB32
            | O::DsMskorB32
            | O::DsWrxchgRtnB32
    )
}

fn ds_source_count(op: O) -> usize {
    match op {
        O::DsWrite2B32
        | O::DsWrite2st64B32
        | O::DsWrite2B64
        | O::DsWrite2st64B64
        | O::DsMskorB32 => 3,
        O::DsMinF32 | O::DsMaxF32 | O::DsPermuteB32 | O::DsBpermuteB32 => 2,
        O::DsReadAddtidB32 | O::DsConsume | O::DsAppend => 0,
        O::DsOrderedCount => 1,
        _ if is_ds_write(op) || is_ds_atomic(op) => 2,
        _ => 1,
    }
}

pub(crate) fn decode_ds(input: &mut Input<'_>, at: usize) -> Result<Record, DecodeError> {
    let family = Family::Ds;
    let (mut inst, w0, w1) = start(input, family, at)?;
    let id = ((w0 >> 18) & 0xff) as u16;
    let offset0 = w0 & 0xff;
    let offset1 = (w0 >> 8) & 0xff;
    let data1 = (w1 >> 16) & 0xff;
    let data0 = (w1 >> 8) & 0xff;
    let gds = bit(w0, 17);
    // The source keeps decoding after SetUnsupported, but every later DS check
    // compares against the replaced opcode and no DS operand can abort, so the
    // first reason is the source's final reason.
    let info = opcode::ds(id).ok_or(unsupported_opcode(at, family, id))?;
    let op = info.opcode;
    let fail = |reason| Err(unsupported(at, family, id, reason));
    let mut access = memory(Some(info));
    access.offset = offset0 | (offset1 << 8);
    access.set(mf::GDS, gds);
    if op == O::DsOrderedCount {
        // Keep OFFSET1's wave type as well as its add/swap selector for GDS addressing.
        access.offset = offset0 & 0xfc;
        access.secondary_offset = offset1;
        if !gds || ((offset1 >> 4) & 3) > 1 || ((offset1 >> 6) & 3) != 0 {
            return fail(R::DsOrderedCountVariant);
        }
    }
    if op == O::DsSwizzleB32 && access.offset >= 0xe000 {
        return fail(R::DsSwizzleFft);
    }
    if gds
        && matches!(
            op,
            O::DsSwizzleB32
                | O::DsPermuteB32
                | O::DsBpermuteB32
                | O::DsWriteAddtidB32
                | O::DsReadAddtidB32
        )
    {
        return fail(R::DsLaneOperationGds);
    }
    if op == O::DsWriteAddtidB32 && data1 != 0 {
        return fail(R::DsWriteAddtidData1);
    }
    if op == O::DsReadAddtidB32 && (data0 != 0 || data1 != 0) {
        return fail(R::DsReadAddtidData);
    }
    let scale = match op {
        O::DsWrite2B32 | O::DsRead2B32 => 4,
        O::DsWrite2st64B32 | O::DsRead2st64B32 => 256,
        O::DsWrite2B64 | O::DsRead2B64 => 8,
        O::DsWrite2st64B64 | O::DsRead2st64B64 => 512,
        _ => 0,
    };
    if scale != 0 {
        access.offset = offset0 * scale;
        access.secondary_offset = offset1 * scale;
    }
    inst.memory = Some(access);
    inst.dst = Operand::vgpr(((w1 >> 24) & 0xff) as u16);
    match op {
        // Native D16 reads update only the selected destination half.
        O::DsReadU16D16 => inst.dst.sdwa_select = SdwaSelect::Word0,
        O::DsReadU16D16Hi => inst.dst.sdwa_select = SdwaSelect::Word1,
        _ => {}
    }
    inst.src[0] = Operand::vgpr((w1 & 0xff) as u16);
    inst.src[1] = Operand::vgpr(data0 as u16);
    match op {
        O::DsWriteB8D16Hi => inst.src[1].sdwa_select = SdwaSelect::Byte2,
        O::DsWriteB16D16Hi => inst.src[1].sdwa_select = SdwaSelect::Word1,
        _ => {}
    }
    inst.src[2] = Operand::vgpr(data1 as u16);
    inst.src_count = ds_source_count(op);
    finish(inst, id, Some(info))
}
