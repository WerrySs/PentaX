// SPDX-License-Identifier: GPL-2.0-or-later

//! Original ELF fixtures exercise the complete parser, planner and staging path.

use super::*;
use pentax_linker::{Limits, plan_relative};
use pentax_loader::open_executable;

const RX: u64 = 0x4000;
const RW: u64 = 0x8000;
const RX_TARGET: u64 = RX + 0x20;
const RW_TARGET: u64 = RW + 0x200;
const DEFERRED_TARGET: u64 = RW + 0x220;
const BSS_TARGET: u64 = 0xc000;
const BIAS: u64 = 0x20000;
const ORIGINAL: u64 = 0xfeed_face_1234_5678;

#[test]
fn relative_staging_patches_original_bytes_and_bss_before_final_protection() {
    let bytes = fixture(BSS_TARGET);
    let source_before = bytes.clone();
    let view = open_executable(&bytes).unwrap();
    let plan = plan_relative(&view, &Limits::default(), BIAS).unwrap();
    let options = options();
    let mut image = load_relative(&plan, &options).unwrap();

    assert_eq!(image.entry(), BIAS + RX);
    assert_eq!(image.applied_relocations(), 3);
    assert_eq!(image.deferred_relocations(), 1);
    assert_eq!(image.memory().mapped_bytes(), PAGE_SIZE * 3);
    assert_eq!(image.memory().resident_bytes(), PAGE_SIZE * 3);
    assert_eq!(read_u64(image.memory(), BIAS + RX_TARGET), BIAS + 0x1234);
    assert_eq!(read_u64(image.memory(), BIAS + RW_TARGET), BIAS - 0x20);
    assert_eq!(read_u64(image.memory(), BIAS + BSS_TARGET), BIAS + 0x9000);
    assert_eq!(read_u64(image.memory(), BIAS + DEFERRED_TARGET), ORIGINAL);

    let mut zeros = [1; 8];
    image
        .memory()
        .read(BIAS + BSS_TARGET - 8, &mut zeros)
        .unwrap();
    assert_eq!(zeros, [0; 8]);
    image
        .memory()
        .read(BIAS + BSS_TARGET + 8, &mut zeros)
        .unwrap();
    assert_eq!(zeros, [0; 8]);
    let mut patched_instruction_bytes = [0; 8];
    image
        .memory()
        .fetch(BIAS + RX_TARGET, &mut patched_instruction_bytes)
        .unwrap();
    assert_eq!(patched_instruction_bytes, (BIAS + 0x1234).to_le_bytes());
    // The public API has no mutable memory getter. This child module can also
    // verify that the underlying page is protected after staging completes.
    assert_eq!(
        image.process.memory.write(BIAS + RX_TARGET, &[0]),
        Err(MemoryError::PermissionDenied(BIAS + RX_TARGET))
    );
    assert_eq!(
        image.memory().fetch(BIAS + RW_TARGET, &mut [0]),
        Err(MemoryError::PermissionDenied(BIAS + RW_TARGET))
    );
    assert_eq!(bytes, source_before);
}

#[test]
fn boundary_patch_initializes_both_guest_pages() {
    let target = BSS_TARGET - 4;
    let bytes = fixture(target);
    let view = open_executable(&bytes).unwrap();
    let plan = plan_relative(&view, &Limits::default(), BIAS).unwrap();
    let image = load_relative(&plan, &options()).unwrap();
    assert_eq!(read_u64(image.memory(), BIAS + target), BIAS + 0x9000);
    assert_eq!(image.memory().resident_bytes(), PAGE_SIZE * 3);
}

#[test]
fn mismatched_bias_is_rejected_before_mapping_or_allocation() {
    let bytes = fixture(BSS_TARGET);
    let view = open_executable(&bytes).unwrap();
    let plan = plan_relative(&view, &Limits::default(), BIAS).unwrap();
    let options = LoadOptions {
        load_bias: BIAS * 2,
        resident_limit: 0,
        mapped_limit: 0,
    };
    assert!(matches!(
        load_relative(&plan, &options),
        Err(ProcessError::RelocationBiasMismatch { planned: BIAS, requested })
            if requested == BIAS * 2
    ));
}

