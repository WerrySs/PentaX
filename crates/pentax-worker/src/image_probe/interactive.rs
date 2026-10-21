// SPDX-License-Identifier: GPL-2.0-only

//! Explicit interactive lifetime accounting, independent of live resources and
//! individual-operation admission. Counters remain checked and never reset.

use crate::native_session::NativeSessionLimits;
use pentax_core::process::hle::NativeImageLimits;

/// Explicit command-line duration; invalid requests fail before file intake or
/// RUNNING. Do not reinterpret this as a supervisor or per-operation deadline.
pub(super) fn stop_after_seconds(args: &[std::ffi::OsString]) -> Result<Option<u32>, ()> {
    let mut values = args
        .chunks_exact(2)
        .filter(|pair| pair[0] == "--stop-after-seconds");
    let Some(pair) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some()
        || args
            .chunks_exact(2)
            .filter(|pair| pair[0] == "--execution-profile")
            .count()
            != 1
        || !args
            .chunks_exact(2)
            .any(|pair| pair[0] == "--execution-profile" && pair[1] == "interactive")
    {
        return Err(());
    }
    let text = pair[1].to_str().ok_or(())?;
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    let seconds = text.parse::<u32>().map_err(|_| ())?;
    if !(1..=3600).contains(&seconds) {
        return Err(());
    }
    Ok(Some(seconds))
}

pub(super) fn configure_image(limits: &mut NativeImageLimits) {
    // Preserve finite JSON/metadata and shader helper work before extending
    // only their containing monotonic ledgers. Reapplication cannot loosen an
    // already selected individual-operation ceiling.
    let provider = limits.startup.max_provider_work;
    let call = limits
        .startup
        .max_provider_call_work
        .get_or_insert(provider);
    *call = (*call).min(provider);
    let metadata = limits
        .startup
        .max_provider_metadata_bytes
        .get_or_insert(provider);
    *metadata = (*metadata).min(provider);
    let shader = limits.graphics.max_preparation_work;
    let call = limits.graphics.max_shader_call_work.get_or_insert(shader);
    *call = (*call).min(shader);

    let kernel = &mut limits.startup.kernel;
    kernel.max_attempts = u64::MAX;
    kernel.max_bytes = u64::MAX;
    kernel.max_work = u64::MAX;
    kernel.max_save_io_bytes = u64::MAX;

    let services = &mut limits.startup.startup.hle.services;
    services.max_attempts = u64::MAX;
    services.max_total_bytes = u64::MAX;
    services.max_work_units = u64::MAX;
    limits.startup.max_provider_work = u64::MAX;
    limits.startup.max_native_service_requests = Some(u64::MAX);

    let graphics = &mut limits.graphics;
    graphics.max_calls = u64::MAX;
    graphics.max_dispatches = u64::MAX;
    graphics.max_draws = u64::MAX;
    graphics.max_vertices = u64::MAX;
    graphics.max_command_buffers = u64::MAX;
    graphics.max_command_bytes = u64::MAX;
    graphics.max_invocations = u64::MAX;
    graphics.max_preparation_work = u64::MAX;
    graphics.max_memory_write_bytes = u64::MAX;
    // Nested command, draw, compute and Vulkan limits remain per-operation.
    // No live resource, backing, scratch, transfer or output bound changes.
}

pub(super) fn configure_execution(limits: &mut NativeSessionLimits) {
    limits.dispatch.max_blocks = u64::MAX;
    limits.dispatch.max_source_bytes = u64::MAX;
    limits.dispatch.max_emitted_bytes = u64::MAX;
    limits.dispatch.max_work_units = u64::MAX;
    limits.max_total_window_bytes = u64::MAX;
    limits.max_admission_work = u64::MAX;
    // Source-window allocation, BlockLimits, cache/context capacity, sampling
    // policy and fairness are independent. The caller selects interactive mode.
}

#[cfg(test)]
mod tests;
