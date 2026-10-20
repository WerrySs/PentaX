// SPDX-License-Identifier: GPL-2.0-only
//! Original ABI examples for the pinned local PlayGo metadata profile.

use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{CallArgs, Failure, GuestAccess, GuestError, Kernel, Limits, Service};

const BAD_ARGUMENT: u64 = 0x80b2_0004;
const BAD_HANDLE: u64 = 0x80b2_0009;
const BAD_POINTER: u64 = 0x80b2_000a;
const BAD_SIZE: u64 = 0x80b2_000b;
const BAD_CHUNK: u64 = 0x80b2_000c;
const BAD_SPEED: u64 = 0x80b2_000d;
const NOT_SUPPORTED: u64 = 0x80b2_000e;
const BAD_LOCUS: u64 = 0x80b2_0010;
const BAD_OPTIONAL: u64 = 0x80b2_0024;
const OUT: u64 = ARG + 0x200;
const IDS: u64 = ARG + 0x100;

fn call(
    owner: &mut Kernel,
    guest: &mut impl GuestAccess,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [u64::MAX; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}

fn loaded() -> Kernel {
    let mut owner = kernel(Limits::default());
    owner.playgo.chunks = Some(3);
    owner
}

#[test]
fn stateless_lifecycle_preserves_cached_chunks_and_initialize_only_reads_required_fields() {
    let mut owner = kernel(Limits::default());
    let mut guest = TestGuest::new();
    guest.put(0x200, &[0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoClose, &[1]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoOpen, &[0, 1]),
        Ok(BAD_POINTER)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoOpen, &[OUT, 1]),
        Ok(BAD_ARGUMENT)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoOpen, &[OUT, 0]),
        Ok(NOT_SUPPORTED)
    );
    assert_eq!(guest.get(OUT, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoInitialize, &[0]),
        Ok(BAD_POINTER)
    );
    let init = ARG + PAGE - 12;
    guest.put(PAGE - 12, &[0; 12]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoInitialize, &[init]),
        Ok(BAD_POINTER)
    );
    // The buffer pointer is opaque. Reserved logging bytes are beyond the
    // readable region; only the pointer and low32 size are semantically read.
    guest.put(PAGE - 12, &u64::MAX.to_le_bytes());
    guest.put(PAGE - 4, &(2_u32 * 1024 * 1024 - 1).to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoInitialize, &[init]),
        Ok(BAD_SIZE)
    );
    guest.put(PAGE - 4, &(2_u32 * 1024 * 1024).to_le_bytes());
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoInitialize, &[init]),
        Ok(0)
    );
    assert_eq!(owner.playgo.chunks, None);
    owner.playgo.chunks = Some(3);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoOpen, &[OUT, 0]),
        Ok(0)
    );
    assert_eq!(guest.get(OUT, 8), [1, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoTerminate, &[]),
        Ok(0)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoClose,
            &[0xeeee_0000_0000_0001]
        ),
        Ok(0)
    );
    assert_eq!(owner.playgo.chunks, Some(3));
    assert_eq!(
        call(&mut owner, &mut guest, Service::PlayGoOpen, &[OUT, 0]),
        Ok(0)
    );
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(owner.usage().mapped_bytes, 0);
}

#[test]
fn loci_observe_prior_staged_aliases_and_publish_only_admitted_bad_chunk_prefix() {
    let mut owner = loaded();
    let mut guest = TestGuest::new();
    guest.put(0x100, &[0, 0, 1, 0, 99, 0]);
    guest.put(0x200, &[0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[1, IDS, 3, OUT]
        ),
        Ok(BAD_CHUNK)
    );
    assert_eq!(
        guest.get(OUT, 8),
        [3, 3, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]
    );
    guest.put(0x100, &[0; 8]);
    // The first store changes the NEXT input ID from0 to3, so iteration1
    // fails. Reading an immutable snapshot would incorrectly accept all3.
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[1, IDS, 0xaaaa_0000_0000_0003, IDS + 2]
        ),
        Ok(BAD_CHUNK)
    );
    assert_eq!(guest.get(IDS, 8), [0, 0, 3, 0, 0, 0, 0, 0]);
    guest.put(0x100, &[2, 0, 2, 0, 0, 0]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[0xffff_0000_0000_0001, IDS, 3, OUT]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(OUT, 4), [3, 3, 3, 0xa5]);
}

