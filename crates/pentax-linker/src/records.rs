// SPDX-License-Identifier: GPL-2.0-or-later

//! Raw ELF64 symbol and RELA records.
//!
//! Symbol layout: <https://gabi.xinuos.com/elf/05-symtab.html>.
//! RELA layout: <https://gabi.xinuos.com/elf/06-reloc.html>.
//! Fields are decoded little-endian and kept verbatim. Helper views label the
//! generic values those pages define without rejecting or reinterpreting the
//! OS-, processor- or SCE-specific values they leave open.

use std::fmt;

use crate::limits::NameBudget;
use crate::strings::StringTable;
use crate::{LinkError, TableKind};

/// Size of one ELF64 `Elf64_Sym` record.
pub const ELF64_SYMBOL_SIZE: u64 = 24;
/// Size of one ELF64 `Elf64_Rela` record.
pub const ELF64_RELA_SIZE: u64 = 24;

const RECORD_SIZE: usize = 24;

/// Owned symbol-name bytes, excluding the NUL terminator.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SymbolName(Box<[u8]>);

impl SymbolName {
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// A text view, available only when the bytes happen to be valid UTF-8.
    #[must_use]
    pub fn to_utf8(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SymbolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b\"{}\"", self.0.escape_ascii())
    }
}

/// The high four bits of `st_info`, retained verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolBinding(u8);

impl SymbolBinding {
    pub const LOCAL: Self = Self(0);
    pub const GLOBAL: Self = Self(1);
    pub const WEAK: Self = Self(2);

    #[must_use]
    pub const fn from_info(info: u8) -> Self {
        Self(info >> 4)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// `STB_LOOS..=STB_HIOS`; the meaning belongs to the OS ABI.
    #[must_use]
    pub const fn is_os_specific(self) -> bool {
        matches!(self.0, 10..=12)
    }

    /// `STB_LOPROC..=STB_HIPROC`; the meaning belongs to the processor ABI.
    #[must_use]
    pub const fn is_processor_specific(self) -> bool {
        matches!(self.0, 13..=15)
    }

    /// The generic name, or `None` for values the gABI leaves to other ABIs.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("LOCAL"),
            1 => Some("GLOBAL"),
            2 => Some("WEAK"),
            _ => None,
        }
    }
}

/// The low four bits of `st_info`, retained verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolType(u8);

impl SymbolType {
    pub const NOTYPE: Self = Self(0);
    pub const OBJECT: Self = Self(1);
    pub const FUNC: Self = Self(2);
    pub const SECTION: Self = Self(3);
    pub const FILE: Self = Self(4);
    pub const COMMON: Self = Self(5);
    pub const TLS: Self = Self(6);

    #[must_use]
    pub const fn from_info(info: u8) -> Self {
        Self(info & 0xf)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// `STT_LOOS..=STT_HIOS`; the meaning belongs to the OS ABI.
    #[must_use]
    pub const fn is_os_specific(self) -> bool {
        matches!(self.0, 10..=12)
    }

    /// `STT_LOPROC..=STT_HIPROC`; the meaning belongs to the processor ABI.
    #[must_use]
    pub const fn is_processor_specific(self) -> bool {
        matches!(self.0, 13..=15)
    }

    /// The generic name, or `None` for values the gABI leaves to other ABIs.
    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("NOTYPE"),
            1 => Some("OBJECT"),
            2 => Some("FUNC"),
            3 => Some("SECTION"),
            4 => Some("FILE"),
            5 => Some("COMMON"),
            6 => Some("TLS"),
            _ => None,
        }
    }
}

/// The low three bits of `st_other`, preserving unnamed visibility values;
/// the remaining `st_other` bits stay available through [`Symbol::other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolVisibility(u8);

impl SymbolVisibility {
    pub const DEFAULT: Self = Self(0);
    pub const INTERNAL: Self = Self(1);
    pub const HIDDEN: Self = Self(2);
    pub const PROTECTED: Self = Self(3);
    pub const EXPORTED: Self = Self(4);
    pub const SINGLETON: Self = Self(5);
    pub const ELIMINATE: Self = Self(6);

    #[must_use]
    pub const fn from_other(other: u8) -> Self {
        Self(other & 7)
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn name(self) -> Option<&'static str> {
        match self.0 {
            0 => Some("DEFAULT"),
            1 => Some("INTERNAL"),
            2 => Some("HIDDEN"),
            3 => Some("PROTECTED"),
            4 => Some("EXPORTED"),
            5 => Some("SINGLETON"),
            6 => Some("ELIMINATE"),
            _ => None,
        }
    }
}

/// One `Elf64_Sym` in table order. Its index is its position in
/// [`LinkMetadata::symbols`](crate::LinkMetadata::symbols).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Raw `st_name`. Zero means the entry has no name.
    pub name_offset: u32,
    /// Bytes of a nonzero `st_name`; `None` when `st_name` is zero, as in the
    /// reserved zeroth entry. No placeholder name is ever invented.
    pub name: Option<SymbolName>,
    /// Raw `st_info`.
    pub info: u8,
    /// Raw `st_other`, including bits outside the visibility field.
    pub other: u8,
    /// Raw `st_shndx`, including reserved and escape values.
    pub section_index: u16,
    pub value: u64,
    pub size: u64,
}

impl Symbol {
    #[must_use]
    pub const fn binding(&self) -> SymbolBinding {
        SymbolBinding::from_info(self.info)
    }

