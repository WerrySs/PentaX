// SPDX-License-Identifier: GPL-2.0-or-later

//! Fresh SCE images with invented names and NIDs; no upstream instruction bytes.

use crate::memory::PAGE_SIZE;

pub(super) const DATA: u64 = 0x4000;
/// First never file-backed page of every data segment.
pub(super) const BSS: u64 = 0x8000;
pub(super) const TLS_HEADER: usize = 3;
pub(super) const ORIGINAL: u64 = 0x5eed_0000_0000_5eed;
pub(super) const PROVIDER_WORDS: [u64; 2] = [0x7072_6f76_0000_0001, 0x7072_6f76_0000_0002];
pub(super) const DTPMOD: u32 = 16;
pub(super) const DTPOFF: u32 = 17;
pub(super) const TPOFF: u32 = 18;
pub(super) const RELATIVE: u32 = 8;

const ENTRY: u64 = 0x1f0;
const STRINGS: u64 = 0x200;
const SYMBOLS: u64 = 0x280;
const DYNAMIC: u64 = 0x400;
const RELA: u64 = 0x600;
const DATA_FILE: u64 = 0x100;
// Symbol indices 1 and 2 use these canonical invented NIDs.
const NIDS: [&[u8]; 2] = [b"AAAAAAAAAAA", b"AAAAAAAAAAE"];

#[derive(Clone, Copy)]
pub(super) struct Tls {
    pub address: u64,
    pub initialized: u64,
    pub memory: u64,
    pub alignment: u64,
}

#[derive(Clone, Copy)]
pub(super) struct Record {
    pub target: u64,
    pub symbol: u32,
    pub kind: u32,
    pub addend: i64,
}

pub(super) const fn record(target: u64, symbol: u32, kind: u32, addend: i64) -> Record {
    Record {
        target,
        symbol,
        kind,
        addend,
    }
}

/// Primary image importing two GLOBAL STT_TLS symbols through classic
/// needed-module/import-library declarations. File-backed targets hold ORIGINAL.
pub(super) fn consumer(tls: Option<Tls>, records: &[Record], relro: bool) -> Vec<u8> {
    image(false, tls, records, relro, [0, 0])
}

/// Exporting image whose GLOBAL DEFAULT STT_TLS definitions have the given
/// module-relative offsets and size eight. Initialized TLS words are fixed.
pub(super) fn provider(tls: Tls, offsets: [u64; 2]) -> Vec<u8> {
    image(true, Some(tls), &[], false, offsets)
}

/// A metadata-free, TLS-free module that only shifts module positions.
pub(super) fn filler() -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    header(&mut bytes, 0, 1);
    program(&mut bytes, 0, 1, 4, [0, 0, 0x100, 0x100, PAGE_SIZE]);
    bytes
}

