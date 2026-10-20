// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX fixture over actual native backing. No guest code is run.

use super::*;
use pentax_kernel::{CallArgs, Kernel, ProcessIdentity, Service};
use pentax_platform::native_memory::NativeAddressSpace;

const PAGE: u64 = 0x4000;
const INFO_BYTES: usize = 16_704;
const OUTPUT_BYTES: u64 = 16_429;

fn adapter<'borrow, 'owner>(
    paused: &'borrow mut PausedNativeMemory<'owner>,
) -> KernelMemory<'borrow, 'owner, 'static> {
    KernelMemory {
        paused,
        source: None,
        supplied_modules: 0,
        lookup_limits: None,
        lookup_work: None,
        loaded_modules: None,
        thread_tls: None,
        initial_entry: None,
    }
}

#[test]
fn receive_intent_late_native_output_refusal_preserves_whole_info_then_retry_commits_exact_spans() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 2 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let info = base + 0x100;
    let offset = (info - base) as usize;
    let mut before = [0xa5; (2 * PAGE) as usize];
    // A deliberately undersized header is opaque in the source fallback.
    before[offset..offset + 8].copy_from_slice(&1_u64.to_le_bytes());
    space.pause().unwrap().write(base, &before).unwrap();
    space
        .protect(base + PAGE, PAGE, NativePermissions::READ)
        .unwrap();
    let protected_backing = space.statistics();
    let mut kernel = Kernel::new(
        Default::default(),
        ProcessIdentity {
            process_id: 1,
            initial_thread_id: 1,
        },
        Vec::new(),
    )
    .unwrap();
    let initial_usage = kernel.usage();
    let args = CallArgs([info, 0, 0, 0, 0, 0]);
    {
        let mut paused = space.pause().unwrap();
        let mut guest = adapter(&mut paused);
        assert_eq!(
            kernel.invoke_library(Service::GameIntentReceiveIntent, args, &mut guest),
            Err(Failure::Guest(GuestError::WriteDenied))
        );
        let mut actual = [0; (2 * PAGE) as usize];
        guest.read(base, &mut actual).unwrap();
        // Includes the valid first output, the full 16,704-byte structure and
        // canaries on both sides: no partial prefix publication is allowed.
        assert_eq!(actual, before);
    }
    assert_eq!(space.statistics(), protected_backing);
    let failed_usage = kernel.usage();
    assert_eq!(failed_usage.attempts, initial_usage.attempts + 1);
    assert_eq!(failed_usage.bytes_read, initial_usage.bytes_read);
    assert_eq!(
        failed_usage.bytes_written,
        initial_usage.bytes_written + OUTPUT_BYTES
    );
    assert!(failed_usage.work > initial_usage.work);
    assert!(failed_usage.scratch_peak >= OUTPUT_BYTES as usize);
    let mut resources = failed_usage;
    resources.attempts = initial_usage.attempts;
    resources.bytes_written = initial_usage.bytes_written;
    resources.work = initial_usage.work;
    resources.scratch_peak = initial_usage.scratch_peak;
    assert_eq!(resources, initial_usage);

    space
        .protect(base + PAGE, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let writable_backing = space.statistics();
    {
        let mut paused = space.pause().unwrap();
        let mut guest = adapter(&mut paused);
        assert_eq!(
            kernel.invoke_library(Service::GameIntentReceiveIntent, args, &mut guest),
            Ok(0x8055_3806)
        );
        let mut actual = [0; (2 * PAGE) as usize];
        guest.read(base, &mut actual).unwrap();
        let mut expected = before;
        expected[offset + 8..offset + 12].fill(0xff);
        expected[offset + 12..offset + 45].fill(0);
        expected[offset + 308..offset + 16700].fill(0);
        assert_eq!(actual, expected);
        for range in [0..8, 45..52, 52..308, 16700..INFO_BYTES] {
            assert_eq!(
                &actual[offset + range.start..offset + range.end],
                &before[offset + range.start..offset + range.end]
            );
        }
    }
    assert_eq!(space.statistics(), writable_backing);
    let completed_usage = kernel.usage();
    assert_eq!(completed_usage.attempts, initial_usage.attempts + 2);
    assert_eq!(completed_usage.bytes_read, initial_usage.bytes_read);
    assert_eq!(
        completed_usage.bytes_written,
        initial_usage.bytes_written + 2 * OUTPUT_BYTES
    );
    assert!(completed_usage.work > failed_usage.work);
    let mut resources = completed_usage;
    resources.attempts = initial_usage.attempts;
    resources.bytes_written = initial_usage.bytes_written;
    resources.work = initial_usage.work;
    resources.scratch_peak = initial_usage.scratch_peak;
    assert_eq!(resources, initial_usage);
}
