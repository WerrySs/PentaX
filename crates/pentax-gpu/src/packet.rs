// SPDX-License-Identifier: GPL-2.0-or-later

//! Pure header classification and single-packet body decoding.
//!
//! Header and body field positions follow AMD's Gfx9+ PFP/ME/MEC packet
//! definitions and packet writer pinned in the crate README. Nothing here reads
//! outside the span it is given, touches guest memory, keeps GPU state or
//! looks at any other packet.

use crate::StopReason;

/// The only type-2 word admitted, as one dword of padding.
const TYPE2_PADDING: u32 = 0x8000_0000;
const NOP: u8 = 0x10;
const INDIRECT_BUFFER: u8 = 0x3f;
/// A NOP with this count is one header dword rather than count + 2 dwords.
const HEADER_ONLY_NOP_COUNT: u16 = 0x3fff;
/// The common indirect-buffer reference has three body dwords.
const INDIRECT_BUFFER_COUNT: u16 = 2;
const INDIRECT_BUFFER_DWORDS: u16 = 4;
/// Exclusive end of the 16-bit relative register index domain.
const RELATIVE_INDEX_END: u32 = 0x1_0000;

const PREDICATE_BIT: u32 = 1 << 0;
const SHADER_TYPE_BIT: u32 = 1 << 1;
const FILTER_CAM_RESET_BIT: u32 = 1 << 2;
const RESERVED_HEADER_BITS: u32 = 0xf8;

const IB_SIZE_MASK: u32 = 0x000f_ffff;
const IB_CHAIN_BIT: u32 = 1 << 20;
const IB_UPPER_CONTROL_BITS: u32 = !(IB_SIZE_MASK | IB_CHAIN_BIT);
const IB_ALIGNMENT_BITS: u32 = 0x3;

/// One raw packet header dword with pure field accessors.
///
/// Bits 31:30 hold the packet type. For type 3, bits 29:16 hold the count,
/// 15:8 the opcode, bit 2 the filter-CAM reset flag, bit 1 the shader type and
/// bit 0 the predicate; bits 7:3 are reserved in AMD's ME/PFP layout. The
/// type-3 accessors are plain bit extractions and mean nothing for other
/// packet types. Decoding a header never implies that the packet is admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Header(u32);

impl Header {
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Bits 31:30.
    #[must_use]
    pub const fn packet_type(self) -> u8 {
        (self.0 >> 30) as u8
    }

    /// Bits 29:16. For ordinary type-3 packets this is the number of body
    /// dwords minus one, so the packet spans count + 2 dwords.
    #[must_use]
    pub const fn count(self) -> u16 {
        ((self.0 >> 16) & 0x3fff) as u16
    }

    /// Bits 15:8.
    #[must_use]
    pub const fn opcode(self) -> u8 {
        (self.0 >> 8) as u8
    }

    /// Bits 7:0: the three named flags and the reserved bits together.
    #[must_use]
    pub const fn low_byte(self) -> u8 {
        self.0 as u8
    }

    /// Bit 0.
    #[must_use]
    pub const fn predicate(self) -> bool {
        self.0 & PREDICATE_BIT != 0
    }

    /// Bit 1.
    #[must_use]
    pub const fn shader_type(self) -> bool {
        self.0 & SHADER_TYPE_BIT != 0
    }

    /// Bit 2.
    #[must_use]
    pub const fn filter_cam_reset(self) -> bool {
        self.0 & FILTER_CAM_RESET_BIT != 0
    }

    /// Bits 7:3, left in place.
    #[must_use]
    pub const fn reserved_bits(self) -> u8 {
        (self.0 & RESERVED_HEADER_BITS) as u8
    }
}

/// Register space of an admitted direct relative register write.
///
/// The space only names which write grammar was used. Offsets are relative
/// indices in a 16-bit domain, not byte addresses, guest pointers or proof
/// that a PS5 register exists at that index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegisterSpace {
    /// Opcode 0x69.
    Context,
    /// Opcode 0x76.
    Sh,
    /// Opcode 0x79.
    Uconfig,
}

