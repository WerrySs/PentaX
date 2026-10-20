// SPDX-License-Identifier: GPL-2.0-or-later

//! Fresh ELF, SCE metadata and instructions generated for PentaX fixture policy.

use super::{
    ARG0_OFFSET, ARG1_OFFSET, Result, SERVICE, StaticTlsLimits, fixture_template, link_original,
    plan_static_tls, require,
};

pub(super) const CONSUMER_BIAS: u64 = 0x10_0000;
pub(super) const PROVIDER_BIAS: u64 = 0x20_0000;
pub(super) const ENTRY: u64 = 0x400;
pub(super) const PROVIDER_ENTRY: u64 = 0x400;
pub(super) const CTOR: u64 = 0x700;
pub(super) const GOT: u64 = 0x4000;
pub(super) const TLS_HEADER: usize = 2;

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub stack: u64,
    pub boot: u64,
    pub tls_pages: [u64; 2],
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            stack: 0x80_0000,
            boot: 0x90_0000,
            tls_pages: [0xa0_4000, 0xa1_0000],
        }
    }
}

pub(super) struct Code {
    pub tls_offset: u64,
    pub after_provider: u64,
    pub service_resume: u64,
    pub halt_resume: u64,
    #[cfg(test)]
    pub service_number_at: usize,
    #[cfg(test)]
    pub service_argument_at: usize,
}
pub(super) struct Images {
    pub consumer: Vec<u8>,
    pub provider: Vec<u8>,
    pub code: Code,
}

fn assertion(code: &mut Vec<u8>, expected: u32, wide: bool) {
    if wide {
        code.push(0x48);
    }
    code.push(0x3d); // CMP EAX/RAX, imm32, then skip a failure breakpoint.
    code.extend_from_slice(&expected.to_le_bytes());
    code.extend_from_slice(&[0x74, 1, 0xcc]);
}

fn entry(layout: Layout, tls_offset: u64) -> (Vec<u8>, Code) {
    let mut bytes = vec![0x8b, 0x07]; // argc = u32 [RDI+0].
    assertion(&mut bytes, 2, false);
    bytes.extend_from_slice(&[0x48, 0x8b, 0x47, 8]);
    assertion(&mut bytes, (layout.boot + ARG0_OFFSET) as u32, true);
    bytes.extend_from_slice(&[0x8b, 0x00]);
    assertion(&mut bytes, u32::from_le_bytes(*b"pent"), false);
    bytes.extend_from_slice(&[0x48, 0x8b, 0x47, 8, 0x8b, 0x40, 4]);
    assertion(&mut bytes, u32::from_le_bytes(*b"ax\0\0"), false);
    bytes.extend_from_slice(&[0x48, 0x8b, 0x47, 16]);
    assertion(&mut bytes, (layout.boot + ARG1_OFFSET) as u32, true);
    bytes.extend_from_slice(&[0x8b, 0x40, 8]);
    assertion(&mut bytes, u32::from_le_bytes(*b"est\0"), false);
    bytes.extend_from_slice(&[0x48, 0x8b, 0x47, 24]);
    assertion(&mut bytes, 0, true); // Explicit terminating argv slot.
    bytes.push(0xe8);
    let next = ENTRY + bytes.len() as u64 + 4;
    bytes.extend_from_slice(&((CTOR as i64 - next as i64) as i32).to_le_bytes());
    bytes.extend_from_slice(&[0xff, 0x15]); // CALL qword [RIP+disp32] through relocated GOT.
    let after_provider = ENTRY + bytes.len() as u64 + 4;
    bytes.extend_from_slice(&((GOT as i64 - after_provider as i64) as i32).to_le_bytes());
    assertion(&mut bytes, 42, true);
    bytes.push(0xbf); // Synthetic service arguments RDI=19, RSI=23.
    #[cfg(test)]
    let service_argument_at = ENTRY as usize + bytes.len();
    bytes.extend_from_slice(&19_u32.to_le_bytes());
    bytes.push(0xbe);
    bytes.extend_from_slice(&23_u32.to_le_bytes());
    bytes.push(0xb8);
    #[cfg(test)]
    let service_number_at = ENTRY as usize + bytes.len();
    bytes.extend_from_slice(&(SERVICE as u32).to_le_bytes());
    bytes.extend_from_slice(&[0x0f, 0x05]);
    let service_resume = ENTRY + bytes.len() as u64;
    assertion(&mut bytes, 42, true);
    bytes.push(0xcc); // Only this breakpoint can complete the fixture.
    let halt_resume = ENTRY + bytes.len() as u64;
    (
        bytes,
        Code {
            tls_offset,
            after_provider,
            service_resume,
            halt_resume,
            #[cfg(test)]
            service_number_at,
            #[cfg(test)]
            service_argument_at,
        },
    )
}

