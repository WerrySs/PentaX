// SPDX-License-Identifier: GPL-2.0-or-later

//! Closed invented ELF images. All names, data and instructions originate here.

use super::{CONSUMER_BIAS, HleError, TLS_POINTERS};
use crate::memory::PAGE_SIZE;
use crate::process::hle::KnownFunction;
use crate::process::hle::provider::{program, put};

pub(super) const TLS_RVA: u64 = 0x4108;
pub(super) const GOT: u64 = 0x4000;
pub(super) const ENTRY: usize = 0x300;
pub(super) const RELA: usize = 0xd00;
pub(super) const CONSUMER_OFFSET: u64 = 56;
pub(super) const PROVIDER_OFFSET: u64 = 88;
pub(super) const ORIGINAL_POINTER: u64 = CONSUMER_BIAS + 0x123;
const DYNAMIC: usize = 0x900;
const STRINGS: usize = 0xb00;
const SYMBOLS: usize = 0xc00;
const PLT: usize = 0xe00;
const IMAGE_SIZE: usize = 0x4200;

pub(super) struct Images {
    pub consumer: Vec<u8>,
    pub provider: Vec<u8>,
    pub halt: u64,
    pub retired: u64,
}

fn zeroed(size: usize) -> Result<Vec<u8>, HleError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| HleError::Allocation)?;
    bytes.resize(size, 0);
    Ok(bytes)
}

struct Code {
    bytes: Vec<u8>,
    retired: u64,
}
impl Code {
    fn emit(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
        self.retired += 1;
    }
    fn fs_load(&mut self, offset: i8, wide: bool) {
        if wide {
            self.emit(&[0x64, 0x48, 0x8b, 0x43, offset as u8]);
        } else {
            self.emit(&[0x64, 0x8b, 0x43, offset as u8]);
        }
    }
    fn assert(&mut self, expected: i32, wide: bool) {
        if wide {
            self.bytes.push(0x48);
        }
        self.bytes.push(0x3d);
        self.bytes.extend_from_slice(&expected.to_le_bytes());
        // A failed assertion stops at a different breakpoint and cannot publish.
        self.bytes.extend_from_slice(&[0x74, 1, 0xcc]);
        self.retired += 2;
    }
    fn call(&mut self, slot: u64) {
        let after = ENTRY as u64 + self.bytes.len() as u64 + 6;
        let displacement = i32::try_from(GOT as i64 + (8 * slot) as i64 - after as i64).unwrap();
        self.bytes.extend_from_slice(&[0xff, 0x15]);
        self.bytes.extend_from_slice(&displacement.to_le_bytes());
        self.retired += 4; // CALL plus MOV/SYSCALL/RET in the attested provider.
    }
}

fn code() -> Result<Code, HleError> {
    let mut code = Code {
        bytes: Vec::new(),
        retired: 0,
    };
    code.bytes
        .try_reserve_exact(0x300)
        .map_err(|_| HleError::Allocation)?;
    let start = -(CONSUMER_OFFSET as i8);
    code.fs_load(start, true);
    code.assert(2, true); // Provider's runtime ID, not its module index or selector.
    code.fs_load(start + 8, true);
    code.assert(8, true); // Module-relative TLS offset, without any load bias.
    code.fs_load(start + 16, true);
    code.assert(8 - PROVIDER_OFFSET as i32, true);
    // LEA RBX,[RBP+RAX] wraps the signed TPOFF bits into a guest integer.
    // RBP is the modeled TP; no host segment register or pointer participates.
    code.emit(&[0x48, 0x8d, 0x5c, 0x05, 0]);
    code.emit(&[0x8b, 0x03]);
    code.assert(37, false);
    code.emit(&[0xbb, 0, 0, 0, 0]);
    code.fs_load(start + 24, true);
    code.assert(ORIGINAL_POINTER as i32, true); // Copied AFTER ordinary relocation.
    // Recover source through the exact typed scalar again.
    code.fs_load(start + 16, true);
    code.emit(&[0x48, 0x8d, 0x5c, 0x05, 0]);
    code.emit(&[0x48, 0x8d, 0x7d, (start + 24) as u8]); // RDI=TP-O+24.
    code.emit(&[0x48, 0x89, 0xde]); // RSI=RBX, provider TLS value address.
    code.emit(&[0xba, 8, 0, 0, 0]);
    code.call(0);
    code.assert((TLS_POINTERS[0] - CONSUMER_OFFSET + 24) as i32, true);
    code.call(1);
    code.assert(0, false);
    code.emit(&[0xbb, 0, 0, 0, 0]);
    code.fs_load(start + 24, false);
    code.emit(&[0x05, 5, 0, 0, 0]);
    code.assert(42, false);
    code.emit(&[0x64, 0x48, 0x89, 0x43, (start + 24) as u8]);
    code.emit(&[0xcc]);
    Ok(code)
}

