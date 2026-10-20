// SPDX-License-Identifier: GPL-2.0-or-later

//! Runtime payload selection follows KytyPS5's LoadProgramToMemory and Elf64::Open:
//! https://github.com/KytyPS5/KytyPS5/blob/master/src/loader/runtimeLinker.cpp
//! https://github.com/KytyPS5/KytyPS5/blob/master/src/loader/elf.cpp
//! Selection is explicit; strict whole-image parsing retains its existing contract.

use std::{error::Error, fmt};

use super::{
    ExecutableMetadata, ProgramHeaderMetadata, checked_add, inspect_executable, resolve_payload,
    uses_sce_auxiliary_copy_semantics, validate_payload_descriptors,
};
use crate::{
    LoadError, LoadSegment, PT_DYNAMIC, PT_LOAD, file_slice, validate_load_segment_with_policy,
};

mod dynamic;
pub use dynamic::{
    DynamicAddress, DynamicEntry, DynamicError, DynamicRegion, DynamicTable, parse_dynamic,
};

const PT_TLS: u32 = 7;
const PT_SCE_DYNLIBDATA: u32 = 0x61000000;
const PT_SCE_PROCPARAM: u32 = 0x61000001;
// Observed module-parameter header; contents remain uninterpreted:
// https://github.com/SvenGDK/SharpProspero/blob/5dfdb4b0b166e83dc9ae275001afb695f12c7174/tools/SharpProspero.Link/DynamicWriter.cs
const PT_SCE_MODULE_PARAM: u32 = 0x61000002;
const PT_SCE_RELRO: u32 = 0x61000010;
const PT_GNU_EH_FRAME: u32 = 0x6474e550;
const PT_GNU_STACK: u32 = 0x6474e551;
const PT_GNU_RELRO: u32 = 0x6474e552;

/// Guest mapping granularity shared with downstream runtime memory mappers.
/// This is independent of a segment's ELF `p_align` constraint.
pub const RUNTIME_PAGE_SIZE: u64 = 0x4000;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RuntimeError {
    ProgramHeaderIndex { index: usize, count: usize },
    UnsupportedHeader { index: usize, kind: u32 },
    DuplicateHeader { kind: u32 },
    UnmappedVirtualRange { address: u64, size: u64 },
    AmbiguousVirtualRange { address: u64 },
    MissingDynlibData,
    Dynamic(DynamicError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProgramHeaderIndex { index, count } => {
                write!(f, "program header index {index} exceeds count {count}")
            }
            Self::UnsupportedHeader { index, kind } => write!(
                f,
                "unsupported runtime program header {index}, type {kind:#x}"
            ),
            Self::DuplicateHeader { kind } => {
                write!(f, "duplicate singleton program header type {kind:#x}")
            }
            Self::UnmappedVirtualRange { address, size } => write!(
                f,
                "virtual range {address:#x} + {size:#x} has no initialized load mapping"
            ),
            Self::AmbiguousVirtualRange { address } => write!(
                f,
                "virtual address {address:#x} has conflicting initialized mappings"
            ),
            Self::MissingDynlibData => write!(f, "SCE dynamic offsets require PT_SCE_DYNLIBDATA"),
            Self::Dynamic(error) => write!(f, "dynamic table: {error}"),
        }
    }
}

impl Error for RuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        if let Self::Dynamic(error) = self {
            Some(error)
        } else {
            None
        }
    }
}

impl From<RuntimeError> for LoadError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualRange {
    /// Original guest virtual address, before load bias.
    pub address: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPayload<'a> {
    pub program_header_index: usize,
    pub file_bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsTemplate<'a> {
    pub initialized: ResolvedPayload<'a>,
    pub mem_size: u64,
    pub align: u64,
}

