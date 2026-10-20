// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5-Performance 209dffe, ShaderDecoder.h, ImageOps.cpp and shader.h.
// PentaX modifications: private owned records, borrowing views, signed targets,
// compact typed operand/memory/image metadata with fixed-size component arrays.

use crate::Opcode;

/// The pinned decoder's encoding families. Classification is not decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Family {
    Unknown,
    Sop1,
    Sop2,
    Sopk,
    Sopc,
    Sopp,
    Vop1,
    Vop2,
    Vop3,
    Vop3p,
    Vopc,
    Vintrp,
    Smem,
    Mubuf,
    Mtbuf,
    Flat,
    Ds,
    Mimg,
    Exp,
}

/// Caller-supplied stage metadata, using the pinned source's stage vocabulary.
/// This decoder does not infer or validate a stage from instructions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShaderStage {
    Unknown,
    Vertex,
    Pixel,
    Fetch,
    Compute,
    Mesh,
    Local,
    TessellationControl,
    TessellationEvaluation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperandKind {
    Unknown,
    LiteralConstant,
    IntegerInlineConstant,
    FloatInlineConstant,
    Sgpr,
    VccLo,
    VccHi,
    VccZ,
    ExecLo,
    ExecHi,
    ExecZ,
    Scc,
    M0,
    PopsExitingWaveId,
    Null,
    Vgpr,
}

/// The source operand's `sdwa_sel` value. Besides SDWA words, the pinned decoder
/// reuses it for native 16-bit/D16 partial destinations and sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdwaSelect {
    Byte0,
    Byte1,
    Byte2,
    Byte3,
    Word0,
    Word1,
    /// The source default (selector 6): the whole dword.
    Dword,
}

impl SdwaSelect {
    pub(crate) const fn from_bits(bits: u32) -> Option<Self> {
        Some(match bits {
            0 => Self::Byte0,
            1 => Self::Byte1,
            2 => Self::Byte2,
            3 => Self::Byte3,
            4 => Self::Word0,
            5 => Self::Word1,
            6 => Self::Dword,
            _ => return None,
        })
    }

    pub fn bits(self) -> u8 {
        self as u8
    }
}

/// The source operand's `sdwa_dst_unused` value. Encoding 3 is always rejected
/// by the pinned SDWA validators before it can be retained.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SdwaUnused {
    Pad,
    SignExtend,
    /// The source default (2), also used for native partial destinations.
    Preserve,
}

impl SdwaUnused {
    pub(crate) const fn from_bits(bits: u32) -> Option<Self> {
        Some(match bits {
            0 => Self::Pad,
            1 => Self::SignExtend,
            2 => Self::Preserve,
            _ => return None,
        })
    }

    pub fn bits(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DppKind {
    /// Source selector 250: 9-bit control, row/bank masks and bound control.
    Dpp16,
    /// Source selectors 233/234: eight 3-bit lane selects in `control`.
    Dpp8,
}

/// DPP fields retained from the modifier word, without lane-semantics claims.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dpp {
    kind: DppKind,
    control: u32,
    fetch_inactive: bool,
    bound_control: bool,
    row_mask: u8,
    bank_mask: u8,
}

impl Dpp {
    pub fn kind(self) -> DppKind {
        self.kind
    }
    /// DPP16 control (9 bits) or the DPP8 lane-select field (24 bits).
    pub fn control(self) -> u32 {
        self.control
    }
    pub fn fetch_inactive(self) -> bool {
        self.fetch_inactive
    }
    /// Always false for DPP8, as in the pinned decoder.
    pub fn bound_control(self) -> bool {
        self.bound_control
    }
    /// The source default 0xf is retained for DPP8.
    pub fn row_mask(self) -> u8 {
        self.row_mask
    }
    /// The source default 0xf is retained for DPP8.
    pub fn bank_mask(self) -> u8 {
        self.bank_mask
    }
}

pub(crate) mod flag {
    pub const OP_SEL: u16 = 1 << 0;
    pub const OP_SEL_HI: u16 = 1 << 1;
    pub const NEGATE: u16 = 1 << 2;
    pub const NEGATE_HI: u16 = 1 << 3;
    pub const ABSOLUTE: u16 = 1 << 4;
    pub const CLAMP: u16 = 1 << 5;
    pub const SDWA_SEXT: u16 = 1 << 6;
    pub const EXPLICIT_SDWA_DST: u16 = 1 << 7;
    pub const DPP: u16 = 1 << 8;
    pub const DPP8: u16 = 1 << 9;
    pub const DPP_FETCH_INACTIVE: u16 = 1 << 10;
    pub const DPP_BOUND_CTRL: u16 = 1 << 11;
}

/// Operand representation and observed modifiers of the pinned decoder.
/// Defaults match the source (`Dword` select, `Preserve`, masks 0xf, no flags).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Operand {
    pub(crate) kind: OperandKind,
    pub(crate) value: u32,
    pub(crate) signed_value: i32,
    pub(crate) register: u16,
    pub(crate) sdwa_select: SdwaSelect,
    pub(crate) sdwa_unused: SdwaUnused,
    pub(crate) omod: u8,
    pub(crate) flags: u16,
    pub(crate) dpp_control: u32,
    pub(crate) dpp_row_mask: u8,
    pub(crate) dpp_bank_mask: u8,
}