    #[must_use]
    pub const fn symbol_type(&self) -> SymbolType {
        SymbolType::from_info(self.info)
    }

    #[must_use]
    pub const fn visibility(&self) -> SymbolVisibility {
        SymbolVisibility::from_other(self.other)
    }

    /// Whether `st_shndx` is `SHN_UNDEF`. This is a property of the record,
    /// not a claim that the symbol is or can be resolved.
    #[must_use]
    pub const fn is_undefined(&self) -> bool {
        self.section_index == 0
    }
}

/// One `Elf64_Rela` in table order. Nothing has been applied or resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Relocation {
    /// Raw `r_offset`, untranslated and without load bias.
    pub offset: u64,
    /// Raw `r_info`.
    pub info: u64,
    pub addend: i64,
}

impl Relocation {
    /// The high 32 bits of `r_info`. Zero is `STN_UNDEF`.
    #[must_use]
    pub const fn symbol_index(self) -> u32 {
        (self.info >> 32) as u32
    }

    /// The low 32 bits of `r_info`, a processor-specific type kept verbatim.
    #[must_use]
    pub const fn kind(self) -> u32 {
        self.info as u32
    }

    #[must_use]
    pub const fn references_symbol(self) -> bool {
        self.symbol_index() != 0
    }
}

/// Decode a complete symbol table, copying each referenced name once per
/// symbol and charging it to `budget`.
pub(crate) fn decode_symbols(
    table: &[u8],
    strings: Option<&StringTable<'_>>,
    budget: &mut NameBudget,
) -> Result<Vec<Symbol>, LinkError> {
    let (records, remainder) = table.as_chunks::<RECORD_SIZE>();
    if !remainder.is_empty() {
        return Err(partial(TableKind::Symbols, table));
    }
    let mut symbols = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let name_offset = u32::from_le_bytes(field(record, 0));
        let name = if name_offset == 0 {
            None
        } else {
            let strings = strings.ok_or(LinkError::MissingStringTable {
                symbol: index,
                name_offset,
            })?;
            let bytes = strings.name(index, name_offset, budget)?;
            budget.charge(index, name_offset, bytes.len())?;
            Some(SymbolName(bytes.into()))
        };
        symbols.push(Symbol {
            name_offset,
            name,
            info: record[4],
            other: record[5],
            section_index: u16::from_le_bytes(field(record, 6)),
            value: u64::from_le_bytes(field(record, 8)),
            size: u64::from_le_bytes(field(record, 16)),
        });
    }
    Ok(symbols)
}

/// Decode a complete RELA table. Nonzero symbol indices are checked against
/// `symbol_count`, which is `None` when the image has no symbol table. Index
/// zero (`STN_UNDEF`) needs no table. Nothing is resolved.
pub(crate) fn decode_relocations(
    kind: TableKind,
    table: &[u8],
    symbol_count: Option<usize>,
) -> Result<Vec<Relocation>, LinkError> {
    let (records, remainder) = table.as_chunks::<RECORD_SIZE>();
    if !remainder.is_empty() {
        return Err(partial(kind, table));
    }
    let mut relocations = Vec::with_capacity(records.len());
    for (index, record) in records.iter().enumerate() {
        let relocation = Relocation {
            offset: u64::from_le_bytes(field(record, 0)),
            info: u64::from_le_bytes(field(record, 8)),
            addend: i64::from_le_bytes(field(record, 16)),
        };
        let symbol = relocation.symbol_index();
        if symbol != 0 {
            let symbol_count = symbol_count.ok_or(LinkError::MissingSymbolTable {
                table: kind,
                relocation: index,
                symbol,
            })?;
            if u64::from(symbol) >= symbol_count as u64 {
                return Err(LinkError::SymbolIndexOutOfRange {
                    table: kind,
                    relocation: index,
                    symbol,
                    symbol_count,
                });
            }
        }
        relocations.push(relocation);
    }
    Ok(relocations)
}

fn partial(table: TableKind, bytes: &[u8]) -> LinkError {
    LinkError::PartialRecord {
        table,
        size: bytes.len() as u64,
        entry_size: RECORD_SIZE as u64,
    }
}

/// Copy a fixed-width field. Callers pass constant, format-defined offsets
/// that lie inside the 24-byte record, never untrusted values.
fn field<const N: usize>(record: &[u8; RECORD_SIZE], at: usize) -> [u8; N] {
    let mut bytes = [0; N];
    bytes.copy_from_slice(&record[at..at + N]);
    bytes
}

#[cfg(test)]
mod visibility_tests {
    use super::SymbolVisibility;

    #[test]
    fn visibility_retains_all_gabi_values_and_unnamed_seven() {
        for (other, expected, name) in [
            (4, SymbolVisibility::EXPORTED, "EXPORTED"),
            (0xfd, SymbolVisibility::SINGLETON, "SINGLETON"),
            (0xfe, SymbolVisibility::ELIMINATE, "ELIMINATE"),
        ] {
            let visibility = SymbolVisibility::from_other(other);
            assert_eq!(visibility, expected);
            assert_eq!(visibility.name(), Some(name));
        }
        assert_eq!(SymbolVisibility::from_other(0xff).raw(), 7);
        assert_eq!(SymbolVisibility::from_other(0xff).name(), None);
        assert_eq!(
            SymbolVisibility::from_other(0xf8),
            SymbolVisibility::DEFAULT
        );
    }
}