impl RegisterSpace {
    #[must_use]
    pub const fn opcode(self) -> u8 {
        match self {
            Self::Context => 0x69,
            Self::Sh => 0x76,
            Self::Uconfig => 0x79,
        }
    }

    const fn from_opcode(opcode: u8) -> Option<Self> {
        match opcode {
            0x69 => Some(Self::Context),
            0x76 => Some(Self::Sh),
            0x79 => Some(Self::Uconfig),
            _ => None,
        }
    }
}

/// Observed fields of a four-dword indirect-buffer reference.
///
/// This is a terminal observation. Inspection never reads the target, never
/// transfers its cursor and never commits the reference as a packet. Only
/// the common fields are named: address low (with two alignment bits),
/// address high, size in control bits 19:0 and chain in bit 20. Control bits
/// 31:21 differ between AMD engines and remain opaque. A zero size is
/// reported as zero, not normalized into a no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndirectBuffer {
    header: Header,
    address_low: u32,
    address_high: u32,
    control: u32,
}

impl IndirectBuffer {
    /// The header, including any flag bits, exactly as supplied.
    #[must_use]
    pub const fn header(self) -> Header {
        self.header
    }

    #[must_use]
    pub const fn address_low_raw(self) -> u32 {
        self.address_low
    }

    #[must_use]
    pub const fn address_high_raw(self) -> u32 {
        self.address_high
    }

    #[must_use]
    pub const fn control_raw(self) -> u32 {
        self.control
    }

    /// Raw `address_high << 32 | address_low`. No width, canonicality or
    /// mapping check is made; the value is never dereferenced.
    #[must_use]
    pub const fn address(self) -> u64 {
        ((self.address_high as u64) << 32) | self.address_low as u64
    }

    /// Address-low bits 1:0, which must be zero for a dword-aligned reference.
    #[must_use]
    pub const fn alignment_bits(self) -> u32 {
        self.address_low & IB_ALIGNMENT_BITS
    }

    /// Control bits 19:0, in dwords, without normalization.
    #[must_use]
    pub const fn size_dwords(self) -> u32 {
        self.control & IB_SIZE_MASK
    }

    /// Control bit 20.
    #[must_use]
    pub const fn chain(self) -> bool {
        self.control & IB_CHAIN_BIT != 0
    }

    /// Control bits 31:21, left in place and never interpreted.
    #[must_use]
    pub const fn upper_control_bits(self) -> u32 {
        self.control & IB_UPPER_CONTROL_BITS
    }
}

/// Grammar selected from a header alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    Padding,
    HeaderOnlyNop,
    Nop,
    Register(RegisterSpace),
    IndirectBuffer,
}

/// A header's grammar and the complete packet length it frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Framing {
    pub(crate) shape: Shape,
    /// Header included; at most 0x4001.
    pub(crate) dwords: u16,
}

/// Typed content of a committed record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Padding,
    HeaderOnlyNop,
    Nop {
        body_dwords: u16,
    },
    Register {
        space: RegisterSpace,
        offset: u16,
        values: u16,
    },
}

impl Kind {
    /// Packet length, header included.
    pub(crate) const fn dwords(self) -> u16 {
        match self {
            Self::Padding | Self::HeaderOnlyNop => 1,
            Self::Nop { body_dwords } => body_dwords.saturating_add(1),
            Self::Register { values, .. } => values.saturating_add(2),
        }
    }
}

/// Selects the grammar and framed length from the header alone.
///
/// Type, opcode, flag and count rules are applied here, before any body word
/// is read. The header-only NOP is recognized before ordinary count + 2
/// arithmetic. For register writes, reserved bits are checked before the
/// predicate, and both before an empty count.
pub(crate) fn classify(header: Header) -> Result<Framing, StopReason> {
    match header.packet_type() {
        0 | 1 => Err(StopReason::UnsupportedPacketType { header }),
        2 if header.raw() == TYPE2_PADDING => Ok(Framing {
            shape: Shape::Padding,
            dwords: 1,
        }),
        2 => Err(StopReason::UnsupportedPadding { header }),
        _ => classify_type3(header),
    }
}

