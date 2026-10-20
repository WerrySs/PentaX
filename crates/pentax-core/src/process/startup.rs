// SPDX-License-Identifier: GPL-2.0-or-later

//! A closed original interpreted startup diagnostic, under PentaX fixture policy.
//!
//! The boot object, stack convention, constructor ownership, modeled FS base,
//! TLS block layout and synthetic service are fixture policy, not a Sony ABI,
//! a PS5 TCB/DTV implementation or a general executable-entry capability.

mod fixture;
#[cfg(test)]
mod tests;

use super::tls::{
    PreparedTlsSession, StaticTlsLimits, StaticTlsPlan, StaticTlsStorage, TlsError,
    TlsTemplateLayout, load_symbolic_with_static_tls, plan_static_tls,
};
use super::{ProcessError, SessionLoadLimits};
use crate::{
    cpu::{CpuError, ReferenceCpu, StopReason},
    memory::{ADDRESS_LIMIT, GuestMemory, MemoryError, PAGE_SIZE, Permissions},
};
use fixture::{Images, Layout};
use pentax_linker::{ModuleInput, SymbolicError, SymbolicLimits, SymbolicLinkPlan, plan_symbolic};
use pentax_loader::{LoadError, open_executable};
use std::fmt;

const MAX_INSTRUCTIONS: u64 = 256;
const SERVICE: u64 = 0x5058_0001; // Original synthetic namespace, not a PS5 syscall.
const STACK_SIZE: u64 = 64 * 1024;
const ARGUMENTS: [&[u8]; 2] = [b"pentax", b"--self-test"];
const ARG0_OFFSET: u64 = 0x100;
const ARG1_OFFSET: u64 = 0x120;

/// Completed evidence from the closed original interpreted fixture only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFixtureReport {
    pub value: u64,
    pub provider_value: u64,
    pub service_value: u64,
    pub retired: u64,
    pub constructor_calls: u64,
    pub service_calls: u64,
    pub argc: u32,
    pub argument_assertions: u32,
    pub provider_address: u64,
    pub tls_initialized: u64,
    pub independent_tls_initialized: u64,
    pub independent_tls_zero_tail: u64,
    pub applied_relocations: usize,
    pub mapped_bytes: u64,
    pub resident_bytes: u64,
}

#[derive(Debug)]
pub enum StartupFixtureError {
    Load(LoadError),
    Linking(SymbolicError),
    Staging(ProcessError),
    Tls(TlsError),
    Memory(MemoryError),
    Cpu(CpuError),
    InvalidFixture(&'static str),
    UnknownService(u64),
    InstructionBudget { limit: u64, retired: u64 },
}

impl fmt::Display for StartupFixtureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Load(error) => write!(f, "original startup image: {error}"),
            Self::Linking(error) => write!(f, "original startup linking: {error}"),
            Self::Staging(error) => write!(f, "original startup staging: {error}"),
            Self::Tls(error) => write!(f, "original startup static TLS: {error}"),
            Self::Memory(error) => write!(f, "original startup memory: {error}"),
            Self::Cpu(error) => write!(f, "original startup interpreter: {error}"),
            Self::InvalidFixture(reason) => {
                write!(f, "original startup assertion failed: {reason}")
            }
            Self::UnknownService(number) => write!(f, "unknown PentaX fixture service {number:#x}"),
            Self::InstructionBudget { limit, retired } => {
                write!(
                    f,
                    "startup instruction budget {limit} exhausted after {retired} instructions"
                )
            }
        }
    }
}
impl std::error::Error for StartupFixtureError {}
impl From<MemoryError> for StartupFixtureError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}
impl From<TlsError> for StartupFixtureError {
    fn from(error: TlsError) -> Self {
        Self::Tls(error)
    }
}

type Result<T> = std::result::Result<T, StartupFixtureError>;

#[derive(Clone)]
struct Limits {
    instructions: u64,
    memory: SessionLoadLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            instructions: MAX_INSTRUCTIONS,
            memory: SessionLoadLimits {
                resident_limit: 10 * PAGE_SIZE,
                mapped_limit: 12 * PAGE_SIZE,
            },
        }
    }
}