impl Operand {
    pub(crate) const UNKNOWN: Self = Self::special(OperandKind::Unknown);

    pub(crate) const fn special(kind: OperandKind) -> Self {
        Self {
            kind,
            value: 0,
            signed_value: 0,
            register: 0,
            sdwa_select: SdwaSelect::Dword,
            sdwa_unused: SdwaUnused::Preserve,
            omod: 0,
            flags: 0,
            dpp_control: 0,
            dpp_row_mask: 0xf,
            dpp_bank_mask: 0xf,
        }
    }

    pub(crate) const fn integer(value: i32) -> Self {
        Self {
            value: value as u32,
            signed_value: value,
            ..Self::special(OperandKind::IntegerInlineConstant)
        }
    }

    pub(crate) const fn vgpr(register: u16) -> Self {
        Self {
            register,
            ..Self::special(OperandKind::Vgpr)
        }
    }

    pub(crate) fn set(&mut self, flag: u16, enabled: bool) {
        if enabled {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    pub(crate) fn has(self, flag: u16) -> bool {
        self.flags & flag != 0
    }

    pub fn kind(self) -> OperandKind {
        self.kind
    }
    /// Raw constant bits. Zero for registers and special operands, and for a
    /// selector-255 memory operand whose family never reads a literal word.
    pub fn value(self) -> u32 {
        self.value
    }
    /// The pinned decoder's signed immediate interpretation. Float constants
    /// retain zero here; their bit pattern is available through `value`.
    pub fn signed_value(self) -> i32 {
        self.signed_value
    }
    /// Register number for SGPR/VGPR operands; zero for other kinds. A FLAT
    /// pair continuation retains the source's unchecked `addr + 1` (up to 256).
    pub fn register(self) -> u16 {
        self.register
    }
    pub fn sdwa_select(self) -> SdwaSelect {
        self.sdwa_select
    }
    pub fn sdwa_unused(self) -> SdwaUnused {
        self.sdwa_unused
    }
    /// True only for destinations decoded from a real SDWA modifier word.
    pub fn explicit_sdwa_destination(self) -> bool {
        self.has(flag::EXPLICIT_SDWA_DST)
    }
    pub fn sdwa_sign_extend(self) -> bool {
        self.has(flag::SDWA_SEXT)
    }
    /// Encoded two-bit output modifier, retained where the source keeps it.
    pub fn output_modifier(self) -> u8 {
        self.omod
    }
    pub fn clamp(self) -> bool {
        self.has(flag::CLAMP)
    }
    pub fn negate(self) -> bool {
        self.has(flag::NEGATE)
    }
    pub fn negate_hi(self) -> bool {
        self.has(flag::NEGATE_HI)
    }
    pub fn absolute(self) -> bool {
        self.has(flag::ABSOLUTE)
    }
    pub fn op_sel(self) -> bool {
        self.has(flag::OP_SEL)
    }
    pub fn op_sel_hi(self) -> bool {
        self.has(flag::OP_SEL_HI)
    }
    pub fn dpp(self) -> Option<Dpp> {
        self.has(flag::DPP).then(|| Dpp {
            kind: if self.has(flag::DPP8) {
                DppKind::Dpp8
            } else {
                DppKind::Dpp16
            },
            control: self.dpp_control,
            fetch_inactive: self.has(flag::DPP_FETCH_INACTIVE),
            bound_control: self.has(flag::DPP_BOUND_CTRL),
            row_mask: self.dpp_row_mask,
            bank_mask: self.dpp_bank_mask,
        })
    }
}

/// A relative branch encoding, without a claim that its target is in the span
/// or at an instruction boundary. Signed arithmetic avoids wrapping targets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Branch {
    pub(crate) displacement_words: i16,
    pub(crate) target_byte_offset: i64,
}

impl Branch {
    pub fn displacement_words(self) -> i16 {
        self.displacement_words
    }
    pub fn target_byte_offset(self) -> i64 {
        self.target_byte_offset
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Export {
    pub(crate) target: u8,
    pub(crate) enabled: u8,
    pub(crate) compressed: bool,
    pub(crate) done: bool,
    pub(crate) valid_mask: bool,
}

impl Export {
    pub fn target(self) -> u8 {
        self.target
    }
    pub fn enabled(self) -> u8 {
        self.enabled
    }
    pub fn compressed(self) -> bool {
        self.compressed
    }
    pub fn done(self) -> bool {
        self.done
    }
    pub fn valid_mask(self) -> bool {
        self.valid_mask
    }
}

pub(crate) mod memory_flag {
    pub const DATA_SIGNED: u16 = 1 << 0;
    pub const TYPED: u16 = 1 << 1;
    pub const FORMATTED: u16 = 1 << 2;
    pub const GDS: u16 = 1 << 3;
    pub const GLC: u16 = 1 << 4;
    pub const DLC: u16 = 1 << 5;
    pub const SLC: u16 = 1 << 6;
    pub const IDXEN: u16 = 1 << 7;
    pub const OFFEN: u16 = 1 << 8;
    pub const TFE: u16 = 1 << 9;
    pub const LWE: u16 = 1 << 10;
}

/// SMEM/MUBUF/MTBUF/FLAT/DS/MIMG fields kept by the pinned decoder. Register
/// operands stay in the instruction operands; no resource layout is implied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryAccess {
    pub(crate) offset: u32,
    pub(crate) secondary_offset: u32,
    pub(crate) data_dwords: u8,
    pub(crate) data_bits: u8,
    pub(crate) data_format: u8,
    pub(crate) number_format: u8,
    pub(crate) segment: u8,
    pub(crate) flags: u16,
}

impl MemoryAccess {
    pub(crate) const fn new(data_dwords: u8, data_bits: u8) -> Self {
        Self {
            offset: 0,
            secondary_offset: 0,
            data_dwords,
            data_bits,
            data_format: 0,
            number_format: 0,
            segment: 0,
            flags: 0,
        }
    }