fn classify_type3(header: Header) -> Result<Framing, StopReason> {
    let opcode = header.opcode();
    let count = header.count();
    if opcode == NOP {
        if header.low_byte() != 0 {
            return Err(StopReason::UnsupportedNopProfile { header });
        }
        if count == HEADER_ONLY_NOP_COUNT {
            return Ok(Framing {
                shape: Shape::HeaderOnlyNop,
                dwords: 1,
            });
        }
        return Ok(Framing {
            shape: Shape::Nop,
            dwords: ordinary_dwords(count),
        });
    }
    if let Some(space) = RegisterSpace::from_opcode(opcode) {
        if header.reserved_bits() != 0 {
            return Err(StopReason::UnsupportedHeaderEncoding { header });
        }
        if header.predicate() {
            return Err(StopReason::PredicatedWrite { header });
        }
        if count == 0 {
            return Err(StopReason::EmptyRegisterWrite { header });
        }
        return Ok(Framing {
            shape: Shape::Register(space),
            dwords: ordinary_dwords(count),
        });
    }
    if opcode == INDIRECT_BUFFER {
        if count != INDIRECT_BUFFER_COUNT {
            return Err(StopReason::UnsupportedIndirectBufferLength { header });
        }
        return Ok(Framing {
            shape: Shape::IndirectBuffer,
            dwords: INDIRECT_BUFFER_DWORDS,
        });
    }
    Err(StopReason::UnsupportedOpcode { header })
}

/// Ordinary type-3 length; a 14-bit count gives at most 0x4001.
const fn ordinary_dwords(count: u16) -> u16 {
    count.saturating_add(2)
}

/// Decodes the body of a classified packet from `span`, which starts at its
/// header. Every check completes before a [`Kind`] is returned; an indirect
/// buffer always yields a terminal stop.
pub(crate) fn decode_body(
    header: Header,
    framing: Framing,
    span: &[[u8; 4]],
) -> Result<Kind, StopReason> {
    let truncated = StopReason::Truncated {
        header,
        packet_dwords: framing.dwords,
        available_dwords: span.len(),
    };
    let Some(span) = span.get(..usize::from(framing.dwords)) else {
        return Err(truncated);
    };
    match (framing.shape, span) {
        (Shape::Padding, _) => Ok(Kind::Padding),
        (Shape::HeaderOnlyNop, _) => Ok(Kind::HeaderOnlyNop),
        (Shape::Nop, _) => Ok(Kind::Nop {
            body_dwords: framing.dwords.saturating_sub(1),
        }),
        (Shape::Register(space), [_, offset_word, ..]) => register(
            header,
            space,
            u32::from_le_bytes(*offset_word),
            framing.dwords.saturating_sub(2),
        ),
        (Shape::IndirectBuffer, [_, low, high, control]) => {
            let reference = IndirectBuffer {
                header,
                address_low: u32::from_le_bytes(*low),
                address_high: u32::from_le_bytes(*high),
                control: u32::from_le_bytes(*control),
            };
            Err(if reference.alignment_bits() == 0 {
                StopReason::IndirectBufferReference(reference)
            } else {
                StopReason::MisalignedIndirectBuffer(reference)
            })
        }
        _ => Err(truncated),
    }
}

/// The offset word's high 16 bits must be zero; they are never masked. The
/// run of `values` indices must end at or before 0x10000.
fn register(
    header: Header,
    space: RegisterSpace,
    offset_word: u32,
    values: u16,
) -> Result<Kind, StopReason> {
    let Ok(offset) = u16::try_from(offset_word) else {
        return Err(StopReason::UnsupportedRegisterSelector {
            header,
            offset_word,
        });
    };
    if u32::from(offset) + u32::from(values) > RELATIVE_INDEX_END {
        return Err(StopReason::RegisterRangeOverflow {
            header,
            offset,
            values,
        });
    }
    Ok(Kind::Register {
        space,
        offset,
        values,
    })
}
