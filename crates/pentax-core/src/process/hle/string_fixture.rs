// SPDX-License-Identifier: GPL-2.0-only
// Original PentaX integration fixture for the Kyty string adaptation, 2026-10-17.
// Adapted interfaces/source notices: process/hle_strings.rs.

//! Closed original consumer of all nine string services plus a memory service.

use super::HleError;
use super::catalog::KnownFunction;
use super::fixture::{
    Asm, CODE, CONSUMER_BIAS, Consumer, DATA, DATA_SIZE, FixtureLimits, GOT, Import, PROVIDER_BIAS,
    RESULTS, SHIM, STACK, STACK_SIZE, launch,
};
use super::provider::stub;
use crate::memory::{GuestMemory, MemoryError, PAGE_SIZE};

const FIRST: u64 = DATA + 0x100;
const SECOND: u64 = DATA + 0x140;
const NEEDLE: u64 = DATA + 0x180;
const OUTPUT: u64 = DATA + 0x200;
const OPERATIONS: [(KnownFunction, [u64; 3], u64); 11] = [
    (KnownFunction::Strlen, [FIRST, 0, u64::MAX], 6),
    (KnownFunction::Strcmp, [FIRST, SECOND, 0], u32::MAX as u64),
    (KnownFunction::Strncmp, [FIRST, SECOND, 6], 0),
    (KnownFunction::Strcpy, [OUTPUT, FIRST, 0], OUTPUT),
    (
        KnownFunction::Strncpy,
        [OUTPUT + 0x40, SECOND, 3],
        OUTPUT + 0x40,
    ),
    (KnownFunction::Strcat, [OUTPUT, NEEDLE, 0], OUTPUT),
    (KnownFunction::Strchr, [OUTPUT, b'a' as u64, 0], OUTPUT + 1),
    (KnownFunction::Strrchr, [OUTPUT, b'a' as u64, 0], OUTPUT + 8),
    (KnownFunction::Strstr, [OUTPUT, NEEDLE, 0], OUTPUT + 1),
    (KnownFunction::Strlen, [OUTPUT, 0, u64::MAX], 9),
    (
        KnownFunction::Memset,
        [OUTPUT + 0x60, b'Q' as u64, 4],
        OUTPUT + 0x60,
    ),
];

/// Completed numeric evidence from a fresh, no-input original consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleStringFixtureReport {
    pub registered_functions: usize,
    pub string_calls: u64,
    pub memory_calls: u64,
    pub service_attempts: u64,
    /// Memory-op RDX lengths plus string scan/snapshot/write byte charges.
    pub charged_bytes: u64,
    pub string_read_bytes: u64,
    pub string_write_bytes: u64,
    pub work_units: u64,
    pub string_scratch_peak_bytes: u64,
    pub retired: u64,
    pub provider_bindings: usize,
    pub applied_relocations: usize,
    pub provider_bias: u64,
    /// Guest-observed results in the fixed fixture's call order.
    pub results: [u64; OPERATIONS.len()],
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

fn equal(memory: &GuestMemory, address: u64, expected: &[u8]) -> Result<bool, HleError> {
    // All expected values are original fixture literals shorter than this bound.
    let mut bytes = [0; 16];
    let output = bytes
        .get_mut(..expected.len())
        .ok_or(HleError::Fixture("string assertion bound"))?;
    memory.read(address, output)?;
    Ok(output == expected)
}

