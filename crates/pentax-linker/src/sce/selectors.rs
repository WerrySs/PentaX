// SPDX-License-Identifier: GPL-2.0-or-later

//! Pure, byte-based NID token and selector helpers.
//!
//! The qualified form `NID#library#module` and the variable-width positional
//! radix-64 selector encoding follow the encoder observed in shadPS4
//! (`EncodeId`, `LoadSymbols`). The canonicality rules enforced here (no
//! leading zero digit, at most three digits, zero padding bits in the final
//! NID digit) are PentaX validation policy derived from that encoder; they are
//! not independently confirmed PS5 acceptance rules. Nothing here computes a
//! NID from a symbol name.

use std::{error::Error, fmt};

/// The 64-symbol alphabet shared by NID tokens and selectors. It differs from
/// standard base64 in its last two symbols and has no padding character.
pub const SELECTOR_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+-";

/// Number of bytes in a canonical NID token.
pub const NID_TOKEN_LEN: usize = 11;

/// The most digits a canonical `u16` selector can need.
const MAX_SELECTOR_DIGITS: usize = 3;

const DELIMITER: u8 = b'#';

/// The value of one alphabet byte, or `None` for any other byte.
const fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'-' => Some(63),
        _ => None,
    }
}

/// Why a single NID token or selector is not canonical.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TokenError {
    Empty,
    /// A NID must have exactly 11 bytes; a selector at most three.
    InvalidLength {
        length: usize,
    },
    /// `position` is zero-based within the token.
    InvalidByte {
        position: usize,
        byte: u8,
    },
    /// A multi-digit selector starts with the zero digit `A`.
    LeadingZero,
    /// The selector's value does not fit in `u16`.
    Overflow,
    /// The final NID digit has nonzero low padding bits.
    NonzeroPadding {
        byte: u8,
    },
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("token is empty"),
            Self::InvalidLength { length } => write!(f, "token length {length} is invalid"),
            Self::InvalidByte { position, byte } => write!(
                f,
                "byte b'{}' at position {position} is outside the selector alphabet",
                byte.escape_ascii()
            ),
            Self::LeadingZero => f.write_str("selector has a leading zero digit"),
            Self::Overflow => f.write_str("selector value exceeds 65535"),
            Self::NonzeroPadding { byte } => write!(
                f,
                "final NID digit b'{}' has nonzero padding bits",
                byte.escape_ascii()
            ),
        }
    }
}

impl Error for TokenError {}

/// A scoped `u16` descriptor ID as written in a qualified symbol name.
///
/// This is an integer in positional radix 64, most significant digit first,
/// not base64 of two bytes: `0 = A`, `63 = -`, `64 = BA`, `65535 = P--`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SelectorId(u16);

impl SelectorId {
    #[must_use]
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }

    /// Parse one canonical selector of 1..=3 alphabet bytes.
    ///
    /// # Errors
    ///
    /// Returns [`TokenError`] for empty or overlong input, bytes outside the
    /// alphabet, a leading zero digit or a value above `u16::MAX`.
    pub fn parse(bytes: &[u8]) -> Result<Self, TokenError> {
        if bytes.is_empty() {
            return Err(TokenError::Empty);
        }
        if bytes.len() > MAX_SELECTOR_DIGITS {
            return Err(TokenError::InvalidLength {
                length: bytes.len(),
            });
        }
        let mut value: u16 = 0;
        for (position, &byte) in bytes.iter().enumerate() {
            let digit = digit(byte).ok_or(TokenError::InvalidByte { position, byte })?;
            value = value
                .checked_mul(64)
                .and_then(|value| value.checked_add(u16::from(digit)))
                .ok_or(TokenError::Overflow)?;
        }
        if bytes.len() > 1 && bytes[0] == b'A' {
            return Err(TokenError::LeadingZero);
        }
        Ok(Self(value))
    }

    /// The canonical text of this selector.
    #[must_use]
    pub fn encode(self) -> SelectorText {
        let mut digits = [0; MAX_SELECTOR_DIGITS];
        let mut start = MAX_SELECTOR_DIGITS;
        let mut value = self.0;
        loop {
            start -= 1;
            digits[start] = SELECTOR_ALPHABET[usize::from(value % 64)];
            value /= 64;
            if value == 0 {
                break;
            }
        }
        SelectorText { digits, start }
    }
}

impl fmt::Display for SelectorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.encode(), f)
    }
}

/// Canonical selector text: one to three alphabet bytes, without allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SelectorText {
    digits: [u8; MAX_SELECTOR_DIGITS],
    start: usize,
}

impl SelectorText {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.digits[self.start..]
    }
}

impl fmt::Display for SelectorText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Every byte comes from the ASCII alphabet.
        for &byte in self.as_bytes() {
            fmt::Write::write_char(f, char::from(byte))?;
        }
        Ok(())
    }
}

