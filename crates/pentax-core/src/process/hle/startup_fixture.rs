// SPDX-License-Identifier: GPL-2.0-or-later

//! One closed original consumer exercises typed TLS and attested GOT services.
//! It accepts no image, plan, path or entry input and returns completed evidence
//! only. The guest FS base is an integer TP under PentaX layout policy, without
//! host FS, firmware TCB/DTV, native execution or startup ABI claims.

mod image;
#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod native_tests;
#[cfg(test)]
mod tests;

use super::dispatch::{SITE_LOOKUP_WORK, ServiceBudget, work};
use super::gateway;
use super::{
    HleError, HleProfile, HleStartupLimits, HleStartupPlan, KnownFunction, PreparedHleStartup,
    ProviderPlacement, build_hle_providers, plan_hle_startup, prepare_hle_startup,
};
use crate::cpu::ReferenceCpu;
use crate::memory::{GuestMemory, MemoryError, PAGE_SIZE, Permissions};
use crate::process::SessionLoadLimits;
use crate::process::tls::StaticTlsStorage;
use image::{CONSUMER_OFFSET, GOT, ORIGINAL_POINTER, PROVIDER_OFFSET, TLS_RVA, images};
use pentax_linker::ModuleInput;
use pentax_loader::open_executable;

const CONSUMER_BIAS: u64 = 0x10_0000;
const TLS_PROVIDER_BIAS: u64 = 0x20_0000;
const HLE_PROVIDER_BIAS: u64 = 0x30_0000;
const TLS_POINTERS: [u64; 2] = [0xa0_8000, 0xa1_4000];
const STACK: u64 = 0x90_0000;
const STACK_SENTINEL: u64 = 0x5045_4e54_4158_4854;
const INSTRUCTIONS: u64 = 128;

/// Completed evidence from the combined original consumer, never an executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleStartupFixtureReport {
    pub module_count: usize,
    pub tls_templates: usize,
    pub tls_instances: usize,
    pub tls_bindings: usize,
    pub provider_bindings: usize,
    pub applied_relocations: usize,
    pub provider_tls_runtime_id: u64,
    pub dtpoff: u64,
    pub tpoff: i64,
    pub relocated_template_word: u64,
    pub final_value: u64,
    pub independent_value: u64,
    pub service_attempts: u64,
    pub memcpy_calls: u64,
    pub memcmp_calls: u64,
    pub requested_bytes: u64,
    pub work_units: u64,
    pub retired: u64,
    pub mapped_bytes: u64,
    pub resident_bytes: u64,
}

fn require(condition: bool, reason: &'static str) -> Result<(), HleError> {
    if condition {
        Ok(())
    } else {
        Err(HleError::Fixture(reason))
    }
}
fn word(memory: &GuestMemory, address: u64) -> Result<u64, HleError> {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn plan<'a>(
    images: &'a image::Images,
    providers: &'a super::GeneratedProviders,
    limits: &HleStartupLimits,
) -> Result<HleStartupPlan<'a>, HleError> {
    let inputs = [
        ModuleInput::new(
            open_executable(&images.consumer).map_err(HleError::Load)?,
            CONSUMER_BIAS,
        ),
        ModuleInput::new(
            open_executable(&images.provider).map_err(HleError::Load)?,
            TLS_PROVIDER_BIAS,
        ),
    ];
    plan_hle_startup(
        &inputs,
        providers,
        ProviderPlacement {
            load_bias: HLE_PROVIDER_BIAS,
        },
        limits,
    )
}

fn memory_limits() -> SessionLoadLimits {
    SessionLoadLimits {
        resident_limit: 9 * PAGE_SIZE,
        mapped_limit: 9 * PAGE_SIZE,
    }
}

/// Keeps the actual prepared TLS owner, including all instance diagnostics,
/// throughout interpretation. No memory or metadata is detached into Gateway.
struct ClosedConsumer {
    prepared: PreparedHleStartup,
    cpu: ReferenceCpu,
    budget: ServiceBudget,
}

