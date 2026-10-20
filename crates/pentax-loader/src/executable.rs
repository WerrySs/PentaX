// SPDX-License-Identifier: GPL-2.0-or-later

//! Container inspection and zero-copy access to unencrypted SELF payloads.
//!
//! SELF layout and descriptor-based range translation were independently
//! implemented from observed format declarations and behavior in the primary
//! [KytyPS5 loader](https://github.com/KytyPS5/KytyPS5/blob/master/src/loader/elf.cpp).
//! Protection flag meanings are corroborated by
//! [shadPS4's declarations](https://github.com/shadps4-emu/shadPS4/blob/main/src/core/loader/elf.h).
//! The narrow SCE ABI 2 permissionless-segment copy policy is corroborated by
//! [KytyPS5's runtime linker](https://github.com/KytyPS5/KytyPS5/blob/master/src/loader/runtimeLinker.cpp).
//! No keys, decryption, decompression, signatures, or console code are included.

use std::error::Error;
use std::fmt;

use super::{
    ALTERNATE_SELF_MAGIC, ELF_HEADER_SIZE, ELF_MAGIC, EXTENDED_PROGRAM_HEADER_COUNT, ElfImage,
    ElfType, LoadError, LoadSegment, PROGRAM_HEADER_SIZE, PT_LOAD, ProgramHeader, SELF_MAGIC,
    SegmentFlags, file_slice, parse, read_u16, read_u32, read_u64,
    validate_load_segment_with_policy,
};

const SELF_HEADER_SIZE: u64 = 32;
const SELF_SEGMENT_SIZE: usize = 32;
const PT_SCE_LIBVERSION: u32 = 0x6fffff01;

mod runtime;
pub use runtime::*;

/// On-disk container, without a claim about execution compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutableFormat {
    Elf,
    SelfContainer,
}

/// Numeric ELF program-header values, suitable for bounded header inspection.
///
/// This contains no payload slices. It can describe headers whose payloads were
/// not included in the inspection prefix or cannot yet be resolved from SELF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgramHeaderMetadata {
    pub kind: u32,
    pub flags: SegmentFlags,
    pub file_offset: u64,
    pub vaddr: u64,
    pub physical_address: u64,
    pub file_size: u64,
    pub mem_size: u64,
    pub align: u64,
}

/// A SELF segment descriptor. File offsets refer to the container, while its
/// program-header index refers to the embedded ELF program-header table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfSegmentMetadata {
    pub flags: u64,
    pub file_offset: u64,
    pub stored_size: u64,
    pub decoded_size: u64,
}

impl SelfSegmentMetadata {
    /// Whether this descriptor carries an ELF program segment's payload.
    #[must_use]
    pub const fn is_payload(self) -> bool {
        self.flags & 0x800 != 0
    }

    #[must_use]
    pub const fn is_encrypted(self) -> bool {
        self.flags & 2 != 0
    }

    #[must_use]
    pub const fn is_compressed(self) -> bool {
        self.flags & 8 != 0
    }

    /// The embedded ELF index, only meaningful for a payload descriptor.
    #[must_use]
    pub const fn program_header_index(self) -> Option<usize> {
        if self.is_payload() {
            Some(((self.flags >> 20) & 0xfff) as usize)
        } else {
            None
        }
    }
}

/// Structural SELF metadata, retaining unknown identification and flag values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfMetadata {
    pub ident: [u8; 12],
    pub header_size: u16,
    pub metadata_size: u16,
    /// Container size declared by SELF. Additional library-version bytes may
    /// follow this boundary without being covered by a segment descriptor.
    pub file_size: u64,
    /// Derived from the descriptor count, rather than a fixed ELF offset.
    pub elf_offset: u64,
    pub segments: Vec<SelfSegmentMetadata>,
}

/// Header-only inspection. Payload protection and loadability are separate from
/// envelope validity: inspect flags to see protection; use [`parse_executable`]
/// to require resolvable, unencrypted, uncompressed ELF segment bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableMetadata {
    pub format: ExecutableFormat,
    pub entry: u64,
    pub elf_type: ElfType,
    pub os_abi: u8,
    pub abi_version: u8,
    pub program_headers: Vec<ProgramHeaderMetadata>,
    pub self_metadata: Option<SelfMetadata>,
}

/// A parsed image borrowing its payload bytes directly from ELF or SELF input.
///
/// The metadata is owned and the segment slices borrow the caller's file bytes.
/// No reconstructed ELF buffer or self-referential ownership is required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableImage<'a> {
    pub metadata: ExecutableMetadata,
    pub elf: ElfImage<'a>,
}

