// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: live guest-only unwind metadata; see NOTICE.

use super::{KernelMemory, error};
use pentax_kernel::{Failure, GuestError, ModuleUnwindInfo};
use pentax_loader::VirtualRange;
use pentax_platform::native_memory::NativeMemoryError;

fn charge(work: &mut u64, amount: u64, maximum: u64) -> Result<(), Failure> {
    *work = work
        .checked_add(amount)
        .filter(|total| *total <= maximum)
        .ok_or(Failure::Limit("module unwind work"))?;
    Ok(())
}

fn relocated(bias: u64, range: VirtualRange) -> Result<VirtualRange, Failure> {
    let address = bias
        .checked_add(range.address)
        .ok_or(GuestError::AddressOverflow)?;
    let end = address
        .checked_add(range.size)
        .ok_or(GuestError::AddressOverflow)?;
    if end > crate::memory::ADDRESS_LIMIT {
        return Err(GuestError::AddressOverflow.into());
    }
    Ok(VirtualRange {
        address,
        size: range.size,
    })
}

pub(super) fn lookup(
    memory: &mut KernelMemory<'_, '_, '_>,
    address: u64,
) -> Result<Option<ModuleUnwindInfo>, Failure> {
    let source = memory
        .source
        .ok_or(Failure::Unsupported("image-bound module unwind metadata"))?;
    let maximum = memory
        .lookup_limits
        .ok_or(Failure::Unsupported("module unwind limits"))?
        .max_work;
    let work = memory
        .lookup_work
        .as_deref_mut()
        .ok_or(Failure::Unsupported("module unwind work"))?;
    // Guest-handle membership is a bounded linear lookup per module. Charge
    // the complete quadratic ceiling before walking either retained table.
    let count = source.modules().len() as u64;
    let initial = count
        .checked_mul(count)
        .and_then(|n| n.checked_add(16))
        .ok_or(Failure::Limit("module unwind work"))?;
    let scan = source.modules().iter().try_fold(initial, |total, module| {
        total
            .checked_add(module.runtime_plan().segments.len() as u64 + 2)
            .ok_or(Failure::Limit("module unwind work"))
    })?;
    charge(work, scan, maximum)?;
    let mut found = None;
    for (index, module) in source.modules().iter().enumerate() {
        let guest = memory
            .loaded_modules
            .as_deref()
            .map_or(index < memory.supplied_modules, |registry| {
                registry.contains_guest_module(index)
            });
        if !guest {
            continue;
        }
        for segment in &module.runtime_plan().segments {
            let range = relocated(
                module.load_bias(),
                VirtualRange {
                    address: segment.vaddr,
                    size: segment.mem_size,
                },
            )?;
            if range.address <= address && address < range.address + range.size {
                if found.is_some_and(|previous| previous != index) {
                    return Err(Failure::Unsupported("overlapping guest module ranges"));
                }
                found = Some(index);
            }
        }
    }
    let Some(index) = found else {
        return Ok(None);
    };
    let module = &source.modules()[index];
    let runtime = module.runtime_plan();
    // Precharge every parser step and live page walk before inspecting tables.
    let sections = if runtime.metadata.self_metadata.is_none() {
        let raw = module
            .source_bytes()
            .get(60..62)
            .ok_or(Failure::Unsupported("module ELF header"))?;
        u16::from_le_bytes([raw[0], raw[1]]) as u64
    } else {
        0
    };
    let mut amount = sections
        .checked_mul(128)
        .and_then(|n| n.checked_add(1024))
        .ok_or(Failure::Limit("module unwind work"))?;
    for segment in &runtime.segments {
        amount = amount
            .checked_add(segment.mem_size.div_ceil(crate::memory::PAGE_SIZE) + 4)
            .ok_or(Failure::Limit("module unwind work"))?;
    }
    for ph in &runtime.metadata.program_headers {
        amount = amount
            .checked_add(4)
            .ok_or(Failure::Limit("module unwind work"))?;
        if ph.kind == 0x6474_e550 {
            amount = amount
                .checked_add(
                    ph.mem_size
                        .checked_mul(3)
                        .ok_or(Failure::Limit("module unwind work"))?,
                )
                .ok_or(Failure::Limit("module unwind work"))?;
        }
    }
    if let Some(dynamic) = &runtime.dynamic {
        amount = amount
            .checked_add(dynamic.entries.len() as u64 * 4)
            .ok_or(Failure::Limit("module unwind work"))?;
    }
    charge(work, amount, maximum)?;
    let metadata = runtime
        .unwind_metadata(module.source_bytes())
        .map_err(|_| Failure::Unsupported("checked module unwind metadata"))?;
    let mut first = u64::MAX;
    let mut end = 0;
    for segment in &runtime.segments {
        if segment.mem_size == 0 {
            continue;
        }
        let range = relocated(
            module.load_bias(),
            VirtualRange {
                address: segment.vaddr,
                size: segment.mem_size,
            },
        )?;
        match memory.paused.preflight_read(range.address, range.size) {
            Ok(()) => (),
            Err(NativeMemoryError::Unmapped(_)) => return Ok(None),
            Err(value) => return Err(Failure::Guest(error(value, false))),
        }
        first = first.min(range.address);
        end = end.max(range.address + range.size);
    }
    if end <= first {
        return Ok(None);
    }
    // Attest the queried initialized source byte and every header byte. The
    // header's decoded pointer must describe the live owned image, not a stale
    // address reused for unrelated writable data.
    let original_pc = address
        .checked_sub(module.load_bias())
        .ok_or(GuestError::AddressOverflow)?;
    let immutable_code = runtime.segments.iter().any(|segment| {
        segment.flags.executable()
            && !segment.flags.writable()
            && segment.vaddr <= original_pc
            && segment
                .vaddr
                .checked_add(segment.file_bytes.len() as u64)
                .is_some_and(|end| original_pc < end)
    });
    if immutable_code {
        let expected = runtime
            .unwind_source(VirtualRange {
                address: original_pc,
                size: 1,
            })
            .map_err(|_| Failure::Unsupported("module address source"))?;
        let mut actual = [0];
        memory
            .paused
            .read(address, &mut actual)
            .map_err(|v| Failure::Guest(error(v, false)))?;
        if actual.as_slice() != expected {
            return Err(Failure::Unsupported("changed module address source"));
        }
    }
    let mut header_address = 0;
    if let Some(range) = metadata.header {
        let expected = runtime
            .unwind_source(range)
            .map_err(|_| Failure::Unsupported("unwind header source"))?;
        let live = relocated(module.load_bias(), range)?;
        header_address = live.address;
        let mut actual = [0; 512];
        for (index, chunk) in expected.chunks(512).enumerate() {
            memory
                .paused
                .read(
                    live.address + index as u64 * 512,
                    &mut actual[..chunk.len()],
                )
                .map_err(|v| Failure::Guest(error(v, false)))?;
            if actual[..chunk.len()] != *chunk {
                return Err(Failure::Unsupported("changed module unwind header"));
            }
        }
    }
    let frame = metadata
        .frame
        .map(|range| relocated(module.load_bias(), range))
        .transpose()?;
    Ok(Some(ModuleUnwindInfo {
        name: metadata.name,
        eh_frame_header: header_address,
        eh_frame: frame.map_or(0, |range| range.address),
        eh_frame_size: frame.map_or(0, |range| range.size),
        image_address: first,
        image_size: end - first,
    }))
}