/// Link, stage and interpret fresh original images through all nine strings.
///
/// Repeated strlen and a tail JMP are actual guest calls. Memset also executes
/// under the same retained budget. No image, pointer, plan, CPU or memory input
/// is accepted, and the private running session never escapes.
///
/// # Errors
/// Returns the first typed preparation/execution/assertion error and drops the
/// complete unpublished attempt. No partial success report is produced.
pub fn run_hle_string_fixture() -> Result<HleStringFixtureReport, HleError> {
    let mut main = Asm::new(CODE);
    for (index, &(function, arguments, _)) in OPERATIONS.iter().enumerate() {
        for (register, value) in [7, 6, 2].into_iter().zip(arguments) {
            main.mov(register, value);
        }
        if index == 9 {
            main.call(SHIM);
        } else {
            main.call_slot(function.index());
        }
        main.store_rax(RESULTS as u32 + index as u32 * 8);
    }
    main.int3();
    let halt = CONSUMER_BIAS + main.rva() as u64;
    let mut shim = Asm::new(SHIM);
    shim.jmp_slot(KnownFunction::Strlen.index());
    let retired = main.retired + shim.retired + OPERATIONS.len() as u64 * 3;
    let mut consumer = Consumer::importing(vec![(CODE, main.bytes), (SHIM, shim.bytes)]);
    consumer.imports = KnownFunction::LIBC.map(Import::function).to_vec();
    let image = consumer.build()?;
    let mut launched = launch(&image, &FixtureLimits::default())?;
    for (address, bytes) in [
        (FIRST, &b"banana\0"[..]),
        (SECOND, &b"bananas\0"[..]),
        (NEEDLE, &b"ana\0"[..]),
    ] {
        launched.gateway.memory.write(address, bytes)?;
    }
    launched.gateway.memory.write(OUTPUT, &[0xcc; 0x80])?;
    require(launched.gateway.run()? == halt, "string fixture halt")?;
    let gateway = &launched.gateway;
    require(
        gateway.cpu.retired == retired,
        "string fixture shared instruction count",
    )?;
    let memory = &gateway.memory;
    let mut results = [0; OPERATIONS.len()];
    for (index, &(_, _, expected)) in OPERATIONS.iter().enumerate() {
        results[index] = word(memory, DATA + RESULTS + index as u64 * 8)?;
        require(results[index] == expected, "guest-observed string result")?;
    }
    for (address, expected) in [
        (OUTPUT, &b"bananaana\0\xcc"[..]),
        (OUTPUT + 0x40, &b"ban\xcc"[..]),
        (OUTPUT + 0x60, &b"QQQQ\xcc"[..]),
        (FIRST, &b"banana\0"[..]),
        (SECOND, &b"bananas\0"[..]),
        (NEEDLE, &b"ana\0"[..]),
    ] {
        require(
            equal(memory, address, expected)?,
            "string data/source/canary",
        )?;
    }
    for register in [3, 4, 5, 12, 13, 14, 15] {
        require(
            gateway.cpu.registers[register] == launched.initial.registers[register],
            "string callee registers/stack",
        )?;
    }
    require(
        gateway.cpu.fs_base == launched.initial.fs_base,
        "string guest FS",
    )?;
    require(
        word(memory, gateway.cpu.registers[4])? == 0x5045_4e54_4158_5350,
        "string stack sentinel",
    )?;
    for site in gateway.sites() {
        let mut bytes = [0; 8];
        memory.fetch(site.entry, &mut bytes)?;
        require(bytes == stub(site.function), "string provider immutable")?;
    }
    for address in [DATA, STACK, CONSUMER_BIAS + GOT] {
        require(
            matches!(
                memory.fetch(address, &mut [0]),
                Err(MemoryError::PermissionDenied(_))
            ),
            "string data/stack/GOT NX",
        )?;
    }
    for address in [
        DATA - PAGE_SIZE,
        DATA + DATA_SIZE,
        STACK - PAGE_SIZE,
        STACK + STACK_SIZE,
    ] {
        require(
            matches!(
                memory.read(address, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ),
            "string storage guards",
        )?;
    }
    let budget = &gateway.budget;
    require(
        budget.attempts() == OPERATIONS.len() as u64
            && budget.completed() == OPERATIONS.len() as u64,
        "string completion count",
    )?;
    for function in KnownFunction::LIBC {
        require(
            budget.calls(function)
                == OPERATIONS
                    .iter()
                    .filter(|operation| operation.0 == function)
                    .count() as u64,
            "string per-service count",
        )?;
    }
    let memory_calls = KnownFunction::MEMORY
        .iter()
        .map(|&function| budget.calls(function))
        .sum::<u64>();
    require(consumer.build()? == image, "string source image immutable")?;
    Ok(HleStringFixtureReport {
        registered_functions: KnownFunction::ALL.len(),
        string_calls: budget.completed() - memory_calls,
        memory_calls,
        service_attempts: budget.attempts(),
        charged_bytes: budget.total_bytes(),
        string_read_bytes: budget.string_reads(),
        string_write_bytes: budget.string_writes(),
        work_units: budget.work(),
        string_scratch_peak_bytes: budget.string_scratch_peak(),
        retired: gateway.cpu.retired,
        provider_bindings: launched.bindings,
        applied_relocations: launched.applied,
        provider_bias: PROVIDER_BIAS,
        results,
        mapped_bytes: memory.mapped_bytes(),
        resident_bytes: memory.resident_bytes(),
    })
}
