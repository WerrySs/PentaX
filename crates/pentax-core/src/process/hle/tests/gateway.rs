// SPDX-License-Identifier: GPL-2.0-or-later

//! Closed gateway, site authority, budget and end-to-end fixture tests.

use super::super::fixture::{
    Asm, DATA, FixtureLimits, Import, Launched, OPS, Program, STACK, STACK_SIZE, expected_data,
    launch, program,
};
use super::super::provider::stub;
use super::*;
use crate::cpu::CpuError;
use crate::memory::{GuestMemory, MemoryError, PAGE_SIZE};

fn word(memory: &GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

fn data(memory: &GuestMemory, offset: u64, size: usize) -> Vec<u8> {
    let mut bytes = vec![0; size];
    memory.read(DATA + offset, &mut bytes).unwrap();
    bytes
}

/// Launch a consumer whose main code is built by `code` at the entry RVA.
fn launch_code(imports: Vec<Import>, code: impl FnOnce(&mut Asm)) -> Launched {
    let mut asm = Asm::new(CODE);
    code(&mut asm);
    let mut consumer = Consumer::importing(vec![(CODE, asm.bytes)]);
    consumer.imports = imports;
    launch(&consumer.build().unwrap(), &FixtureLimits::default()).unwrap()
}

fn all_imports() -> Vec<Import> {
    KnownFunction::LIBC.map(Import::function).to_vec()
}

fn original() -> (Program, Vec<u8>) {
    let program = program(&OPS).unwrap();
    let consumer = program.consumer().build().unwrap();
    (program, consumer)
}

#[test]
fn closed_original_fixture_reports_exact_completed_evidence() {
    let report = run_hle_memory_fixture().unwrap();
    assert_eq!(
        report,
        HleFixtureReport {
            memcpy_calls: 4,
            memmove_calls: 2,
            memset_calls: 2,
            memcmp_calls: 3,
            service_attempts: 11,
            requested_bytes: 224,
            work_units: 230_248,
            retired: 90,
            provider_bindings: 4,
            applied_relocations: 4,
            provider_bias: PROVIDER_BIAS,
            memcmp_results: [0, -1, 1],
            mapped_bytes: 10 * PAGE_SIZE,
            resident_bytes: 7 * PAGE_SIZE,
        }
    );
    // The diagnostic is deterministic and does not affect later runs.
    assert_eq!(run_hle_memory_fixture().unwrap(), report);
}

#[test]
fn host_model_covers_overlap_directions_fill_bytes_and_unsigned_comparisons() {
    let expected = expected_data(&OPS);
    // memcpy, then a forward (dst > src) and a backward (dst < src) memmove.
    let mut region = (0..64_u8)
        .map(|index| index.wrapping_mul(7).wrapping_add(1))
        .collect::<Vec<_>>();
    region.copy_within(0..32, 0x10);
    region.copy_within(8..32, 0);
    assert_eq!(&expected[0x200..0x240], region);
    assert_eq!(&expected[0x100..0x120], [0x5a; 32]);
    assert_eq!(&expected[0x700..0x708], [0xc3; 8]);
    assert_eq!(&expected[0x708..0x710], [0; 8]);
    let slot = |index: usize| {
        u64::from_le_bytes(
            expected[0x800 + 8 * index..0x808 + 8 * index]
                .try_into()
                .unwrap(),
        )
    };
    assert_eq!(slot(5), 0);
    assert_eq!(slot(6), 0xffff_ffff);
    assert_eq!(slot(7), 1);
    assert_eq!(slot(9), DATA + 0x700);
    assert_eq!(slot(10), 0);
    let (program, _) = original();
    assert_eq!(program.retired, 90);
}

#[test]
fn one_retired_instruction_budget_is_shared_across_every_resume() {
    let (program, consumer) = original();
    let limits = |instructions| FixtureLimits {
        instructions,
        ..FixtureLimits::default()
    };
    let mut launched = launch(&consumer, &limits(program.retired - 1)).unwrap();
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::InstructionLimit {
            limit: program.retired - 1,
            retired: program.retired - 1,
        }
    );
    // Every service resumed under the same count; only the INT3 is missing.
    assert_eq!(launched.gateway.budget.completed(), OPS.len() as u64);
    let mut launched = launch(&consumer, &limits(program.retired)).unwrap();
    assert_eq!(
        launched.gateway.run().unwrap(),
        CONSUMER_BIAS + program.halt
    );
    assert!(launch(&consumer, &limits(513)).is_err());
}

