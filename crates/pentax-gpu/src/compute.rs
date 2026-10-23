// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded compute packet/resource preparation; see NOTICE.

//! Flat compute preparation and bounded stateful command plans without GPU writes.
//! Register fields follow the pinned Kyty compute paths, not inferred reset state.
pub use crate::marker::{Marker, MarkerKind};

use crate::Header;
pub use crate::memory_sync::{Synchronization, SynchronizationKind};
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{
    BufferAccess, BufferDescription, CommonDescription, CompiledShader, ComputeDescription,
    Limits as CompilerLimits, Profile, ShaderProgram, StageDescription, SubgroupProfile, WaveSize,
    compile,
};
use std::fmt;

mod commands;
mod ordered;
pub use commands::{
    Command, CommandLimits, CommandPlan, CommandRange, CommandUsage, Dispatch, MemoryAction,
    MemoryWait, MemoryWrite, Release, plan_commands,
};
pub(crate) use commands::{memory_packet, tagged_wait_dwords};
pub use ordered::{OrderedCommands, OrderedStep};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryError {
    Unavailable,
    ReadDenied,
    WriteDenied,
}
/// Reads from one actual exclusively retained source; this trait exposes no pointers.
pub trait Memory {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError>;
    /// Preserve initial buffer contents while admitting compiler-derived GPU
    /// access. Native adapters must enforce both read and write requirements.
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        _read: bool,
        _written: bool,
    ) -> Result<(), MemoryError> {
        self.read(address, output)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Limit(&'static str),
    Allocation,
    Memory(MemoryError),
    Packet { dword: usize, reason: &'static str },
    Unsupported(&'static str),
    MissingRegister(u32),
    Compile,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "compute submission: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_command_bytes: usize,
    pub max_shader_words: usize,
    pub max_spirv_words: usize,
    pub max_buffer_bytes: usize,
    pub max_invocations: u64,
    pub max_work: u64,
    pub max_host_bytes: usize,
    pub compiler_phase_bytes: usize,
    pub compiler_phase_work: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_command_bytes: 64 * 1024,
            max_shader_words: 256,
            max_spirv_words: 32 * 1024,
            max_buffer_bytes: 16 * 1024 * 1024,
            max_invocations: 1024 * 1024,
            max_work: 64 * 1024 * 1024,
            max_host_bytes: 64 * 1024 * 1024,
            compiler_phase_bytes: 8 * 1024 * 1024,
            compiler_phase_work: 4 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub packets: u64,
    pub work: u64,
    pub host_bytes: usize,
    pub shader_words: usize,
    pub invocations: u64,
}

/// Owned compiler product, immutable upload and manifest-derived push data.
/// This is data, not an authority to access a live guest or arbitrary SPIR-V.
pub struct PreparedCompute {
    shader: CompiledShader,
    input: Vec<u8>,
    push: [u8; 128],
    groups: [u32; 3],
    address: u64,
    usage: Usage,
}
impl fmt::Debug for PreparedCompute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedCompute")
            .field("buffer_bytes", &self.input.len())
            .field("groups", &self.groups)
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}
impl PreparedCompute {
    pub fn shader(&self) -> &CompiledShader {
        &self.shader
    }
    pub fn input(&self) -> &[u8] {
        &self.input
    }
    pub fn push_data(&self) -> &[u8; 128] {
        &self.push
    }
    pub fn groups(&self) -> [u32; 3] {
        self.groups
    }
    pub fn buffer_address(&self) -> u64 {
        self.address
    }
    pub fn usage(&self) -> Usage {
        self.usage
    }
    pub fn writes_buffer(&self) -> bool {
        self.shader.manifest().buffers()[0].written
    }
}

struct Budget<'a> {
    limits: &'a Limits,
    usage: Usage,
}
impl Budget<'_> {
    fn work(&mut self, count: u64) -> Result<(), Error> {
        self.usage.work = self
            .usage
            .work
            .checked_add(count)
            .filter(|n| *n <= self.limits.max_work)
            .ok_or(Error::Limit("work"))?;
        Ok(())
    }
    fn host(&mut self, bytes: usize) -> Result<(), Error> {
        self.usage.host_bytes = self
            .usage
            .host_bytes
            .checked_add(bytes)
            .filter(|n| *n <= self.limits.max_host_bytes)
            .ok_or(Error::Limit("host bytes"))?;
        Ok(())
    }
}
/// Known compute registers from one admitted queue history. Unknown reset values
/// remain unknown. Fields are private and Debug never exposes register contents.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ComputeState {
    values: [Option<u32>; 27],
    indirect_base: Option<u64>,
}
impl Default for ComputeState {
    fn default() -> Self {
        Self::empty()
    }
}
impl fmt::Debug for ComputeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ComputeState")
            .field(
                "known_registers",
                &self.values.iter().filter(|r| r.is_some()).count(),
            )
            .finish()
    }
}
impl ComputeState {
    pub const fn empty() -> Self {
        Self {
            values: [None; 27],
            indirect_base: None,
        }
    }
    // Only defaults with an exact public field/register correspondence are
    // restored. Packed raw resource controls and START registers remain unknown.
    fn reset(&mut self) {
        *self = Self::empty();
        for index in [
            3, 4, 5, 6, 7, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
        ] {
            self.values[index] = Some(0);
        }
        self.indirect_base = Some(0);
    }
    fn index(reg: u32) -> Option<usize> {
        Some(match reg {
            0x204..=0x209 => (reg - 0x204) as usize,
            0x20c..=0x20d => 6 + (reg - 0x20c) as usize,
            0x212..=0x213 => 8 + (reg - 0x212) as usize,
            0x228 => 10,
            0x240..=0x24f => 11 + (reg - 0x240) as usize,
            _ => return None,
        })
    }
    fn get(&self, reg: u32) -> Result<u32, Error> {
        Self::index(reg)
            .and_then(|i| self.values[i])
            .ok_or(Error::MissingRegister(reg))
    }
    fn write(&mut self, reg: u32, value: u32) -> Result<(), Error> {
        let index = Self::index(reg).ok_or(Error::Unsupported("compute SH register"))?;
        self.values[index] = Some(value);
        Ok(())
    }
}

