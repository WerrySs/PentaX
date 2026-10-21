// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use pentax_kernel::{
    CallArgs, Failure, GuestAccess, GuestError, GuestWrite, Kernel, MemoryEffect, ProcessIdentity,
    Service,
};

#[test]
fn interactive_lifetime_profile_preserves_live_and_individual_operation_policies() {
    let before = NativeImageLimits::default();
    let mut image = before.clone();
    configure_image(&mut image);
    configure_image(&mut image); // Preserve the original finite call ceilings.
    assert_eq!(
        image.startup.max_provider_call_work,
        Some(before.startup.max_provider_work)
    );
    assert_eq!(
        image.startup.max_provider_metadata_bytes,
        Some(before.startup.max_provider_work)
    );
    assert_eq!(image.startup.max_native_service_requests, Some(u64::MAX));
    assert_eq!(
        image.graphics.max_shader_call_work,
        Some(before.graphics.max_preparation_work)
    );
    assert_eq!(image.startup.kernel.max_output_bytes, 8 * 1024 * 1024);
    let mut kernel = image.startup.kernel;
    assert_eq!(
        (
            kernel.max_attempts,
            kernel.max_bytes,
            kernel.max_work,
            kernel.max_save_io_bytes
        ),
        (u64::MAX, u64::MAX, u64::MAX, u64::MAX)
    );
    kernel.max_attempts = before.startup.kernel.max_attempts;
    kernel.max_bytes = before.startup.kernel.max_bytes;
    kernel.max_work = before.startup.kernel.max_work;
    kernel.max_save_io_bytes = before.startup.kernel.max_save_io_bytes;
    assert_eq!(kernel, before.startup.kernel); // Every other kernel field stays exact.
    let old_services = before.startup.startup.hle.services;
    let services = image.startup.startup.hle.services;
    assert_eq!(services.max_span_bytes, old_services.max_span_bytes);
    assert_eq!(services.max_scratch_bytes, old_services.max_scratch_bytes);
    assert_eq!(services.max_exit_callbacks, old_services.max_exit_callbacks);
    assert_eq!(
        image.startup.startup.hle.linking,
        before.startup.startup.hle.linking
    );
    assert_eq!(
        format!("{:?}", image.native),
        format!("{:?}", before.native)
    );
    for (actual, expected) in [
        (
            format!("{:?}", image.graphics.commands),
            format!("{:?}", before.graphics.commands),
        ),
        (
            format!("{:?}", image.graphics.graphics_commands),
            format!("{:?}", before.graphics.graphics_commands),
        ),
        (
            format!("{:?}", image.graphics.compute),
            format!("{:?}", before.graphics.compute),
        ),
        (
            format!("{:?}", image.graphics.graphics),
            format!("{:?}", before.graphics.graphics),
        ),
        (
            format!("{:?}", image.graphics.gpu),
            format!("{:?}", before.graphics.gpu),
        ),
        (
            format!("{:?}", image.graphics.video_presentation),
            format!("{:?}", before.graphics.video_presentation),
        ),
    ] {
        assert_eq!(actual, expected);
    }
    assert_eq!(
        image.graphics.graphics_backend_work,
        before.graphics.graphics_backend_work
    );
    assert_eq!(
        image.graphics.max_batch_buffers,
        before.graphics.max_batch_buffers
    );
    assert_eq!(
        image.graphics.max_shader_mappings,
        before.graphics.max_shader_mappings
    );
    // An explicitly larger optional cap was still constrained by the old
    // finite containing ledger. Lifetime extension cannot enlarge that call.
    let mut oversized = before.clone();
    oversized.startup.max_provider_call_work = Some(u64::MAX);
    oversized.startup.max_provider_metadata_bytes = Some(u64::MAX);
    oversized.graphics.max_shader_call_work = Some(u64::MAX);
    configure_image(&mut oversized);
    assert_eq!(
        oversized.startup.max_provider_call_work,
        image.startup.max_provider_call_work
    );
    assert_eq!(
        oversized.startup.max_provider_metadata_bytes,
        image.startup.max_provider_metadata_bytes
    );
    assert_eq!(
        oversized.graphics.max_shader_call_work,
        image.graphics.max_shader_call_work
    );
    let original = NativeSessionLimits {
        private_samples: true,
        private_thread_states: true,
        ..Default::default()
    };
    let mut execution = original;
    configure_execution(&mut execution);
    assert_eq!(execution.block, original.block);
    assert_eq!(
        execution.max_window_allocation_bytes,
        original.max_window_allocation_bytes
    );
    assert_eq!(
        execution.dispatch.cache_bytes,
        original.dispatch.cache_bytes
    );
    assert_eq!(
        execution.dispatch.cache_slots,
        original.dispatch.cache_slots
    );
    assert_eq!(
        execution.dispatch.cache_search_granules,
        original.dispatch.cache_search_granules
    );
    assert_eq!(
        execution.dispatch.max_segments,
        original.dispatch.max_segments
    );
    assert_eq!(
        execution.dispatch.max_thread_contexts,
        original.dispatch.max_thread_contexts
    );
    assert_eq!(
        execution.progress_interval_blocks,
        original.progress_interval_blocks
    );
    assert_eq!(execution.report_progress, original.report_progress);
    assert_eq!(execution.private_samples, original.private_samples);
    assert_eq!(
        execution.private_thread_states,
        original.private_thread_states
    );
    assert_eq!(execution.interactive, original.interactive);
    assert_eq!(
        execution.libc_memory_builtins,
        original.libc_memory_builtins
    );
}

