// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX fixtures over actual retained native backing. No guest code,
// authentication, sockets or external data are executed or accessed.

use super::*;
use pentax_kernel::{CallArgs, Kernel, Limits, ProcessIdentity, Service};
use pentax_platform::native_memory::NativeAddressSpace;

const PAGE: u64 = 0x4000;
const NOT_SIGNED_IN: u64 = 0x8055_3407;

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

fn kernel() -> Kernel {
    Kernel::new(
        Limits::default(),
        ProcessIdentity {
            process_id: 1,
            initial_thread_id: 1,
        },
        Vec::new(),
    )
    .unwrap()
}

fn put(guest: &mut KernelMemory<'_, '_, '_>, address: u64, bytes: &[u8]) {
    guest
        .apply(MemoryEffect::None, &[GuestWrite { address, bytes }])
        .unwrap();
}

fn read<const N: usize>(guest: &KernelMemory<'_, '_, '_>, address: u64) -> [u8; N] {
    let mut bytes = [0; N];
    guest.read(address, &mut bytes).unwrap();
    bytes
}

fn request(kernel: &mut Kernel, guest: &mut KernelMemory<'_, '_, '_>, output: u64) -> u64 {
    assert_eq!(
        kernel.invoke(
            Service::NpWebApi2CreateRequest,
            CallArgs([1001, 0, 0, 0, u64::MAX, output]),
            guest,
        ),
        Ok(0)
    );
    u64::from_le_bytes(read(guest, output))
}

fn response(pointer: u64, capacity: u64) -> [u8; 32] {
    let mut bytes = [0x5a; 32];
    bytes[8..16].copy_from_slice(&pointer.to_le_bytes());
    bytes[16..24].copy_from_slice(&capacity.to_le_bytes());
    bytes
}

fn send(
    kernel: &mut Kernel,
    guest: &mut KernelMemory<'_, '_, '_>,
    id: u64,
    descriptor: u64,
) -> Result<u64, Failure> {
    // The source's offline Send does not dereference data/data_size.
    kernel.invoke(
        Service::NpWebApi2SendRequest,
        CallArgs([id, u64::MAX, u64::MAX, descriptor, 0, 0]),
        guest,
    )
}

#[test]
fn backing_proof_is_metadata_only_and_caps_extent_before_any_payload_access() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    space
        .pause()
        .unwrap()
        .write(base + 0x100, &[0x9d; 32])
        .unwrap();
    space
        .protect(base, PAGE, NativePermissions::new(false, false, false))
        .unwrap();
    {
        let mut paused = space.pause().unwrap();
        let guest = adapter(&mut paused);
        let mut byte = [0];
        assert_eq!(
            guest.read(base + 0x100, &mut byte),
            Err(GuestError::ReadDenied)
        );
        assert_eq!(guest.preflight_distinct_backing(base + 0x100, 32), Ok(()));
        assert_eq!(
            guest.preflight_distinct_backing(base + PAGE, 32),
            Err(GuestError::Unmapped)
        );
        assert_eq!(
            guest.preflight_distinct_backing(base + PAGE - 16, 32),
            Err(GuestError::Unmapped)
        );
        assert_eq!(
            guest.preflight_distinct_backing(base + 0x100, 33),
            Err(GuestError::UnsupportedMapping)
        );
        assert_eq!(
            guest.preflight_distinct_backing(u64::MAX - 7, 16),
            Err(GuestError::AddressOverflow)
        );
        // Preserve the underlying metadata query's empty-range policy.
        assert_eq!(guest.preflight_distinct_backing(u64::MAX, 0), Ok(()));
    }
    space
        .protect(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = space.pause().unwrap();
    assert_eq!(read::<32>(&adapter(&mut paused), base + 0x100), [0x9d; 32]);
}