/// Why a header is retained without demanding a separate payload for boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPurpose {
    Null,
    AuxiliaryNote,
    BuildInformation,
    UnwindInformation,
    ProgramHeaders,
    StackPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderPurpose {
    Load,
    Dynamic,
    DynlibData,
    Tls,
    ProcessParameters,
    ModuleParameters,
    ReadOnlyAfterRelocation,
    MetadataOnly(MetadataPurpose),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderDisposition {
    pub program_header_index: usize,
    pub purpose: HeaderPurpose,
}

/// Required payloads and explicit header roles; this does not resolve imports or
/// authorize execution. Metadata-only bytes remain available through the view's
/// fallible program_bytes method. No missing payload is represented by fake data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLoadPlan<'a> {
    pub metadata: ExecutableMetadata,
    /// PT_LOAD and standalone PT_SCE_RELRO mappings, in program-header order.
    pub segments: Vec<LoadSegment<'a>>,
    pub dynamic: Option<DynamicTable>,
    pub dynlib_data: Option<ResolvedPayload<'a>>,
    pub tls: Option<TlsTemplate<'a>>,
    pub process_parameters: Option<VirtualRange>,
    /// Module parameters backed by initialized load bytes, without decoding.
    pub module_parameters: Option<VirtualRange>,
    pub relro: Vec<VirtualRange>,
    pub dispositions: Vec<HeaderDisposition>,
}

/// Owned, immutable metadata with lazily resolved slices borrowing the caller.
/// Keeping metadata private prevents forged descriptor indices or ranges.
#[derive(Debug, Clone)]
pub struct ExecutableView<'a> {
    bytes: &'a [u8],
    metadata: ExecutableMetadata,
}

/// Open the complete input without requiring payloads for unused metadata.
/// Envelope, descriptor ranges, encryption, compression and descriptor sizes
/// remain checked globally. Alignment and payload availability are checked when
/// required by runtime_plan. Strict parse_executable remains unchanged.
pub fn open_executable(bytes: &[u8]) -> Result<ExecutableView<'_>, LoadError> {
    let metadata = inspect_executable(bytes, bytes.len() as u64)?;
    if let Some(container) = &metadata.self_metadata {
        validate_payload_descriptors(container, &metadata.program_headers)?;
    }
    Ok(ExecutableView { bytes, metadata })
}

impl<'a> ExecutableView<'a> {
    /// Complete immutable input retained by this inspected view. Reading this
    /// slice grants no file, mapping or execution authority.
    #[must_use]
    pub const fn source_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    #[must_use]
    pub fn metadata(&self) -> &ExecutableMetadata {
        &self.metadata
    }

