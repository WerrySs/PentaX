// SPDX-License-Identifier: GPL-2.0-or-later

//! The closed original HLE memory diagnostic.
//!
//! A freshly generated consumer imports the four catalogue functions through
//! JUMP_SLOT GOT entries, calls them with `CALL [RIP+GOT]`, tail-jumps once
//! through `JMP [RIP+GOT]` from an original shim and stores each RAX result.
//! The fixture owns its RW/NX data and stack with unmapped guards, a shared
//! retired-instruction budget and a host reference model of the final bytes.
//! Register setup, storage layout and result slots are fixture policy, not a
//! PS5 ABI. No input, path, game byte or proprietary code is involved.

use super::catalog::KnownFunction;
use super::dispatch::{SITE_LOOKUP_WORK, work};
use super::gateway::Gateway;
use super::provider::{program as header, put, stub};
use super::{
    HleError, HleLimits, HleProfile, ProviderPlacement, build_hle_providers, plan_hle_session,
    prepare_hle_session,
};
use crate::cpu::ReferenceCpu;
use crate::memory::{GuestMemory, MemoryError, PAGE_SIZE, Permissions};
use crate::process::SessionLoadLimits;
use pentax_linker::{DescriptorEncoding, DescriptorRole, ModuleInput};
use pentax_loader::open_executable;

pub(super) const CONSUMER_BIAS: u64 = 0x10_0000;
pub(super) const PROVIDER_BIAS: u64 = 0x20_0000;
pub(super) const DATA: u64 = 0x80_0000;
pub(super) const DATA_SIZE: u64 = 2 * PAGE_SIZE;
pub(super) const STACK: u64 = 0x90_0000;
pub(super) const STACK_SIZE: u64 = 64 * 1024;
/// Consumer RVAs: the shim and main code share the RX page with the tables.
pub(super) const SHIM: usize = 0x180;
pub(super) const CODE: usize = 0x200;
pub(super) const GOT: u64 = 0x4000;
/// Data offset of the eight-byte result slots written by the guest.
pub(super) const RESULTS: u64 = 0x800;
const MAX_INSTRUCTIONS: u64 = 512;

const TEXT_END: usize = 0x800;
const DYNAMIC_AT: usize = 0x800;
const TABLES_AT: usize = 0x900;
const STRINGS_AT: usize = 0x900;
const SYMBOLS_AT: usize = 0xa00;
const JMPREL_AT: usize = 0xc00;
const RELA_AT: usize = 0xe00;
const TABLES_END: usize = 0x1000;
const FILE_SIZE: usize = 0x4100;
const MAX_IMPORTS: usize = KnownFunction::ALL.len();
/// Consumer-local import selectors: library 3 (`D`), module 2 (`C`).
const IMPORT_SUFFIX: &[u8] = b"#D#C";
const IMPORT_MODULE_SELECTOR: u64 = 2;
const IMPORT_LIBRARY_SELECTOR: u64 = 3;

const PATTERN_LEN: usize = 64;
const COMPARE_AT: u64 = 0x600;
const SENTINELS: [(usize, u64); 5] = [
    (5, 0x5045_4e54_4158_0005),
    (12, 0x5045_4e54_4158_000c),
    (13, 0x5045_4e54_4158_000d),
    (14, 0x5045_4e54_4158_000e),
    (15, 0x5045_4e54_4158_000f),
];
const STACK_SENTINEL: u64 = 0x5045_4e54_4158_5350;
const FS_SENTINEL: u64 = 0x7000_0000;

/// Completed evidence from the closed original HLE fixture only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HleFixtureReport {
    pub memcpy_calls: u64,
    pub memmove_calls: u64,
    pub memset_calls: u64,
    pub memcmp_calls: u64,
    pub service_attempts: u64,
    pub requested_bytes: u64,
    pub work_units: u64,
    pub retired: u64,
    pub provider_bindings: usize,
    pub applied_relocations: usize,
    pub provider_bias: u64,
    /// Low signed 32-bit memcmp results, in guest call order.
    pub memcmp_results: [i32; 3],
    pub mapped_bytes: u64,
    pub resident_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Import {
    pub nid: [u8; 11],
    /// ELF symbol type: 2 FUNC, 1 OBJECT.
    pub symbol_type: u8,
    /// ELF binding: 1 GLOBAL, 2 WEAK.
    pub binding: u8,
    /// Relocation kind writing this import's GOT slot.
    pub relocation: u32,
    pub addend: i64,
}

