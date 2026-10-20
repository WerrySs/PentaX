// SPDX-License-Identifier: GPL-2.0-only
use crate::{Error, HEADER_BYTES, Limits};

const MAGIC: [u8; 8] = *b"PNTXSES\0";
const VERSION: u16 = 1;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Nonce([u8; 16]);
impl Nonce {
    /// Supplied by a separate random session/endpoint owner, never by a guest.
    pub fn new(bytes: [u8; 16]) -> Result<Self, Error> {
        if bytes == [0; 16] {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self(bytes))
    }
    pub fn bytes(self) -> [u8; 16] {
        self.0
    }
}
impl std::fmt::Debug for Nonce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Nonce(..)")
    }
}

/// Wire identity only. This copyable metadata confers no frame authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameId {
    pub(crate) nonce: Nonce,
    pub(crate) sequence: u64,
    pub(crate) generation: u64,
}
impl FrameId {
    pub fn nonce(self) -> Nonce {
        self.nonce
    }
    pub fn sequence(self) -> u64 {
        self.sequence
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlphaMode {
    Straight,
    Premultiplied,
}
/// Describes the supplied converted RGBA8 bytes; it performs no transfer or
/// gamma conversion. Unspecified retains an explicitly unproven source encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorEncoding {
    Unspecified,
    Srgb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameFlags(u16);
impl FrameFlags {
    pub fn new(alpha: AlphaMode, color: ColorEncoding) -> Self {
        Self(
            u16::from(alpha == AlphaMode::Premultiplied)
                | if color == ColorEncoding::Srgb { 2 } else { 0 },
        )
    }
    pub fn alpha(self) -> AlphaMode {
        if self.0 & 1 == 0 {
            AlphaMode::Straight
        } else {
            AlphaMode::Premultiplied
        }
    }
    pub fn color(self) -> ColorEncoding {
        if self.0 & 2 == 0 {
            ColorEncoding::Unspecified
        } else {
            ColorEncoding::Srgb
        }
    }
    fn decode(bits: u16) -> Result<Self, Error> {
        if bits & !3 != 0 {
            return Err(Error::InvalidFlags);
        }
        Ok(Self(bits))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    FrameRgba8 = 1,
    Presented = 2,
    Vblank = 3,
    Close = 4,
}

/// Validated fixed metadata. Control messages have no dimensions/alpha/color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    kind: Kind,
    flags: FrameFlags,
    id: FrameId,
    width: u32,
    height: u32,
    length: u32,
}
impl Header {
    pub(crate) fn frame(
        id: FrameId,
        width: u32,
        height: u32,
        flags: FrameFlags,
        limits: Limits,
    ) -> Result<Self, Error> {
        limits.validate()?;
        if id.sequence == 0 || id.generation == 0 {
            return Err(Error::InvalidIdentity);
        }
        let length = frame_length(width, height, limits)?;
        Ok(Self {
            kind: Kind::FrameRgba8,
            flags,
            id,
            width,
            height,
            length,
        })
    }
    pub fn presented(id: FrameId) -> Result<Self, Error> {
        if id.sequence == 0 || id.generation == 0 {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self {
            kind: Kind::Presented,
            flags: FrameFlags(0),
            id,
            width: 0,
            height: 0,
            length: 0,
        })
    }
    pub fn vblank(nonce: Nonce, generation: u64) -> Result<Self, Error> {
        Self::control(Kind::Vblank, nonce, generation, 8)
    }
    pub fn close(nonce: Nonce, generation: u64) -> Result<Self, Error> {
        Self::control(Kind::Close, nonce, generation, 0)
    }
    fn control(kind: Kind, nonce: Nonce, generation: u64, length: u32) -> Result<Self, Error> {
        if generation == 0 {
            return Err(Error::InvalidIdentity);
        }
        Ok(Self {
            kind,
            flags: FrameFlags(0),
            id: FrameId {
                nonce,
                sequence: 0,
                generation,
            },
            width: 0,
            height: 0,
            length,
        })
    }
    pub fn kind(self) -> Kind {
        self.kind
    }
    pub fn id(self) -> FrameId {
        self.id
    }
    pub fn width(self) -> u32 {
        self.width
    }
    pub fn height(self) -> u32 {
        self.height
    }
    pub fn payload_len(self) -> usize {
        self.length as usize
    }
    pub fn frame_flags(self) -> Option<FrameFlags> {
        (self.kind == Kind::FrameRgba8).then_some(self.flags)
    }
    pub fn stride(self) -> Option<u32> {
        (self.kind == Kind::FrameRgba8).then_some(self.width * 4)
    }
    pub fn encode(self) -> [u8; HEADER_BYTES] {
        let mut out = [0; HEADER_BYTES];
        out[..8].copy_from_slice(&MAGIC);
        out[8..10].copy_from_slice(&VERSION.to_le_bytes());
        out[10..12].copy_from_slice(&(self.kind as u16).to_le_bytes());
        out[12..14].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes());
        out[14..16].copy_from_slice(&self.flags.0.to_le_bytes());
        out[16..32].copy_from_slice(&self.id.nonce.0);
        out[32..40].copy_from_slice(&self.id.sequence.to_le_bytes());
        out[40..48].copy_from_slice(&self.id.generation.to_le_bytes());
        out[48..52].copy_from_slice(&self.width.to_le_bytes());
        out[52..56].copy_from_slice(&self.height.to_le_bytes());
        out[56..60].copy_from_slice(&self.length.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8; HEADER_BYTES], limits: Limits) -> Result<Self, Error> {
        limits.validate()?;
        if bytes[..8] != MAGIC || u16_at(bytes, 12) != HEADER_BYTES as u16 || u32_at(bytes, 60) != 0
        {
            return Err(Error::InvalidHeader);
        }
        if u16_at(bytes, 8) != VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let kind = match u16_at(bytes, 10) {
            1 => Kind::FrameRgba8,
            2 => Kind::Presented,
            3 => Kind::Vblank,
            4 => Kind::Close,
            _ => return Err(Error::UnknownKind),
        };
        let flags = FrameFlags::decode(u16_at(bytes, 14))?;
        let mut nonce = [0; 16];
        nonce.copy_from_slice(&bytes[16..32]);
        let id = FrameId {
            nonce: Nonce::new(nonce)?,
            sequence: u64_at(bytes, 32),
            generation: u64_at(bytes, 40),
        };
        if id.generation == 0 {
            return Err(Error::InvalidIdentity);
        }
        let width = u32_at(bytes, 48);
        let height = u32_at(bytes, 52);
        let length = u32_at(bytes, 56);
        match kind {
            Kind::FrameRgba8 => {
                if id.sequence == 0 {
                    return Err(Error::InvalidIdentity);
                }
                if frame_length(width, height, limits)? != length {
                    return Err(Error::InvalidLength);
                }
            }
            Kind::Presented | Kind::Vblank | Kind::Close => {
                if flags.0 != 0 {
                    return Err(Error::InvalidFlags);
                }
                if width != 0 || height != 0 {
                    return Err(Error::InvalidDimensions);
                }
                if (kind == Kind::Presented) != (id.sequence != 0) {
                    return Err(Error::InvalidIdentity);
                }
                if length != if kind == Kind::Vblank { 8 } else { 0 } {
                    return Err(Error::InvalidLength);
                }
            }
        }
        Ok(Self {
            kind,
            flags,
            id,
            width,
            height,
            length,
        })
    }
    pub fn validate_payload(self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() != self.payload_len() {
            return Err(Error::InvalidLength);
        }
        if self.kind == Kind::Vblank
            && u64::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidLength)?) == 0
        {
            return Err(Error::InvalidVblank);
        }
        Ok(())
    }
}
fn frame_length(width: u32, height: u32, limits: Limits) -> Result<u32, Error> {
    if width == 0 || height == 0 || width > limits.max_dimension || height > limits.max_dimension {
        return Err(Error::InvalidDimensions);
    }
    let length = u64::from(width) * u64::from(height) * 4;
    if length > limits.max_frame_bytes as u64 {
        return Err(Error::Limit);
    }
    u32::try_from(length).map_err(|_| Error::Limit)
}
fn u16_at(bytes: &[u8; HEADER_BYTES], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}
fn u32_at(bytes: &[u8; HEADER_BYTES], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
fn u64_at(bytes: &[u8; HEADER_BYTES], at: usize) -> u64 {
    let mut word = [0; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}