#[test]
fn attempt_cap_counts_every_trap_and_stops_without_a_fresh_budget() {
    let (_, consumer) = original();
    let mut limits = FixtureLimits::default();
    limits.hle.services.max_attempts = OPS.len() as u64 - 1;
    let mut launched = launch(&consumer, &limits).unwrap();
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::Service(MemoryServiceError::LimitExceeded {
            kind: ServiceLimit::Attempts,
            requested: OPS.len() as u64,
            maximum: OPS.len() as u64 - 1,
        })
    );
    assert_eq!(launched.gateway.budget.completed(), OPS.len() as u64 - 1);
}

#[test]
fn injected_scratch_failure_stops_before_any_destination_write() {
    let (_, consumer) = original();
    let mut launched = launch(&consumer, &FixtureLimits::default()).unwrap();
    launched.gateway.budget.refuse_reservation = Some(0);
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::Service(MemoryServiceError::Allocation)
    );
    let gateway = &launched.gateway;
    assert_eq!(data(&gateway.memory, 0x100, 32), [0; 32]);
    assert_eq!(gateway.budget.attempts(), 1);
    assert_eq!(gateway.budget.completed(), 0);
    // The failed service published no result; RAX still holds the ID.
    assert_eq!(
        gateway.cpu.registers[0],
        u64::from(KnownFunction::Memset.service_id())
    );
}

#[test]
fn a_copied_stub_outside_the_provider_cannot_dispatch() {
    let mut launched = launch_code(all_imports(), |asm| {
        asm.mov(7, DATA + 0x100);
        asm.mov(6, 0x77);
        asm.mov32(2, 8);
        // The exact generated memset stub, but in consumer-owned RX bytes.
        asm.raw(&stub(KnownFunction::Memset)[..5]);
        asm.raw(&[0x0f, 0x05]);
        asm.raw(&[0xc3]);
    });
    let rip = CONSUMER_BIAS + CODE as u64 + 15 + 7;
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::SiteMismatch { rip }
    );
    assert_eq!(data(&launched.gateway.memory, 0x100, 8), [0; 8]);
    assert_eq!(launched.gateway.budget.attempts(), 1);
    assert_eq!(launched.gateway.budget.completed(), 0);
}

#[test]
fn unknown_syscalls_are_counted_and_refused() {
    let mut launched = launch_code(all_imports(), |asm| {
        asm.mov32(0, 0x1234);
        asm.raw(&[0x0f, 0x05]);
    });
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::SiteMismatch {
            rip: CONSUMER_BIAS + CODE as u64 + 7
        }
    );
    assert_eq!(launched.gateway.budget.attempts(), 1);
    assert_eq!(
        launched.gateway.budget.work(),
        KnownFunction::ALL.len() as u64
    );
}

#[test]
fn direct_trap_entry_needs_the_full_rax_id_and_gains_no_extra_privilege() {
    // Slot 2 holds memset's entry + 5 through R_X86_64_64 arithmetic, so the
    // guest call lands directly on the SYSCALL with its own RAX.
    let mut imports = all_imports();
    imports[2].relocation = 1;
    imports[2].addend = 5;
    let id = u64::from(KnownFunction::Memset.service_id());
    for (rax, valid) in [(id, true), (id | (1 << 32), false)] {
        let mut launched = launch_code(imports.clone(), |asm| {
            asm.mov(7, DATA + 0x100);
            asm.mov(6, 0x3c);
            asm.mov32(2, 4);
            asm.mov(0, rax);
            asm.call_slot(2);
            asm.store_rax(0x800);
            asm.int3();
        });
        let slot = word(&launched.gateway.memory, CONSUMER_BIAS + 0x4000 + 16);
        assert_eq!(slot, launched.gateway.sites()[2].trap);
        let result = launched.gateway.run();
        if valid {
            result.unwrap();
            assert_eq!(
                data(&launched.gateway.memory, 0x100, 5),
                [0x3c, 0x3c, 0x3c, 0x3c, 0]
            );
            assert_eq!(word(&launched.gateway.memory, DATA + 0x800), DATA + 0x100);
        } else {
            assert_eq!(
                result.unwrap_err(),
                HleError::ServiceIdMismatch {
                    expected: KnownFunction::Memset.service_id(),
                    actual: rax,
                }
            );
            assert_eq!(data(&launched.gateway.memory, 0x100, 4), [0; 4]);
            assert_eq!(launched.gateway.budget.attempts(), 1);
        }
    }
}