impl Import {
    pub(super) const fn function(function: KnownFunction) -> Self {
        Self {
            nid: *function.nid(),
            symbol_type: 2,
            binding: 1,
            relocation: 7,
            addend: 0,
        }
    }
}

/// Original consumer image description; slot `i` is at `GOT + 8 * i`.
#[derive(Debug, Clone)]
pub(super) struct Consumer {
    pub dynlib: bool,
    pub encoding: DescriptorEncoding,
    pub module_name: &'static [u8],
    pub library_name: &'static [u8],
    pub major: u8,
    pub minor: u8,
    pub version: u16,
    pub imports: Vec<Import>,
    pub text: Vec<(usize, Vec<u8>)>,
}

impl Consumer {
    /// The original four memory imports in `KnownFunction::MEMORY` slot order.
    pub(super) fn importing(text: Vec<(usize, Vec<u8>)>) -> Self {
        Self {
            dynlib: false,
            encoding: DescriptorEncoding::Variant1Observed,
            module_name: b"LibcInternal",
            library_name: b"LibcInternal",
            major: 1,
            minor: 1,
            version: 1,
            imports: KnownFunction::MEMORY.map(Import::function).to_vec(),
            text,
        }
    }

    pub(super) fn build(&self) -> Result<Vec<u8>, HleError> {
        let layout = |valid: bool| {
            if valid {
                Ok(())
            } else {
                Err(HleError::Fixture("original consumer layout"))
            }
        };
        layout(self.imports.len() <= MAX_IMPORTS)?;
        let mut bytes = vec![0; FILE_SIZE];
        let (kind, os_abi, abi_version) = if self.dynlib {
            (0xfe10_u16, 9, 2)
        } else {
            (3, 0, 0)
        };
        bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, os_abi, abi_version]);
        put(&mut bytes, 16, &kind.to_le_bytes());
        put(&mut bytes, 18, &62_u16.to_le_bytes());
        put(&mut bytes, 20, &1_u32.to_le_bytes());
        put(&mut bytes, 24, &(CODE as u64).to_le_bytes());
        put(&mut bytes, 32, &64_u64.to_le_bytes());
        put(&mut bytes, 52, &64_u16.to_le_bytes());
        put(&mut bytes, 54, &56_u16.to_le_bytes());
        put(
            &mut bytes,
            56,
            &(if self.dynlib { 5_u16 } else { 4 }).to_le_bytes(),
        );
        header(&mut bytes, 0, 1, 5, [0, 0, 0x1000, 0x4000, 0x4000]);
        header(&mut bytes, 1, 1, 6, [0x4000, GOT, 0x100, 0x4000, 0x4000]);
        // Complete-page RELRO: the relocated GOT becomes read-only and NX.
        header(&mut bytes, 3, 0x6474_e552, 4, [0, GOT, 0, 0x4000, 0x4000]);
        if self.dynlib {
            let size = (TABLES_END - TABLES_AT) as u64;
            header(
                &mut bytes,
                4,
                0x6100_0000,
                4,
                [TABLES_AT as u64, 0, size, 0, 0],
            );
        }
        for (at, code) in &self.text {
            layout(
                *at >= SHIM
                    && at
                        .checked_add(code.len())
                        .is_some_and(|end| end <= TEXT_END),
            )?;
            put(&mut bytes, *at, code);
        }

        let mut strings = vec![0];
        let module_name = string(&mut strings, &[self.module_name]);
        let library_name = string(&mut strings, &[self.library_name]);
        let mut symbols = vec![0; 24];
        let (mut rela, mut plt) = (Vec::new(), Vec::new());
        for (index, import) in self.imports.iter().enumerate() {
            let name = string(&mut strings, &[&import.nid, IMPORT_SUFFIX]);
            symbols.extend_from_slice(&name.to_le_bytes());
            symbols.push((import.binding << 4) | import.symbol_type);
            symbols.extend_from_slice(&[0; 19]); // DEFAULT, undefined, zero value/size.
            let table = if import.relocation == 7 {
                &mut plt
            } else {
                &mut rela
            };
            let symbol = index as u64 + 1;
            table.extend_from_slice(&(GOT + 8 * index as u64).to_le_bytes());
            table.extend_from_slice(&((symbol << 32) | u64::from(import.relocation)).to_le_bytes());
            table.extend_from_slice(&import.addend.to_le_bytes());
        }
        layout(strings.len() <= SYMBOLS_AT - STRINGS_AT)?;
        put(&mut bytes, STRINGS_AT, &strings);
        put(&mut bytes, SYMBOLS_AT, &symbols);
        put(&mut bytes, JMPREL_AT, &plt);
        put(&mut bytes, RELA_AT, &rela);

        let address = |at: usize| {
            if self.dynlib {
                (at - TABLES_AT) as u64
            } else {
                at as u64
            }
        };
        let tags: [i64; 11] = if self.dynlib {
            [
                0x6100_0035,
                0x6100_0037,
                0x6100_0039,
                0x6100_003f,
                0x6100_003b,
                0x6100_002f,
                0x6100_0031,
                0x6100_0033,
                0x6100_0029,
                0x6100_002d,
                0x6100_002b,
            ]
        } else {
            [5, 10, 6, 39, 11, 7, 8, 9, 23, 2, 20]
        };
        let mut entries = vec![
            (tags[0], address(STRINGS_AT)),
            (tags[1], strings.len() as u64),
            (tags[2], address(SYMBOLS_AT)),
            (tags[3], symbols.len() as u64),
            (tags[4], 24),
        ];
        if !rela.is_empty() {
            entries.extend([
                (tags[5], address(RELA_AT)),
                (tags[6], rela.len() as u64),
                (tags[7], 24),
            ]);
        }
        if !plt.is_empty() {
            entries.extend([
                (tags[8], address(JMPREL_AT)),
                (tags[9], plt.len() as u64),
                (tags[10], 7),
            ]);
        }
        entries.extend([
            (
                DescriptorRole::NeededModule.tag(self.encoding),
                u64::from(module_name)
                    | (u64::from(self.minor) << 32)
                    | (u64::from(self.major) << 40)
                    | (IMPORT_MODULE_SELECTOR << 48),
            ),
            (
                DescriptorRole::ImportLibrary.tag(self.encoding),
                u64::from(library_name)
                    | (u64::from(self.version) << 32)
                    | (IMPORT_LIBRARY_SELECTOR << 48),
            ),
            (0, 0),
        ]);
        for (index, (tag, value)) in entries.iter().enumerate() {
            put(&mut bytes, DYNAMIC_AT + index * 16, &tag.to_le_bytes());
            put(
                &mut bytes,
                DYNAMIC_AT + index * 16 + 8,
                &value.to_le_bytes(),
            );
        }
        let dynamic_size = entries.len() as u64 * 16;
        header(
            &mut bytes,
            2,
            2,
            4,
            [
                DYNAMIC_AT as u64,
                DYNAMIC_AT as u64,
                dynamic_size,
                dynamic_size,
                8,
            ],
        );
        Ok(bytes)
    }
}