/// Construct, link, stage and interpret two fresh original fixture modules.
///
/// This has no byte, path, plan or guest-entry input. It returns only completed
/// diagnostics; no mutable memory, CPU, native function or executable session
/// escapes. Each failure drops the entire local unpublished address space.
/// The modeled FS base is a guest integer; host FS/GS is never accessed.
pub fn run_startup_fixture() -> Result<StartupFixtureReport> {
    run_original(
        &fixture::images(Layout::default())?,
        Layout::default(),
        &Limits::default(),
    )
}

fn require(condition: bool, reason: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(StartupFixtureError::InvalidFixture(reason))
    }
}

fn checked_end(address: u64, size: u64) -> Result<u64> {
    address
        .checked_add(size)
        .filter(|end| *end <= ADDRESS_LIMIT)
        .ok_or(StartupFixtureError::InvalidFixture(
            "storage address overflow",
        ))
}

fn word(memory: &GuestMemory, address: u64) -> Result<u64> {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn argument_blob(layout: Layout) -> Result<Vec<u8>> {
    let mut bytes = vec![0; 0x200];
    bytes[..4].copy_from_slice(&2_u32.to_le_bytes());
    for ((argument, offset), slot) in ARGUMENTS
        .into_iter()
        .zip([ARG0_OFFSET, ARG1_OFFSET])
        .zip([8, 16])
    {
        require(
            !argument.contains(&0) && argument.len() <= 512,
            "argument policy",
        )?;
        let pointer =
            layout
                .boot
                .checked_add(offset)
                .ok_or(StartupFixtureError::InvalidFixture(
                    "argument pointer overflow",
                ))?;
        bytes[slot..slot + 8].copy_from_slice(&pointer.to_le_bytes());
        let start = offset as usize;
        bytes[start..start + argument.len()].copy_from_slice(argument);
        // The following zero byte and argv[2] are explicit fixture terminators.
    }
    Ok(bytes)
}

fn check_arguments(memory: &GuestMemory, layout: Layout) -> Result<()> {
    let expected = argument_blob(layout)?;
    let mut actual = vec![0; expected.len()];
    memory.read(layout.boot, &mut actual)?;
    require(actual == expected, "argc, argv, padding or NUL terminators")
}

fn guards(layout: Layout) -> Result<[u64; 6]> {
    let before = |address: u64| {
        address
            .checked_sub(PAGE_SIZE)
            .ok_or(StartupFixtureError::InvalidFixture(
                "guard address underflow",
            ))
    };
    Ok([
        before(layout.stack)?,
        checked_end(layout.stack, STACK_SIZE)?,
        before(layout.tls_pages[0])?,
        checked_end(layout.tls_pages[0], PAGE_SIZE)?,
        before(layout.tls_pages[1])?,
        checked_end(layout.tls_pages[1], PAGE_SIZE)?,
    ])
}

fn check_guards(memory: &GuestMemory, layout: Layout) -> Result<()> {
    for address in guards(layout)? {
        require(
            matches!(
                memory.read(address, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ),
            "guard page is mapped",
        )?;
    }
    Ok(())
}

fn link_original<'a>(consumer: &'a [u8], provider: &'a [u8]) -> Result<SymbolicLinkPlan<'a>> {
    let views = [
        open_executable(consumer).map_err(StartupFixtureError::Load)?,
        open_executable(provider).map_err(StartupFixtureError::Load)?,
    ];
    let inputs = [
        ModuleInput::new(views[0].clone(), fixture::CONSUMER_BIAS),
        ModuleInput::new(views[1].clone(), fixture::PROVIDER_BIAS),
    ];
    plan_symbolic(&inputs, &SymbolicLimits::default()).map_err(StartupFixtureError::Linking)
}

fn fixture_template(plan: &StaticTlsPlan<'_, '_>) -> Result<TlsTemplateLayout> {
    require(
        plan.templates().len() == 1,
        "one original provider TLS template",
    )?;
    let template = plan.templates()[0];
    require(
        template.module.index() == 1
            && template.runtime_id.get() == 1
            && template.program_header_index == fixture::TLS_HEADER
            && template.source_vaddr == 0x908
            && template.load_bias == fixture::PROVIDER_BIAS
            && template.initialized_size == 8
            && template.mem_size == 16
            && template.alignment == 32
            && template.alignment_bias == 8,
        "unsupported fixture TLS template",
    )?;
    require(
        (16..=128).contains(&template.tp_offset),
        "encodable fixture TLS offset",
    )?;
    Ok(template)
}

fn thread_pointers(layout: Layout) -> Result<[u64; 2]> {
    Ok([
        checked_end(layout.tls_pages[0], PAGE_SIZE)?,
        checked_end(layout.tls_pages[1], PAGE_SIZE)?,
    ])
}

fn fixture_blocks(prepared: &PreparedTlsSession, layout: Layout) -> Result<[u64; 2]> {
    require(
        prepared.instances().len() == 2,
        "two original TLS instances",
    )?;
    let mut addresses = [0; 2];
    for ((address, instance), tp) in addresses
        .iter_mut()
        .zip(prepared.instances())
        .zip(thread_pointers(layout)?)
    {
        require(
            instance.thread_pointer == tp && instance.blocks.len() == 1,
            "planned fixture thread pointer",
        )?;
        let block = instance.blocks[0];
        require(
            block.module.index() == 1 && block.runtime_id.get() == 1 && block.size == 16,
            "planned fixture block identity",
        )?;
        require(
            block.address % 32 == 8 && checked_end(block.address, 16)? <= tp,
            "planned fixture block congruence",
        )?;
        require(
            word(prepared.memory(), block.address)? == 37
                && word(prepared.memory(), block.address + 8)? == 0,
            "TLS initialized value or zero tail",
        )?;
        // This profile supplies no TCB/DTV or FS:0 bytes: TP is the upper guard.
        require(
            matches!(
                prepared.memory().read(tp, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ),
            "thread pointer boundary must remain unmapped",
        )?;
        *address = block.address;
    }
    require(
        addresses[0] != addresses[1],
        "independent TLS block addresses",
    )?;
    Ok(addresses)
}

fn initialize_storage(memory: &mut GuestMemory, layout: Layout) -> Result<()> {
    // TLS blocks and guards already belong to the shared materializer. The
    // closed fixture owns only this boot object and stack in addition to them.
    let regions = [(layout.stack, STACK_SIZE), (layout.boot, PAGE_SIZE)];
    for (index, &(start, size)) in regions.iter().enumerate() {
        require(start.is_multiple_of(PAGE_SIZE), "storage page alignment")?;
        let end = checked_end(start, size)?;
        for &(other, other_size) in &regions[..index] {
            require(
                end <= other || checked_end(other, other_size)? <= start,
                "storage ownership overlap",
            )?;
        }
    }
    check_guards(memory, layout)?;
    // Map all owned ranges before initializing any new storage. map rejects
    // image-page collisions; any subsequent error drops the whole session.
    for (start, size) in regions {
        memory.map(start, size, Permissions::READ_WRITE)?;
    }
    check_guards(memory, layout)?;
    memory.write(layout.boot, &argument_blob(layout)?)?;
    let initial_rsp = checked_end(layout.stack, STACK_SIZE)? - 8;
    memory.write(initial_rsp, &0_u64.to_le_bytes())?;
    check_arguments(memory, layout)?;
    Ok(())
}

fn service(
    cpu: &mut ReferenceCpu,
    expected_rip: u64,
    tls: u64,
    rsp: u64,
    count: &mut u64,
) -> Result<u64> {
    if cpu.registers[0] != SERVICE {
        return Err(StartupFixtureError::UnknownService(cpu.registers[0]));
    }
    require(
        cpu.rip == expected_rip && cpu.registers[1] == expected_rip && *count == 0,
        "service site or repeated service",
    )?;
    require(
        cpu.registers[7] == 19
            && cpu.registers[6] == 23
            && cpu.registers[4] == rsp
            && cpu.fs_base == tls,
        "service arguments or context",
    )?;
    let value = cpu.registers[7].checked_add(cpu.registers[6]).ok_or(
        StartupFixtureError::InvalidFixture("synthetic sum overflow"),
    )?;
    cpu.registers[0] = value;
    *count += 1;
    Ok(value)
}

// Private executor accepts only this module's privately generated originals;
// tests can perturb those originals to prove rejection. No input-facing API
// accepts bytes, detached patches, an entry address or a replacement session.
fn run_original(images: &Images, layout: Layout, limits: &Limits) -> Result<StartupFixtureReport> {
    require(
        limits.instructions <= MAX_INSTRUCTIONS,
        "instruction policy maximum",
    )?;
    let plan = link_original(&images.consumer, &images.provider)?;
    require(
        plan.patch_count() == 1 && plan.deferred_count() == 0 && plan.modules().len() == 2,
        "complete two-module fixture relocations",
    )?;
    let tls_plan = plan_static_tls(&plan, &StaticTlsLimits::default())?;
    let template = fixture_template(&tls_plan)?;
    require(
        template.tp_offset == images.code.tls_offset,
        "final image TLS displacement ownership",
    )?;
    let tps = thread_pointers(layout)?;
    let prepared = load_symbolic_with_static_tls(
        &tls_plan,
        &StaticTlsStorage {
            thread_pointers: &tps,
        },
        &limits.memory,
    )?;
    let tls = fixture_blocks(&prepared, layout)?;
    let mut session = prepared.session;
    let provider_address = fixture::PROVIDER_BIAS + fixture::PROVIDER_ENTRY;
    require(
        word(&session.memory, fixture::CONSUMER_BIAS + fixture::GOT)? == provider_address,
        "relocated provider pointer",
    )?;
    initialize_storage(&mut session.memory, layout)?;
    let initial_rsp = checked_end(layout.stack, STACK_SIZE)? - 8;
    let mut cpu = ReferenceCpu::at(session.primary_entry);
    cpu.registers[4] = initial_rsp;
    cpu.registers[7] = layout.boot;
    cpu.fs_base = tps[0];
    let mut services = 0;
    let mut provider_value = None;
    let mut service_value = None;
    loop {
        if cpu.retired >= limits.instructions {
            return Err(StartupFixtureError::InstructionBudget {
                limit: limits.instructions,
                retired: cpu.retired,
            });
        }
        if cpu.rip == fixture::CONSUMER_BIAS + images.code.after_provider {
            require(provider_value.is_none(), "provider returned more than once")?;
            provider_value = Some(cpu.registers[0]);
        }
        match cpu
            .step(&mut session.memory)
            .map_err(StartupFixtureError::Cpu)?
        {
            None => {}
            Some(StopReason::Syscall) => {
                service_value = Some(service(
                    &mut cpu,
                    fixture::CONSUMER_BIAS + images.code.service_resume,
                    tps[0],
                    initial_rsp,
                    &mut services,
                )?);
            }
            Some(StopReason::Breakpoint) => {
                require(
                    cpu.rip == fixture::CONSUMER_BIAS + images.code.halt_resume,
                    "guest argument, constructor or result assertion",
                )?;
                break;
            }
        }
    }
    let constructor_calls = word(&session.memory, tls[0] + 8)?;
    let independent_tls_initialized = word(&session.memory, tls[1])?;
    let independent_tls_zero_tail = word(&session.memory, tls[1] + 8)?;
    require(
        cpu.registers[0] == 42
            && provider_value == Some(42)
            && service_value == Some(42)
            && services == 1
            && constructor_calls == 1,
        "final value or exactly-once counters",
    )?;
    require(
        word(&session.memory, tls[0])? == 37
            && independent_tls_initialized == 37
            && independent_tls_zero_tail == 0,
        "independent initialized and zero-filled TLS",
    )?;
    require(
        cpu.registers[4] == initial_rsp
            && word(&session.memory, initial_rsp - 8)?
                == fixture::CONSUMER_BIAS + images.code.after_provider
            && word(&session.memory, initial_rsp)? == 0,
        "owned CALL/RET stack",
    )?;
    require(
        word(&session.memory, fixture::CONSUMER_BIAS + fixture::GOT)? == provider_address,
        "provider pointer changed",
    )?;
    check_arguments(&session.memory, layout)?;
    check_guards(&session.memory, layout)?;
    // No executable permission is installed on any initialized storage.
    for address in [layout.stack, layout.boot, tls[0], tls[1]] {
        require(
            matches!(
                session.memory.fetch(address, &mut [0]),
                Err(MemoryError::PermissionDenied(_))
            ),
            "startup storage must be NX",
        )?;
    }
    Ok(StartupFixtureReport {
        value: cpu.registers[0],
        provider_value: provider_value.expect("checked result"),
        service_value: service_value.expect("checked result"),
        retired: cpu.retired,
        constructor_calls,
        service_calls: services,
        argc: 2,
        argument_assertions: 7,
        provider_address,
        tls_initialized: 37,
        independent_tls_initialized,
        independent_tls_zero_tail,
        applied_relocations: session.applied_relocations,
        mapped_bytes: session.memory.mapped_bytes(),
        resident_bytes: session.memory.resident_bytes(),
    })
}