#[test]
fn a_forged_continuation_register_is_refused_at_an_owned_site() {
    let (_, consumer) = original();
    let mut launched = launch(&consumer, &FixtureLimits::default()).unwrap();
    let gateway = &mut launched.gateway;
    let site = gateway.sites()[0];
    gateway.cpu.rip = site.resume;
    gateway.cpu.registers[1] = site.trap;
    gateway.cpu.registers[0] = u64::from(site.function.service_id());
    assert_eq!(
        gateway.service().unwrap_err(),
        HleError::ContinuationMismatch {
            rip: site.resume,
            rcx: site.trap,
        }
    );
    assert_eq!(gateway.budget.attempts(), 1);
}

#[test]
fn service_failures_leave_the_post_trap_cpu_and_destination_unchanged() {
    let mut launched = launch_code(all_imports(), |asm| {
        asm.mov(7, DATA + 8);
        asm.mov(6, DATA);
        asm.mov32(2, 16);
        asm.call_slot(0); // Overlapping memcpy.
        asm.int3();
    });
    let error = launched.gateway.run().unwrap_err();
    assert_eq!(
        error,
        HleError::Service(MemoryServiceError::UnsupportedOverlap)
    );
    let gateway = &launched.gateway;
    let site = gateway.sites()[0];
    assert_eq!(gateway.cpu.rip, site.resume);
    assert_eq!(gateway.cpu.registers[1], site.resume);
    assert_eq!(
        gateway.cpu.registers[0],
        u64::from(site.function.service_id())
    );
    assert_eq!(gateway.cpu.registers[4], launched.initial.registers[4] - 8);
    let original = (8..24_u8)
        .map(|index| index.wrapping_mul(7).wrapping_add(1))
        .collect::<Vec<_>>();
    assert_eq!(data(&gateway.memory, 8, 16), original);
}

#[test]
fn a_ret_fault_after_a_valid_service_write_keeps_that_write() {
    let return_slot = STACK + STACK_SIZE - 16;
    let mut launched = launch_code(all_imports(), |asm| {
        // Overwrite this CALL's own return address with memset.
        asm.mov(7, return_slot);
        asm.mov(6, 0xff);
        asm.mov32(2, 8);
        asm.call_slot(2);
        asm.int3();
    });
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::Cpu(CpuError::InvalidAddress)
    );
    let gateway = &launched.gateway;
    assert_eq!(word(&gateway.memory, return_slot), u64::MAX);
    assert_eq!(gateway.budget.completed(), 1);
    // RET failed transactionally at the stub; RAX holds the service result.
    assert_eq!(gateway.cpu.rip, gateway.sites()[2].resume);
    assert_eq!(gateway.cpu.registers[0], return_slot);
}

#[test]
fn owned_storage_is_nx_and_guarded() {
    let mut launched = launch_code(all_imports(), |asm| {
        asm.mov(0, DATA);
        asm.raw(&[0xff, 0xd0]); // CALL RAX into NX data.
    });
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::Cpu(CpuError::Memory(MemoryError::PermissionDenied(DATA)))
    );
    let memory = &launched.gateway.memory;
    for address in [
        DATA - PAGE_SIZE,
        DATA + 2 * PAGE_SIZE,
        STACK - PAGE_SIZE,
        STACK + STACK_SIZE,
    ] {
        assert!(matches!(
            memory.read(address, &mut [0]),
            Err(MemoryError::Unmapped(_))
        ));
    }
    let mut launched = launch_code(all_imports(), |asm| {
        // A destination span running into the unmapped guard after data.
        asm.mov(7, DATA + 2 * PAGE_SIZE - 4);
        asm.mov(6, 0x11);
        asm.mov32(2, 8);
        asm.call_slot(2);
        asm.int3();
    });
    assert_eq!(
        launched.gateway.run().unwrap_err(),
        HleError::Service(MemoryServiceError::Memory(MemoryError::Unmapped(
            DATA + 2 * PAGE_SIZE
        )))
    );
    assert_eq!(data(&launched.gateway.memory, 2 * PAGE_SIZE - 4, 4), [0; 4]);
}