    pub(crate) fn set(&mut self, flag: u16, enabled: bool) {
        if enabled {
            self.flags |= flag;
        }
    }

    fn has(self, flag: u16) -> bool {
        self.flags & flag != 0
    }

    /// The source's u32 offset. SMEM (21-bit) and non-zero-segment FLAT
    /// (12-bit) offsets are sign-extended into it; DS two-address forms are
    /// pre-scaled; DS ordered count keeps `offset0 & 0xfc`.
    pub fn offset(self) -> u32 {
        self.offset
    }
    pub fn signed_offset(self) -> i32 {
        self.offset as i32
    }
    /// DS second address or DS ordered-count OFFSET1; zero elsewhere.
    pub fn secondary_offset(self) -> u32 {
        self.secondary_offset
    }
    /// Source table data dwords. MIMG uses dmask/D16 and is not updated for
    /// 64-bit atomics, matching the pinned decoder.
    pub fn data_dwords(self) -> u8 {
        self.data_dwords
    }
    pub fn data_bits(self) -> u8 {
        self.data_bits
    }
    /// MTBUF DFMT; zero elsewhere.
    pub fn data_format(self) -> u8 {
        self.data_format
    }
    /// MTBUF NFMT; zero elsewhere.
    pub fn number_format(self) -> u8 {
        self.number_format
    }
    /// FLAT SEG field; zero elsewhere.
    pub fn segment(self) -> u8 {
        self.segment
    }
    pub fn data_signed(self) -> bool {
        self.has(memory_flag::DATA_SIGNED)
    }
    pub fn typed(self) -> bool {
        self.has(memory_flag::TYPED)
    }
    pub fn formatted(self) -> bool {
        self.has(memory_flag::FORMATTED)
    }
    pub fn gds(self) -> bool {
        self.has(memory_flag::GDS)
    }
    pub fn glc(self) -> bool {
        self.has(memory_flag::GLC)
    }
    pub fn dlc(self) -> bool {
        self.has(memory_flag::DLC)
    }
    pub fn slc(self) -> bool {
        self.has(memory_flag::SLC)
    }
    pub fn idxen(self) -> bool {
        self.has(memory_flag::IDXEN)
    }
    pub fn offen(self) -> bool {
        self.has(memory_flag::OFFEN)
    }
    pub fn tfe(self) -> bool {
        self.has(memory_flag::TFE)
    }
    pub fn lwe(self) -> bool {
        self.has(memory_flag::LWE)
    }
}

/// The source's image sample flag bits (same values as `ImageSampleFlag`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageSampleFlags(pub(crate) u16);

impl ImageSampleFlags {
    pub const NONE: Self = Self(0);
    pub const LOD: Self = Self(1 << 0);
    pub const BIAS: Self = Self(1 << 1);
    pub const DERIVATIVE: Self = Self(1 << 2);
    pub const COMPARE: Self = Self(1 << 3);
    pub const OFFSET: Self = Self(1 << 4);
    pub const LEVEL_ZERO: Self = Self(1 << 5);
    pub const LOD_CLAMP: Self = Self(1 << 6);
    pub const A16: Self = Self(1 << 7);
    pub const CD: Self = Self(1 << 8);
    pub const GATHER_HORIZONTAL: Self = Self(1 << 9);
    pub const ADJUST: Self = Self(1 << 10);

