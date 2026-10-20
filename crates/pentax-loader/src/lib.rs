// SPDX-License-Identifier: GPL-2.0-or-later

//! Checked, borrowing parser for little-endian x86-64 ELF images.
//!
//! Parsing exposes metadata and the initialized bytes of each load segment. It
//! does not allocate guest memory, resolve imports, decrypt containers, or execute
//! instructions. Section headers are intentionally outside this module's scope.
//!
//! Layout and validation follow the ELF specification's
//! [ELF header](https://gabi.xinuos.com/elf/02-eheader.html) and
//! [program header](https://gabi.xinuos.com/elf/07-pheader.html) descriptions.
//! SCE type values and SELF identification were cross-checked against
//! [shadPS4's format declarations](https://github.com/shadps4-emu/shadPS4/blob/main/src/core/loader/elf.h).
//! The implementation is original Rust and has no external dependencies.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;

mod executable;

pub use executable::{
    DynamicAddress, DynamicEntry, DynamicError, DynamicRegion, DynamicTable, ExecutableFormat,
    ExecutableImage, ExecutableMetadata, ExecutableView, HeaderDisposition, HeaderPurpose,
    MetadataPurpose, ProgramHeaderMetadata, RUNTIME_PAGE_SIZE, ResolvedPayload, RuntimeError,
    RuntimeLoadPlan, SelfError, SelfMetadata, SelfSegmentMetadata, TlsTemplate, VirtualRange,
    inspect_executable, open_executable, parse_dynamic, parse_executable,
};

const ELF_MAGIC: [u8; 4] = *b"\x7fELF";
const SELF_MAGIC: [u8; 4] = [0x4f, 0x15, 0x3d, 0x1d];
const ALTERNATE_SELF_MAGIC: [u8; 4] = [0x54, 0x14, 0xf5, 0xee];
const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const EXTENDED_PROGRAM_HEADER_COUNT: u16 = 0xffff;

/// A program header describing bytes to place in guest memory.
pub const PT_LOAD: u32 = 1;
/// A program header describing dynamic linking information.
pub const PT_DYNAMIC: u32 = 2;

/// Supported executable or shared-object ELF types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ElfType {
    /// Standard ELF executable (`ET_EXEC`).
    Executable = 2,
    /// Standard ELF shared object (`ET_DYN`).
    SharedObject = 3,
    /// SCE executable (`ET_SCE_EXEC`).
    SceExecutable = 0xfe00,
    /// SCE dynamic executable (`ET_SCE_DYNEXEC`).
    SceDynamicExecutable = 0xfe10,
    /// SCE shared object (`ET_SCE_DYNAMIC`).
    SceSharedObject = 0xfe18,
}

impl ElfType {
    /// The on-disk value of `e_type`.
    #[must_use]
    pub const fn raw(self) -> u16 {
        self as u16
    }
}

impl TryFrom<u16> for ElfType {
    type Error = LoadError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            2 => Ok(Self::Executable),
            3 => Ok(Self::SharedObject),
            0xfe00 => Ok(Self::SceExecutable),
            0xfe10 => Ok(Self::SceDynamicExecutable),
            0xfe18 => Ok(Self::SceSharedObject),
            other => Err(LoadError::UnsupportedElfType(other)),
        }
    }
}

/// Raw ELF segment flags, retaining OS- and processor-specific bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SegmentFlags(u32);

impl SegmentFlags {
    /// Retain the complete on-disk `p_flags` value.
    #[must_use]
    pub const fn from_raw(value: u32) -> Self {
        Self(value)
    }

    /// The on-disk flags, including bits unknown to this parser.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Whether `PF_R` permits reading.
    #[must_use]
    pub const fn readable(self) -> bool {
        self.0 & 4 != 0
    }

    /// Whether `PF_W` permits writing.
    #[must_use]
    pub const fn writable(self) -> bool {
        self.0 & 2 != 0
    }

    /// Whether `PF_X` permits instruction fetches.
    #[must_use]
    pub const fn executable(self) -> bool {
        self.0 & 1 != 0
    }
}

/// One program header in its original table order.
///
/// All numeric values are preserved. Non-null headers have checked file ranges.
/// `PT_NULL` members have no defined meaning, so their bytes are always empty and
/// their numeric values are not interpreted or validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramHeader<'a> {
    /// Raw `p_type`, including headers the parser does not interpret.
    pub kind: u32,
    pub flags: SegmentFlags,
    pub file_offset: u64,
    pub vaddr: u64,
    /// `p_paddr`; it is retained without interpreting host physical addresses.
    pub physical_address: u64,
    pub file_size: u64,
    pub mem_size: u64,
    pub align: u64,
    /// Initialized file contents. Empty for null or zero-size headers.
    pub file_bytes: &'a [u8],
}

