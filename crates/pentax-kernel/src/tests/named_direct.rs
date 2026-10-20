// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn runtime_data_initialization_and_backing_failures_publish_nothing_before_retry() {
    let mut k = kernel(Default::default());
    let mut g = TestGuest::new();
    let before = g.effects;
    assert_eq!(
        k.allocate_runtime_data(8, &mut g, |_, _| Err(Failure::Unsupported(
            "original initializer failure"
        ))),
        Err(Failure::Unsupported("original initializer failure"))
    );
    assert_eq!(g.effects, before);
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (0, 0));
    g.refuse = true;
    assert_eq!(
        k.allocate_runtime_data(8, &mut g, |base, out| {
            out.copy_from_slice(&base.to_le_bytes());
            Ok(())
        }),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (0, 0));
    g.refuse = false;
    let address = k
        .allocate_runtime_data(PAGE as usize + 3, &mut g, |base, out| {
            out[..8].copy_from_slice(&(base + 8).to_le_bytes());
            out[PAGE as usize..].fill(37);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        (k.usage().handles_live, k.usage().mapped_bytes),
        (1, 2 * PAGE)
    );
    assert_eq!(g.get(address, 8), (address + 8).to_le_bytes());
    assert_eq!(g.get(address + PAGE, 4), [37, 37, 37, 0]);
    assert_eq!(
        k.invoke(
            Service::Munmap,
            CallArgs([address, 2 * PAGE, 0, 0, 0, 0]),
            &mut g
        ),
        Err(Failure::Unsupported("runtime-owned kernel mapping"))
    );
    for limits in [
        Limits {
            max_handles: 0,
            ..Default::default()
        },
        Limits {
            max_mapped_bytes: 0,
            ..Default::default()
        },
        Limits {
            max_bytes: 0,
            ..Default::default()
        },
    ] {
        let mut k = kernel(limits);
        assert!(
            k.allocate_runtime_data(8, &mut g, |_, out| {
                out.fill(42);
                Ok(())
            })
            .is_err()
        );
        assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (0, 0));
    }
}