fn packets(bytes: &[u8], budget: &mut Budget<'_>) -> Result<(ComputeState, [u32; 3], u32), Error> {
    if !bytes.len().is_multiple_of(4) {
        return Err(Error::Unsupported("command byte alignment"));
    }
    if bytes.len() > budget.limits.max_command_bytes {
        return Err(Error::Limit("command bytes"));
    }
    budget.work(bytes.len() as u64)?;
    let word =
        |index: usize| u32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap());
    let mut regs = ComputeState::empty();
    let mut dispatch = None;
    let mut at = 0;
    while at < bytes.len() / 4 {
        let header = Header::new(word(at));
        let fail = |reason| Error::Packet { dword: at, reason };
        let length = if header.raw() == 0x8000_0000 {
            1
        } else if crate::queue_reset::is_header(header.raw()) {
            2
        } else if header.packet_type() == 3 && matches!(header.low_byte(), 0 | 2) {
            if header.opcode() == 0x10 && header.count() == 0x3fff {
                1
            } else {
                usize::from(header.count()) + 2
            }
        } else {
            return Err(fail("packet header profile"));
        };
        let end = at
            .checked_add(length)
            .filter(|n| *n <= bytes.len() / 4)
            .ok_or(fail("truncated packet"))?;
        if crate::queue_reset::is_header(header.raw()) {
            let reset = crate::queue_reset::decode(header.raw(), word(at + 1)).map_err(fail)?;
            budget.work(std::mem::size_of::<ComputeState>() as u64)?;
            if reset == crate::queue_reset::Reset::Processor {
                // This API returns one dispatch state, not a queue-history
                // product. A later reset cannot retroactively replace it.
                if dispatch.is_some() {
                    return Err(fail("queue reset after single dispatch"));
                }
                regs.reset();
            }
        } else if length == 1 || header.opcode() == 0x10 { /* Hardware NOP payload remains opaque. */
        } else if header.opcode() == 0x76
            && matches!(header.low_byte(), 0 | 2)
            && length >= 3
            && dispatch.is_none()
        {
            let first = word(at + 1);
            if first > 0xffff {
                return Err(fail("register selector"));
            }
            for index in 0..length - 2 {
                regs.write(first + index as u32, word(at + 2 + index))?;
            }
        } else if header.opcode() == 0x15
            && matches!(header.low_byte(), 0 | 2)
            && length == 5
            && dispatch.is_none()
        {
            dispatch = Some(([word(at + 1), word(at + 2), word(at + 3)], word(at + 4)));
        } else {
            return Err(fail("command outside single flat compute profile"));
        }
        budget.usage.packets += 1;
        at = end;
    }
    let (groups, mode) = dispatch.ok_or(Error::Unsupported("missing compute dispatch"))?;
    Ok((regs, groups, mode))
}