/// A validated `PT_LOAD` segment whose initialized bytes borrow the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadSegment<'a> {
    pub vaddr: u64,
    pub mem_size: u64,
    pub file_offset: u64,
    /// Initialize the beginning of the segment with these bytes. A future guest
    /// mapper must zero the remaining `mem_size - file_bytes.len()` bytes.
    pub file_bytes: &'a [u8],
    pub flags: SegmentFlags,
    pub align: u64,
}

impl LoadSegment<'_> {
    /// Bytes that must be zero-initialized after the file-backed contents.
    ///
    /// Parsed segments always return `Some`. Manually constructed metadata with
    /// more initialized bytes than memory returns `None` rather than underflowing.
    #[must_use]
    pub fn zero_fill_size(&self) -> Option<u64> {
        self.mem_size.checked_sub(self.file_bytes.len() as u64)
    }
}

/// Metadata parsed without modifying the source file or allocating guest memory.
///
/// A successful parse is evidence of structural validity for the supported
/// subset, not evidence that a program can execute. Images with no loadable
/// segments are accepted for inspection; execution policy belongs to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElfImage<'a> {
    pub entry: u64,
    pub elf_type: ElfType,
    pub os_abi: u8,
    pub abi_version: u8,
    /// Loadable segments, preserving program table order.
    pub segments: Vec<LoadSegment<'a>>,
    /// All headers, including dynamic and SCE-specific metadata.
    pub program_headers: Vec<ProgramHeader<'a>>,
}

/// Why an image cannot be parsed by this loader.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    InvalidMagic,
    /// A SELF container was recognized; extracting or decrypting it is unsupported.
    UnsupportedSelf,
    InvalidSelf(SelfError),
    Runtime(RuntimeError),
    PrefixLongerThanFile {
        prefix_len: u64,
        file_len: u64,
    },
    UnsupportedClass(u8),
    UnsupportedEndianness(u8),
    UnsupportedIdentificationVersion(u8),
    UnsupportedVersion(u32),
    UnsupportedMachine(u16),
    UnsupportedElfType(u16),
    InvalidHeaderSize(u16),
    InvalidProgramHeaderSize(u16),
    /// `PN_XNUM` needs section-header-based count resolution, which is unsupported.
    UnsupportedExtendedProgramHeaderCount,
    ProgramHeaderTableOverlapsHeader {
        offset: u64,
    },
    Truncated {
        context: &'static str,
        offset: u64,
        size: u64,
        file_len: u64,
    },
    RangeOverflow {
        context: &'static str,
        offset: u64,
        size: u64,
    },
    SegmentFileSizeExceedsMemory {
        index: usize,
        file_size: u64,
        mem_size: u64,
    },
    InvalidSegmentAlignment {
        index: usize,
        align: u64,
    },
    MisalignedSegment {
        index: usize,
        vaddr: u64,
        file_offset: u64,
        align: u64,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "input has no ELF signature"),
            Self::UnsupportedSelf => write!(
                f,
                "SELF is unsupported by the ELF-only parser; use parse_executable or inspect_executable"
            ),
            Self::InvalidSelf(error) => write!(f, "SELF: {error}"),
            Self::Runtime(error) => write!(f, "runtime image: {error}"),
            Self::PrefixLongerThanFile {
                prefix_len,
                file_len,
            } => write!(
                f,
                "inspection prefix length {prefix_len:#x} exceeds declared input length {file_len:#x}"
            ),
            Self::UnsupportedClass(value) => {
                write!(f, "ELF class {value} is unsupported; expected ELF64")
            }
            Self::UnsupportedEndianness(value) => write!(
                f,
                "ELF encoding {value} is unsupported; expected little-endian"
            ),
            Self::UnsupportedIdentificationVersion(value) => {
                write!(f, "ELF identification version {value} is unsupported")
            }
            Self::UnsupportedVersion(value) => write!(f, "ELF version {value} is unsupported"),
            Self::UnsupportedMachine(value) => {
                write!(
                    f,
                    "ELF machine {value} is unsupported; expected x86-64 (62)"
                )
            }
            Self::UnsupportedElfType(value) => write!(f, "ELF type {value:#06x} is unsupported"),
            Self::InvalidHeaderSize(value) => write!(
                f,
                "ELF header size is {value}; expected {ELF_HEADER_SIZE} bytes"
            ),
            Self::InvalidProgramHeaderSize(value) => write!(
                f,
                "ELF program header size is {value}; expected {PROGRAM_HEADER_SIZE} bytes"
            ),
            Self::UnsupportedExtendedProgramHeaderCount => {
                write!(
                    f,
                    "ELF extended program header counts (PN_XNUM) are unsupported"
                )
            }
            Self::ProgramHeaderTableOverlapsHeader { offset } => {
                write!(
                    f,
                    "ELF program header table at {offset:#x} overlaps the ELF header"
                )
            }
            Self::Truncated {
                context,
                offset,
                size,
                file_len,
            } => write!(
                f,
                "{context} range at {offset:#x} of {size:#x} bytes exceeds file length {file_len:#x}"
            ),
            Self::RangeOverflow {
                context,
                offset,
                size,
            } => write!(
                f,
                "{context} range at {offset:#x} of {size:#x} bytes overflows a 64-bit address"
            ),
            Self::SegmentFileSizeExceedsMemory {
                index,
                file_size,
                mem_size,
            } => write!(
                f,
                "load segment {index} has file size {file_size:#x} exceeding memory size {mem_size:#x}"
            ),
            Self::InvalidSegmentAlignment { index, align } => write!(
                f,
                "load segment {index} alignment {align:#x} must be 0, 1, or a power of two"
            ),
            Self::MisalignedSegment {
                index,
                vaddr,
                file_offset,
                align,
            } => write!(
                f,
                "load segment {index} address {vaddr:#x} and file offset {file_offset:#x} are incongruent modulo {align:#x}"
            ),
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidSelf(error) => Some(error),
            Self::Runtime(error) => Some(error),
            _ => None,
        }
    }
}