fn string(strings: &mut Vec<u8>, parts: &[&[u8]]) -> u32 {
    let offset = strings.len() as u32;
    for part in parts {
        strings.extend_from_slice(part);
    }
    strings.push(0);
    offset
}

/// A minimal original encoder for the interpreter's supported forms.
/// `retired` counts emitted instructions for straight-line accounting.
pub(super) struct Asm {
    pub bytes: Vec<u8>,
    pub retired: u64,
    base: usize,
}

impl Asm {
    pub(super) const fn new(base: usize) -> Self {
        Self {
            bytes: Vec::new(),
            retired: 0,
            base,
        }
    }
    pub(super) fn rva(&self) -> usize {
        self.base + self.bytes.len()
    }
    fn emit(&mut self, parts: &[&[u8]]) {
        for part in parts {
            self.bytes.extend_from_slice(part);
        }
        self.retired += 1;
    }
    /// MOV r32, imm32 for low registers 0..=7; zero-extends.
    pub(super) fn mov32(&mut self, register: u8, value: u32) {
        self.emit(&[&[0xb8 + register], &value.to_le_bytes()]);
    }
    /// The shortest supported MOV for `value` into a low register.
    pub(super) fn mov(&mut self, register: u8, value: u64) {
        match u32::try_from(value) {
            Ok(value) => self.mov32(register, value),
            Err(_) => self.emit(&[&[0x48, 0xb8 + register], &value.to_le_bytes()]),
        }
    }
    fn indirect(&mut self, modrm: u8, slot: usize) {
        let next = self.rva() as i64 + 6;
        let displacement = (GOT as i64 + 8 * slot as i64 - next) as i32;
        self.emit(&[&[0xff, modrm], &displacement.to_le_bytes()]);
    }
    /// CALL qword [RIP + GOT slot].
    pub(super) fn call_slot(&mut self, slot: usize) {
        self.indirect(0x15, slot);
    }
    /// JMP qword [RIP + GOT slot].
    pub(super) fn jmp_slot(&mut self, slot: usize) {
        self.indirect(0x25, slot);
    }
    /// CALL rel32 to a consumer RVA.
    pub(super) fn call(&mut self, target: usize) {
        let next = self.rva() as i64 + 5;
        let displacement = (target as i64 - next) as i32;
        self.emit(&[&[0xe8], &displacement.to_le_bytes()]);
    }
    /// MOV qword [RBX + disp32], RAX.
    pub(super) fn store_rax(&mut self, displacement: u32) {
        self.emit(&[&[0x48, 0x89, 0x83], &displacement.to_le_bytes()]);
    }
    pub(super) fn int3(&mut self) {
        self.emit(&[&[0xcc]]);
    }
    #[cfg(test)]
    pub(super) fn raw(&mut self, bytes: &[u8]) {
        self.emit(&[bytes]);
    }
}

