// SPDX-License-Identifier: GPL-2.0-only

//! Actual native copies, rollback, unchanged live storage and capacity refusal.
use super::*;
use crate::memory::PAGE_SIZE;
use crate::process::tls::{StaticTlsLimits, ThreadControlBlockPolicy, plan_relocated_static_tls};
use pentax_linker::{ModuleInput, append_symbolic_module, plan_symbolic, plan_tls_bindings};
use pentax_loader::open_executable;
use pentax_platform::native_memory::{NativeAddressSpace, NativeMemoryLimits};

mod batch;
mod replacement;

fn original(value: u64, size: u64) -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 2)] {
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [(20, 1_u32), (64, 1), (68, 5), (120, 7), (124, 4)] {
        bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    for (at, value) in [
        (24, 0x100_u64),
        (32, 64),
        (96, 4096),
        (104, 65_536),
        (112, PAGE_SIZE),
        (128, 0x300),
        (136, 0x300),
        (152, 8),
        (160, size),
        (168, 16),
        (0x300, value),
    ] {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x100] = 0xc3;
    bytes
}

#[test]
fn failed_finalization_restores_spare_tls_bytes_and_old_history_after_actual_copy_then_allows_clean_component_retry()
 {
    let first = original(37, 32);
    let second = original(43, 32);
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10_0000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let source = plan_symbolic(
        &[ModuleInput::new(open_executable(&first).unwrap(), base)],
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&source, &Default::default()).unwrap();
    let policy = StaticTlsLimits {
        reserved_static_bytes: PAGE_SIZE,
        thread_control: ThreadControlBlockPolicy::KytyObservedSelfPointerV1,
        max_instances: 3,
        max_storage_bytes: 6 * PAGE_SIZE,
        ..Default::default()
    };
    let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
    let tp = base + 0x48000;
    let mut session = super::super::prepare_native_in(
        &layout,
        &StaticTlsStorage {
            thread_pointers: &[tp],
        },
        &NativeLoadLimits::default(),
        Some(owner),
    )
    .unwrap();
    let identity = session.memory.owner_id();
    assert!(session.append_thread_instance(tp).is_err());
    assert!(session.append_thread_instance(base + 0x20_8000).is_err());
    assert_eq!(session.instances().len(), 1);
    session
        .pause()
        .unwrap()
        .write(tp - 32, &99_u64.to_le_bytes())
        .unwrap();
    session
        .pause()
        .unwrap()
        .write(tp - 64, &107_u64.to_le_bytes())
        .unwrap();
    let second_tp = base + 0x68000;
    assert_eq!(session.append_thread_instance(second_tp).unwrap(), 1);
    let mut fresh = [0; 32];
    session
        .pause()
        .unwrap()
        .read(second_tp - 32, &mut fresh)
        .unwrap();
    assert_eq!(&fresh[..8], &37_u64.to_le_bytes());
    assert!(fresh[8..].iter().all(|byte| *byte == 0));
    session
        .pause()
        .unwrap()
        .write(second_tp - 32, &101_u64.to_le_bytes())
        .unwrap();
    let next = append_symbolic_module(
        &source,
        ModuleInput::new(open_executable(&second).unwrap(), base + 0x10000),
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&next, &Default::default()).unwrap();
    let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
    let window = NativeRange {
        address: base + 0x10000,
        size: 0x10000,
    };
    session.memory.fail_append_after_initializer = true;
    assert!(
        session
            .append_module(&layout, window, &NativeLoadLimits::default())
            .is_err()
    );
    assert!(
        !session.memory.fail_append_after_initializer,
        "fault must occur after the real copy"
    );
    assert_eq!(session.memory.owner_id(), identity);
    assert_eq!(session.memory.active_source_identity(), source.identity());
    assert_eq!(session.memory.module_count(), 1);
    assert_eq!(session.templates().len(), 1);
    assert_eq!(session.initializers.len(), 1);
    let mut value = [0; 8];
    session.pause().unwrap().read(tp - 32, &mut value).unwrap();
    assert_eq!(u64::from_le_bytes(value), 99);
    session.pause().unwrap().read(tp - 64, &mut value).unwrap();
    assert_eq!(u64::from_le_bytes(value), 107);
    session
        .pause()
        .unwrap()
        .read(second_tp - 64, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 0);
    assert!(matches!(
        session.pause().unwrap().read(window.address, &mut [0]),
        Err(NativeMemoryError::Unmapped(_))
    ));
    session
        .append_module(&layout, window, &NativeLoadLimits::default())
        .unwrap();
    assert_eq!(session.templates().len(), 2);
    assert_eq!(session.instances()[0].blocks.len(), 2);
    assert_eq!(session.instances()[0].thread_pointer, tp);
    session.pause().unwrap().read(tp - 32, &mut value).unwrap();
    assert_eq!(u64::from_le_bytes(value), 99);
    session.pause().unwrap().read(tp - 64, &mut value).unwrap();
    assert_eq!(u64::from_le_bytes(value), 43);
    session
        .pause()
        .unwrap()
        .read(second_tp - 32, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 101);
    session
        .pause()
        .unwrap()
        .read(second_tp - 64, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 43);
    // Later threads use the original relocated initializers of both modules,
    // even after each previously running instance has changed its own values.
    session
        .pause()
        .unwrap()
        .write(second_tp - 64, &109_u64.to_le_bytes())
        .unwrap();
    let third_tp = base + 0x88000;
    assert_eq!(session.append_thread_instance(third_tp).unwrap(), 2);
    session
        .pause()
        .unwrap()
        .read(third_tp - 32, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 37);
    session
        .pause()
        .unwrap()
        .read(third_tp - 64, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 43);
    session.pause().unwrap().read(third_tp, &mut value).unwrap();
    assert_eq!(u64::from_le_bytes(value), third_tp);
    for guard in session.instances()[2].guard_pages.unwrap() {
        session
            .pause()
            .unwrap()
            .preflight_reserved_unmapped(guard, PAGE_SIZE)
            .unwrap();
    }
    assert!(session.append_thread_instance(base + 0xa8000).is_err());
    assert_eq!(session.instances().len(), 3);
    assert!(
        session
            .append_module(&layout, window, &NativeLoadLimits::default())
            .is_err()
    );
}

#[test]
fn later_tls_cannot_expand_past_admitted_capacity_or_move_the_thread_pointer() {
    let first = original(37, 32);
    let second = original(43, 2 * PAGE_SIZE);
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10_0000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let source = plan_symbolic(
        &[ModuleInput::new(open_executable(&first).unwrap(), base)],
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&source, &Default::default()).unwrap();
    let policy = StaticTlsLimits {
        reserved_static_bytes: PAGE_SIZE,
        ..Default::default()
    };
    let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
    let tp = base + 0x48000;
    let mut session = super::super::prepare_native_in(
        &layout,
        &StaticTlsStorage {
            thread_pointers: &[tp],
        },
        &NativeLoadLimits::default(),
        Some(owner),
    )
    .unwrap();
    let next = append_symbolic_module(
        &source,
        ModuleInput::new(open_executable(&second).unwrap(), base + 0x10000),
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&next, &Default::default()).unwrap();
    let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
    assert!(
        session
            .append_module(
                &layout,
                NativeRange {
                    address: base + 0x10000,
                    size: 0x10000
                },
                &NativeLoadLimits::default()
            )
            .is_err()
    );
    assert_eq!(session.memory.active_source_identity(), source.identity());
    assert_eq!(session.memory.module_count(), 1);
    assert_eq!(session.templates().len(), 1);
    assert_eq!(session.instances()[0].thread_pointer, tp);
    assert!(matches!(
        session.pause().unwrap().read(base + 0x10000, &mut [0]),
        Err(NativeMemoryError::Unmapped(_))
    ));
}

#[test]
fn first_future_template_is_copied_into_independent_prepared_instances_without_a_pointer_move() {
    let mut first = original(37, 32);
    first[56..58].copy_from_slice(&1_u16.to_le_bytes()); // No initial PT_TLS.
    let second = original(43, 32);
    let owner = NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10_0000).unwrap();
    let base = owner.reserved_ranges().next().unwrap().address;
    let source = plan_symbolic(
        &[ModuleInput::new(open_executable(&first).unwrap(), base)],
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&source, &Default::default()).unwrap();
    let policy = StaticTlsLimits {
        reserved_static_bytes: PAGE_SIZE,
        ..Default::default()
    };
    let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
    let pointers = [base + 0x48000, base + 0x58000];
    let mut session = super::super::prepare_native_in(
        &layout,
        &StaticTlsStorage {
            thread_pointers: &pointers,
        },
        &NativeLoadLimits::default(),
        Some(owner),
    )
    .unwrap();
    assert!(session.templates().is_empty());
    let next = append_symbolic_module(
        &source,
        ModuleInput::new(open_executable(&second).unwrap(), base + 0x10000),
        &Default::default(),
    )
    .unwrap();
    let bindings = plan_tls_bindings(&next, &Default::default()).unwrap();
    let layout = plan_relocated_static_tls(&bindings, &policy).unwrap();
    session
        .append_module(
            &layout,
            NativeRange {
                address: base + 0x10000,
                size: 0x10000,
            },
            &NativeLoadLimits::default(),
        )
        .unwrap();
    assert_eq!(session.templates().len(), 1);
    for (instance, tp) in session.instances().iter().zip(pointers) {
        assert_eq!(instance.thread_pointer, tp);
        assert_eq!(instance.blocks[0].runtime_id.get(), 1);
    }
    session
        .pause()
        .unwrap()
        .write(pointers[0] - 32, &99_u64.to_le_bytes())
        .unwrap();
    let mut value = [0; 8];
    session
        .pause()
        .unwrap()
        .read(pointers[1] - 32, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 43);
    session
        .pause()
        .unwrap()
        .read(pointers[0] - 24, &mut value)
        .unwrap();
    assert_eq!(u64::from_le_bytes(value), 0);
}