#[test]
fn image_protection_backing_failure_preserves_metadata_accounting_and_retry() {
    struct ImageGuest(TestGuest);
    impl GuestAccess for ImageGuest {
        fn read(&self, a: u64, b: &mut [u8]) -> Result<(), GuestError> {
            self.0.read(a, b)
        }
        fn apply(
            &mut self,
            effect: MemoryEffect,
            writes: &[GuestWrite<'_>],
        ) -> Result<(), GuestError> {
            self.0.apply(effect, writes)
        }
        fn preflight_image_memory(&self, a: u64, n: u64) -> Result<bool, Failure> {
            Ok(a == ARG && n == PAGE)
        }
    }
    let mut k = kernel(Default::default());
    let mut g = ImageGuest(TestGuest::new());
    g.0.regions.push(Region {
        start: ARG + PAGE,
        length: PAGE,
        protection: 3,
        physical: None,
        bytes: vec![0; PAGE as usize],
    });
    let protect = CallArgs([ARG, PAGE, 1, 0, 0, 0]);
    let query = CallArgs([ARG, 0, 0, ARG + PAGE, 0, 0]);
    g.0.refuse = true;
    assert_eq!(
        k.invoke(Service::Mprotect, protect, &mut g),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (0, 0));
    assert!(g.0.region(ARG, 1, true).is_ok());
    g.0.refuse = false;
    assert_eq!(
        k.invoke(Service::QueryProtection, query, &mut g),
        Err(Failure::Errno(Errno::Access))
    );
    assert_eq!(k.invoke(Service::Mprotect, protect, &mut g), Ok(0));
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (1, PAGE));
    assert!(g.0.region(ARG, 1, true).is_err());
    g.0.refuse = true;
    assert_eq!(
        k.invoke(Service::Mprotect, CallArgs([ARG, PAGE, 0, 0, 0, 0]), &mut g),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    g.0.refuse = false;
    assert_eq!(k.invoke(Service::QueryProtection, query, &mut g), Ok(0));
    assert_eq!(g.0.get(ARG + PAGE, 4), 1_u32.to_le_bytes());
    assert_eq!((k.usage().handles_live, k.usage().mapped_bytes), (1, PAGE));
}

#[test]
fn cpu_write_mapping_and_protection_are_read_write_while_query_metadata_stays_original() {
    let (mut k, mut g) = allocated();
    let requested = 0x8000_0002;
    call(
        &mut k,
        &mut g,
        Service::MapDirect,
        &[ARG + 8, PAGE, requested, 0, 0, PAGE],
    )
    .unwrap();
    let mapped = g.u64(8);
    assert!(g.region(mapped, 1, false).is_ok());
    assert!(g.region(mapped, 1, true).is_ok());
    for (protection, readable, writable) in
        [(1, true, false), (0, false, false), (requested, true, true)]
    {
        call(
            &mut k,
            &mut g,
            Service::Mprotect,
            &[mapped, PAGE, protection],
        )
        .unwrap();
        call(
            &mut k,
            &mut g,
            Service::QueryProtection,
            &[mapped, 0, 0, ARG + 16],
        )
        .unwrap();
        assert_eq!(g.get(ARG + 16, 4), (protection as u32).to_le_bytes());
        assert_eq!(g.region(mapped, 1, false).is_ok(), readable);
        assert_eq!(g.region(mapped, 1, true).is_ok(), writable);
    }
}

#[test]
fn nonfixed_mapping_honors_external_hint_with_explicit_ceiling_and_atomic_refusal() {
    let defaults = Limits::default();
    let initial_end = defaults.virtual_base + defaults.virtual_bytes;
    let hint = initial_end + PAGE;
    let mut k = kernel(Limits {
        nonfixed_virtual_end: Some(hint + PAGE),
        ..defaults
    });
    let mut g = TestGuest::new();
    call(
        &mut k,
        &mut g,
        Service::AllocateMainDirect,
        &[PAGE, PAGE, 3, ARG],
    )
    .unwrap();
    g.put(8, &hint.to_le_bytes());
    call(
        &mut k,
        &mut g,
        Service::MapDirect,
        &[ARG + 8, PAGE, 3, 0, 0, PAGE],
    )
    .unwrap();
    assert_eq!(g.u64(8), hint);
    assert_eq!(k.memory_map_refusal(), None);
    let before = (g.effects, k.usage().handles_live, k.usage().mapped_bytes);
    // The occupied external hint cannot search past the explicit ceiling.
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 8, PAGE, 3, 0, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(g.u64(8), hint);
    assert_eq!(
        (g.effects, k.usage().handles_live, k.usage().mapped_bytes),
        before
    );
    // Fixed placement still retains the original bounded policy.
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 8, PAGE, 3, 0x10, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(g.u64(8), hint);
    assert_eq!(
        (g.effects, k.usage().handles_live, k.usage().mapped_bytes),
        before
    );
    for end in [initial_end - PAGE, initial_end + 1] {
        assert!(
            Kernel::new(
                Limits {
                    nonfixed_virtual_end: Some(end),
                    ..defaults
                },
                ProcessIdentity {
                    process_id: 1,
                    initial_thread_id: 1
                },
                vec![]
            )
            .is_err()
        );
    }
}

#[test]
fn mapping_refusal_provenance_distinguishes_coverage_window_and_collision_without_stale_state() {
    let (mut k, mut g) = allocated();
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 8, 2 * PAGE, 3, 0, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(
        k.memory_map_refusal(),
        Some(MemoryMapRefusal::PhysicalCoverage)
    );
    assert_eq!(call(&mut k, &mut g, Service::PageSize, &[]), Ok(PAGE));
    assert_eq!(k.memory_map_refusal(), None);
    g.put(
        8,
        &(Limits::default().virtual_base + Limits::default().virtual_bytes).to_le_bytes(),
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 8, PAGE, 3, 0x10, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(
        k.memory_map_refusal(),
        Some(MemoryMapRefusal::VirtualWindow)
    );
    g.put(8, &0_u64.to_le_bytes());
    call(
        &mut k,
        &mut g,
        Service::MapDirect,
        &[ARG + 8, PAGE, 3, 0, 0, PAGE],
    )
    .unwrap();
    assert_eq!(k.memory_map_refusal(), None);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 8, PAGE, 3, 0x90, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::NoMemory))
    );
    assert_eq!(
        k.memory_map_refusal(),
        Some(MemoryMapRefusal::FixedCollision)
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 8, 0, 3, 0, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::Invalid))
    );
    assert_eq!(k.memory_map_refusal(), None);
}