/// One guest call. A `tail` call goes through the shared shim's tail JMP.
#[derive(Debug, Clone, Copy)]
pub(super) struct Op {
    pub function: KnownFunction,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub tail: bool,
}

const fn op(function: KnownFunction, rdi: u64, rsi: u64, rdx: u64) -> Op {
    Op {
        function,
        rdi,
        rsi,
        rdx,
        tail: false,
    }
}

pub(super) const OPS: [Op; 11] = [
    // High RSI bits are ignored: only the int's low byte fills.
    op(
        KnownFunction::Memset,
        DATA + 0x100,
        0xffff_ffff_ffff_ff5a,
        32,
    ),
    op(KnownFunction::Memcpy, DATA + 0x200, DATA, 64),
    // Overlap with destination above, then below, the source.
    op(KnownFunction::Memmove, DATA + 0x210, DATA + 0x200, 32),
    op(KnownFunction::Memmove, DATA + 0x200, DATA + 0x208, 24),
    op(KnownFunction::Memcpy, DATA + 0x500, DATA, 16),
    op(KnownFunction::Memcmp, DATA, DATA + 0x500, 16),
    // First difference 0x16 versus 0x90: unsigned less, signed greater.
    op(KnownFunction::Memcmp, DATA, DATA + COMPARE_AT, 8),
    op(KnownFunction::Memcmp, DATA + COMPARE_AT, DATA, 8),
    // Crosses into the second, still lazy, data page.
    op(KnownFunction::Memcpy, DATA + PAGE_SIZE - 8, DATA, 16),
    Op {
        tail: true,
        ..op(
            KnownFunction::Memset,
            DATA + 0x700,
            0x0123_4567_89ab_cdc3,
            8,
        )
    },
    // Zero length accepts null pointers without dereference.
    op(KnownFunction::Memcpy, 0, 0, 0),
];