/// A canonical 11-byte NID token, retained verbatim.
///
/// The token is validated syntactically only; it is never computed from or
/// checked against a symbol name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NidToken([u8; NID_TOKEN_LEN]);

impl NidToken {
    /// Parse exactly 11 alphabet bytes whose final digit has zero low two
    /// bits (`AEIMQUYcgkosw048`).
    ///
    /// # Errors
    ///
    /// Returns [`TokenError`] for a wrong length, a byte outside the alphabet
    /// or nonzero final padding bits.
    pub fn parse(bytes: &[u8]) -> Result<Self, TokenError> {
        let token: [u8; NID_TOKEN_LEN] = bytes.try_into().map_err(|_| {
            if bytes.is_empty() {
                TokenError::Empty
            } else {
                TokenError::InvalidLength {
                    length: bytes.len(),
                }
            }
        })?;
        let mut last = 0;
        for (position, &byte) in token.iter().enumerate() {
            last = digit(byte).ok_or(TokenError::InvalidByte { position, byte })?;
        }
        if last & 0b11 != 0 {
            return Err(TokenError::NonzeroPadding {
                byte: token[NID_TOKEN_LEN - 1],
            });
        }
        Ok(Self(token))
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; NID_TOKEN_LEN] {
        &self.0
    }
}

impl fmt::Debug for NidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NidToken({self})")
    }
}

impl fmt::Display for NidToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &byte in &self.0 {
            fmt::Write::write_char(f, char::from(byte))?;
        }
        Ok(())
    }
}

/// A field of the qualified form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QualifiedField {
    Nid,
    Library,
    Module,
}

impl fmt::Display for QualifiedField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nid => "NID",
            Self::Library => "library selector",
            Self::Module => "module selector",
        })
    }
}

/// Why bytes are not a canonical `NID#library#module` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QualifiedNameError {
    /// Exactly two `#` delimiters are required.
    DelimiterCount { count: usize },
    Field {
        field: QualifiedField,
        error: TokenError,
    },
}

impl fmt::Display for QualifiedNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DelimiterCount { count } => write!(
                f,
                "qualified name has {count} '#' delimiters; expected exactly 2"
            ),
            Self::Field { field, error } => write!(f, "qualified {field}: {error}"),
        }
    }
}

impl Error for QualifiedNameError {}

fn in_field(field: QualifiedField) -> impl Fn(TokenError) -> QualifiedNameError {
    move |error| QualifiedNameError::Field { field, error }
}

/// A canonical qualified name: a NID scoped by library and module selectors.
///
/// The selectors are IDs local to the image that contains the name; they are
/// not global module IDs, runtime handles or provider identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QualifiedNid {
    nid: NidToken,
    library: SelectorId,
    module: SelectorId,
}

impl QualifiedNid {
    #[must_use]
    pub const fn new(nid: NidToken, library: SelectorId, module: SelectorId) -> Self {
        Self {
            nid,
            library,
            module,
        }
    }

    /// Parse exactly `NID#library#module` with three nonempty canonical
    /// fields. Names without `#` are ordinary names and are rejected here
    /// rather than guessed at; nothing is truncated.
    ///
    /// # Errors
    ///
    /// Returns [`QualifiedNameError`] for a delimiter count other than two or
    /// for the first non-canonical field.
    pub fn parse(bytes: &[u8]) -> Result<Self, QualifiedNameError> {
        let count = bytes.iter().filter(|&&byte| byte == DELIMITER).count();
        if count != 2 {
            return Err(QualifiedNameError::DelimiterCount { count });
        }
        let mut fields = bytes.splitn(3, |&byte| byte == DELIMITER);
        let (Some(nid), Some(library), Some(module)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return Err(QualifiedNameError::DelimiterCount { count });
        };
        Ok(Self {
            nid: NidToken::parse(nid).map_err(in_field(QualifiedField::Nid))?,
            library: SelectorId::parse(library).map_err(in_field(QualifiedField::Library))?,
            module: SelectorId::parse(module).map_err(in_field(QualifiedField::Module))?,
        })
    }

    #[must_use]
    pub const fn nid(&self) -> NidToken {
        self.nid
    }

    #[must_use]
    pub const fn library(&self) -> SelectorId {
        self.library
    }

    #[must_use]
    pub const fn module(&self) -> SelectorId {
        self.module
    }

    /// The canonical byte form, which [`QualifiedNid::parse`] accepts.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let library = self.library.encode();
        let module = self.module.encode();
        let mut bytes = Vec::with_capacity(
            NID_TOKEN_LEN + 2 + library.as_bytes().len() + module.as_bytes().len(),
        );
        bytes.extend_from_slice(&self.nid.0);
        bytes.push(DELIMITER);
        bytes.extend_from_slice(library.as_bytes());
        bytes.push(DELIMITER);
        bytes.extend_from_slice(module.as_bytes());
        bytes
    }
}

impl fmt::Display for QualifiedNid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}#{}", self.nid, self.library, self.module)
    }
}
