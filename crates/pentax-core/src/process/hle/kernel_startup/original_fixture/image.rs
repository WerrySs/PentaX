// SPDX-License-Identifier: GPL-2.0-only

//! Original encoded consumer and metadata; no SDK, title or upstream bytes.

use crate::memory::PAGE_SIZE;
use crate::process::hle::provider::{program, put};
use crate::process::hle::{HleError, KnownFunction};
use pentax_kernel::Service;

pub(super) const ENTRY: usize = 0x400;
pub(super) const EXIT_CALLBACK: usize = 0x780;
pub(super) const TLS_RVA: u64 = 0x4108;
pub(super) const SYMBOL_OFFSET: u64 = 24;
pub(super) const OUTPUT: u64 = 0x8000;
pub(super) const NAME: u64 = OUTPUT + 0x40;
const GOT: u64 = 0x4000;
const CODE_CAPACITY: usize = 0x300;

struct Code {
    bytes: Vec<u8>,
}
impl Code {
    fn emit(&mut self, bytes: &[u8]) -> Result<(), HleError> {
        if self.bytes.len() + bytes.len() > CODE_CAPACITY {
            return Err(HleError::InvalidProvider("original instruction layout"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    fn absolute(&mut self, opcode: &[u8], address: u64) -> Result<(), HleError> {
        self.emit(opcode)?;
        self.emit(&address.to_le_bytes())
    }
    fn rip(&mut self, opcode: &[u8], rva: u64) -> Result<(), HleError> {
        let next = ENTRY as u64 + self.bytes.len() as u64 + opcode.len() as u64 + 4;
        let displacement = i32::try_from(i128::from(rva) - i128::from(next))
            .map_err(|_| HleError::InvalidProvider("original RIP displacement"))?;
        self.emit(opcode)?;
        self.emit(&displacement.to_le_bytes())
    }
}

pub(super) fn code(base: u64) -> Result<Vec<u8>, HleError> {
    let mut code = Code { bytes: Vec::new() };
    code.bytes
        .try_reserve_exact(CODE_CAPACITY)
        .map_err(|_| HleError::Allocation)?;
    code.emit(&[0x41, 0x54])?; // PUSH R12; preserve guest nonvolatile and align calls.
    // Observe the actual initial call context in guest code. These outputs are
    // checked by the factory after return; Rust does not fill them in advance.
    code.rip(&[0x48, 0x89, 0x3d], OUTPUT + 0x90)?; // RDI: argument object.
    code.rip(&[0x48, 0x89, 0x35], OUTPUT + 0x98)?; // RSI: guest callback address.
    code.rip(&[0x48, 0x89, 0x2d], OUTPUT + 0xa0)?; // RBP: zero root frame.
    code.emit(&[0x48, 0x8b, 0x07])?; // argc and zero pad.
    code.rip(&[0x48, 0x89, 0x05], OUTPUT + 0xa8)?;
    code.emit(&[0x48, 0x8b, 0x47, 8])?; // argv[0].
    code.emit(&[0x48, 0x8b, 0x00])?; // Actual program-name bytes.
    code.rip(&[0x48, 0x89, 0x05], OUTPUT + 0xb0)?;
    code.emit(&[0x48, 0x8b, 0x45, 0])?;
    code.rip(&[0x48, 0x89, 0x05], OUTPUT + 0xb8)?;
    code.emit(&[0x48, 0x8b, 0x45, 8])?;
    code.rip(&[0x48, 0x89, 0x05], OUTPUT + 0xc0)?;
    code.rip(&[0xff, 0x15], GOT)?; // PthreadSelf, real owned prefix.
    code.emit(&[0x44, 0x8b, 0x20])?; // MOV R12D,[RAX]: initial thread ID73.
    code.absolute(&[0x48, 0xbf], base + OUTPUT + 8)?; // RDI=semaphore output.
    code.absolute(&[0x48, 0xbe], base + NAME)?;
    code.emit(&[0xba, 1, 0, 0, 0])?;
    code.emit(&[0xb9, 17, 0, 0, 0])?; // Original RCX, never syscall continuation.
    code.emit(&[0x41, 0xb8, 23, 0, 0, 0])?;
    code.emit(&[0x45, 0x31, 0xc9])?; // R9D=0, sixth library-call argument.
    code.rip(&[0xff, 0x15], GOT + 8)?;
    code.rip(&[0x48, 0x8b, 0x3d], OUTPUT + 8)?; // Handle from actual kernel output.
    code.emit(&[0xbe, 17, 0, 0, 0])?;
    code.rip(&[0xff, 0x15], GOT + 16)?; // Poll consumes exactly the original17.
    code.emit(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0])?; // MOV RAX,FS:[0]: actual TCB self pointer.
    code.emit(&[0x64, 0x4c, 0x89, 0x24, 0x25, 0x28, 0, 0, 0])?; // MOV FS:[0x28],R12.
    code.emit(&[0x64, 0x4c, 0x8b, 0x14, 0x25, 0x28, 0, 0, 0])?; // MOV R10,FS:[0x28].
    code.rip(&[0x48, 0x03, 0x05], TLS_RVA + 16)?; // Add actual typed TPOFF64 bits.
    code.emit(&[0x48, 0x89, 0xc7])?; // RDI=first instance's actual TLS symbol.
    code.emit(&[0x48, 0x8b, 0x00])?;
    code.emit(&[0x4c, 0x01, 0xd0])?; // RAX=37+73, using the modeled FS load.
    code.emit(&[0x48, 0x83, 0xe8, 68])?; // RAX=42.
    code.rip(&[0x48, 0x89, 0x05], OUTPUT + 0x80)?;
    code.absolute(&[0x48, 0xbe], base + OUTPUT + 0x80)?;
    code.emit(&[0xba, 8, 0, 0, 0])?;
    code.rip(&[0xff, 0x15], GOT + 24)?; // Memcpy writes the same native TLS page.
    code.emit(&[0x48, 0x8b, 0x00])?; // Read the returned TLS pointer, obtain42.
    code.emit(&[0x41, 0x5c, 0xc3])?;
    Ok(code.bytes)
}

fn string(strings: &mut Vec<u8>, bytes: &[u8]) -> Result<u64, HleError> {
    if strings.len() + bytes.len() + 1 > 0x200 {
        return Err(HleError::InvalidProvider("original string layout"));
    }
    let at = strings.len() as u64;
    strings.extend_from_slice(bytes);
    strings.push(0);
    Ok(at)
}

pub(super) fn original_image(base: u64) -> Result<Vec<u8>, HleError> {
    if base
        .checked_add(super::ARENA)
        .is_none_or(|end| end > crate::memory::ADDRESS_LIMIT)
    {
        return Err(HleError::InvalidProvider("original address range"));
    }
    let mut image = Vec::new();
    image
        .try_reserve_exact(0x8100)
        .map_err(|_| HleError::Allocation)?;
    image.resize(0x8100, 0);
    image[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut image, 16, &3_u16.to_le_bytes());
    put(&mut image, 18, &62_u16.to_le_bytes());
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    put(&mut image, 52, &64_u16.to_le_bytes());
    put(&mut image, 54, &56_u16.to_le_bytes());
    put(&mut image, 56, &6_u16.to_le_bytes());
    program(&mut image, 0, 1, 5, [0, 0, 0x1000, PAGE_SIZE, PAGE_SIZE]);
    program(&mut image, 1, 1, 6, [GOT, GOT, 0x200, PAGE_SIZE, PAGE_SIZE]);
    program(
        &mut image,
        2,
        1,
        6,
        [OUTPUT, OUTPUT, 0x100, PAGE_SIZE, PAGE_SIZE],
    );
    program(&mut image, 4, 7, 4, [TLS_RVA, TLS_RVA, 32, 48, 32]);
    program(
        &mut image,
        5,
        0x6474_e552,
        4,
        [0, GOT, 0, PAGE_SIZE, PAGE_SIZE],
    );
    put(&mut image, ENTRY, &code(base)?);
    // Original callback, never a substituted implementation of module shutdown.
    image[EXIT_CALLBACK] = 0xc3;
    put(
        &mut image,
        TLS_RVA as usize + SYMBOL_OFFSET as usize,
        &37_u64.to_le_bytes(),
    );
    put(&mut image, NAME as usize, b"original-semaphore\0");
    let mut strings = Vec::new();
    strings
        .try_reserve_exact(0x200)
        .map_err(|_| HleError::Allocation)?;
    strings.push(0);
    let kernel_module = string(&mut strings, b"libkernel")?;
    let kernel_library = string(&mut strings, b"libkernel")?;
    let libc_module = string(&mut strings, b"LibcInternal")?;
    let libc_library = string(&mut strings, b"LibcInternal")?;
    for (index, service) in [Service::PthreadSelf, Service::CreateSema, Service::PollSema]
        .into_iter()
        .enumerate()
    {
        let registration = Service::registrations()
            .iter()
            .find(|registration| registration.service == service)
            .ok_or(HleError::InvalidProvider("original kernel registration"))?;
        if registration.nid.len() != 11 {
            return Err(HleError::InvalidProvider("original kernel NID width"));
        }
        let mut qualified = [0; 15];
        qualified[..11].copy_from_slice(registration.nid.as_bytes());
        qualified[11..].copy_from_slice(b"#D#C");
        let name = string(&mut strings, &qualified)?;
        imported(&mut image, index, name);
    }
    let mut qualified = [0; 15];
    qualified[..11].copy_from_slice(KnownFunction::Memcpy.nid());
    qualified[11..].copy_from_slice(b"#F#E");
    let name = string(&mut strings, &qualified)?;
    imported(&mut image, 3, name);
    put(&mut image, 0x900, &strings);
    let tls_symbol = 0xb00 + 5 * 24;
    image[tls_symbol + 4] = 6; // LOCAL STT_TLS definition, symbol5.
    put(&mut image, tls_symbol + 6, &1_u16.to_le_bytes());
    put(&mut image, tls_symbol + 8, &SYMBOL_OFFSET.to_le_bytes());
    put(&mut image, tls_symbol + 16, &8_u64.to_le_bytes());
    for (index, kind) in [16_u64, 17, 18].into_iter().enumerate() {
        put(
            &mut image,
            0xc00 + index * 24,
            &(TLS_RVA + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut image,
            0xc08 + index * 24,
            &((5_u64 << 32) | kind).to_le_bytes(),
        );
    }
    let entries = [
        (5_i64, 0x900),
        (10, strings.len() as u64),
        (6, 0xb00),
        (39, 6 * 24),
        (11, 24),
        (23, 0xd00),
        (2, 4 * 24),
        (20, 7),
        (7, 0xc00),
        (8, 3 * 24),
        (9, 24),
        (
            0x6100_000f,
            kernel_module | (1 << 32) | (1 << 40) | (2 << 48),
        ),
        (0x6100_0015, kernel_library | (1 << 32) | (3 << 48)),
        (0x6100_000f, libc_module | (1 << 32) | (1 << 40) | (4 << 48)),
        (0x6100_0015, libc_library | (1 << 32) | (5 << 48)),
        (0, 0),
    ];
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(&mut image, 0x800 + index * 16, &tag.to_le_bytes());
        put(&mut image, 0x808 + index * 16, &value.to_le_bytes());
    }
    program(
        &mut image,
        3,
        2,
        4,
        [
            0x800,
            0x800,
            (entries.len() * 16) as u64,
            (entries.len() * 16) as u64,
            8,
        ],
    );
    Ok(image)
}

fn imported(image: &mut [u8], index: usize, name: u64) {
    let symbol = index + 1;
    put(image, 0xb00 + symbol * 24, &(name as u32).to_le_bytes());
    image[0xb00 + symbol * 24 + 4] = 0x12;
    put(
        image,
        0xd00 + index * 24,
        &(GOT + index as u64 * 8).to_le_bytes(),
    );
    put(
        image,
        0xd08 + index * 24,
        &(((symbol as u64) << 32) | 7).to_le_bytes(),
    );
}
