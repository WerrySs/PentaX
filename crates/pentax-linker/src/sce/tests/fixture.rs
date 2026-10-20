// SPDX-License-Identifier: GPL-2.0-or-later

//! Minimal, original ELF64 images with SCE dynamic metadata, built in memory.
//!
//! Layout: ELF header at `0x000`, program headers at `0x040`, table data at
//! [`DATA_OFFSET`], then the dynamic section, 16-byte aligned, after the data
//! so it can hold any number of entries. One initialized `PT_LOAD` maps the
//! whole file at [`BASE`], followed by 0x100 bytes of BSS. In the dynlib
//! domain, `PT_SCE_DYNLIBDATA` covers exactly the table data, so its offset
//! zero is file offset [`DATA_OFFSET`], not the ELF header.

pub(super) const BASE: u64 = 0x40_0000;
pub(super) const DATA_OFFSET: usize = 0x200;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_SCE_DYNLIBDATA: u32 = 0x6100_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Domain {
    /// Standard tags holding original guest virtual addresses.
    Guest,
    /// SCE tags holding offsets into `PT_SCE_DYNLIBDATA`.
    Dynlib,
}

pub(super) const DOMAINS: [Domain; 2] = [Domain::Guest, Domain::Dynlib];

struct Tags {
    strtab: i64,
    strsz: i64,
    symtab: i64,
    symsz: i64,
    syment: i64,
}

const STANDARD: Tags = Tags {
    strtab: 5,
    strsz: 10,
    symtab: 6,
    symsz: 39,
    syment: 11,
};

const SCE: Tags = Tags {
    strtab: 0x6100_0035,
    strsz: 0x6100_0037,
    symtab: 0x6100_0039,
    symsz: 0x6100_003f,
    syment: 0x6100_003b,
};

/// A packed module descriptor word. The classic and observed `_1` tags share
/// this layout.
pub(super) fn module(name_offset: u32, major: u8, minor: u8, id: u16) -> u64 {
    u64::from(name_offset)
        | (u64::from(minor) << 32)
        | (u64::from(major) << 40)
        | (u64::from(id) << 48)
}

/// A packed library descriptor word, for either encoding.
pub(super) fn library(name_offset: u32, version: u16, id: u16) -> u64 {
    u64::from(name_offset) | (u64::from(version) << 32) | (u64::from(id) << 48)
}

pub(super) fn symbol(name_offset: u32, info: u8, section: u16, value: u64) -> [u8; 24] {
    let mut record = [0; 24];
    record[0..4].copy_from_slice(&name_offset.to_le_bytes());
    record[4] = info;
    record[6..8].copy_from_slice(&section.to_le_bytes());
    record[8..16].copy_from_slice(&value.to_le_bytes());
    record
}

/// A string table starting with the empty name, and each name's offset.
pub(super) fn strtab<T: AsRef<[u8]>>(names: &[T]) -> (Vec<u8>, Vec<u32>) {
    let mut bytes = vec![0];
    let mut offsets = Vec::new();
    for name in names {
        offsets.push(bytes.len() as u32);
        bytes.extend_from_slice(name.as_ref());
        bytes.push(0);
    }
    (bytes, offsets)
}

/// Address of `DATA_OFFSET + offset` in the given domain.
pub(super) fn address(domain: Domain, offset: usize) -> u64 {
    match domain {
        Domain::Guest => BASE + (DATA_OFFSET + offset) as u64,
        Domain::Dynlib => offset as u64,
    }
}

/// Places tables and records dynamic entries in call order.
pub(super) struct Builder {
    domain: Domain,
    pub(super) entries: Vec<(i64, u64)>,
    data: Vec<u8>,
}