/// Structured reasons an identified SELF cannot be inspected or translated.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelfError {
    UnsupportedIdentification {
        ident: [u8; 12],
        marker: u16,
    },
    HeaderTooSmall {
        declared: u16,
        required: u64,
    },
    InvalidFileSize {
        declared: u64,
        minimum: u64,
    },
    SegmentOverlapsHeaders {
        segment_index: usize,
        offset: u64,
        minimum: u64,
    },
    ProgramHeaderIndexOutOfBounds {
        segment_index: usize,
        program_header_index: usize,
        program_header_count: usize,
    },
    DuplicatePayload {
        program_header_index: usize,
    },
    EncryptedSegment {
        segment_index: usize,
    },
    CompressedSegment {
        segment_index: usize,
    },
    PayloadSizeMismatch {
        segment_index: usize,
        program_header_index: usize,
        stored_size: u64,
        decoded_size: u64,
        elf_size: u64,
    },
    MissingSegment {
        program_header_index: usize,
        file_offset: u64,
        size: u64,
    },
    AmbiguousSegment {
        program_header_index: usize,
    },
}

impl fmt::Display for SelfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedIdentification { ident, marker } => write!(
                f,
                "unsupported identification {ident:02x?} and marker {marker:#x}"
            ),
            Self::HeaderTooSmall { declared, required } => write!(
                f,
                "header size {declared:#x} cannot contain the ELF headers ending at {required:#x}"
            ),
            Self::InvalidFileSize { declared, minimum } => write!(
                f,
                "declared file size {declared:#x} is smaller than header and metadata size {minimum:#x}"
            ),
            Self::SegmentOverlapsHeaders {
                segment_index,
                offset,
                minimum,
            } => write!(
                f,
                "segment {segment_index} at {offset:#x} overlaps header or metadata ending at {minimum:#x}"
            ),
            Self::ProgramHeaderIndexOutOfBounds {
                segment_index,
                program_header_index,
                program_header_count,
            } => write!(
                f,
                "segment {segment_index} references ELF program header {program_header_index}, but only {program_header_count} exist"
            ),
            Self::DuplicatePayload {
                program_header_index,
            } => write!(
                f,
                "multiple payload descriptors reference ELF program header {program_header_index}"
            ),
            Self::EncryptedSegment { segment_index } => {
                write!(
                    f,
                    "encrypted payload segment {segment_index} is unsupported"
                )
            }
            Self::CompressedSegment { segment_index } => {
                write!(
                    f,
                    "compressed payload segment {segment_index} is unsupported"
                )
            }
            Self::PayloadSizeMismatch {
                segment_index,
                program_header_index,
                stored_size,
                decoded_size,
                elf_size,
            } => write!(
                f,
                "segment {segment_index} sizes (stored {stored_size:#x}, decoded {decoded_size:#x}) do not match ELF program header {program_header_index} size {elf_size:#x}"
            ),
            Self::MissingSegment {
                program_header_index,
                file_offset,
                size,
            } => write!(
                f,
                "no supported payload mapping for ELF program header {program_header_index} at logical offset {file_offset:#x}, size {size:#x}"
            ),
            Self::AmbiguousSegment {
                program_header_index,
            } => write!(
                f,
                "ELF program header {program_header_index} resolves to different SELF payload ranges"
            ),
        }
    }
}

impl Error for SelfError {}

impl From<SelfError> for LoadError {
    fn from(error: SelfError) -> Self {
        Self::InvalidSelf(error)
    }
}

/// Inspect an ELF or supported SELF using only the beginning of a file.
///
/// `prefix` must contain the ELF header and complete program-header table (and,
/// for SELF, its header and descriptor table). `file_len` is the actual full file
/// length obtained from file metadata, not the prefix length. Payload ranges are
/// checked against this length without reading their contents. This function
/// performs no I/O and never extracts container bytes to disk.
///
/// Inspection preserves protection flags and does not claim that load segment
/// alignment, payload translation, signatures, or entry points are executable.
/// Section headers are not read, including section tables outside the prefix.
///
/// # Errors
///
/// Returns [`LoadError`] for an incomplete prefix, malformed envelope or header
/// ranges, unsupported ELF encoding, or an unknown SELF identification variant.
pub fn inspect_executable(prefix: &[u8], file_len: u64) -> Result<ExecutableMetadata, LoadError> {
    if prefix.len() as u64 > file_len {
        return Err(LoadError::PrefixLongerThanFile {
            prefix_len: prefix.len() as u64,
            file_len,
        });
    }
    let magic = file_slice(prefix, 0, 4, "file signature")?;
    if magic == SELF_MAGIC || magic == ALTERNATE_SELF_MAGIC {
        inspect_self(prefix, file_len)
    } else {
        let (mut metadata, _) = inspect_elf(prefix, 0, file_len)?;
        for header in &metadata.program_headers {
            if header.kind != 0 {
                check_file_range(
                    file_len,
                    header.file_offset,
                    header.file_size,
                    "program segment",
                )?;
            }
        }
        metadata.format = ExecutableFormat::Elf;
        Ok(metadata)
    }
}