pub(super) fn images(layout: Layout) -> Result<Images> {
    let mut consumer = module(false);
    let mut provider = module(true);
    put(&mut provider, 0x908, &37_u64.to_le_bytes());
    // Inspect only these fixed original prototypes to derive the shared TLS
    // policy's encodable offset. Final immutable images are linked again and
    // must retain this exact offset before the closed interpreter can start.
    let tls_offset = {
        let link = link_original(&consumer, &provider)?;
        let plan = plan_static_tls(&link, &StaticTlsLimits::default())?;
        fixture_template(&plan)?.tp_offset
    };
    require((16..=128).contains(&tls_offset), "fixture disp8 TLS range")?;
    let value_disp = (-(tls_offset as i16)) as i8 as u8;
    let counter_disp = (-(tls_offset as i16) + 8) as i8 as u8;
    let (entry, code) = entry(layout, tls_offset);
    put(&mut consumer, ENTRY as usize, &entry);
    let mut constructor = vec![0x64, 0x48, 0x8b, 0x43, counter_disp];
    assertion(&mut constructor, 0, true);
    constructor.extend_from_slice(&[0x05, 1, 0, 0, 0, 0x64, 0x48, 0x89, 0x43, counter_disp, 0xc3]);
    put(&mut consumer, CTOR as usize, &constructor);
    put(
        &mut provider,
        PROVIDER_ENTRY as usize,
        &[0x64, 0x8b, 0x43, value_disp, 0x05, 5, 0, 0, 0, 0xc3],
    );
    Ok(Images {
        consumer,
        provider,
        code,
    })
}

fn module(provider: bool) -> Vec<u8> {
    let mut bytes = vec![0; if provider { 0x1000 } else { 0x4100 }];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        24,
        &(if provider { 0 } else { ENTRY }).to_le_bytes(),
    );
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(
        &mut bytes,
        56,
        &(if provider { 3_u16 } else { 4 }).to_le_bytes(),
    );
    program(&mut bytes, 0, 1, 5, [0, 0, 0x1000, 0x1000, 0x4000]);
    let dynamic_header = if provider { 1 } else { 2 };
    if provider {
        // Nonzero p_vaddr alignment bias is deliberately retained by startup.
        program(&mut bytes, TLS_HEADER, 7, 4, [0x908, 0x908, 8, 16, 32]);
    } else {
        program(&mut bytes, 1, 1, 6, [0x4000, 0x4000, 0x100, 0x4000, 0x4000]);
        program(
            &mut bytes,
            3,
            0x6474_e552,
            4,
            [0, 0x4000, 0, 0x4000, 0x4000],
        );
    }
    let mut strings = vec![0];
    let qualified = string(
        &mut strings,
        if provider {
            b"AAAAAAAAAAA#R#f"
        } else {
            b"AAAAAAAAAAA#B#C"
        },
    );
    let module_name = string(&mut strings, b"pentax-startup-provider");
    let library_name = string(&mut strings, b"pentax-original-library");
    put(&mut bytes, 0xa00, &strings);
    let symbol = 0xb00 + 24;
    put(&mut bytes, symbol, &qualified.to_le_bytes());
    bytes[symbol + 4] = 0x12; // GLOBAL FUNC, DEFAULT visibility.
    put(&mut bytes, symbol + 6, &u16::from(provider).to_le_bytes());
    put(
        &mut bytes,
        symbol + 8,
        &(if provider { PROVIDER_ENTRY } else { 0 }).to_le_bytes(),
    );
    put(&mut bytes, symbol + 16, &10_u64.to_le_bytes());
    let (module_tag, module_id, library_tag, library_id) = if provider {
        (0x6100_000d_i64, 31_u64, 0x6100_0013_i64, 17_u64)
    } else {
        (0x6100_0045, 2, 0x6100_0049, 1)
    };
    let mut entries = vec![
        (5_i64, 0xa00),
        (10, strings.len() as u64),
        (6, 0xb00),
        (11, 24),
        (39, 48),
        (
            module_tag,
            u64::from(module_name) | (1 << 40) | (module_id << 48),
        ),
        (
            library_tag,
            u64::from(library_name) | (1 << 32) | (library_id << 48),
        ),
    ];
    if !provider {
        put(&mut bytes, 0xb80, &GOT.to_le_bytes());
        put(&mut bytes, 0xb88, &((1_u64 << 32) | 7).to_le_bytes());
        put(&mut bytes, 0xb90, &99_i64.to_le_bytes()); // JUMP_SLOT ignores A.
        entries.extend([(23, 0xb80), (2, 24), (20, 7)]);
    }
    entries.push((0, 0));
    for (index, (tag, value)) in entries.iter().enumerate() {
        put(&mut bytes, 0x800 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x808 + index * 16, &value.to_le_bytes());
    }
    let dynamic_size = entries.len() as u64 * 16;
    program(
        &mut bytes,
        dynamic_header,
        2,
        4,
        [0x800, 0x800, dynamic_size, dynamic_size, 8],
    );
    bytes
}

fn string(bytes: &mut Vec<u8>, value: &[u8]) -> u32 {
    let offset = bytes.len() as u32;
    bytes.extend_from_slice(value);
    bytes.push(0);
    offset
}

pub(super) fn program(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, address, file_size, mem_size, alignment]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &address.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &alignment.to_le_bytes());
}
pub(super) fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
