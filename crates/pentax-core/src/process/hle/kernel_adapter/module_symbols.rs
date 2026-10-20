// SPDX-License-Identifier: GPL-2.0-only

//! Runtime handles select only actual supplied modules, never generated markers.
use super::{KernelMemory, error};
use pentax_kernel::{Failure, GuestError};
use pentax_linker::{ModuleLookupError, SymbolType, lookup_module_export};
use pentax_platform::native_memory::NativeMemoryError;

fn charge(work: &mut u64, amount: u64, maximum: u64) -> Result<(), Failure> {
    *work = work
        .checked_add(amount)
        .filter(|total| *total <= maximum)
        .ok_or(Failure::Limit("module lookup work"))?;
    Ok(())
}

pub(super) fn lookup(
    memory: &mut KernelMemory<'_, '_, '_>,
    handle: i32,
    name: &str,
) -> Result<Option<u64>, Failure> {
    let source = memory
        .source
        .ok_or(Failure::Unsupported("image-bound module symbols"))?;
    let mut limits = memory
        .lookup_limits
        .ok_or(Failure::Unsupported("image-bound module symbols"))?;
    let work = memory
        .lookup_work
        .as_deref_mut()
        .ok_or(Failure::Unsupported("image-bound module symbols"))?;
    let maximum = limits.max_work;
    let index = if let Some(registry) = memory.loaded_modules.as_deref() {
        registry.resolve_handle(handle)
    } else {
        (handle > 0 && handle as usize <= memory.supplied_modules).then(|| handle as usize - 1)
    };
    let Some(index) = index else {
        return Ok(None);
    };
    limits.max_work = maximum.saturating_sub(*work);
    let attempt = lookup_module_export(source, index, name.as_bytes(), &limits);
    charge(work, attempt.work, maximum)?;
    let selected = attempt.result.map_err(|failure| match failure {
        ModuleLookupError::Budget
        | ModuleLookupError::Symbolic(
            pentax_linker::SymbolicError::LimitExceeded { .. }
            | pentax_linker::SymbolicError::SizeOverflow { .. },
        ) => Failure::Limit("module lookup work"),
        ModuleLookupError::Symbolic(pentax_linker::SymbolicError::Allocation) => {
            Failure::Allocation
        }
        _ => Failure::Unsupported("module export lookup profile"),
    })?;
    let Some(selected) = selected else {
        return Ok(None);
    };
    if !std::ptr::eq(selected.source_plan(), source)
        || selected.definition().module.index() != index
    {
        return Err(Failure::Unsupported("module export source identity"));
    }
    let size = selected.size().max(1);
    let address = selected.address();
    let end = address
        .checked_add(size)
        .ok_or(Failure::Guest(GuestError::AddressOverflow))?;
    let runtime = source.modules()[index].runtime_plan();
    let pages = (address % crate::memory::PAGE_SIZE)
        .checked_add(size)
        .ok_or(Failure::Limit("module export extent"))?
        .div_ceil(crate::memory::PAGE_SIZE);
    charge(
        work,
        pages
            .checked_mul(3)
            .and_then(|steps| steps.checked_add(runtime.segments.len() as u64 + 16))
            .ok_or(Failure::Limit("module lookup work"))?,
        maximum,
    )?;
    memory
        .paused
        .preflight_read(address, size)
        .map_err(|value| Failure::Guest(error(value, false)))?;
    if selected.symbol_type() == SymbolType::FUNC {
        let bias = source.modules()[index].load_bias();
        if !runtime.segments.iter().any(|segment| {
            let start = bias.checked_add(segment.vaddr);
            segment.flags.executable()
                && !segment.flags.writable()
                && start.is_some_and(|start| {
                    start <= address
                        && start
                            .checked_add(segment.file_bytes.len() as u64)
                            .is_some_and(|bound| end <= bound)
                })
        }) {
            return Err(Failure::Unsupported(
                "function export is not initialized immutable source",
            ));
        }
        memory
            .paused
            .preflight_fetch_source(address, size)
            .map_err(|value| Failure::Guest(error(value, false)))?;
        let mut page = address;
        while page < end {
            if !matches!(
                memory.paused.preflight_write(page, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ) {
                return Err(Failure::Unsupported("writable function export"));
            }
            page = (page | (crate::memory::PAGE_SIZE - 1))
                .checked_add(1)
                .ok_or(Failure::Guest(GuestError::AddressOverflow))?;
        }
    }
    Ok(Some(address))
}