fn shader(
    memory: &impl Memory,
    address: u64,
    budget: &mut Budget<'_>,
    limits: &CompilerLimits,
) -> Result<ShaderProgram, Error> {
    let count = budget.limits.max_shader_words;
    if count == 0 {
        return Err(Error::Limit("shader words"));
    }
    // Candidate termination is always validated by the real instruction walker.
    // A literal equal to ENDPGM is never treated as a delimiter on its own.
    budget.work(
        (count as u64)
            .checked_mul(count as u64)
            .and_then(|n| n.checked_mul(64))
            .ok_or(Error::Limit("shader walk work"))?,
    )?;
    let mut words = Vec::new();
    let reserved = count.checked_mul(4).ok_or(Error::Limit("shader words"))?;
    budget.host(reserved)?;
    words
        .try_reserve_exact(count)
        .map_err(|_| Error::Allocation)?;
    budget.host(
        words
            .capacity()
            .checked_mul(4)
            .ok_or(Error::Limit("shader words"))?
            - reserved,
    )?;
    for index in 0..count {
        let at = address
            .checked_add(index as u64 * 4)
            .filter(|n| *n < 1 << 47)
            .ok_or(Error::Unsupported("shader address"))?;
        let mut bytes = [0; 4];
        memory.read(at, &mut bytes).map_err(Error::Memory)?;
        words.push(u32::from_le_bytes(bytes));
        if words[index] == 0xbf81_0000
            && let Ok(program) = ShaderProgram::decode(&words, ShaderStage::Compute, limits)
        {
            for inst in program.instructions() {
                if inst
                    .branch()
                    .is_some_and(|b| b.target_byte_offset() <= i64::from(inst.byte_offset()))
                {
                    return Err(Error::Unsupported("unbounded compute shader loop"));
                }
            }
            budget.usage.shader_words = words.len();
            return Ok(program);
        }
    }
    Err(Error::Limit("terminated shader words"))
}

pub fn prepare(
    bytes: &[u8],
    memory: &impl Memory,
    limits: &Limits,
) -> Result<PreparedCompute, Error> {
    let mut budget = Budget {
        limits,
        usage: Usage::default(),
    };
    budget.host(bytes.len())?;
    let (regs, groups, mode) = packets(bytes, &mut budget)?;
    prepare_state(regs, groups, mode, memory, budget)
}

/// Compile a private register snapshot derived by complete command admission.
/// Resource reads happen now, so an earlier completed dispatch's output can be
/// consumed by the next dispatch on the same exclusively paused owner.
pub fn prepare_dispatch(
    dispatch: &Dispatch,
    memory: &impl Memory,
    limits: &Limits,
) -> Result<PreparedDispatch, Error> {
    let mut budget = Budget {
        limits,
        usage: Usage::default(),
    };
    if !matches!(dispatch.mode, 0x8001 | 0x8005 | 0x8041) {
        return Err(Error::Unsupported("wave32 dispatch initiator"));
    }
    let mut groups = dispatch.groups;
    if let Some(address) = dispatch.indirect {
        budget.work(12)?;
        let mut bytes = [0; 12];
        memory.read(address, &mut bytes).map_err(Error::Memory)?;
        for (i, word) in bytes.chunks_exact(4).enumerate() {
            groups[i] = u32::from_le_bytes(word.try_into().unwrap());
        }
    }
    if groups.contains(&0) {
        return Ok(PreparedDispatch {
            product: None,
            usage: budget.usage,
        });
    }
    prepare_state(dispatch.state, groups, dispatch.mode, memory, budget).map(|product| {
        PreparedDispatch {
            usage: product.usage(),
            product: Some(product),
        }
    })
}

