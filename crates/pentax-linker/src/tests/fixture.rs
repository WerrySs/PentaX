// SPDX-License-Identifier: GPL-2.0-or-later

//! Minimal, original ELF64 images built in memory.
//!
//! Layout: ELF header at `0x000`, program headers at `0x040`, the dynamic
//! section at `0x100` and table data at `0x200`. One initialized `PT_LOAD`
//! maps the whole file at [`BASE`], followed by 0x100 bytes of BSS. In the
//! dynlib domain, `PT_SCE_DYNLIBDATA` covers exactly the table data, so its
//! offset zero is file offset `0x200`, not the ELF header.

pub(super) const BASE: u64 = 0x10000;
pub(super) const DATA_OFFSET: usize = 0x200;
const DYNAMIC_OFFSET: usize = 0x100;
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

struct Tags {
    strtab: i64,
    strsz: i64,
    symtab: i64,
    symsz: i64,
    syment: i64,
    rela: i64,
    relasz: i64,
    relaent: i64,
    jmprel: i64,
    pltrelsz: i64,
    pltrel: i64,
}

/// Standard tags. Tag 39 is the symbol-table size companion that
/// `pentax-loader` recognizes; standard images commonly omit it.
const STANDARD: Tags = Tags {
    strtab: 5,
    strsz: 10,
    symtab: 6,
    symsz: 39,
    syment: 11,
    rela: 7,
    relasz: 8,
    relaent: 9,
    jmprel: 23,
    pltrelsz: 2,
    pltrel: 20,
};

pub(super) const SCE_SYMTAB: i64 = 0x6100_0039;
pub(super) const SCE_SYMTABSZ: i64 = 0x6100_003f;
pub(super) const SCE_SYMENT: i64 = 0x6100_003b;

const SCE: Tags = Tags {
    strtab: 0x6100_0035,
    strsz: 0x6100_0037,
    symtab: SCE_SYMTAB,
    symsz: SCE_SYMTABSZ,
    syment: SCE_SYMENT,
    rela: 0x6100_002f,
    relasz: 0x6100_0031,
    relaent: 0x6100_0033,
    jmprel: 0x6100_0029,
    pltrelsz: 0x6100_002d,
    pltrel: 0x6100_002b,
};

/// Raw table contents. `None` omits the table and its tags entirely.
#[derive(Debug, Clone, Default)]
pub(super) struct Tables {
    pub(super) strings: Option<Vec<u8>>,
    pub(super) symbols: Option<Vec<u8>>,
    /// Omit the symbol-table size tag, leaving its extent unknown.
    pub(super) unsized_symbols: bool,
    pub(super) rela: Option<Vec<u8>>,
    pub(super) plt_rela: Option<Vec<u8>>,
}

pub(super) fn symbol(
    name_offset: u32,
    info: u8,
    other: u8,
    section: u16,
    value: u64,
    size: u64,
) -> [u8; 24] {
    let mut record = [0; 24];
    record[0..4].copy_from_slice(&name_offset.to_le_bytes());
    record[4] = info;
    record[5] = other;
    record[6..8].copy_from_slice(&section.to_le_bytes());
    record[8..16].copy_from_slice(&value.to_le_bytes());
    record[16..24].copy_from_slice(&size.to_le_bytes());
    record
}

pub(super) fn null_symbol() -> [u8; 24] {
    symbol(0, 0, 0, 0, 0, 0)
}

pub(super) fn rela(offset: u64, symbol: u32, kind: u32, addend: i64) -> [u8; 24] {
    let info = (u64::from(symbol) << 32) | u64::from(kind);
    let mut record = [0; 24];
    record[0..8].copy_from_slice(&offset.to_le_bytes());
    record[8..16].copy_from_slice(&info.to_le_bytes());
    record[16..24].copy_from_slice(&addend.to_le_bytes());
    record
}

/// Address of `DATA_OFFSET + offset` in the given domain.
pub(super) fn address(domain: Domain, offset: usize) -> u64 {
    match domain {
        Domain::Guest => BASE + (DATA_OFFSET + offset) as u64,
        Domain::Dynlib => offset as u64,
    }
}

pub(super) fn image(domain: Domain, tables: &Tables) -> Vec<u8> {
    let tags = match domain {
        Domain::Guest => &STANDARD,
        Domain::Dynlib => &SCE,
    };
    let mut data = Vec::new();
    let mut entries = Vec::new();
    if let Some(bytes) = &tables.strings {
        let at = place(&mut data, domain, bytes);
        entries.extend([(tags.strtab, at), (tags.strsz, bytes.len() as u64)]);
    }
    if let Some(bytes) = &tables.symbols {
        let at = place(&mut data, domain, bytes);
        entries.extend([(tags.symtab, at), (tags.syment, 24)]);
        if !tables.unsized_symbols {
            entries.push((tags.symsz, bytes.len() as u64));
        }
    }
    if let Some(bytes) = &tables.rela {
        let at = place(&mut data, domain, bytes);
        entries.extend([
            (tags.rela, at),
            (tags.relasz, bytes.len() as u64),
            (tags.relaent, 24),
        ]);
    }
    if let Some(bytes) = &tables.plt_rela {
        let at = place(&mut data, domain, bytes);
        entries.extend([
            (tags.jmprel, at),
            (tags.pltrelsz, bytes.len() as u64),
            (tags.pltrel, 7),
        ]);
    }
    assemble(domain, &entries, &data)
}

/// An image with a single `PT_LOAD` and no `PT_DYNAMIC` header.
pub(super) fn static_image() -> Vec<u8> {
    let mut bytes = image(Domain::Guest, &Tables::default());
    put(&mut bytes, 56, &1_u16.to_le_bytes());
    bytes
}

/// Build an image from explicit dynamic entries and table data.
pub(super) fn assemble(domain: Domain, entries: &[(i64, u64)], data: &[u8]) -> Vec<u8> {
    let mut dynamic = Vec::new();
    for &(tag, value) in entries.iter().chain(&[(0, 0)]) {
        dynamic.extend_from_slice(&tag.to_le_bytes());
        dynamic.extend_from_slice(&value.to_le_bytes());
    }
    assert!(dynamic.len() <= DATA_OFFSET - DYNAMIC_OFFSET);

    let file_len = DATA_OFFSET + data.len();
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
        6,
        [0, BASE, file_len, file_len + 0x100, 0x1000],
    );
    program_header(
        &mut bytes,
        1,
        PT_DYNAMIC,
        6,
        [
            DYNAMIC_OFFSET as u64,
            BASE + DYNAMIC_OFFSET as u64,
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
            0,
            [DATA_OFFSET as u64, 0, data.len() as u64, 0, 0],
        );
    }
    put(&mut bytes, DYNAMIC_OFFSET, &dynamic);
    put(&mut bytes, DATA_OFFSET, data);
    bytes
}

fn place(data: &mut Vec<u8>, domain: Domain, bytes: &[u8]) -> u64 {
    while !data.len().is_multiple_of(8) {
        data.push(0);
    }
    let at = data.len();
    data.extend_from_slice(bytes);
    address(domain, at)
}

fn program_header(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, vaddr, file_size, mem_size, align]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &vaddr.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &align.to_le_bytes());
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
