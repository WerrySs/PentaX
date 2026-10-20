// SPDX-License-Identifier: GPL-2.0-only

use super::super::dispatch::{AbiArgs, ServiceBudget, invoke};
use super::super::fixture::Import;
use super::super::provider::{HEAP_TRACE_BYTES, HEAP_TRACE_RVA};
use super::*;
use crate::memory::{MemoryError, PAGE_SIZE, Permissions};
use crate::process::SessionLoadLimits;

fn trace_consumer() -> Consumer {
    let mut consumer = minimal_consumer();
    consumer.library_name = b"LibcInternalExt";
    consumer.imports = vec![Import::function(KnownFunction::HeapGetTraceInfo)];
    consumer
}

fn query(
    memory: &mut crate::memory::GuestMemory,
    budget: &mut ServiceBudget,
    output: u64,
) -> Result<u64, MemoryServiceError> {
    let attempt = budget.begin_attempt()?;
    invoke(
        attempt,
        KnownFunction::HeapGetTraceInfo,
        AbiArgs {
            rdi: output,
            rsi: u64::MAX,
            rdx: u64::MAX,
        },
        memory,
        budget,
    )
    .map(|v| v.rax)
}

#[test]
fn heap_trace_info_uses_exact_extension_scope_and_persistent_owned_nx_storage() {
    let providers = providers();
    let bytes = trace_consumer().build().unwrap();
    let plan = plan_one(&bytes, &providers).unwrap();
    assert_eq!(plan.provider_bindings(), 1);
    let backing = PROVIDER_BIAS + HEAP_TRACE_RVA;
    for _ in 0..2 {
        let mut prepared = prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
        let mut budget =
            ServiceBudget::new(plan.service_limits()).with_provider(plan.provider_bias());
        let memory = &mut prepared.session.memory;
        let output = 0x80_0000;
        memory
            .map(output, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        let mut record = [0x5a; 32];
        record[..8].copy_from_slice(&32_u64.to_le_bytes());
        memory.write(output, &record).unwrap();
        let mut initial = [1; HEAP_TRACE_BYTES as usize];
        memory.read(backing, &mut initial).unwrap();
        assert_eq!(initial, [0; HEAP_TRACE_BYTES as usize]);
        assert!(matches!(
            memory.fetch(backing, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ));
        assert_eq!(query(memory, &mut budget, output), Ok(0));
        let mut actual = [0; 32];
        memory.read(output, &mut actual).unwrap();
        assert_eq!(&actual[..16], &record[..16]);
        assert_eq!(
            u64::from_le_bytes(actual[16..24].try_into().unwrap()),
            backing
        );
        assert_eq!(
            u64::from_le_bytes(actual[24..].try_into().unwrap()),
            backing + 8
        );
        memory.write(backing, &17_u64.to_le_bytes()).unwrap();
        memory.write(backing + 512, &29_u64.to_le_bytes()).unwrap();
        query(memory, &mut budget, output).unwrap();
        let mut value = [0; 8];
        memory.read(backing, &mut value).unwrap();
        assert_eq!(u64::from_le_bytes(value), 17);
        memory.read(backing + 512, &mut value).unwrap();
        assert_eq!(u64::from_le_bytes(value), 29);
    }
    for (module, library, version) in [
        (b"LibcInternal".as_slice(), b"LibcInternal".as_slice(), 1),
        (b"libkernel", b"LibcInternalExt", 1),
        (b"LibcInternal", b"LibcInternalExt", 2),
    ] {
        let mut consumer = trace_consumer();
        consumer.module_name = module;
        consumer.library_name = library;
        consumer.version = version;
        assert!(plan_one(&consumer.build().unwrap(), &providers).is_err());
    }
}

#[test]
fn heap_trace_info_rejects_bad_size_partial_output_and_revoked_backing_without_partial_results() {
    let providers = providers();
    let bytes = trace_consumer().build().unwrap();
    let plan = plan_one(&bytes, &providers).unwrap();
    for refusal in 0..5 {
        let mut prepared = prepare_hle_session(&plan, &SessionLoadLimits::default()).unwrap();
        let mut budget =
            ServiceBudget::new(plan.service_limits()).with_provider(plan.provider_bias());
        let memory = &mut prepared.session.memory;
        let base = 0x80_0000;
        memory
            .map(base, PAGE_SIZE, Permissions::READ_WRITE)
            .unwrap();
        let output = if refusal == 1 {
            base + PAGE_SIZE - 24
        } else {
            base
        };
        let mut before = [0xa5; 24];
        before[..8].copy_from_slice(&(if refusal == 0 { 31_u64 } else { 32 }).to_le_bytes());
        memory.write(output, &before).unwrap();
        let backing = PROVIDER_BIAS + HEAP_TRACE_RVA;
        match refusal {
            2 => memory
                .protect(backing, PAGE_SIZE, Permissions::READ)
                .unwrap(),
            3 => memory.protect(base, PAGE_SIZE, Permissions::READ).unwrap(),
            4 => {
                budget = ServiceBudget::new(MemoryServiceLimits {
                    max_total_bytes: 31,
                    ..Default::default()
                })
                .with_provider(plan.provider_bias())
            }
            _ => {}
        }
        assert!(query(memory, &mut budget, output).is_err());
        let mut after = [0; 24];
        memory.read(output, &mut after).unwrap();
        assert_eq!(after, before);
        assert_eq!(budget.completed(), 0);
    }
}
