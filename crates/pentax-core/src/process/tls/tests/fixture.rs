// SPDX-License-Identifier: GPL-2.0-or-later

//! Fresh tiny ELF images for static TLS planning; no upstream instruction bytes.

pub(super) const DATA: u64 = 0x4000;
pub(super) const DATA_FILE: usize = 0x300;
pub(super) const ENTRY: u64 = 0x1e0;
pub(super) const TLS_HEADER: usize = 2;
pub(super) const TEMPLATE: u64 = 37;
pub(super) const ADDEND: u64 = 0x1234;

#[derive(Clone, Copy)]
pub(super) struct Options {
    pub tls: bool,
    pub address: u64,
    pub initialized: u64,
    pub memory: u64,
    pub alignment: u64,
    pub tls_offset: u64,
    pub relocation: Option<u32>,
    pub data_file_size: u64,
    pub data_mem_size: u64,
    pub data_flags: u32,
    pub data_kind: u32,
    pub split: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            tls: true,
            address: DATA + 8,
            initialized: 8,
            memory: 16,
            alignment: 32,
            tls_offset: (DATA_FILE + 8) as u64,
            relocation: None,
            data_file_size: 0x100,
            data_mem_size: 0x4000,
            data_flags: 6,
            data_kind: 1,
            split: false,
        }
    }
}

pub(super) fn image(options: Options) -> Vec<u8> {
    let mut bytes = vec![0; 0x900];
    bytes[..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &ENTRY.to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(&mut bytes, 56, &7_u16.to_le_bytes());
    program(&mut bytes, 0, 1, 5, [0, 0, 0x200, 0x4000, 1]);
    program(
        &mut bytes,
        1,
        options.data_kind,
        options.data_flags,
        [
            DATA_FILE as u64,
            DATA,
            options.data_file_size,
            options.data_mem_size,
            1,
        ],
    );
    if options.tls {
        program(
            &mut bytes,
            TLS_HEADER,
            7,
            4,
            [
                options.tls_offset,
                options.address,
                options.initialized,
                options.memory,
                options.alignment,
            ],
        );
    }
    bytes[ENTRY as usize] = 0xc3;
    put(&mut bytes, DATA_FILE + 8, &TEMPLATE.to_le_bytes());
    let tags = if let Some(kind) = options.relocation {
        put(&mut bytes, DATA_FILE + 0x40, &options.address.to_le_bytes());
        put(&mut bytes, DATA_FILE + 0x48, &(kind as u64).to_le_bytes());
        put(&mut bytes, DATA_FILE + 0x50, &ADDEND.to_le_bytes());
        vec![
            (5, DATA + 0x80),
            (10, 1),
            (6, DATA + 0x90),
            (39, 24),
            (11, 24),
            (7, DATA + 0x40),
            (8, 24),
            (9, 24),
        ]
    } else {
        Vec::new()
    };
    for (index, &(tag, value)) in tags.iter().enumerate() {
        put(&mut bytes, 0x500 + index * 16, &(tag as u64).to_le_bytes());
        put(&mut bytes, 0x508 + index * 16, &value.to_le_bytes());
    }
    let dynamic_size = ((tags.len() + 1) * 16) as u64;
    program(
        &mut bytes,
        3,
        2,
        4,
        [0x500, 0x6000, dynamic_size, dynamic_size, 1],
    );
    if options.split {
        let tail = bytes[DATA_FILE + 12..DATA_FILE + 0x100].to_vec();
        put(&mut bytes, 0x700, &tail);
        program(
            &mut bytes,
            1,
            options.data_kind,
            options.data_flags,
            [DATA_FILE as u64, DATA, 12, 12, 1],
        );
        program(
            &mut bytes,
            4,
            1,
            options.data_flags,
            [0x700, DATA + 12, 0xf4, options.data_mem_size - 12, 1],
        );
    }
    bytes
}

pub(super) fn program(bytes: &mut [u8], index: usize, kind: u32, flags: u32, values: [u64; 5]) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    for (offset, value) in [8, 16, 32, 40, 48].into_iter().zip(values) {
        put(bytes, at + offset, &value.to_le_bytes());
    }
}

pub(super) fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