pub(super) struct Program {
    pub main: Vec<u8>,
    pub shim: Vec<u8>,
    /// Exact straight-line retired count, including stubs and the final INT3.
    pub retired: u64,
    /// Consumer RVA after the final INT3.
    pub halt: u64,
}

impl Program {
    pub(super) fn consumer(&self) -> Consumer {
        Consumer::importing(vec![(SHIM, self.shim.clone()), (CODE, self.main.clone())])
    }
}

/// Encode `ops`; slot `i` belongs to `KnownFunction::ALL[i]`.
pub(super) fn program(ops: &[Op]) -> Result<Program, HleError> {
    let mut main = Asm::new(CODE);
    let mut shim = Asm::new(SHIM);
    let mut tail_used = false;
    let mut stub_instructions = 0;
    for (index, op) in ops.iter().enumerate() {
        main.mov(7, op.rdi);
        main.mov(6, op.rsi);
        let rdx = u32::try_from(op.rdx).map_err(|_| HleError::Fixture("fixture length"))?;
        if op.tail {
            if tail_used {
                return Err(HleError::Fixture("one shared tail-call shim"));
            }
            tail_used = true;
            shim.mov32(2, rdx);
            shim.jmp_slot(op.function.index());
            main.call(SHIM);
        } else {
            main.mov32(2, rdx);
            main.call_slot(op.function.index());
        }
        // MOV EAX, imm32; SYSCALL; RET inside the generated stub.
        stub_instructions += 3;
        let slot = u32::try_from(RESULTS + 8 * index as u64)
            .map_err(|_| HleError::Fixture("fixture result slot"))?;
        main.store_rax(slot);
    }
    main.int3();
    if main.rva() > TEXT_END || shim.rva() > CODE {
        return Err(HleError::Fixture("fixture code size"));
    }
    Ok(Program {
        retired: main.retired + shim.retired + stub_instructions,
        halt: main.rva() as u64,
        main: main.bytes,
        shim: shim.bytes,
    })
}

/// Data bytes written by the host before the guest starts.
fn initial_data() -> (Vec<u8>, [u8; 8]) {
    let pattern = (0..PATTERN_LEN)
        .map(|index| (index as u8).wrapping_mul(7).wrapping_add(1))
        .collect::<Vec<_>>();
    let mut compare = [0; 8];
    compare.copy_from_slice(&pattern[..8]);
    compare[3] = 0x90;
    compare[5] = 0;
    (pattern, compare)
}

/// Independent host reference model of the complete final data region.
pub(super) fn expected_data(ops: &[Op]) -> Vec<u8> {
    let mut data = vec![0; DATA_SIZE as usize];
    let (pattern, compare) = initial_data();
    data[..PATTERN_LEN].copy_from_slice(&pattern);
    data[COMPARE_AT as usize..COMPARE_AT as usize + 8].copy_from_slice(&compare);
    for (index, op) in ops.iter().enumerate() {
        let length = op.rdx as usize;
        let offset = |address: u64| (address - DATA) as usize;
        let rax = if length == 0 {
            if op.function == KnownFunction::Memcmp {
                0
            } else {
                op.rdi
            }
        } else {
            let destination = offset(op.rdi);
            match op.function {
                KnownFunction::Memcpy | KnownFunction::Memmove => {
                    let source = offset(op.rsi);
                    data.copy_within(source..source + length, destination);
                    op.rdi
                }
                KnownFunction::Memset => {
                    data[destination..destination + length].fill(op.rsi as u8);
                    op.rdi
                }
                KnownFunction::Memcmp => {
                    let source = offset(op.rsi);
                    let ordering =
                        data[destination..destination + length].cmp(&data[source..source + length]);
                    u64::from(ordering as i8 as i32 as u32)
                }
                _ => unreachable!("the closed memory model accepts memory operations only"),
            }
        };
        let slot = (RESULTS as usize) + 8 * index;
        data[slot..slot + 8].copy_from_slice(&rax.to_le_bytes());
    }
    data
}