    pub fn bits(self) -> u16 {
        self.0
    }
    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// The pinned decoder's DIM interpretation. Encodings 3 and 5 both map to
/// `Dim2DArray`; the raw field remains available from `Image::dimension_bits`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImageDimension {
    Dim1D,
    Dim1DArray,
    Dim2D,
    Dim3D,
    Dim2DArray,
    Dim2DMsaa,
    Dim2DMsaaArray,
}

impl ImageDimension {
    pub(crate) const fn from_bits(bits: u8) -> Self {
        match bits & 7 {
            0 => Self::Dim1D,
            1 => Self::Dim2D,
            2 => Self::Dim3D,
            3 | 5 => Self::Dim2DArray,
            4 => Self::Dim1DArray,
            6 => Self::Dim2DMsaa,
            _ => Self::Dim2DMsaaArray,
        }
    }
}

/// Maximum NSA address bytes in the pinned decoder (three NSA dwords).
pub const MAX_IMAGE_NSA_ADDRESSES: usize = 12;

/// MIMG fields kept by the pinned decoder.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Image {
    pub(crate) dmask: u8,
    pub(crate) data_components: u8,
    pub(crate) sample_flags: ImageSampleFlags,
    pub(crate) dimension_bits: u8,
    pub(crate) address_components: u8,
    pub(crate) nsa_dwords: u8,
    pub(crate) nsa_addresses: [u8; MAX_IMAGE_NSA_ADDRESSES],
    pub(crate) r128: bool,
}

impl Image {
    pub fn dmask(self) -> u8 {
        self.dmask
    }
    /// Four for gathers, otherwise the dmask population count (minimum one).
    pub fn data_components(self) -> u8 {
        self.data_components
    }
    /// Opcode sample/gather flags, plus `A16` from the A16 bit.
    pub fn sample_flags(self) -> ImageSampleFlags {
        self.sample_flags
    }
    pub fn dimension(self) -> ImageDimension {
        ImageDimension::from_bits(self.dimension_bits)
    }
    pub fn dimension_bits(self) -> u8 {
        self.dimension_bits
    }
    /// Source address component count (BVH: address dwords).
    pub fn address_components(self) -> u8 {
        self.address_components
    }
    pub fn nsa_dwords(self) -> u8 {
        self.nsa_dwords
    }
    /// Additional NSA VGPR numbers in encoded byte order (four per NSA dword).
    pub fn nsa_addresses(&self) -> &[u8] {
        &self.nsa_addresses[..usize::from(self.nsa_dwords) * 4]
    }
    pub fn r128(self) -> bool {
        self.r128
    }
}

/// One address component's packed bit position (source `ImageAddressComponent`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageAddressComponent {
    pub bit_offset: u32,
    pub bit_width: u32,
}

#[derive(Debug)]
pub(crate) struct Record {
    pub word_offset: usize,
    pub word_count: usize,
    pub family: Family,
    pub opcode_id: u16,
    pub opcode: Opcode,
    pub dst: Operand,
    pub dst2: Operand,
    pub src: [Operand; 4],
    pub src_count: usize,
    pub branch: Option<Branch>,
    pub export: Option<Export>,
    pub memory: Option<MemoryAccess>,
    pub image: Option<Image>,
}