fn allocated() -> (Kernel, TestGuest) {
    let mut k = kernel(Limits::default());
    let mut g = TestGuest::new();
    call(
        &mut k,
        &mut g,
        Service::AllocateMainDirect,
        &[PAGE, 0, 3, ARG],
    )
    .unwrap();
    g.put(72, &(ARG + 128).to_le_bytes());
    g.put(128, &[0xfe; 31]);
    g.put(159, &[0]);
    (k, g)
}

#[test]
fn named_direct_uses_the_seventh_stack_argument_and_shares_physical_backing() {
    let (mut k, mut g) = allocated();
    let args = CallArgs([ARG + 8, PAGE, 3, 0, 0, PAGE]);
    assert_eq!(
        k.invoke_library_with_stack(Service::MapNamedDirect, args, Some(ARG + 64), &mut g),
        Ok(0)
    );
    let named = g.u64(8);
    call(
        &mut k,
        &mut g,
        Service::MapDirect,
        &[ARG + 16, PAGE, 3, 0, 0, PAGE],
    )
    .unwrap();
    g.copy(named + 17, b"shared");
    assert_eq!(g.get(g.u64(16) + 17, 6), b"shared");
    // The optional stack must not introduce reads for a register-only call.
    let before = k.usage().bytes_read;
    assert_eq!(
        k.invoke_with_stack(Service::PageSize, CallArgs([0; 6]), Some(u64::MAX), &mut g),
        Ok(PAGE)
    );
    assert_eq!(k.usage().bytes_read, before);
}

#[test]
fn named_direct_stack_name_and_backing_errors_preserve_mapping_and_output() {
    for refusal in 0..7 {
        let (mut k, mut g) = allocated();
        let mut stack = Some(ARG + 64);
        match refusal {
            0 => stack = None,
            1 => stack = Some(u64::MAX),
            2 => stack = Some(ARG + PAGE - 12),
            3 => g.put(72, &0_u64.to_le_bytes()),
            4 => g.put(159, &[0xfe]),
            5 => g.put(72, &u64::MAX.to_le_bytes()),
            6 => g.refuse = true,
            _ => unreachable!(),
        }
        let before = (g.effects, k.usage().handles_live, k.usage().mapped_bytes);
        let result = k.invoke_library_with_stack(
            Service::MapNamedDirect,
            CallArgs([ARG + 8, PAGE, 3, 0, 0, PAGE]),
            stack,
            &mut g,
        );
        match refusal {
            3 => assert_eq!(result, Ok(u64::from(Errno::Fault.kernel_code() as u32))),
            4 => assert_eq!(
                result,
                Ok(u64::from(Errno::NameTooLong.kernel_code() as u32))
            ),
            _ => assert!(result.is_err()),
        }
        assert_eq!(g.u64(8), 0);
        assert_eq!(
            (g.effects, k.usage().handles_live, k.usage().mapped_bytes),
            before
        );
    }
}

#[test]
fn non_access_metadata_never_grants_rights_and_queries_retain_the_complete_word() {
    let (mut k, mut g) = allocated();
    let protection = 0x8000_0003;
    call(
        &mut k,
        &mut g,
        Service::MapDirect,
        &[ARG + 8, PAGE, protection, 0, 0, PAGE],
    )
    .unwrap();
    let mapped = g.u64(8);
    for (word, writable) in [(0x8000_0001, false), (protection, true)] {
        call(&mut k, &mut g, Service::Mprotect, &[mapped, PAGE, word]).unwrap();
        call(
            &mut k,
            &mut g,
            Service::QueryProtection,
            &[mapped, 0, 0, ARG + 16],
        )
        .unwrap();
        assert_eq!(g.get(ARG + 16, 4), (word as u32).to_le_bytes());
        assert_eq!(g.region(mapped, 1, true).is_ok(), writable);
    }
    let before = (g.effects, k.usage().handles_live, k.usage().mapped_bytes);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 24, PAGE, 0x8000_0000, 0, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::Invalid))
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::Mprotect,
            &[mapped, PAGE, 0x8000_0000]
        ),
        Err(Failure::Errno(Errno::Invalid))
    );
    assert_eq!(
        call(
            &mut k,
            &mut g,
            Service::MapDirect,
            &[ARG + 24, PAGE, 0x8000_0004, 0, 0, PAGE]
        ),
        Err(Failure::Errno(Errno::Access))
    );
    assert_eq!(
        (g.effects, k.usage().handles_live, k.usage().mapped_bytes),
        before
    );
    assert_eq!(g.u64(24), 0);
}