struct Bytes;
impl GuestAccess for Bytes {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        let start = usize::try_from(address.checked_sub(1).ok_or(GuestError::Unmapped)?)
            .map_err(|_| GuestError::Unmapped)?;
        let source = b"ab"
            .get(
                start
                    ..start
                        .checked_add(output.len())
                        .ok_or(GuestError::AddressOverflow)?,
            )
            .ok_or(GuestError::Unmapped)?;
        output.copy_from_slice(source);
        Ok(())
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        Err(GuestError::UnsupportedMapping)
    }
}

#[test]
fn interactive_repeated_calls_still_refuse_output_transfer_retention_and_single_sleep_limits() {
    let mut image = NativeImageLimits::default();
    image.startup.kernel.max_attempts = 1;
    image.startup.kernel.max_io_bytes = 1;
    image.startup.kernel.max_output_bytes = 2;
    image.startup.kernel.max_sleep_nanoseconds = 0;
    let identity = ProcessIdentity {
        process_id: 1,
        initial_thread_id: 2,
    };
    let write = CallArgs([1, 1, 1, 0, 0, 0]);
    let mut bounded = Kernel::new(image.startup.kernel, identity, Vec::new()).unwrap();
    assert_eq!(bounded.invoke(Service::Write, write, &mut Bytes), Ok(1));
    assert_eq!(
        bounded.invoke(Service::Write, write, &mut Bytes),
        Err(Failure::Limit("attempts"))
    );
    configure_image(&mut image);
    let mut interactive = Kernel::new(image.startup.kernel, identity, Vec::new()).unwrap();
    for _ in 0..2 {
        assert_eq!(interactive.invoke(Service::Write, write, &mut Bytes), Ok(1));
    }
    assert_eq!(interactive.usage().attempts, 2);
    assert_eq!(interactive.output_usage().retained_bytes, 2);
    assert_eq!(
        interactive.invoke(Service::Write, write, &mut Bytes),
        Err(Failure::Limit("output bytes"))
    );
    assert_eq!(
        interactive.invoke(Service::Write, CallArgs([1, 1, 2, 0, 0, 0]), &mut Bytes),
        Err(Failure::Limit("output transfer"))
    );
    assert_eq!(
        interactive.invoke(Service::Usleep, CallArgs([1, 0, 0, 0, 0, 0]), &mut Bytes),
        Err(Failure::Limit("sleep duration"))
    );
    assert_eq!(interactive.usage().attempts, 5);
    assert_eq!(interactive.output_usage().retained_bytes, 2);
}