#[test]
fn late_patch_budget_failure_returns_no_image_and_leaves_source_reusable() {
    // Copying both initialized segments succeeds with two resident pages.
    // The final patch crosses into a previously sparse third page and fails.
    let bytes = fixture(BSS_TARGET - 4);
    let source_before = bytes.clone();
    let view = open_executable(&bytes).unwrap();
    let plan = plan_relative(&view, &Limits::default(), BIAS).unwrap();
    let limited = LoadOptions {
        resident_limit: PAGE_SIZE * 2,
        ..options()
    };
    let unrelocated = load_runtime(plan.runtime_plan(), &limited).unwrap();
    assert_eq!(unrelocated.memory.resident_bytes(), PAGE_SIZE * 2);
    assert!(matches!(
        load_relative(&plan, &limited),
        Err(ProcessError::Memory(MemoryError::BudgetExceeded))
    ));
    assert_eq!(bytes, source_before);
    assert_eq!(read_u64(&unrelocated.memory, BIAS + RX_TARGET), ORIGINAL);
    let image = load_relative(&plan, &options()).unwrap();
    assert_eq!(image.applied_relocations(), 3);
    assert_eq!(read_u64(image.memory(), BIAS + RX_TARGET), BIAS + 0x1234);
}

#[test]
fn existing_load_apis_keep_original_unrelocated_payloads() {
    let bytes = fixture(BSS_TARGET);
    let view = open_executable(&bytes).unwrap();
    let runtime = view.runtime_plan().unwrap();
    let elf = pentax_loader::parse(&bytes).unwrap();
    for image in [
        load(&elf, &options()).unwrap(),
        load_runtime(&runtime, &options()).unwrap(),
    ] {
        assert_eq!(read_u64(&image.memory, BIAS + RX_TARGET), ORIGINAL);
        assert_eq!(read_u64(&image.memory, BIAS + RW_TARGET), ORIGINAL);
        assert_eq!(read_u64(&image.memory, BIAS + BSS_TARGET), 0);
        assert_eq!(image.memory.resident_bytes(), PAGE_SIZE * 2);
    }
}

fn options() -> LoadOptions {
    LoadOptions {
        load_bias: BIAS,
        ..LoadOptions::default()
    }
}

fn read_u64(memory: &GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

/// Two independent LOAD pages followed by sparse BSS. Linking records reside
/// in initialized RW data. There are no symbols and all record indices are zero.
fn fixture(bss_target: u64) -> Vec<u8> {
    let mut bytes = vec![0; 0x8400];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &RX.to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(&mut bytes, 56, &3_u16.to_le_bytes());
    program(&mut bytes, 0, 1, 5, [RX, RX, 0x100, 0x4000, 0x4000]);
    program(&mut bytes, 1, 1, 6, [RW, RW, 0x400, 0x8000, 0x4000]);
    program(&mut bytes, 2, 2, 6, [RW, RW, 0x40, 0x40, 8]);
    for (index, (tag, value)) in [(7_i64, RW + 0x100), (8, 120), (9, 24), (0, 0)]
        .into_iter()
        .enumerate()
    {
        let at = RW as usize + index * 16;
        put(&mut bytes, at, &tag.to_le_bytes());
        put(&mut bytes, at + 8, &value.to_le_bytes());
    }
    for (index, (address, kind, addend)) in [
        (RX_TARGET, 8_u64, 0x1234_i64),
        (RW_TARGET, 8, -0x20),
        (bss_target, 8, 0x9000),
        (DEFERRED_TARGET, 42, 5),
        (u64::MAX, 0, i64::MIN),
    ]
    .into_iter()
    .enumerate()
    {
        let at = RW as usize + 0x100 + index * 24;
        put(&mut bytes, at, &address.to_le_bytes());
        put(&mut bytes, at + 8, &kind.to_le_bytes());
        put(&mut bytes, at + 16, &addend.to_le_bytes());
    }
    bytes[RX as usize] = 0xc3;
    for address in [RX_TARGET, RW_TARGET, DEFERRED_TARGET] {
        put(&mut bytes, address as usize, &ORIGINAL.to_le_bytes());
    }
    bytes
}

fn program(
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