impl Builder {
    pub(super) fn new(domain: Domain) -> Self {
        Self {
            domain,
            entries: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Unrelated leading bytes, so no table starts at offset zero.
    pub(super) fn pad(&mut self, length: usize) -> &mut Self {
        self.data.resize(self.data.len() + length, 0xcc);
        self
    }

    /// Place bytes at an eight-byte aligned position and return their
    /// address in this builder's domain.
    pub(super) fn place(&mut self, bytes: &[u8]) -> u64 {
        while !self.data.len().is_multiple_of(8) {
            self.data.push(0xcc);
        }
        let at = self.data.len();
        self.data.extend_from_slice(bytes);
        address(self.domain, at)
    }

    pub(super) fn entry(&mut self, tag: i64, value: u64) -> &mut Self {
        self.entries.push((tag, value));
        self
    }

    pub(super) fn strings(&mut self, bytes: &[u8]) -> &mut Self {
        let at = self.place(bytes);
        self.strings_at(at, bytes.len() as u64)
    }

    /// Declare a string table without placing bytes.
    pub(super) fn strings_at(&mut self, at: u64, size: u64) -> &mut Self {
        let tags = self.tags();
        self.entry(tags.strtab, at).entry(tags.strsz, size)
    }

    pub(super) fn symbols(&mut self, records: &[[u8; 24]]) -> &mut Self {
        let bytes = records.concat();
        let at = self.place(&bytes);
        let tags = self.tags();
        self.entry(tags.symtab, at)
            .entry(tags.syment, 24)
            .entry(tags.symsz, bytes.len() as u64)
    }

    fn tags(&self) -> &'static Tags {
        match self.domain {
            Domain::Guest => &STANDARD,
            Domain::Dynlib => &SCE,
        }
    }

    pub(super) fn build(&self) -> Vec<u8> {
        let mut dynamic = Vec::new();
        for &(tag, value) in self.entries.iter().chain(&[(0, 0)]) {
            dynamic.extend_from_slice(&tag.to_le_bytes());
            dynamic.extend_from_slice(&value.to_le_bytes());
        }
        assemble(self.domain, &dynamic, &self.data)
    }
}

/// Build an image from raw dynamic-section bytes and table data.
pub(super) fn assemble(domain: Domain, dynamic: &[u8], data: &[u8]) -> Vec<u8> {
    let dynamic_offset = (DATA_OFFSET + data.len()).next_multiple_of(16);
    let file_len = dynamic_offset + dynamic.len();
    let mut bytes = vec![0; file_len];
    bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
    let (elf_type, header_count) = match domain {
        Domain::Guest => (3_u16, 2_u16),
        Domain::Dynlib => (0xfe10, 3),
    };
    put(&mut bytes, 16, &elf_type.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &BASE.to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(&mut bytes, 56, &header_count.to_le_bytes());

    let file_len = file_len as u64;
    let dynamic_len = dynamic.len() as u64;
    program_header(
        &mut bytes,
        0,
        PT_LOAD,
        [0, BASE, file_len, file_len + 0x100, 0x1000],
    );
    program_header(
        &mut bytes,
        1,
        PT_DYNAMIC,
        [
            dynamic_offset as u64,
            BASE + dynamic_offset as u64,
            dynamic_len,
            dynamic_len,
            8,
        ],
    );
    if domain == Domain::Dynlib {
        program_header(
            &mut bytes,
            2,
            PT_SCE_DYNLIBDATA,
            [DATA_OFFSET as u64, 0, data.len() as u64, 0, 0],
        );
    }
    put(&mut bytes, DATA_OFFSET, data);
    put(&mut bytes, dynamic_offset, dynamic);
    bytes
}

/// An image whose only program header is a `PT_LOAD`.
pub(super) fn static_image() -> Vec<u8> {
    let mut bytes = Builder::new(Domain::Guest).build();
    put(&mut bytes, 56, &1_u16.to_le_bytes());
    bytes
}

fn program_header(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    [offset, vaddr, file_size, mem_size, align]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &6_u32.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &vaddr.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &align.to_le_bytes());
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