#[test]
fn generated_chunk_lists_commit_count_last_and_todo_output_is_not_accessed() {
    let mut owner = loaded();
    let mut guest = TestGuest::new();
    for service in [Service::PlayGoGetChunkId, Service::PlayGoGetInstallChunkId] {
        guest.put(0x200, &[0xa5; 12]);
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                service,
                &[1, OUT, 0x1234_0000_0000_0008, OUT + 2]
            ),
            Ok(0)
        );
        assert_eq!(
            guest.get(OUT, 12),
            [0, 0, 3, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5]
        );
        assert_eq!(
            call(&mut owner, &mut guest, service, &[1, 0, 0, OUT]),
            Ok(0)
        );
        assert_eq!(guest.get(OUT, 4), [0; 4]);
    }
    guest.put(0x200, &[0xa5; 8]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetToDoList,
            &[1, u64::MAX, 0xffff_ffff, OUT]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(OUT, 8), [0, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetToDoList,
            &[1, 0, 1, OUT]
        ),
        Ok(BAD_POINTER)
    );
}

#[test]
fn source_error_order_low_word_domains_and_opaque_optional_inputs_are_preserved() {
    let mut owner = loaded();
    let mut guest = TestGuest::new();
    guest.put(0x200, &[0xa5; 16]);
    for (service, args, expected) in [
        (Service::PlayGoGetLocus, [0, 0, 0, 0], BAD_HANDLE),
        (Service::PlayGoGetLocus, [1, 0, 0, 0], BAD_POINTER),
        (Service::PlayGoGetLocus, [1, IDS, 0, OUT], BAD_SIZE),
        (Service::PlayGoSetToDoList, [1, 0, 0, 0], BAD_POINTER),
        (Service::PlayGoSetToDoList, [1, IDS, 0, 0], BAD_SIZE),
        (Service::PlayGoPrefetch, [1, 0, 0, 1], BAD_POINTER),
        (Service::PlayGoPrefetch, [1, u64::MAX, 0, 1], BAD_SIZE),
        (Service::PlayGoPrefetch, [1, u64::MAX, 1, 1], BAD_LOCUS),
        (Service::PlayGoGetOptionalChunk, [1, 2, 0, 0], BAD_POINTER),
        (
            Service::PlayGoGetOptionalChunk,
            [1, 2, OUT, 0],
            BAD_OPTIONAL,
        ),
        (
            Service::PlayGoPrefetchOptionalChunk,
            [1, 2, 0, 0],
            BAD_POINTER,
        ),
        (
            Service::PlayGoPrefetchOptionalChunk,
            [1, 2, u64::MAX, 0],
            BAD_OPTIONAL,
        ),
        (Service::PlayGoSetInstallSpeed, [0, 3, 0, 0], BAD_HANDLE),
        (Service::PlayGoSetInstallSpeed, [1, 3, 0, 0], BAD_SPEED),
        (
            Service::PlayGoSetInstallSpeed,
            [1, u64::MAX, 0, 0],
            BAD_SPEED,
        ),
    ] {
        assert_eq!(call(&mut owner, &mut guest, service, &args), Ok(expected));
    }
    assert_eq!(guest.get(OUT, 16), [0xa5; 16]);
    for speed in 0..=2 {
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::PlayGoSetInstallSpeed,
                &[1, 0xaaaa_0000_0000_0000 | speed]
            ),
            Ok(0)
        );
    }
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetInstallSpeed,
            &[1, OUT]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(OUT, 8), [2, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
    guest.put(0x100, &[2, 0, 3, 0xff]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoSetToDoList,
            &[1, IDS, 1]
        ),
        Ok(0)
    );
    guest.put(0x100, &[3, 0, 1, 0xff]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoSetToDoList,
            &[1, IDS, 1]
        ),
        Ok(BAD_CHUNK)
    );
    guest.put(0x100, &[2, 0, 1, 0xff]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoSetToDoList,
            &[1, IDS, 1]
        ),
        Ok(BAD_LOCUS)
    );
    for kind in [0, 1] {
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::PlayGoPrefetchOptionalChunk,
                &[1, 0xbbbb_0000_0000_0000 | kind, u64::MAX]
            ),
            Ok(0)
        );
    }
}