#[test]
fn actual_send_late_nul_permission_failure_preserves_all_outputs_then_retry_commits() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, 3 * PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let descriptor = base + 0x100;
    let nul = base + PAGE + 0x100;
    let before = response(nul, 7);
    let mut kernel = kernel();
    let id;
    {
        let mut paused = space.pause().unwrap();
        let mut guest = adapter(&mut paused);
        id = request(&mut kernel, &mut guest, base + 0x200);
        put(&mut guest, descriptor, &before);
        put(&mut guest, nul, &[0xb6]);
    }
    let usage = kernel.usage();
    space
        .protect(base + PAGE, PAGE, NativePermissions::READ)
        .unwrap();
    {
        let mut paused = space.pause().unwrap();
        let mut guest = adapter(&mut paused);
        assert_eq!(
            send(&mut kernel, &mut guest, id, descriptor),
            Err(Failure::Guest(GuestError::WriteDenied))
        );
        assert_eq!(read::<32>(&guest, descriptor), before);
        assert_eq!(read::<1>(&guest, nul), [0xb6]);
        assert_eq!(kernel.usage().handles_live, usage.handles_live);
        assert_eq!(kernel.usage().mapped_bytes, usage.mapped_bytes);
    }
    space
        .protect(base + PAGE, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = space.pause().unwrap();
    let mut guest = adapter(&mut paused);
    assert_eq!(
        send(&mut kernel, &mut guest, id, descriptor),
        Ok(NOT_SIGNED_IN)
    );
    let mut expected = before;
    expected[..4].fill(0);
    expected[24..].fill(0);
    assert_eq!(read::<32>(&guest, descriptor), expected);
    assert_eq!(read::<1>(&guest, nul), [0]);
    assert_eq!(
        kernel.invoke(
            Service::NpWebApi2AbortRequest,
            CallArgs([id, 0, 0, 0, 0, 0]),
            &mut guest
        ),
        Ok(0)
    );
}

#[test]
fn actual_send_coalesces_virtual_nul_aliases_and_keeps_other_descriptor_fields_exact() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 4 * PAGE).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let mut paused = space.pause().unwrap();
    let mut guest = adapter(&mut paused);
    let mut kernel = kernel();
    let id = request(&mut kernel, &mut guest, base + 0x200);
    let descriptor = base + 0x100;
    // Status and response-length aliases coalesce. Padding, pointer and
    // capacity aliases remain separate final byte stores after those reads.
    for offset in [0, 2, 24, 31, 5, 8, 16] {
        let before = response(descriptor + offset, 1);
        put(&mut guest, descriptor, &before);
        assert_eq!(
            send(&mut kernel, &mut guest, id, descriptor),
            Ok(NOT_SIGNED_IN)
        );
        let mut expected = before;
        expected[..4].fill(0);
        expected[24..].fill(0);
        expected[offset as usize] = 0;
        assert_eq!(read::<32>(&guest, descriptor), expected);
    }
}

#[test]
fn actual_send_rejects_distinct_virtual_physical_output_aliases_before_any_copy() {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x40000).unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    space
        .map_at(base + 0x20000, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    for effect in [
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: PAGE,
        },
        NativeMemoryEffect::MapPhysical {
            address: base,
            size: PAGE,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
        NativeMemoryEffect::MapPhysical {
            address: base + 0x10000,
            size: PAGE,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
    ] {
        space
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect,
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    let mut paused = space.pause().unwrap();
    let mut guest = adapter(&mut paused);
    let mut kernel = kernel();
    let id = request(&mut kernel, &mut guest, base + 0x20080);
    let descriptor = base + 0x100;
    let alias = base + 0x10100;
    let before = response(alias, 1);
    put(&mut guest, descriptor, &before);
    assert_eq!(guest.preflight_distinct_backing(descriptor, 32), Ok(()));
    assert_eq!(read::<32>(&guest, alias), before);
    assert_eq!(
        send(&mut kernel, &mut guest, id, descriptor),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(read::<32>(&guest, descriptor), before);
    assert_eq!(read::<32>(&guest, alias), before);
    let independent = base + 0x20100;
    put(&mut guest, descriptor, &response(independent, 1));
    put(&mut guest, independent, &[0x7c]);
    assert_eq!(
        send(&mut kernel, &mut guest, id, descriptor),
        Ok(NOT_SIGNED_IN)
    );
    assert_eq!(read::<1>(&guest, independent), [0]);
    assert_eq!(read::<4>(&guest, alias), [0; 4]);
}