pub(super) struct FixtureLimits {
    pub hle: HleLimits,
    pub memory: SessionLoadLimits,
    pub instructions: u64,
}

impl Default for FixtureLimits {
    fn default() -> Self {
        Self {
            hle: HleLimits::default(),
            memory: SessionLoadLimits {
                resident_limit: 8 * PAGE_SIZE,
                mapped_limit: 10 * PAGE_SIZE,
            },
            instructions: MAX_INSTRUCTIONS,
        }
    }
}

pub(super) struct Launched {
    pub gateway: Gateway,
    pub bindings: usize,
    pub applied: usize,
    pub initial: ReferenceCpu,
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

fn guards() -> [u64; 4] {
    [
        DATA - PAGE_SIZE,
        DATA + DATA_SIZE,
        STACK - PAGE_SIZE,
        STACK + STACK_SIZE,
    ]
}

fn check_storage(memory: &GuestMemory) -> Result<(), HleError> {
    for address in guards() {
        require(
            matches!(
                memory.read(address, &mut [0]),
                Err(MemoryError::Unmapped(_))
            ),
            "storage guard page is mapped",
        )?;
    }
    Ok(())
}

/// Generate, plan, prepare and initialize one closed session for `consumer`.
pub(super) fn launch(consumer: &[u8], limits: &FixtureLimits) -> Result<Launched, HleError> {
    require(
        limits.instructions <= MAX_INSTRUCTIONS,
        "instruction policy maximum",
    )?;
    let providers = build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.hle)?;
    let inputs = [ModuleInput::new(
        open_executable(consumer).map_err(HleError::Load)?,
        CONSUMER_BIAS,
    )];
    let plan = plan_hle_session(
        &inputs,
        &providers,
        ProviderPlacement {
            load_bias: PROVIDER_BIAS,
        },
        &limits.hle,
    )?;
    let bindings = plan.provider_bindings();
    let prepared = prepare_hle_session(&plan, &limits.memory)?;
    let applied = prepared.applied_relocations();
    let mut gateway = Gateway::new(prepared, limits.instructions);
    let memory = &mut gateway.memory;
    check_storage(memory)?;
    // map rejects any collision with image pages; failures drop the session.
    memory.map(DATA, DATA_SIZE, Permissions::READ_WRITE)?;
    memory.map(STACK, STACK_SIZE, Permissions::READ_WRITE)?;
    check_storage(memory)?;
    let (pattern, compare) = initial_data();
    memory.write(DATA, &pattern)?;
    memory.write(DATA + COMPARE_AT, &compare)?;
    let initial_rsp = STACK + STACK_SIZE - 8;
    memory.write(initial_rsp, &STACK_SENTINEL.to_le_bytes())?;
    let cpu = &mut gateway.cpu;
    cpu.registers[3] = DATA;
    cpu.registers[4] = initial_rsp;
    for (register, value) in SENTINELS {
        cpu.registers[register] = value;
    }
    cpu.fs_base = FS_SENTINEL;
    let initial = cpu.clone();
    Ok(Launched {
        gateway,
        bindings,
        applied,
        initial,
    })
}

/// Construct, link, stage and interpret fresh original images.
///
/// This has no byte, path, plan or entry input. It returns only completed
/// diagnostics; the address space, CPU and gateway never escape, and any
/// failure drops the whole unpublished attempt.
///
/// # Errors
/// Returns the first typed generation, linking, staging, interpreter, service
/// or fixture-assertion failure.
pub fn run_hle_memory_fixture() -> Result<HleFixtureReport, HleError> {
    let program = program(&OPS)?;
    let consumer = program.consumer().build()?;
    let mut launched = launch(&consumer, &FixtureLimits::default())?;
    require(
        launched.bindings == KnownFunction::MEMORY.len()
            && launched.applied == KnownFunction::MEMORY.len(),
        "four relocated generated imports",
    )?;
    for site in launched
        .gateway
        .sites()
        .iter()
        .filter(|site| KnownFunction::MEMORY.contains(&site.function))
    {
        let slot = CONSUMER_BIAS + GOT + 8 * site.function.index() as u64;
        require(
            word(&launched.gateway.memory, slot)? == site.entry,
            "relocated GOT slot",
        )?;
    }
    let halt = launched.gateway.run()?;
    verify(&launched, &program, halt, &consumer)
}