#[test]
fn progress_is_entry_count_with_duplicate_ids_and_optional_masks_have_exact_spans() {
    let mut owner = loaded();
    let mut guest = TestGuest::new();
    guest.put(0x100, &[2, 0, 2, 0, 0, 0]);
    guest.put(0x1ff, &[0xa5; 34]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetProgress,
            &[1, IDS, 3, OUT]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x200), 3);
    assert_eq!(guest.u64(0x208), 3);
    assert_eq!(guest.get(OUT - 1, 1), [0xa5]);
    assert_eq!(guest.get(OUT + 16, 1), [0xa5]);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetEta,
            &[1, IDS, 3, OUT + 16]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x210), 0);
    assert_eq!(guest.get(OUT + 24, 1), [0xa5]);
    guest.put(0x208, &[0xa5]);
    for service in [
        Service::PlayGoGetOptionalChunk,
        Service::PlayGoGetSupportedOptionalChunk,
    ] {
        for (kind, mask) in [(0, u64::MAX), (1, 0x1f)] {
            assert_eq!(
                call(&mut owner, &mut guest, service, &[1, kind, OUT]),
                Ok(0)
            );
            assert_eq!(guest.u64(0x200), mask);
            assert_eq!(guest.get(OUT + 8, 1), [0xa5]);
        }
    }
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLanguageMask,
            &[1, OUT]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x200), u64::MAX);
    // All input IDs are checked before progress overwrites the same bytes.
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetProgress,
            &[1, IDS, 3, IDS]
        ),
        Ok(0)
    );
    assert_eq!(guest.u64(0x100), 3);
    assert_eq!(guest.u64(0x108), 3);
}

#[test]
fn typed_read_write_and_resource_refusals_publish_no_prefix_or_count_and_retry_is_safe() {
    let mut owner = loaded();
    let mut guest = TestGuest::new();
    guest.put(0x100, &[0, 0, 1, 0]);
    guest.put(0x200, &[0xa5; 16]);
    guest.put(PAGE - 1, &[0xa5]);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[1, IDS, 2, ARG + PAGE - 1]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(ARG + PAGE - 1, 1), [0xa5]);
    guest.put(PAGE - 2, &[0, 0]);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[1, ARG + PAGE - 2, 2, OUT]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(OUT, 16), [0xa5; 16]);
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetChunkId,
            &[1, OUT, 3, ARG + PAGE - 2]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(guest.get(OUT, 16), [0xa5; 16]);
    guest.refuse = true;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[1, IDS, 2, OUT]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(OUT, 16), [0xa5; 16]);
    guest.refuse = false;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::PlayGoGetLocus,
            &[1, IDS, 2, OUT]
        ),
        Ok(0)
    );
    assert_eq!(guest.get(OUT, 3), [3, 3, 0xa5]);
    for bound in 0..3 {
        let mut limited = loaded();
        match bound {
            0 => limited.budget.limits.max_bytes = 0,
            1 => limited.budget.limits.max_work = limited.usage().work,
            _ => limited.budget.limits.max_scratch_bytes = 0,
        }
        guest.put(0x200, &[0xa5; 16]);
        assert!(matches!(
            call(
                &mut limited,
                &mut guest,
                Service::PlayGoGetLocus,
                &[1, IDS, 2, OUT]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(guest.get(OUT, 16), [0xa5; 16]);
        assert_eq!(limited.playgo.chunks, Some(3));
        assert_eq!(limited.usage().handles_live, 0);
        assert_eq!(limited.usage().mapped_bytes, 0);
    }
}