impl ClosedConsumer {
    fn initialize(mut prepared: PreparedHleStartup) -> Result<Self, HleError> {
        require(
            prepared.tls.templates().len() == 2 && prepared.tls.instances().len() == 2,
            "combined TLS shape",
        )?;
        for (index, (offset, memory)) in [(CONSUMER_OFFSET, 48), (PROVIDER_OFFSET, 24)]
            .into_iter()
            .enumerate()
        {
            let template = &prepared.tls.templates()[index];
            require(
                template.module.index() == index
                    && template.runtime_id.get() == index as u64 + 1
                    && template.tp_offset == offset
                    && template.mem_size == memory
                    && template.alignment_bias == 8,
                "combined retained TLS ownership",
            )?;
        }
        for (instance, &tp) in prepared.tls.instances().iter().zip(&TLS_POINTERS) {
            require(
                instance.thread_pointer == tp && instance.blocks.len() == 2,
                "combined TLS instances",
            )?;
            for (index, offset) in [CONSUMER_OFFSET, PROVIDER_OFFSET].into_iter().enumerate() {
                require(
                    instance.blocks[index].module.index() == index
                        && instance.blocks[index].address == tp - offset,
                    "combined TLS block addresses",
                )?;
            }
        }
        let memory = &mut prepared.tls.session.memory;
        for address in [STACK - PAGE_SIZE, STACK, STACK + PAGE_SIZE] {
            require(
                matches!(
                    memory.read(address, &mut [0]),
                    Err(MemoryError::Unmapped(_))
                ),
                "combined stack collision",
            )?;
        }
        memory.map(STACK, PAGE_SIZE, Permissions::READ_WRITE)?;
        memory.write(STACK + PAGE_SIZE - 8, &STACK_SENTINEL.to_le_bytes())?;
        let mut cpu = ReferenceCpu::at(prepared.primary_entry());
        cpu.registers[4] = STACK + PAGE_SIZE - 8;
        cpu.registers[5] = TLS_POINTERS[0];
        cpu.fs_base = TLS_POINTERS[0];
        let budget =
            ServiceBudget::new(prepared.service_limits()).with_provider(prepared.provider_bias());
        Ok(Self {
            prepared,
            cpu,
            budget,
        })
    }

    fn run(&mut self, limit: u64) -> Result<u64, HleError> {
        gateway::run(
            &mut self.prepared.tls.session.memory,
            &mut self.cpu,
            &mut self.budget,
            &self.prepared.sites,
            limit,
        )
    }
}