/// Parse ELF or map unencrypted, uncompressed SELF payloads without copying them.
///
/// Translation uses SELF descriptors and containment in their indexed ELF
/// program headers. Merely slicing input at the embedded ELF header is incorrect
/// because ELF logical offsets and SELF physical offsets differ. Non-loadable
/// metadata must also resolve; missing notes or dynamic data are explicit errors.
/// File/address congruence is required except for permissionless SCE ABI 2 SELF
/// load segments handled through the documented auxiliary copy policy. Their
/// alignment must still be 0, 1, or a power of two, and all ranges remain checked.
///
/// # Errors
///
/// Returns [`LoadError`] for malformed ELF/SELF, unresolved or ambiguous mappings,
/// and [`SelfError::EncryptedSegment`] or [`SelfError::CompressedSegment`] for
/// protected payloads. No decryption, decompression, or execution is attempted.
pub fn parse_executable(bytes: &[u8]) -> Result<ExecutableImage<'_>, LoadError> {
    let metadata = inspect_executable(bytes, bytes.len() as u64)?;
    if metadata.format == ExecutableFormat::Elf {
        return Ok(ExecutableImage {
            metadata,
            elf: parse(bytes)?,
        });
    }
    // The format was determined by inspect_executable, which always sets this
    // field for SELF. Keep the public boundary free of panic assumptions.
    let container = metadata
        .self_metadata
        .as_ref()
        .ok_or(LoadError::UnsupportedSelf)?;
    validate_payload_descriptors(container, &metadata.program_headers)?;

    let mut program_headers = Vec::with_capacity(metadata.program_headers.len());
    let mut segments = Vec::new();
    for (index, header) in metadata.program_headers.iter().enumerate() {
        if header.kind == PT_LOAD {
            validate_load_segment_with_policy(
                index,
                header.file_offset,
                header.vaddr,
                header.file_size,
                header.mem_size,
                header.align,
                !uses_sce_auxiliary_copy_semantics(&metadata, header),
            )?;
        }
        let file_bytes = if header.kind == 0 || header.file_size == 0 {
            &bytes[..0]
        } else {
            resolve_payload(bytes, index, header, container, &metadata.program_headers)?
        };
        program_headers.push(ProgramHeader {
            kind: header.kind,
            flags: header.flags,
            file_offset: header.file_offset,
            vaddr: header.vaddr,
            physical_address: header.physical_address,
            file_size: header.file_size,
            mem_size: header.mem_size,
            align: header.align,
            file_bytes,
        });
        if header.kind == PT_LOAD {
            segments.push(LoadSegment {
                vaddr: header.vaddr,
                mem_size: header.mem_size,
                file_offset: header.file_offset,
                file_bytes,
                flags: header.flags,
                align: header.align,
            });
        }
    }
    let elf = ElfImage {
        entry: metadata.entry,
        elf_type: metadata.elf_type,
        os_abi: metadata.os_abi,
        abi_version: metadata.abi_version,
        segments,
        program_headers,
    };
    Ok(ExecutableImage { metadata, elf })
}

