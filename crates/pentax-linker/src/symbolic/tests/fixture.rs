// SPDX-License-Identifier: GPL-2.0-or-later

//! Original, in-memory modules. No fixture bytes come from software artifacts.

use crate::{DescriptorEncoding, DescriptorRole};

pub(super) const BASE: u64 = 0x40_0000;
pub(super) const A_BIAS: u64 = 0x100_0000;
pub(super) const B_BIAS: u64 = 0x200_0000;
pub(super) const TARGET: u64 = BASE + 0x900;
pub(super) const DEFINITION: u64 = BASE + 0x180;

#[derive(Clone)]
pub(super) struct Sym {
    pub name: Vec<u8>,
    pub binding: u8,
    pub kind: u8,
    pub other: u8,
    pub section: u16,
    pub value: u64,
    pub size: u64,
}

#[derive(Clone)]
pub(super) struct Declaration {
    pub role: DescriptorRole,
    pub encoding: DescriptorEncoding,
    pub name: Vec<u8>,
    pub id: u16,
    /// Module major/minor as high/low bytes, or the opaque library version.
    pub version: u16,
}

#[derive(Clone)]
pub(super) struct Module {
    pub primary: bool,
    pub provider: bool,
    pub dynlib: bool,
    pub module_encoding: DescriptorEncoding,
    pub library_encoding: DescriptorEncoding,
    pub module_name: Vec<u8>,
    pub library_name: Vec<u8>,
    pub major: u8,
    pub minor: u8,
    pub version: u16,
    pub module_id: u16,
    pub library_id: u16,
    pub symbols: Vec<Sym>,
    pub rela: Vec<(u64, u32, u32, i64)>,
    pub plt: Vec<(u64, u32, u32, i64)>,
    pub flags: u32,
    pub extra: Vec<(i64, u64)>,
    pub declarations: Vec<Declaration>,
}