fn verify(
    launched: &Launched,
    program: &Program,
    halt: u64,
    consumer: &[u8],
) -> Result<HleFixtureReport, HleError> {
    let Launched {
        gateway, initial, ..
    } = launched;
    let (cpu, memory, budget) = (&gateway.cpu, &gateway.memory, &gateway.budget);
    require(halt == CONSUMER_BIAS + program.halt, "final breakpoint")?;
    require(cpu.retired == program.retired, "shared retired count")?;
    for register in [3, 4, 5, 12, 13, 14, 15] {
        require(
            cpu.registers[register] == initial.registers[register],
            "callee-saved registers and RSP",
        )?;
    }
    require(cpu.fs_base == initial.fs_base, "modeled FS base")?;
    require(
        word(memory, initial.registers[4])? == STACK_SENTINEL,
        "stack sentinel",
    )?;
    let mut data = vec![0; DATA_SIZE as usize];
    memory.read(DATA, &mut data)?;
    require(data == expected_data(&OPS), "final guest data")?;
    for site in gateway.sites() {
        let mut staged = [0; 8];
        memory.fetch(site.entry, &mut staged)?;
        require(staged == stub(site.function), "provider stub changed")?;
    }
    check_storage(memory)?;
    for address in [DATA, STACK, CONSUMER_BIAS + GOT] {
        require(
            matches!(
                memory.fetch(address, &mut [0]),
                Err(MemoryError::PermissionDenied(_))
            ),
            "data, stack and GOT must be NX",
        )?;
    }
    let expected_bytes = OPS.iter().map(|op| op.rdx).sum::<u64>();
    let expected_work = OPS
        .iter()
        .map(|op| work(op.function, op.rdx).map(|units| units + SITE_LOOKUP_WORK))
        .sum::<Option<u64>>();
    let count =
        |function: KnownFunction| OPS.iter().filter(|op| op.function == function).count() as u64;
    require(
        budget.attempts() == OPS.len() as u64
            && budget.completed() == OPS.len() as u64
            && KnownFunction::ALL
                .iter()
                .all(|&function| budget.calls(function) == count(function))
            && budget.total_bytes() == expected_bytes
            && budget.string_reads() == 0
            && budget.string_writes() == 0
            && budget.string_scratch_peak() == 0
            && Some(budget.work()) == expected_work,
        "service accounting",
    )?;
    require(
        program.consumer().build()? == consumer,
        "consumer source bytes",
    )?;
    let result = |index: usize| -> Result<i32, HleError> {
        Ok(word(memory, DATA + RESULTS + 8 * index as u64)? as u32 as i32)
    };
    Ok(HleFixtureReport {
        memcpy_calls: budget.calls(KnownFunction::Memcpy),
        memmove_calls: budget.calls(KnownFunction::Memmove),
        memset_calls: budget.calls(KnownFunction::Memset),
        memcmp_calls: budget.calls(KnownFunction::Memcmp),
        service_attempts: budget.attempts(),
        requested_bytes: budget.total_bytes(),
        work_units: budget.work(),
        retired: cpu.retired,
        provider_bindings: launched.bindings,
        applied_relocations: launched.applied,
        provider_bias: PROVIDER_BIAS,
        memcmp_results: [result(5)?, result(6)?, result(7)?],
        mapped_bytes: memory.mapped_bytes(),
        resident_bytes: memory.resident_bytes(),
    })
}