/// Parse ELF64 little-endian x86-64 metadata and validate its referenced ranges.
///
/// The parser does not inspect section headers or validate an entry point against
/// executable mappings. These checks belong to later format and runtime stages.
/// Load segment overlap and guest page-size policy also belong to the mapper.
///
/// # Errors
///
/// Returns [`LoadError`] for truncated input, unsupported formats, overflowing
/// ranges, malformed `PT_LOAD` sizes, or invalid segment alignment. SELF
/// containers and extended program-header counts are rejected explicitly.
pub fn parse(bytes: &[u8]) -> Result<ElfImage<'_>, LoadError> {
    let magic = file_slice(bytes, 0, 4, "file signature")?;
    if magic == SELF_MAGIC || magic == ALTERNATE_SELF_MAGIC {
        return Err(LoadError::UnsupportedSelf);
    }
    if magic != ELF_MAGIC {
        return Err(LoadError::InvalidMagic);
    }

    let header = file_slice(bytes, 0, ELF_HEADER_SIZE as u64, "ELF header")?;
    // The complete 64-byte header was checked above. All field reads below use
    // fixed, format-defined offsets, rather than untrusted array indices.
    if header[4] != 2 {
        return Err(LoadError::UnsupportedClass(header[4]));
    }
    if header[5] != 1 {
        return Err(LoadError::UnsupportedEndianness(header[5]));
    }
    if header[6] != 1 {
        return Err(LoadError::UnsupportedIdentificationVersion(header[6]));
    }

    let elf_type = ElfType::try_from(read_u16(header, 16))?;
    let machine = read_u16(header, 18);
    if machine != 62 {
        return Err(LoadError::UnsupportedMachine(machine));
    }
    let version = read_u32(header, 20);
    if version != 1 {
        return Err(LoadError::UnsupportedVersion(version));
    }
    let header_size = read_u16(header, 52);
    if usize::from(header_size) != ELF_HEADER_SIZE {
        return Err(LoadError::InvalidHeaderSize(header_size));
    }

    let ph_offset = read_u64(header, 32);
    let ph_size = read_u16(header, 54);
    let ph_count = read_u16(header, 56);
    if ph_count == EXTENDED_PROGRAM_HEADER_COUNT {
        return Err(LoadError::UnsupportedExtendedProgramHeaderCount);
    }
    if ph_count != 0 {
        if usize::from(ph_size) != PROGRAM_HEADER_SIZE {
            return Err(LoadError::InvalidProgramHeaderSize(ph_size));
        }
        if ph_offset < ELF_HEADER_SIZE as u64 {
            return Err(LoadError::ProgramHeaderTableOverlapsHeader { offset: ph_offset });
        }
    }

    // The operands are both 16-bit, so multiplication cannot overflow u64.
    let table_size = u64::from(ph_size) * u64::from(ph_count);
    let table = file_slice(bytes, ph_offset, table_size, "program header table")?;
    let mut program_headers = Vec::with_capacity(usize::from(ph_count));
    let mut segments = Vec::new();

    for (index, raw_header) in table.chunks_exact(PROGRAM_HEADER_SIZE).enumerate() {
        // Each chunk is exactly 56 bytes. No guest offsets are used as indices.
        let kind = read_u32(raw_header, 0);
        let flags = SegmentFlags::from_raw(read_u32(raw_header, 4));
        let file_offset = read_u64(raw_header, 8);
        let vaddr = read_u64(raw_header, 16);
        let physical_address = read_u64(raw_header, 24);
        let file_size = read_u64(raw_header, 32);
        let mem_size = read_u64(raw_header, 40);
        let align = read_u64(raw_header, 48);

        if kind == PT_LOAD {
            validate_load_segment(index, file_offset, vaddr, file_size, mem_size, align)?;
        }
        let file_bytes = if kind == 0 {
            // All PT_NULL values except p_type are undefined by the format.
            &bytes[..0]
        } else {
            file_slice(bytes, file_offset, file_size, "program segment")?
        };

        program_headers.push(ProgramHeader {
            kind,
            flags,
            file_offset,
            vaddr,
            physical_address,
            file_size,
            mem_size,
            align,
            file_bytes,
        });
        if kind == PT_LOAD {
            segments.push(LoadSegment {
                vaddr,
                mem_size,
                file_offset,
                file_bytes,
                flags,
                align,
            });
        }
    }

    Ok(ElfImage {
        entry: read_u64(header, 24),
        elf_type,
        os_abi: header[7],
        abi_version: header[8],
        segments,
        program_headers,
    })
}