fn string(bytes: &mut Vec<u8>, value: &[u8]) -> u32 {
    let offset = bytes.len() as u32;
    bytes.extend_from_slice(value);
    bytes.push(0);
    offset
}

fn relocation(bytes: &mut [u8], at: usize, target: u64, symbol: u32, kind: u32, addend: i64) {
    put(bytes, at, &target.to_le_bytes());
    put(
        bytes,
        at + 8,
        &((u64::from(symbol) << 32) | u64::from(kind)).to_le_bytes(),
    );
    put(bytes, at + 16, &addend.to_le_bytes());
}

fn image(provider: bool, text: &[u8]) -> Result<Vec<u8>, HleError> {
    let mut bytes = zeroed(IMAGE_SIZE)?;
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        24,
        &(if provider { 0 } else { ENTRY as u64 }).to_le_bytes(),
    );
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(
        &mut bytes,
        56,
        &(if provider { 4_u16 } else { 5 }).to_le_bytes(),
    );
    program(&mut bytes, 0, 1, 5, [0, 0, 0x1000, PAGE_SIZE, PAGE_SIZE]);
    program(&mut bytes, 1, 1, 6, [GOT, GOT, 0x200, PAGE_SIZE, PAGE_SIZE]);
    let (initialized, memory) = if provider { (16, 24) } else { (32, 48) };
    program(
        &mut bytes,
        3,
        7,
        4,
        [TLS_RVA, TLS_RVA, initialized, memory, 32],
    );
    if !provider {
        program(
            &mut bytes,
            4,
            0x6474_e552,
            4,
            [0, GOT, 0, PAGE_SIZE, PAGE_SIZE],
        );
        put(&mut bytes, ENTRY, text);
    }
    let mut strings = Vec::new();
    strings
        .try_reserve_exact(0x100)
        .map_err(|_| HleError::Allocation)?;
    strings.push(0);
    let tls_module = string(&mut strings, b"pentax-original-tls");
    let tls_library = string(&mut strings, b"pentax-original-tls-lib");
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(20)
        .map_err(|_| HleError::Allocation)?;
    if provider {
        let name = string(&mut strings, b"AAAAAAAAAAA#H#G");
        let at = SYMBOLS + 24;
        put(&mut bytes, at, &name.to_le_bytes());
        bytes[at + 4] = 0x16; // GLOBAL TLS; st_value is an offset, not a VA.
        put(&mut bytes, at + 6, &1_u16.to_le_bytes());
        put(&mut bytes, at + 8, &8_u64.to_le_bytes());
        put(&mut bytes, at + 16, &8_u64.to_le_bytes());
        put(&mut bytes, TLS_RVA as usize, &91_u64.to_le_bytes());
        put(&mut bytes, TLS_RVA as usize + 8, &37_u64.to_le_bytes());
        entries.extend([
            (
                0x6100_000d_i64,
                u64::from(tls_module) | (2 << 32) | (1 << 40) | (6 << 48),
            ),
            (0x6100_0013, u64::from(tls_library) | (1 << 32) | (7 << 48)),
        ]);
    } else {
        let libc_module = string(&mut strings, b"LibcInternal");
        let libc_library = string(&mut strings, b"LibcInternal");
        for (index, function) in [KnownFunction::Memcpy, KnownFunction::Memcmp]
            .iter()
            .enumerate()
        {
            let mut qualified = [0; 15];
            qualified[..11].copy_from_slice(function.nid());
            qualified[11..].copy_from_slice(b"#D#C");
            let name = string(&mut strings, &qualified);
            let at = SYMBOLS + (index + 1) * 24;
            put(&mut bytes, at, &name.to_le_bytes());
            bytes[at + 4] = 0x12; // GLOBAL undefined FUNC.
            relocation(
                &mut bytes,
                PLT + index * 24,
                GOT + index as u64 * 8,
                index as u32 + 1,
                7,
                0,
            );
        }
        let name = string(&mut strings, b"AAAAAAAAAAA#F#E");
        put(&mut bytes, SYMBOLS + 3 * 24, &name.to_le_bytes());
        bytes[SYMBOLS + 3 * 24 + 4] = 0x16; // Undefined STT_TLS.
        for (index, kind) in [16, 17, 18].into_iter().enumerate() {
            relocation(
                &mut bytes,
                RELA + index * 24,
                TLS_RVA + index as u64 * 8,
                3,
                kind,
                0,
            );
        }
        relocation(&mut bytes, RELA + 3 * 24, TLS_RVA + 24, 0, 8, 0x123);
        for index in 0..4 {
            put(
                &mut bytes,
                TLS_RVA as usize + index * 8,
                &(0xabab_0000_u64 + index as u64).to_le_bytes(),
            );
        }
        entries.extend([
            (7_i64, RELA as u64),
            (8, 4 * 24),
            (9, 24),
            (23, PLT as u64),
            (2, 2 * 24),
            (20, 7),
            (
                0x6100_000f,
                u64::from(libc_module) | (1 << 32) | (1 << 40) | (2 << 48),
            ),
            (0x6100_0015, u64::from(libc_library) | (1 << 32) | (3 << 48)),
            (
                0x6100_000f,
                u64::from(tls_module) | (2 << 32) | (1 << 40) | (4 << 48),
            ),
            (0x6100_0015, u64::from(tls_library) | (1 << 32) | (5 << 48)),
        ]);
    }
    entries.extend([
        (5, STRINGS as u64),
        (10, strings.len() as u64),
        (6, SYMBOLS as u64),
        (11, 24),
        (39, if provider { 48 } else { 96 }),
        (0, 0),
    ]);
    if strings.len() > SYMBOLS - STRINGS
        || DYNAMIC + entries.len() * 16 > STRINGS
        || ENTRY + text.len() > DYNAMIC
    {
        return Err(HleError::Fixture("combined original image layout"));
    }
    put(&mut bytes, STRINGS, &strings);
    for (index, (tag, value)) in entries.iter().enumerate() {
        put(&mut bytes, DYNAMIC + index * 16, &tag.to_le_bytes());
        put(&mut bytes, DYNAMIC + index * 16 + 8, &value.to_le_bytes());
    }
    let size = entries.len() as u64 * 16;
    program(
        &mut bytes,
        2,
        2,
        4,
        [DYNAMIC as u64, DYNAMIC as u64, size, size, 8],
    );
    Ok(bytes)
}

/// Only these two fixed original sources are accepted by the closed consumer.
pub(super) fn images() -> Result<Images, HleError> {
    let code = code()?;
    Ok(Images {
        consumer: image(false, &code.bytes)?,
        provider: image(true, &[])?,
        halt: ENTRY as u64 + code.bytes.len() as u64,
        retired: code.retired,
    })
}