impl Module {
    pub fn consumer() -> Self {
        Self::new(false)
    }
    pub fn provider() -> Self {
        Self::new(true)
    }
    fn new(provider: bool) -> Self {
        Self {
            primary: !provider,
            provider,
            dynlib: false,
            module_encoding: DescriptorEncoding::Classic,
            library_encoding: DescriptorEncoding::Classic,
            module_name: b"fixture-module".to_vec(),
            library_name: b"fixture-library".to_vec(),
            major: 2,
            minor: 3,
            version: 4,
            module_id: if provider { 31 } else { 2 },
            library_id: if provider { 17 } else { 1 },
            symbols: vec![Sym {
                name: if provider {
                    b"AAAAAAAAAAA#R#f".to_vec()
                } else {
                    b"AAAAAAAAAAA#B#C".to_vec()
                },
                binding: 1,
                kind: 2,
                other: 0,
                section: u16::from(provider),
                value: if provider { DEFINITION } else { 0 },
                size: if provider { 8 } else { 0 },
            }],
            rela: if provider {
                vec![]
            } else {
                vec![(TARGET, 1, 1, -8)]
            },
            plt: vec![],
            flags: 5,
            extra: vec![],
            declarations: vec![],
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
        put(
            &mut bytes,
            16,
            &(if self.dynlib { 0xfe10_u16 } else { 3_u16 }).to_le_bytes(),
        );
        put(&mut bytes, 18, &62_u16.to_le_bytes());
        put(&mut bytes, 20, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            24,
            &(if self.primary { BASE } else { 0 }).to_le_bytes(),
        );
        put(&mut bytes, 32, &64_u64.to_le_bytes());
        put(&mut bytes, 52, &64_u16.to_le_bytes());
        put(&mut bytes, 54, &56_u16.to_le_bytes());
        put(
            &mut bytes,
            56,
            &(if self.dynlib { 3_u16 } else { 2 }).to_le_bytes(),
        );
        header(
            &mut bytes,
            0,
            1,
            self.flags,
            [0, BASE, 0x1000, 0x1400, 0x1000],
        );

        let mut strings = vec![0];
        let mut add = |name: &[u8]| {
            let offset = strings.len() as u32;
            strings.extend_from_slice(name);
            strings.push(0);
            offset
        };
        let module_name = add(&self.module_name);
        let library_name = add(&self.library_name);
        let names: Vec<_> = self
            .symbols
            .iter()
            .map(|symbol| add(&symbol.name))
            .collect();
        let declarations: Vec<_> = self
            .declarations
            .iter()
            .map(|declaration| {
                (
                    declaration.role.tag(declaration.encoding),
                    u64::from(add(&declaration.name))
                        | (u64::from(declaration.version) << 32)
                        | (u64::from(declaration.id) << 48),
                )
            })
            .collect();
        let mut cursor = 0x200;
        let mut place = |table: &[u8]| {
            cursor = (cursor + 7) & !7;
            let at = cursor;
            put(&mut bytes, at, table);
            cursor += table.len();
            if self.dynlib {
                (at - 0x200) as u64
            } else {
                BASE + at as u64
            }
        };
        let strings_at = place(&strings);
        let mut symbols = vec![0; 24];
        for (symbol, offset) in self.symbols.iter().zip(names) {
            symbols.extend_from_slice(&offset.to_le_bytes());
            symbols.push(symbol.binding << 4 | symbol.kind);
            symbols.push(symbol.other);
            symbols.extend_from_slice(&symbol.section.to_le_bytes());
            symbols.extend_from_slice(&symbol.value.to_le_bytes());
            symbols.extend_from_slice(&symbol.size.to_le_bytes());
        }
        let symbols_at = place(&symbols);
        let rela_bytes = relocations(&self.rela);
        let rela_at = place(&rela_bytes);
        let plt_bytes = relocations(&self.plt);
        let plt_at = place(&plt_bytes);
        let tags: [i64; 11] = if self.dynlib {
            [
                0x61000035, 0x61000037, 0x61000039, 0x6100003f, 0x6100003b, 0x6100002f, 0x61000031,
                0x61000033, 0x61000029, 0x6100002d, 0x6100002b,
            ]
        } else {
            [5, 10, 6, 39, 11, 7, 8, 9, 23, 2, 20]
        };
        let mut entries = vec![
            (tags[0], strings_at),
            (tags[1], strings.len() as u64),
            (tags[2], symbols_at),
            (tags[3], symbols.len() as u64),
            (tags[4], 24),
        ];
        if !self.rela.is_empty() {
            entries.extend([
                (tags[5], rela_at),
                (tags[6], rela_bytes.len() as u64),
                (tags[7], 24),
            ]);
        }
        if !self.plt.is_empty() {
            entries.extend([
                (tags[8], plt_at),
                (tags[9], plt_bytes.len() as u64),
                (tags[10], 7),
            ]);
        }
        let module_word = u64::from(module_name)
            | (u64::from(self.minor) << 32)
            | (u64::from(self.major) << 40)
            | (u64::from(self.module_id) << 48);
        let library_word = u64::from(library_name)
            | (u64::from(self.version) << 32)
            | (u64::from(self.library_id) << 48);
        entries.push((
            if self.provider {
                DescriptorRole::ModuleInfo
            } else {
                DescriptorRole::NeededModule
            }
            .tag(self.module_encoding),
            module_word,
        ));
        entries.push((
            if self.provider {
                DescriptorRole::ExportLibrary
            } else {
                DescriptorRole::ImportLibrary
            }
            .tag(self.library_encoding),
            library_word,
        ));
        entries.extend_from_slice(&self.extra);
        entries.extend(declarations);
        entries.push((0, 0));
        let mut dynamic = Vec::new();
        for (tag, value) in entries {
            dynamic.extend_from_slice(&tag.to_le_bytes());
            dynamic.extend_from_slice(&value.to_le_bytes());
        }
        put(&mut bytes, 0xc00, &dynamic);
        header(
            &mut bytes,
            1,
            2,
            4,
            [
                0xc00,
                BASE + 0xc00,
                dynamic.len() as u64,
                dynamic.len() as u64,
                8,
            ],
        );
        if self.dynlib {
            header(
                &mut bytes,
                2,
                0x61000000,
                4,
                [0x200, 0, (cursor - 0x200) as u64, 0, 0],
            );
        }
        bytes
    }
}

pub(super) fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn header(bytes: &mut [u8], index: usize, kind: u32, flags: u32, values: [u64; 5]) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    for (offset, value) in [8, 16, 32, 40, 48].into_iter().zip(values) {
        put(bytes, at + offset, &value.to_le_bytes());
    }
}
fn relocations(records: &[(u64, u32, u32, i64)]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for &(offset, symbol, kind, addend) in records {
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&(u64::from(symbol) << 32 | u64::from(kind)).to_le_bytes());
        bytes.extend_from_slice(&addend.to_le_bytes());
    }
    bytes
}
