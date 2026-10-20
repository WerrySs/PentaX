// SPDX-License-Identifier: GPL-2.0-only

//! Original byte fixtures and ELF metadata, without external binaries.

use pentax_linker::{ModuleInput, SymbolicLimits, SymbolicLinkPlan, plan_symbolic};
use pentax_loader::open_executable;

pub(super) const BIAS: u64 = 0x10_0000;
pub(super) const ENTRY: usize = 0x200;
pub(super) const DATA: u64 = 0x4000;
pub(super) const PAGE: u64 = 0x4000;

pub(super) fn image(code: &[u8]) -> Vec<u8> {
    assert!(ENTRY + code.len() <= 0x800);
    let mut bytes = vec![0; 0x4100];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &(ENTRY as u64).to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(&mut bytes, 56, &2_u16.to_le_bytes());
    program(
        &mut bytes,
        0,
        1,
        5,
        [0, 0, (ENTRY + code.len()) as u64, PAGE, PAGE],
    );
    program(&mut bytes, 1, 1, 6, [DATA, DATA, 0x100, PAGE, PAGE]);
    put(&mut bytes, ENTRY, code);
    bytes
}

pub(super) fn with_relocation(kind: u32, target: u64, addend: i64) -> Vec<u8> {
    // MOV RAX,imm64;RET. The retained relative patch changes the immediate.
    let mut bytes = image(&[0x48, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0xc3]);
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    program(&mut bytes, 0, 1, 5, [0, 0, 0xa00, PAGE, PAGE]);
    let dynamic = [(7_i64, 0x900_u64), (8, 24), (9, 24), (0, 0)];
    for (index, (tag, value)) in dynamic.iter().enumerate() {
        put(&mut bytes, 0x800 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x808 + index * 16, &value.to_le_bytes());
    }
    program(&mut bytes, 2, 2, 4, [0x800, 0x800, 64, 64, 8]);
    put(&mut bytes, 0x900, &target.to_le_bytes());
    put(&mut bytes, 0x908, &u64::from(kind).to_le_bytes());
    put(&mut bytes, 0x910, &addend.to_le_bytes());
    bytes
}

pub(super) fn link<'a>(images: &[(&'a [u8], u64)]) -> SymbolicLinkPlan<'a> {
    let inputs = images
        .iter()
        .map(|(bytes, bias)| ModuleInput::new(open_executable(bytes).unwrap(), *bias))
        .collect::<Vec<_>>();
    plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap()
}
fn program(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, address, file, memory, align]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    for (field, value) in [
        (8, offset),
        (16, address),
        (32, file),
        (40, memory),
        (48, align),
    ] {
        put(bytes, at + field, &value.to_le_bytes());
    }
}
fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
