// SPDX-License-Identifier: GPL-2.0-or-later
//! Bounded exception-table metadata, without interpreting or executing a CFI.
//! Header encodings: https://refspecs.linuxfoundation.org/LSB_2.0.0/LSB-Core/LSB-Core/ehframehdr.html

use super::{DynamicAddress, RuntimeLoadPlan, VirtualRange};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindError {
    Extent,
    Unsupported,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnwindMetadata {
    pub header: Option<VirtualRange>,
    pub frame: Option<VirtualRange>,
    /// Source DT_SONAME, truncated to leave a trailing NUL. No host path.
    pub name: [u8; 256],
}

fn add(a: u64, b: u64) -> Result<u64, UnwindError> {
    a.checked_add(b).ok_or(UnwindError::Extent)
}
fn u16_at(bytes: &[u8], at: usize) -> Result<u16, UnwindError> {
    Ok(u16::from_le_bytes(
        bytes
            .get(at..at + 2)
            .ok_or(UnwindError::Extent)?
            .try_into()
            .map_err(|_| UnwindError::Extent)?,
    ))
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32, UnwindError> {
    Ok(u32::from_le_bytes(
        bytes
            .get(at..at + 4)
            .ok_or(UnwindError::Extent)?
            .try_into()
            .map_err(|_| UnwindError::Extent)?,
    ))
}
fn u64_at(bytes: &[u8], at: usize) -> Result<u64, UnwindError> {
    Ok(u64::from_le_bytes(
        bytes
            .get(at..at + 8)
            .ok_or(UnwindError::Extent)?
            .try_into()
            .map_err(|_| UnwindError::Extent)?,
    ))
}
fn slice(bytes: &[u8], at: u64, size: u64) -> Result<&[u8], UnwindError> {
    let end = add(at, size)?;
    bytes
        .get(
            usize::try_from(at).map_err(|_| UnwindError::Extent)?
                ..usize::try_from(end).map_err(|_| UnwindError::Extent)?,
        )
        .ok_or(UnwindError::Extent)
}

impl RuntimeLoadPlan<'_> {
    /// Inspect only the actual retained input paired with this plan. This pure
    /// result is not a live mapping capability. Section-table work is capped
    /// at 4096 entries; no allocation or unbounded section/string scan occurs.
    pub fn unwind_metadata(&self, source: &[u8]) -> Result<UnwindMetadata, UnwindError> {
        let mut result = UnwindMetadata {
            header: None,
            frame: None,
            name: [0; 256],
        };
        for ph in &self.metadata.program_headers {
            if ph.kind == 0x6474_e550 && ph.mem_size != 0 {
                if result.header.is_some() {
                    return Err(UnwindError::Ambiguous);
                }
                if ph.mem_size < 4 || ph.file_size < ph.mem_size {
                    return Err(UnwindError::Extent);
                }
                let range = VirtualRange {
                    address: ph.vaddr,
                    size: ph.mem_size,
                };
                self.unwind_source(range)?;
                result.header = Some(range);
            }
        }
        // Section offsets in a SELF refer to the original ELF, not arbitrary
        // container offsets. SELF uses its checked initialized GNU header only.
        if self.metadata.self_metadata.is_none() {
            let elf = slice(source, 0, 64)?;
            let shoff = u64_at(elf, 40)?;
            let count = u16_at(elf, 60)? as u64;
            let names_index = u16_at(elf, 62)? as u64;
            if shoff != 0 {
                if count == 0 || count > 4096 || u16_at(elf, 58)? != 64 || names_index >= count {
                    return Err(UnwindError::Unsupported);
                }
                let sections = slice(source, shoff, count * 64)?;
                let names_header = slice(sections, names_index * 64, 64)?;
                if u32_at(names_header, 4)? != 3 {
                    return Err(UnwindError::Extent);
                }
                let names_size = u64_at(names_header, 32)?;
                if names_size > 1024 * 1024 {
                    return Err(UnwindError::Unsupported);
                }
                let names = slice(source, u64_at(names_header, 24)?, names_size)?;
                for sh in sections.chunks_exact(64) {
                    let name = u32_at(sh, 0)? as usize;
                    if names.get(name..name.saturating_add(10)) != Some(b".eh_frame\0".as_slice()) {
                        continue;
                    }
                    if result.frame.is_some() {
                        return Err(UnwindError::Ambiguous);
                    }
                    if u32_at(sh, 4)? != 1 || u64_at(sh, 8)? & 2 == 0 {
                        return Err(UnwindError::Extent);
                    }
                    let range = VirtualRange {
                        address: u64_at(sh, 16)?,
                        size: u64_at(sh, 32)?,
                    };
                    let actual = self.unwind_source(range)?;
                    if range.size == 0
                        || !std::ptr::eq(actual, slice(source, u64_at(sh, 24)?, range.size)?)
                    {
                        return Err(UnwindError::Extent);
                    }
                    result.frame = Some(range);
                }
            } else if count != 0 || names_index != 0 {
                return Err(UnwindError::Extent);
            }
        }
        if let Some(header) = result.header {
            let bytes = self.unwind_source(header)?;
            if bytes[0] != 1 {
                return Err(UnwindError::Unsupported);
            }
            if bytes[1] != 0xff {
                let address = decode_pointer(&bytes[4..], bytes[1], add(header.address, 4)?)?;
                if let Some(frame) = result.frame {
                    if frame.address != address {
                        return Err(UnwindError::Ambiguous);
                    }
                } else {
                    let remaining = self.unwind_tail(address)?;
                    let size = if address < header.address {
                        remaining.min(header.address - address)
                    } else {
                        remaining
                    };
                    if size == 0 {
                        return Err(UnwindError::Extent);
                    }
                    result.frame = Some(VirtualRange { address, size });
                }
            }
        }
        if let Some(dynamic) = &self.dynamic {
            let mut soname = None;
            for entry in &dynamic.entries {
                if entry.tag == 14 && soname.replace(entry.value).is_some() {
                    return Err(UnwindError::Ambiguous);
                }
            }
            if let Some(offset) = soname {
                let strings = dynamic.strings.ok_or(UnwindError::Extent)?;
                let length = strings.size.ok_or(UnwindError::Extent)?;
                if offset >= length {
                    return Err(UnwindError::Extent);
                }
                let available = (length - offset).min(256);
                let bytes = match strings.address {
                    DynamicAddress::GuestVirtual(address) => self.unwind_source(VirtualRange {
                        address: add(address, offset)?,
                        size: available,
                    })?,
                    DynamicAddress::DynlibOffset(address) => slice(
                        self.dynlib_data.ok_or(UnwindError::Extent)?.file_bytes,
                        add(address, offset)?,
                        available,
                    )?,
                };
                let count = bytes
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(bytes.len())
                    .min(255);
                result.name[..count].copy_from_slice(&bytes[..count]);
            }
        }
        Ok(result)
    }

    /// Require initialized logical bytes from exactly one admitted segment.
    pub fn unwind_source(&self, range: VirtualRange) -> Result<&[u8], UnwindError> {
        let end = add(range.address, range.size)?;
        let mut found = None;
        for segment in &self.segments {
            let bound = add(segment.vaddr, segment.file_bytes.len() as u64)?;
            if range.address >= segment.vaddr && end <= bound {
                if found.is_some() {
                    return Err(UnwindError::Ambiguous);
                }
                found = Some(slice(
                    segment.file_bytes,
                    range.address - segment.vaddr,
                    range.size,
                )?);
            }
        }
        found.ok_or(UnwindError::Extent)
    }

    fn unwind_tail(&self, address: u64) -> Result<u64, UnwindError> {
        let mut found = None;
        for segment in &self.segments {
            let end = add(segment.vaddr, segment.file_bytes.len() as u64)?;
            if segment.vaddr <= address && address < end && found.replace(end - address).is_some() {
                return Err(UnwindError::Ambiguous);
            }
        }
        found.ok_or(UnwindError::Extent)
    }
}

fn decode_pointer(bytes: &[u8], encoding: u8, field: u64) -> Result<u64, UnwindError> {
    if encoding & 0x80 != 0 || !matches!(encoding & 0x70, 0 | 0x10) {
        return Err(UnwindError::Unsupported);
    }
    let signed = matches!(encoding & 15, 0xa..=0xc);
    let value = match encoding & 15 {
        0 | 4 => u64_at(bytes, 0)?,
        2 => u16_at(bytes, 0)? as u64,
        3 => u32_at(bytes, 0)? as u64,
        0xa => (u16_at(bytes, 0)? as i16 as i64) as u64,
        0xb => (u32_at(bytes, 0)? as i32 as i64) as u64,
        0xc => u64_at(bytes, 0)?,
        _ => return Err(UnwindError::Unsupported),
    };
    if encoding & 0x70 == 0 {
        return Ok(value);
    }
    if signed {
        field
            .checked_add_signed(value as i64)
            .ok_or(UnwindError::Extent)
    } else {
        add(field, value)
    }
}

#[cfg(test)]
mod tests;