fn inspect_self(prefix: &[u8], file_len: u64) -> Result<ExecutableMetadata, LoadError> {
    let header = file_slice(prefix, 0, SELF_HEADER_SIZE, "SELF header")?;
    let mut ident = [0; 12];
    ident.copy_from_slice(&header[..12]);
    let marker = read_u16(header, 26);
    let classic = header[4..12] == [0, 1, 1, 0x12, 1, 1, 0, 0] && marker == 0x22;
    let alternate = header[4..12] == [0x10, 1, 1, 0x12, 1, 1, 0, 0x10] && marker == 0x32;
    if !classic && !alternate {
        return Err(SelfError::UnsupportedIdentification { ident, marker }.into());
    }

    let header_size = read_u16(header, 12);
    let metadata_size = read_u16(header, 14);
    let declared_file_size = read_u64(header, 16);
    let descriptor_count = read_u16(header, 24);
    let descriptor_size = u64::from(descriptor_count) * SELF_SEGMENT_SIZE as u64;
    let elf_offset = SELF_HEADER_SIZE + descriptor_size;
    let payload_start = u64::from(header_size) + u64::from(metadata_size);
    if declared_file_size < payload_start {
        return Err(SelfError::InvalidFileSize {
            declared: declared_file_size,
            minimum: payload_start,
        }
        .into());
    }
    check_file_range(file_len, 0, declared_file_size, "SELF declared file")?;
    check_file_range(
        file_len,
        SELF_HEADER_SIZE,
        descriptor_size,
        "SELF descriptor table",
    )?;
    let descriptors = file_slice(
        prefix,
        SELF_HEADER_SIZE,
        descriptor_size,
        "SELF descriptor table",
    )?;
    let (mut metadata, elf_headers_end) = inspect_elf(prefix, elf_offset, file_len)?;
    if u64::from(header_size) < elf_headers_end {
        return Err(SelfError::HeaderTooSmall {
            declared: header_size,
            required: elf_headers_end,
        }
        .into());
    }

    let mut segments = Vec::with_capacity(usize::from(descriptor_count));
    let mut referenced = vec![false; metadata.program_headers.len()];
    for (index, descriptor) in descriptors.chunks_exact(SELF_SEGMENT_SIZE).enumerate() {
        let segment = SelfSegmentMetadata {
            flags: read_u64(descriptor, 0),
            file_offset: read_u64(descriptor, 8),
            stored_size: read_u64(descriptor, 16),
            decoded_size: read_u64(descriptor, 24),
        };
        if segment.stored_size != 0 && segment.file_offset < payload_start {
            return Err(SelfError::SegmentOverlapsHeaders {
                segment_index: index,
                offset: segment.file_offset,
                minimum: payload_start,
            }
            .into());
        }
        check_file_range(
            declared_file_size,
            segment.file_offset,
            segment.stored_size,
            "SELF segment",
        )?;
        if let Some(ph_index) = segment.program_header_index() {
            let ph_count = referenced.len();
            let already_referenced =
                referenced
                    .get_mut(ph_index)
                    .ok_or(SelfError::ProgramHeaderIndexOutOfBounds {
                        segment_index: index,
                        program_header_index: ph_index,
                        program_header_count: ph_count,
                    })?;
            if *already_referenced {
                return Err(SelfError::DuplicatePayload {
                    program_header_index: ph_index,
                }
                .into());
            }
            *already_referenced = true;
        }
        segments.push(segment);
    }
    metadata.format = ExecutableFormat::SelfContainer;
    metadata.self_metadata = Some(SelfMetadata {
        ident,
        header_size,
        metadata_size,
        file_size: declared_file_size,
        elf_offset,
        segments,
    });
    Ok(metadata)
}

fn inspect_elf(
    prefix: &[u8],
    elf_offset: u64,
    file_len: u64,
) -> Result<(ExecutableMetadata, u64), LoadError> {
    check_file_range(file_len, elf_offset, ELF_HEADER_SIZE as u64, "ELF header")?;
    let header = file_slice(prefix, elf_offset, ELF_HEADER_SIZE as u64, "ELF header")?;
    if header[..4] != ELF_MAGIC {
        return Err(LoadError::InvalidMagic);
    }
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
    let table_size = u64::from(ph_size) * u64::from(ph_count);
    // No table exists when ph_count == 0, even if an unused e_phoff is large.
    let table_start = if ph_count == 0 {
        elf_offset + ELF_HEADER_SIZE as u64
    } else {
        checked_add(elf_offset, ph_offset, "embedded program header table")?
    };
    check_file_range(file_len, table_start, table_size, "program header table")?;
    let table = file_slice(prefix, table_start, table_size, "program header table")?;
    let mut program_headers = Vec::with_capacity(usize::from(ph_count));
    for raw in table.chunks_exact(PROGRAM_HEADER_SIZE) {
        let header = ProgramHeaderMetadata {
            kind: read_u32(raw, 0),
            flags: SegmentFlags::from_raw(read_u32(raw, 4)),
            file_offset: read_u64(raw, 8),
            vaddr: read_u64(raw, 16),
            physical_address: read_u64(raw, 24),
            file_size: read_u64(raw, 32),
            mem_size: read_u64(raw, 40),
            align: read_u64(raw, 48),
        };
        if header.kind != 0 && header.file_size != 0 {
            checked_add(
                header.file_offset,
                header.file_size,
                "logical program segment",
            )?;
        }
        program_headers.push(header);
    }
    let headers_end = checked_add(table_start, table_size, "ELF headers")?;
    Ok((
        ExecutableMetadata {
            format: ExecutableFormat::Elf,
            entry: read_u64(header, 24),
            elf_type,
            os_abi: header[7],
            abi_version: header[8],
            program_headers,
            self_metadata: None,
        },
        headers_end,
    ))
}