fn image(
    provider: bool,
    tls: Option<Tls>,
    records: &[Record],
    relro: bool,
    offsets: [u64; 2],
) -> Vec<u8> {
    assert!(records.len() <= 8);
    let mut bytes = vec![0; (DATA + DATA_FILE) as usize];
    program(&mut bytes, 0, 1, 5, [0, 0, 0x800, 0x800, PAGE_SIZE]);
    program(
        &mut bytes,
        1,
        1,
        6,
        [DATA, DATA, DATA_FILE, 2 * PAGE_SIZE, PAGE_SIZE],
    );
    let mut headers = 3;
    if let Some(tls) = tls {
        program(
            &mut bytes,
            TLS_HEADER,
            7,
            4,
            [
                tls.address,
                tls.address,
                tls.initialized,
                tls.memory,
                tls.alignment,
            ],
        );
        headers += 1;
        if provider {
            for (index, word) in PROVIDER_WORDS.iter().enumerate() {
                put(
                    &mut bytes,
                    tls.address + index as u64 * 8,
                    &word.to_le_bytes(),
                );
            }
        }
    }
    if relro {
        program(
            &mut bytes,
            headers,
            0x6474_e552,
            4,
            [0, DATA, 0, PAGE_SIZE, PAGE_SIZE],
        );
        headers += 1;
    }
    header(&mut bytes, if provider { 0 } else { ENTRY }, headers as u16);
    if !provider {
        bytes[ENTRY as usize] = 0xc3;
    }
    // Image-local selectors: library R=17/module f=31, or library B=1/module C=2.
    let (library, module) = if provider { ("R", "f") } else { ("B", "C") };
    let mut strings = vec![0];
    let names = NIDS.map(|nid| {
        let mut qualified = nid.to_vec();
        for field in [library, module] {
            qualified.push(b'#');
            qualified.extend_from_slice(field.as_bytes());
        }
        string(&mut strings, &qualified)
    });
    let module_name = string(&mut strings, b"tls-original-module");
    let library_name = string(&mut strings, b"tls-original-library");
    assert!(STRINGS + strings.len() as u64 <= SYMBOLS);
    put(&mut bytes, STRINGS, &strings);
    for (index, name) in names.iter().enumerate() {
        let at = SYMBOLS + (index as u64 + 1) * 24;
        put(&mut bytes, at, &name.to_le_bytes());
        bytes[at as usize + 4] = 0x16; // GLOBAL TLS, DEFAULT visibility.
        if provider {
            put(&mut bytes, at + 6, &1_u16.to_le_bytes());
            put(&mut bytes, at + 8, &offsets[index].to_le_bytes());
            put(&mut bytes, at + 16, &8_u64.to_le_bytes());
        }
    }
    // Exact declarations: module version 1.0 and library version 1 in both images.
    let (module_tag, module_id, library_tag, library_id) = if provider {
        (0x6100_000d_i64, 31_u64, 0x6100_0013_i64, 17_u64)
    } else {
        (0x6100_000f, 2, 0x6100_0015, 1)
    };
    let mut entries = vec![
        (5_i64, STRINGS),
        (10, strings.len() as u64),
        (6, SYMBOLS),
        (11, 24),
        (39, 72),
        (
            module_tag,
            u64::from(module_name) | (1 << 32) | (module_id << 48),
        ),
        (
            library_tag,
            u64::from(library_name) | (1 << 32) | (library_id << 48),
        ),
    ];
    if !records.is_empty() {
        entries.extend([(7, RELA), (8, records.len() as u64 * 24), (9, 24)]);
    }
    for (index, record) in records.iter().enumerate() {
        let at = RELA + index as u64 * 24;
        put(&mut bytes, at, &record.target.to_le_bytes());
        put(
            &mut bytes,
            at + 8,
            &((u64::from(record.symbol) << 32) | u64::from(record.kind)).to_le_bytes(),
        );
        put(&mut bytes, at + 16, &record.addend.to_le_bytes());
        if (DATA..DATA + DATA_FILE - 7).contains(&record.target) {
            put(&mut bytes, record.target, &ORIGINAL.to_le_bytes());
        }
    }
    entries.push((0, 0));
    for (index, (tag, value)) in entries.iter().enumerate() {
        put(&mut bytes, DYNAMIC + index as u64 * 16, &tag.to_le_bytes());
        put(
            &mut bytes,
            DYNAMIC + index as u64 * 16 + 8,
            &value.to_le_bytes(),
        );
    }
    let dynamic_size = entries.len() as u64 * 16;
    assert!(DYNAMIC + dynamic_size <= RELA);
    program(
        &mut bytes,
        2,
        2,
        4,
        [DYNAMIC, DYNAMIC, dynamic_size, dynamic_size, 8],
    );
    bytes
}

fn header(bytes: &mut [u8], entry: u64, headers: u16) {
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(bytes, 16, &3_u16.to_le_bytes());
    put(bytes, 18, &62_u16.to_le_bytes());
    put(bytes, 20, &1_u32.to_le_bytes());
    put(bytes, 24, &entry.to_le_bytes());
    put(bytes, 32, &64_u64.to_le_bytes());
    put(bytes, 52, &64_u16.to_le_bytes());
    put(bytes, 54, &56_u16.to_le_bytes());
    put(bytes, 56, &headers.to_le_bytes());
}

fn string(strings: &mut Vec<u8>, value: &[u8]) -> u32 {
    let offset = strings.len() as u32;
    strings.extend_from_slice(value);
    strings.push(0);
    offset
}

fn program(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, address, file_size, mem_size, alignment]: [u64; 5],
) {
    let at = 64 + index as u64 * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &address.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &alignment.to_le_bytes());
}

fn put(bytes: &mut [u8], at: u64, value: &[u8]) {
    let at = at as usize;
    bytes[at..at + value.len()].copy_from_slice(value);
}
