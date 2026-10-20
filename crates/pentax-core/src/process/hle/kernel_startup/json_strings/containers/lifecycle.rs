// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned default-engine readiness.
// KytyPS5 f186b357 libJson2.cpp493-513/1036-1047; see NOTICE and provenance.
//! Initializer arguments are opaque. The supported engine is process-owned;
//! empty public Terminate/Dtor bodies do not reclaim live values or defaults.
use super::*;

pub(in super::super) fn handles(function: JsonFunction) -> bool {
    matches!(
        function,
        JsonFunction::InitializerInitializeV1
            | JsonFunction::InitializerTerminate
            | JsonFunction::InitializerDtor
            | JsonFunction::InitializerSetGlobalNullAccessCallback
    )
}

pub(in super::super) fn service(
    state: &mut JsonStrings,
    memory: &mut PausedNativeMemory<'_>,
    kernel: &mut pentax_kernel::Kernel,
    call: JsonCall,
    work: &mut u64,
    maximum: u64,
) -> Result<u64, KernelStartupError> {
    charge(work, 1024, maximum)?;
    match call.function {
        JsonFunction::InitializerTerminate | JsonFunction::InitializerDtor => Ok(0),
        JsonFunction::InitializerSetGlobalNullAccessCallback => {
            // Fixed work and the caller's owner/source/site checks precede the
            // only mutation. Store the complete opaque words; no guest read,
            // allocation, target validation or callback execution occurs.
            if call.args[1] == 0 {
                let _ = state.null_access_registration.take();
            } else {
                let _ = state
                    .null_access_registration
                    .replace(NullAccessRegistration {
                        callback: call.args[1],
                        context: call.args[2],
                    });
            }
            state.owner = Some(memory.owner_id());
            Ok(0)
        }
        JsonFunction::InitializerInitializeV1 => {
            if state.containers.ready || state.containers.defaults.is_some() {
                validate(state, memory, work, maximum)?;
                state.containers.ready = true;
                state.owner = Some(memory.owner_id());
                return Ok(0);
            }
            // Build the same actual Null/Array/Object storage used by queries.
            // Readiness and private registries commit only after the one real
            // MapZeroed+blob transaction. Neither opaque argument is inspected.
            let mut transaction = Transaction::new(state, maximum - *work)?;
            let result = transaction.stage(state).and_then(|()| {
                transaction.defaults()?;
                transaction.containers.ready = true;
                transaction.publish(state, memory, kernel, 0)
            });
            *work += transaction.admission.work;
            if result.is_ok() {
                state.owner = Some(memory.owner_id());
            }
            result
        }
        _ => Err(failure()),
    }
}

fn validate(
    state: &JsonStrings,
    memory: &PausedNativeMemory<'_>,
    work: &mut u64,
    maximum: u64,
) -> Result<(), KernelStartupError> {
    // Two bounded registry scans plus one complete fixed backing/byte check.
    charge(
        work,
        (state.containers.records.len() as u64)
            .checked_mul(2)
            .and_then(|n| n.checked_add(512))
            .ok_or(KernelStartupError::Budget("JSON default readiness"))?,
        maximum,
    )?;
    let defaults = state.containers.defaults.ok_or_else(failure)?;
    if defaults.array != at(defaults.null, 32)? || defaults.object != at(defaults.null, 40)? {
        return Err(failure());
    }
    memory
        .preflight_read(defaults.null, 64)
        .map_err(|e| guest(e, false))?;
    memory
        .preflight_write(defaults.null, 64)
        .map_err(|e| guest(e, true))?;
    memory
        .preflight_non_executable(defaults.null, 64)
        .map_err(|e| guest(e, false))?;
    memory
        .preflight_distinct_backing(defaults.null, 64, 128)
        .map_err(|e| guest(e, false))?;
    let mut actual = [0; 64];
    memory
        .read(defaults.null, &mut actual)
        .map_err(|e| guest(e, false))?;
    let mut expected = [0; 64];
    for (wrapper, kind, offset) in [(defaults.array, 6, 48), (defaults.object, 7, 56)] {
        let token = at(defaults.null, offset)?;
        let end = at(token, 8)?;
        let mut matches = state
            .containers
            .records
            .iter()
            .filter(|record| record.wrapper == wrapper && record.token == token);
        let record = matches.next().ok_or_else(failure)?;
        if matches.next().is_some()
            || !record.live
            || record.kind != kind
            || record.owner_value != 0
            || record.length != 0
            || record.entries != end
            || record.keys_begin != end
            || record.keys_end != end
        {
            return Err(failure());
        }
        let from = (wrapper - defaults.null) as usize;
        expected[from..from + 8].copy_from_slice(&token.to_le_bytes());
    }
    if actual != expected {
        return Err(failure());
    }
    Ok(())
}