fn validate_payload_descriptors(
    container: &SelfMetadata,
    headers: &[ProgramHeaderMetadata],
) -> Result<(), LoadError> {
    for (index, segment) in container.segments.iter().enumerate() {
        if let Some(ph_index) = segment.program_header_index() {
            if segment.is_encrypted() {
                return Err(SelfError::EncryptedSegment {
                    segment_index: index,
                }
                .into());
            }
            if segment.is_compressed() {
                return Err(SelfError::CompressedSegment {
                    segment_index: index,
                }
                .into());
            }
            // Index validation happened during inspection, before any indexing.
            let elf_size = headers[ph_index].file_size;
            if segment.stored_size != segment.decoded_size || segment.decoded_size != elf_size {
                return Err(SelfError::PayloadSizeMismatch {
                    segment_index: index,
                    program_header_index: ph_index,
                    stored_size: segment.stored_size,
                    decoded_size: segment.decoded_size,
                    elf_size,
                }
                .into());
            }
        }
    }
    Ok(())
}

fn uses_sce_auxiliary_copy_semantics(
    metadata: &ExecutableMetadata,
    header: &ProgramHeaderMetadata,
) -> bool {
    // The next-generation SCE ABI can describe auxiliary, permissionless
    // PT_LOAD bytes with incongruent logical file and virtual address offsets.
    // KytyPS5's runtimeLinker.cpp LoadProgramToMemory copies these segments,
    // uses p_align to round their memory size, and explicitly treats their
    // protection differently. File mapping congruence is therefore inappropriate
    // for this narrow SELF/SCE case. Keep size, address, and power-of-two checks;
    // preserve the raw flags rather than granting guest permissions here.
    metadata.format == ExecutableFormat::SelfContainer
        && matches!(
            metadata.elf_type,
            ElfType::SceExecutable | ElfType::SceDynamicExecutable | ElfType::SceSharedObject
        )
        && metadata.os_abi == 9
        && metadata.abi_version == 2
        && header.kind == PT_LOAD
        && header.flags.raw() == 0
}

fn resolve_payload<'a>(
    bytes: &'a [u8],
    index: usize,
    requested: &ProgramHeaderMetadata,
    container: &SelfMetadata,
    headers: &[ProgramHeaderMetadata],
) -> Result<&'a [u8], LoadError> {
    let requested_end = checked_add(requested.file_offset, requested.file_size, "ELF payload")?;
    let mut resolved_offset = None;
    for segment in &container.segments {
        let Some(ph_index) = segment.program_header_index() else {
            continue;
        };
        let source = &headers[ph_index];
        let source_end = checked_add(source.file_offset, source.file_size, "SELF source segment")?;
        if requested.file_offset >= source.file_offset && requested_end <= source_end {
            let relative = requested.file_offset - source.file_offset;
            let physical = checked_add(segment.file_offset, relative, "SELF translated payload")?;
            if resolved_offset.is_some_and(|previous| previous != physical) {
                return Err(SelfError::AmbiguousSegment {
                    program_header_index: index,
                }
                .into());
            }
            resolved_offset = Some(physical);
        }
    }
    // Kyty recognizes extra bytes after the declared container as a library
    // version payload when the sizes match. Restrict this exceptional mapping
    // to its known metadata type; never guess that arbitrary missing code or
    // dynamic data happens to be the file tail.
    if resolved_offset.is_none()
        && requested.kind == PT_SCE_LIBVERSION
        && bytes.len() as u64 - container.file_size == requested.file_size
    {
        resolved_offset = Some(container.file_size);
    }
    let offset = resolved_offset.ok_or(SelfError::MissingSegment {
        program_header_index: index,
        file_offset: requested.file_offset,
        size: requested.file_size,
    })?;
    file_slice(
        bytes,
        offset,
        requested.file_size,
        "SELF translated payload",
    )
}

fn checked_add(offset: u64, size: u64, context: &'static str) -> Result<u64, LoadError> {
    offset.checked_add(size).ok_or(LoadError::RangeOverflow {
        context,
        offset,
        size,
    })
}

fn check_file_range(
    file_len: u64,
    offset: u64,
    size: u64,
    context: &'static str,
) -> Result<(), LoadError> {
    if size == 0 {
        return Ok(());
    }
    let end = checked_add(offset, size, context)?;
    if end > file_len {
        return Err(LoadError::Truncated {
            context,
            offset,
            size,
            file_len,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