#[derive(Debug)]
pub struct PreparedDispatch {
    product: Option<PreparedCompute>,
    usage: Usage,
}
impl PreparedDispatch {
    pub fn into_parts(self) -> (Option<PreparedCompute>, Usage) {
        (self.product, self.usage)
    }
}

fn prepare_state(
    regs: ComputeState,
    groups: [u32; 3],
    mode: u32,
    memory: &impl Memory,
    mut budget: Budget<'_>,
) -> Result<PreparedCompute, Error> {
    let limits = budget.limits;
    if !matches!(mode, 0x8001 | 0x8005 | 0x8041) {
        return Err(Error::Unsupported("wave32 direct dispatch initiator"));
    }
    if mode & 4 == 0 {
        for reg in 0x204..=0x206 {
            if regs.get(reg)? != 0 {
                return Err(Error::Unsupported("compute start coordinates"));
            }
        }
    }
    let local = [regs.get(0x207)?, regs.get(0x208)?, regs.get(0x209)?];
    let mut invocations = 1_u64;
    for value in groups.into_iter().chain(local) {
        invocations = invocations
            .checked_mul(u64::from(value))
            .filter(|n| *n != 0 && *n <= limits.max_invocations)
            .ok_or(Error::Limit("dispatch invocations"))?;
    }
    budget.usage.invocations = invocations;
    let r1 = regs.get(0x212)?;
    let r2 = regs.get(0x213)?;
    if r1 & !0x3f != 0x2c_0000 || r2 & !0x1fbe != 0 || regs.get(0x228)? != 0 {
        return Err(Error::Unsupported("compute resource configuration"));
    }
    let user_count = ((r2 >> 1) & 31) as u8;
    let tid = ((r2 >> 11) & 3) as u8;
    if !(4..=16).contains(&user_count) || tid > 2 {
        return Err(Error::Unsupported("compute initial registers"));
    }
    let lo = regs.get(0x20c)?;
    let hi = regs.get(0x20d)?;
    if hi > 255 {
        return Err(Error::Unsupported("shader address high bits"));
    }
    let code_address = (u64::from(lo) << 8) | (u64::from(hi) << 40);
    let descriptor = [
        regs.get(0x240)?,
        regs.get(0x241)?,
        regs.get(0x242)?,
        regs.get(0x243)?,
    ];
    if descriptor[1] & 0xffff_0000 != 0 || descriptor[3] != 0 {
        return Err(Error::Unsupported("raw unswizzled buffer descriptor"));
    }
    let address = u64::from(descriptor[0]) | (u64::from(descriptor[1]) << 32);
    let length = descriptor[2] as usize;
    if length == 0
        || !length.is_multiple_of(4)
        || !address.is_multiple_of(4)
        || address
            .checked_add(length as u64)
            .is_none_or(|n| n > 1 << 47)
    {
        return Err(Error::Unsupported("buffer extent"));
    }
    if length > limits.max_buffer_bytes {
        return Err(Error::Limit("buffer bytes"));
    }
    if limits.compiler_phase_bytes == 0 || limits.compiler_phase_work == 0 {
        return Err(Error::Limit("compiler budget"));
    }
    // Decoder records, retained program tables and compiler arenas have
    // independent caps. Decoder source words are separate from record bytes.
    budget.host(
        limits
            .max_shader_words
            .checked_mul(4)
            .ok_or(Error::Limit("shader words"))?,
    )?;
    budget.host(
        limits
            .compiler_phase_bytes
            .checked_mul(3)
            .ok_or(Error::Limit("compiler bytes"))?,
    )?;
    budget.work(
        limits
            .compiler_phase_work
            .checked_mul(3)
            .ok_or(Error::Limit("compiler work"))?,
    )?;
    let compiler = CompilerLimits {
        decode: pentax_shader::Limits {
            max_source_words: limits.max_shader_words,
            max_instructions: limits.max_shader_words,
            max_record_bytes: limits.compiler_phase_bytes,
            max_work: usize::try_from(limits.compiler_phase_work)
                .map_err(|_| Error::Limit("decode work"))?,
        },
        max_bytes: limits.compiler_phase_bytes,
        max_work: limits.compiler_phase_work,
        max_spirv_words: limits.max_spirv_words,
        ..Default::default()
    };
    let program = shader(memory, code_address, &mut budget, &compiler)?;
    budget.work(
        invocations
            .checked_mul(program.instruction_count() as u64)
            .ok_or(Error::Limit("shader execution work"))?,
    )?;
    let mut common = CommonDescription::new(WaveSize::Wave32);
    common.subgroup = Some(SubgroupProfile { size: 32 });
    common.user_sgpr_count = user_count;
    budget.host(std::mem::size_of::<BufferDescription>())?;
    common
        .buffers
        .try_reserve_exact(1)
        .map_err(|_| Error::Allocation)?;
    budget.host((common.buffers.capacity() - 1) * std::mem::size_of::<BufferDescription>())?;
    common.buffers.push(BufferDescription {
        user_sgpr: 0,
        stride: 0,
        swizzle: false,
        index_stride: 0,
        access: BufferAccess::ReadWrite,
    });
    let profile = Profile::new(StageDescription::Compute(ComputeDescription {
        common,
        workgroup_size: local,
        thread_ids: tid + 1,
        group_id: [r2 & 0x80 != 0, r2 & 0x100 != 0, r2 & 0x200 != 0],
        workgroup_register: user_count,
        tg_size: r2 & 0x400 != 0,
        float_mode: 0xc0,
    }))
    .map_err(|_| Error::Compile)?;
    let compiled = compile(&program, &profile, &compiler).map_err(|_| Error::Compile)?;
    let manifest = compiled.manifest();
    if manifest.buffers().len() != 1
        || manifest.buffers()[0].user_sgpr != 0
        || manifest.buffers()[0].coherent
        || manifest.shader_data_binding().is_some()
        || manifest.undefined_register_reads() != 0
    {
        return Err(Error::Unsupported("compute resource manifest"));
    }
    let mut push = [0; 128];
    if let Some(layout) = manifest.push_data() {
        for (index, reg) in layout.user_sgprs.iter().enumerate() {
            let value = regs.get(0x240 + u32::from(*reg))?;
            let offset = (layout.start_dword as usize)
                .checked_add(index)
                .and_then(|n| n.checked_mul(4))
                .filter(|n| *n <= 124)
                .ok_or(Error::Unsupported("compute push layout"))?;
            push[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        // The single storage binding is uploaded from its exact guest base;
        // its packed buffer-byte-offset remains explicitly zero.
        if layout.memory_offset_count != 1 {
            return Err(Error::Unsupported("compute buffer offset layout"));
        }
    }
    budget.host(length)?;
    let mut input = Vec::new();
    input
        .try_reserve_exact(length)
        .map_err(|_| Error::Allocation)?;
    budget.host(input.capacity() - length)?;
    budget.work(length as u64)?;
    input.resize(length, 0);
    memory
        .snapshot_buffer(
            address,
            &mut input,
            manifest.buffers()[0].read,
            manifest.buffers()[0].written,
        )
        .map_err(Error::Memory)?;
    Ok(PreparedCompute {
        shader: compiled,
        input,
        push,
        groups,
        address,
        usage: budget.usage,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "compute/reset_tests.rs"]
mod reset_tests;

#[cfg(test)]
#[path = "compute/sync_tests.rs"]
mod sync_tests;

#[cfg(test)]
#[path = "compute/marker_tests.rs"]
mod marker_tests;