    /// Resolve one payload strictly, including metadata-only headers.
    pub fn program_bytes(&self, index: usize) -> Result<&'a [u8], LoadError> {
        let header =
            self.metadata
                .program_headers
                .get(index)
                .ok_or(RuntimeError::ProgramHeaderIndex {
                    index,
                    count: self.metadata.program_headers.len(),
                })?;
        if header.kind == 0 || header.file_size == 0 {
            return Ok(&self.bytes[..0]);
        }
        if let Some(container) = &self.metadata.self_metadata {
            resolve_payload(
                self.bytes,
                index,
                header,
                container,
                &self.metadata.program_headers,
            )
        } else {
            file_slice(
                self.bytes,
                header.file_offset,
                header.file_size,
                "program segment",
            )
        }
    }

    /// Read original guest virtual addresses from initialized source bytes.
    /// BSS and gaps are errors. Loader access does not grant guest CPU permissions.
    /// Discontiguous physical payloads can back a contiguous virtual range.
    /// Output is unchanged if range validation fails.
    pub fn read_initialized_vaddr(&self, address: u64, output: &mut [u8]) -> Result<(), LoadError> {
        self.validate_initialized_range(address, output.len() as u64)?;
        let mut cursor = address;
        let mut written = 0;
        while written < output.len() {
            let bytes = self.initialized_chunk(cursor)?;
            let count = bytes.len().min(output.len() - written);
            output[written..written + count].copy_from_slice(&bytes[..count]);
            written += count;
            cursor = checked_add(cursor, count as u64, "virtual read")?;
        }
        Ok(())
    }

    fn validate_initialized_range(&self, address: u64, size: u64) -> Result<(), LoadError> {
        let end = checked_add(address, size, "virtual read")?;
        let mut cursor = address;
        while cursor < end {
            let chunk = self.initialized_chunk(cursor)?;
            cursor += (chunk.len() as u64).min(end - cursor);
        }
        Ok(())
    }

    fn initialized_chunk(&self, address: u64) -> Result<&'a [u8], LoadError> {
        let mut found: Option<&[u8]> = None;
        let mut next_boundary = u64::MAX;
        for (index, header) in self.metadata.program_headers.iter().enumerate() {
            if !matches!(header.kind, PT_LOAD | PT_SCE_RELRO) {
                continue;
            }
            if header.vaddr > address {
                next_boundary = next_boundary.min(header.vaddr);
            }
            let end = checked_add(header.vaddr, header.file_size, "initialized load segment")?;
            if address >= header.vaddr && address < end {
                self.validate_segment(index, header)?;
                let bytes = self.program_bytes(index)?;
                let offset = usize::try_from(address - header.vaddr)
                    .map_err(|_| RuntimeError::UnmappedVirtualRange { address, size: 1 })?;
                let chunk = &bytes[offset..];
                if let Some(previous) = found {
                    if previous.as_ptr() != chunk.as_ptr() {
                        return Err(RuntimeError::AmbiguousVirtualRange { address }.into());
                    }
                    if chunk.len() < previous.len() {
                        found = Some(chunk);
                    }
                } else {
                    found = Some(chunk);
                }
            }
        }
        let found = found.ok_or(RuntimeError::UnmappedVirtualRange { address, size: 1 })?;
        let length = (found.len() as u64).min(next_boundary - address);
        Ok(&found[..length as usize])
    }

    pub fn runtime_plan(&self) -> Result<RuntimeLoadPlan<'a>, LoadError> {
        let mut plan = RuntimeLoadPlan {
            metadata: self.metadata.clone(),
            segments: Vec::new(),
            dynamic: None,
            dynlib_data: None,
            tls: None,
            process_parameters: None,
            module_parameters: None,
            relro: Vec::new(),
            dispositions: Vec::new(),
        };
        let mut dynamic_payload = None;
        for (index, header) in self.metadata.program_headers.iter().enumerate() {
            let purpose = match header.kind {
                PT_LOAD | PT_SCE_RELRO => {
                    self.validate_segment(index, header)?;
                    plan.segments.push(LoadSegment {
                        vaddr: header.vaddr,
                        mem_size: header.mem_size,
                        file_offset: header.file_offset,
                        file_bytes: self.program_bytes(index)?,
                        flags: header.flags,
                        align: header.align,
                    });
                    HeaderPurpose::Load
                }
                PT_DYNAMIC => {
                    if dynamic_payload.is_some() {
                        return Err(RuntimeError::DuplicateHeader { kind: header.kind }.into());
                    }
                    dynamic_payload = Some(self.program_bytes(index)?);
                    HeaderPurpose::Dynamic
                }
                PT_SCE_DYNLIBDATA => {
                    if plan.dynlib_data.is_some() {
                        return Err(RuntimeError::DuplicateHeader { kind: header.kind }.into());
                    }
                    plan.dynlib_data = Some(ResolvedPayload {
                        program_header_index: index,
                        file_bytes: self.program_bytes(index)?,
                    });
                    HeaderPurpose::DynlibData
                }
                PT_TLS => {
                    if plan.tls.is_some() {
                        return Err(RuntimeError::DuplicateHeader { kind: header.kind }.into());
                    }
                    validate_load_segment_with_policy(
                        index,
                        header.file_offset,
                        header.vaddr,
                        header.file_size,
                        header.mem_size,
                        header.align,
                        false,
                    )?;
                    plan.tls = Some(TlsTemplate {
                        initialized: ResolvedPayload {
                            program_header_index: index,
                            file_bytes: self.program_bytes(index)?,
                        },
                        mem_size: header.mem_size,
                        align: header.align,
                    });
                    HeaderPurpose::Tls
                }
                PT_SCE_PROCPARAM => {
                    if plan.process_parameters.is_some() {
                        return Err(RuntimeError::DuplicateHeader { kind: header.kind }.into());
                    }
                    self.validate_initialized_range(header.vaddr, header.file_size)?;
                    plan.process_parameters = Some(VirtualRange {
                        address: header.vaddr,
                        size: header.file_size,
                    });
                    HeaderPurpose::ProcessParameters
                }
                PT_SCE_MODULE_PARAM => {
                    if plan.module_parameters.is_some() {
                        return Err(RuntimeError::DuplicateHeader { kind: header.kind }.into());
                    }
                    self.validate_initialized_range(header.vaddr, header.file_size)?;
                    plan.module_parameters = Some(VirtualRange {
                        address: header.vaddr,
                        size: header.file_size,
                    });
                    HeaderPurpose::ModuleParameters
                }
                PT_GNU_RELRO => {
                    checked_add(header.vaddr, header.mem_size, "RELRO")?;
                    self.validate_mapped_range(header.vaddr, header.mem_size)?;
                    plan.relro.push(VirtualRange {
                        address: header.vaddr,
                        size: header.mem_size,
                    });
                    HeaderPurpose::ReadOnlyAfterRelocation
                }
                0 => HeaderPurpose::MetadataOnly(MetadataPurpose::Null),
                4 => HeaderPurpose::MetadataOnly(MetadataPurpose::AuxiliaryNote),
                6 => HeaderPurpose::MetadataOnly(MetadataPurpose::ProgramHeaders),
                PT_GNU_EH_FRAME => HeaderPurpose::MetadataOnly(MetadataPurpose::UnwindInformation),
                PT_GNU_STACK => HeaderPurpose::MetadataOnly(MetadataPurpose::StackPolicy),
                0x6fffff00 | 0x6fffff01 => {
                    HeaderPurpose::MetadataOnly(MetadataPurpose::BuildInformation)
                }
                kind => return Err(RuntimeError::UnsupportedHeader { index, kind }.into()),
            };
            plan.dispositions.push(HeaderDisposition {
                program_header_index: index,
                purpose,
            });
        }
        if let Some(payload) = dynamic_payload {
            let dynamic = parse_dynamic(payload)?;
            for region in dynamic.regions() {
                let required_size = region.size.unwrap_or(region.entry_size.unwrap_or(1));
                match region.address {
                    DynamicAddress::GuestVirtual(address) => {
                        self.validate_initialized_range(address, required_size)?
                    }
                    DynamicAddress::DynlibOffset(offset) => {
                        let backing = plan.dynlib_data.ok_or(RuntimeError::MissingDynlibData)?;
                        file_slice(
                            backing.file_bytes,
                            offset,
                            required_size,
                            "SCE dynamic table",
                        )?;
                    }
                }
            }
            plan.dynamic = Some(dynamic);
        }
        Ok(plan)
    }

    fn validate_segment(
        &self,
        index: usize,
        header: &ProgramHeaderMetadata,
    ) -> Result<(), LoadError> {
        validate_load_segment_with_policy(
            index,
            header.file_offset,
            header.vaddr,
            header.file_size,
            header.mem_size,
            header.align,
            !uses_sce_auxiliary_copy_semantics(&self.metadata, header),
        )
    }

    fn validate_mapped_range(&self, address: u64, size: u64) -> Result<(), LoadError> {
        let end = checked_add(address, size, "mapped range")?;
        let mut cursor = address;
        while cursor < end {
            let mut next = cursor;
            for header in &self.metadata.program_headers {
                if matches!(header.kind, PT_LOAD | PT_SCE_RELRO) && header.mem_size != 0 {
                    // Protectable page coverage follows the runtime mapper:
                    // floor(vaddr)..ceil(vaddr+mem_size) at guest-page granularity.
                    // ELF p_align never creates additional mapped bytes. Keep
                    // these pages distinct from logical or initialized ranges.
                    let logical_end = checked_add(header.vaddr, header.mem_size, "mapped segment")?;
                    let mapped_start = header.vaddr & !(RUNTIME_PAGE_SIZE - 1);
                    let mapped_end =
                        checked_add(logical_end, RUNTIME_PAGE_SIZE - 1, "mapped page range")?
                            & !(RUNTIME_PAGE_SIZE - 1);
                    if cursor >= mapped_start && cursor < mapped_end {
                        next = next.max(mapped_end.min(end));
                    }
                }
            }
            if next == cursor {
                return Err(RuntimeError::UnmappedVirtualRange {
                    address: cursor,
                    size: end - cursor,
                }
                .into());
            }
            cursor = next;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