fn validate_load_segment(
    index: usize,
    file_offset: u64,
    vaddr: u64,
    file_size: u64,
    mem_size: u64,
    align: u64,
) -> Result<(), LoadError> {
    validate_load_segment_with_policy(index, file_offset, vaddr, file_size, mem_size, align, true)
}

fn validate_load_segment_with_policy(
    index: usize,
    file_offset: u64,
    vaddr: u64,
    file_size: u64,
    mem_size: u64,
    align: u64,
    require_file_congruence: bool,
) -> Result<(), LoadError> {
    if file_size > mem_size {
        return Err(LoadError::SegmentFileSizeExceedsMemory {
            index,
            file_size,
            mem_size,
        });
    }
    vaddr
        .checked_add(mem_size)
        .ok_or(LoadError::RangeOverflow {
            context: "load segment memory",
            offset: vaddr,
            size: mem_size,
        })?;
    if align > 1 {
        if !align.is_power_of_two() {
            return Err(LoadError::InvalidSegmentAlignment { index, align });
        }
        if require_file_congruence && vaddr % align != file_offset % align {
            return Err(LoadError::MisalignedSegment {
                index,
                vaddr,
                file_offset,
                align,
            });
        }
    }
    Ok(())
}

fn file_slice<'a>(
    bytes: &'a [u8],
    offset: u64,
    size: u64,
    context: &'static str,
) -> Result<&'a [u8], LoadError> {
    // Empty ranges access no bytes. A pure BSS segment can legally describe no
    // initialized bytes even when its file offset lies beyond the end of a file.
    if size == 0 {
        return Ok(&bytes[..0]);
    }
    let end = offset.checked_add(size).ok_or(LoadError::RangeOverflow {
        context,
        offset,
        size,
    })?;
    let file_len = bytes.len() as u64;
    if end > file_len {
        return Err(LoadError::Truncated {
            context,
            offset,
            size,
            file_len,
        });
    }
    // Both values are <= bytes.len(), so conversion is safe even on 32-bit hosts.
    let start = usize::try_from(offset).map_err(|_| LoadError::Truncated {
        context,
        offset,
        size,
        file_len,
    })?;
    let end = usize::try_from(end).map_err(|_| LoadError::Truncated {
        context,
        offset,
        size,
        file_len,
    })?;
    Ok(&bytes[start..end])
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

#[cfg(test)]
mod tests;