/// An immutable instruction view that cannot outlive its owned source span.
///
/// ```compile_fail
/// use pentax_shader::{decode, Limits, ShaderStage};
/// let instruction = {
///     let owner = decode(&[0xbf80_0000], ShaderStage::Compute, &Limits::default()).unwrap();
///     owner.instruction(0).unwrap()
/// };
/// assert_eq!(instruction.raw_words(), &[0xbf80_0000]);
/// ```
#[derive(Clone, Copy, Debug)]
pub struct Instruction<'a> {
    record: &'a Record,
    source: &'a [u32],
}

impl<'a> Instruction<'a> {
    pub fn word_offset(self) -> usize {
        self.record.word_offset
    }
    pub fn byte_offset(self) -> u32 {
        (self.record.word_offset * 4) as u32
    }
    pub fn family(self) -> Family {
        self.record.family
    }
    pub fn opcode_id(self) -> u16 {
        self.record.opcode_id
    }
    pub fn opcode(self) -> Opcode {
        self.record.opcode
    }
    /// The source's sample variant name for `ImageSample` (all 56 sample
    /// encodings share that opcode), otherwise `Opcode::mnemonic`.
    pub fn mnemonic(self) -> &'static str {
        if self.record.opcode == Opcode::ImageSample
            && let Some((name, _)) = crate::opcode::mimg_sample(self.record.opcode_id)
        {
            return name;
        }
        self.record.opcode.mnemonic()
    }
    /// The source `IsBvhIntersect` test (MIMG 0xe6/0xe7). BVH instructions are
    /// decoded unconditionally; the program walker's disable policy is not ported.
    pub fn is_bvh_intersect(self) -> bool {
        self.record.family == Family::Mimg && matches!(self.record.opcode_id, 0xe6 | 0xe7)
    }
    pub fn destination(self) -> Operand {
        self.record.dst
    }
    /// The source `dst2`: VOP2 carry VCC or VOP3B scalar carry destination.
    /// `OperandKind::Unknown` when the instruction has none.
    pub fn secondary_destination(self) -> Operand {
        self.record.dst2
    }
    /// Used source operands. Remaining encoded operands, when any, are
    /// retained separately by `encoded_sources`, matching the pinned decoder.
    pub fn sources(self) -> &'a [Operand] {
        &self.record.src[..self.record.src_count]
    }
    pub fn encoded_sources(self) -> &'a [Operand; 4] {
        &self.record.src
    }
    pub fn branch(self) -> Option<Branch> {
        self.record.branch
    }
    pub fn export(self) -> Option<Export> {
        self.record.export
    }
    pub fn memory(self) -> Option<MemoryAccess> {
        self.record.memory
    }
    pub fn image(self) -> Option<&'a Image> {
        self.record.image.as_ref()
    }
    pub fn raw_words(self) -> &'a [u32] {
        &self.source[self.record.word_offset..self.record.word_offset + self.record.word_count]
    }
}

/// An entirely decoded linear span and its caller-supplied stage. Its vectors
/// cannot be modified or associated with another input through the public API.
#[derive(Debug)]
pub struct DecodedSpan {
    pub(crate) source: Vec<u32>,
    pub(crate) records: Vec<Record>,
    pub(crate) stage: ShaderStage,
    pub(crate) work: usize,
}

impl DecodedSpan {
    pub fn source_words(&self) -> &[u32] {
        &self.source
    }
    pub fn stage(&self) -> ShaderStage {
        self.stage
    }
    pub fn instruction_count(&self) -> usize {
        self.records.len()
    }
    /// Defined decoder work units, not elapsed time or hardware cycles.
    pub fn work_units(&self) -> usize {
        self.work
    }
    /// Rust-reported vector capacities; excludes allocator bookkeeping.
    pub fn source_capacity_words(&self) -> usize {
        self.source.capacity()
    }
    pub fn record_capacity_bytes(&self) -> usize {
        self.records.capacity() * std::mem::size_of::<Record>()
    }
    pub fn instruction(&self, index: usize) -> Option<Instruction<'_>> {
        self.records.get(index).map(|record| Instruction {
            record,
            source: &self.source,
        })
    }
    pub fn instructions(&self) -> impl ExactSizeIterator<Item = Instruction<'_>> {
        self.records.iter().map(|record| Instruction {
            record,
            source: &self.source,
        })
    }
}