fn verify(
    consumer: &ClosedConsumer,
    original: &image::Images,
    halt: u64,
) -> Result<HleStartupFixtureReport, HleError> {
    let (prepared, cpu, budget) = (&consumer.prepared, &consumer.cpu, &consumer.budget);
    let memory = prepared.memory();
    require(
        halt == CONSUMER_BIAS + original.halt
            && cpu.retired == original.retired
            && cpu.registers[0] == 42,
        "combined guest completion",
    )?;
    require(
        cpu.fs_base == TLS_POINTERS[0]
            && cpu.registers[3] == 0
            && cpu.registers[5] == TLS_POINTERS[0]
            && cpu.registers[4] == STACK + PAGE_SIZE - 8
            && word(memory, cpu.registers[4])? == STACK_SENTINEL,
        "combined guest context",
    )?;
    require(
        prepared.module_count() == 3
            && prepared.provider_bindings() == 2
            && prepared.tls_bindings() == 3
            && prepared.applied_relocations() == 6,
        "combined relocation provenance",
    )?;
    for (slot, function) in [KnownFunction::Memcpy, KnownFunction::Memcmp]
        .into_iter()
        .enumerate()
    {
        let site = prepared
            .sites
            .iter()
            .find(|site| site.function == function)
            .unwrap();
        require(
            word(memory, CONSUMER_BIAS + GOT + 8 * slot as u64)? == site.entry,
            "combined attested GOT",
        )?;
    }
    let tpoff = 8_i64 - PROVIDER_OFFSET as i64;
    for (index, &tp) in TLS_POINTERS.iter().enumerate() {
        let block = tp - CONSUMER_OFFSET;
        require(
            word(memory, block)? == 2
                && word(memory, block + 8)? == 8
                && word(memory, block + 16)? as i64 == tpoff,
            "combined copied typed TLS writes",
        )?;
        require(
            word(memory, block + 24)? == if index == 0 { 42 } else { ORIGINAL_POINTER },
            "combined independent TLS",
        )?;
        require(
            word(memory, tp - PROVIDER_OFFSET + 8)? == 37,
            "combined provider TLS value",
        )?;
        for (offset, initialized, size) in [(CONSUMER_OFFSET, 32, 48), (PROVIDER_OFFSET, 16, 24)] {
            for address in tp - offset + initialized..tp - offset + size {
                require(word_byte(memory, address)? == 0, "combined TLS zero tails")?;
            }
        }
    }
    // Staged source remains relocated but separate from each mutable instance.
    require(
        word(memory, CONSUMER_BIAS + TLS_RVA + 24)? == ORIGINAL_POINTER,
        "combined template source unchanged",
    )?;
    for instance in prepared.tls.instances() {
        for guard in instance
            .guard_pages
            .ok_or(HleError::Fixture("combined TLS guards"))?
        {
            require(
                matches!(memory.read(guard, &mut [0]), Err(MemoryError::Unmapped(_))),
                "combined TLS guard mapped",
            )?;
        }
        let range = instance
            .storage
            .ok_or(HleError::Fixture("combined TLS storage"))?;
        require(
            matches!(
                memory.fetch(range.address, &mut [0]),
                Err(MemoryError::PermissionDenied(_))
            ),
            "combined TLS NX",
        )?;
    }
    for guard in [STACK - PAGE_SIZE, STACK + PAGE_SIZE] {
        require(
            matches!(memory.read(guard, &mut [0]), Err(MemoryError::Unmapped(_))),
            "combined stack guard mapped",
        )?;
    }
    for address in [STACK, CONSUMER_BIAS + GOT] {
        require(
            matches!(
                memory.fetch(address, &mut [0]),
                Err(MemoryError::PermissionDenied(_))
            ),
            "combined data NX",
        )?;
    }
    require(
        memory.preflight_write(CONSUMER_BIAS + GOT, 1).is_err(),
        "combined GOT RELRO",
    )?;
    let expected_work = work(KnownFunction::Memcpy, 8).unwrap()
        + work(KnownFunction::Memcmp, 8).unwrap()
        + 2 * SITE_LOOKUP_WORK;
    require(
        budget.attempts() == 2
            && budget.completed() == 2
            && budget.calls(KnownFunction::Memcpy) == 1
            && budget.calls(KnownFunction::Memcmp) == 1
            && budget.total_bytes() == 16
            && budget.work() == expected_work,
        "combined shared service accounting",
    )?;
    let fresh = images()?;
    require(
        fresh.consumer == original.consumer && fresh.provider == original.provider,
        "combined source bytes unchanged",
    )?;
    Ok(HleStartupFixtureReport {
        module_count: prepared.module_count(),
        tls_templates: prepared.tls.templates().len(),
        tls_instances: prepared.tls.instances().len(),
        tls_bindings: prepared.tls_bindings(),
        provider_bindings: prepared.provider_bindings(),
        applied_relocations: prepared.applied_relocations(),
        provider_tls_runtime_id: word(memory, TLS_POINTERS[0] - CONSUMER_OFFSET)?,
        dtpoff: word(memory, TLS_POINTERS[0] - CONSUMER_OFFSET + 8)?,
        tpoff: word(memory, TLS_POINTERS[0] - CONSUMER_OFFSET + 16)? as i64,
        relocated_template_word: word(memory, CONSUMER_BIAS + TLS_RVA + 24)?,
        final_value: word(memory, TLS_POINTERS[0] - CONSUMER_OFFSET + 24)?,
        independent_value: word(memory, TLS_POINTERS[1] - CONSUMER_OFFSET + 24)?,
        service_attempts: budget.attempts(),
        memcpy_calls: budget.calls(KnownFunction::Memcpy),
        memcmp_calls: budget.calls(KnownFunction::Memcmp),
        requested_bytes: budget.total_bytes(),
        work_units: budget.work(),
        retired: cpu.retired,
        mapped_bytes: memory.mapped_bytes(),
        resident_bytes: memory.resident_bytes(),
    })
}

fn word_byte(memory: &GuestMemory, address: u64) -> Result<u8, HleError> {
    let mut byte = [0];
    memory.read(address, &mut byte)?;
    Ok(byte[0])
}

/// Prepare and interpret only the fixed original combined consumer. All typed
/// bindings, template copies, HLE calls and postconditions are real operations;
/// failure publishes no report or mutable session. No external inputs exist.
///
/// # Errors
/// Returns the first planning, storage, attestation, CPU, budget or assertion error.
pub fn run_hle_startup_fixture() -> Result<HleStartupFixtureReport, HleError> {
    let limits = HleStartupLimits::default();
    let original = images()?;
    let providers = build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.hle)?;
    let plan = plan(&original, &providers, &limits)?;
    let prepared = prepare_hle_startup(
        &plan,
        &StaticTlsStorage {
            thread_pointers: &TLS_POINTERS,
        },
        &memory_limits(),
    )?;
    let mut consumer = ClosedConsumer::initialize(prepared)?;
    let halt = consumer.run(INSTRUCTIONS)?;
    verify(&consumer, &original, halt)
}
